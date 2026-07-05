use crate::domain::{
    FolderUsage, ScanChunk, ScanChunkData, ScanComplete,
    ScanError as DomainScanError, ScanProgress, ScanTreeChild, ScanTreeChildren, ScanTreeStarted,
};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};

/// Orchestrates disk usage scans.
///
/// Three-phase approach:
/// - Phase 1: BFS discovers all folders (readdir only, no file stats)
/// - Phase 2: Leaf folders sized in parallel (10 threads)
/// - Phase 3: Rollup propagates totals bottom-up
///
/// Pull-based tree: emits `ScanTreeStarted` at start, `ScanTreeChildren` as
/// children are discovered. Frontend calls `get_scan_tree_children` on expand.
pub struct DiskUsageUseCase;

/// Inter-thread orchestration enum.
enum ScanStep {
    /// Scan started — emit root info to frontend.
    Started(ScanTreeStarted),
    /// Children discovered for a folder — frontend can now render them.
    ChildrenReady(ScanTreeChildren),
    /// One folder sized (leaf or rolled-up parent).
    Folder(FolderUsage),
    /// Scan finished.
    Complete,
    /// Scan was cancelled.
    Cancelled,
}

/// Folders discovered in a single BFS batch.
struct DiscoveredFolder {
    path: String,
    name: String,
}

impl DiskUsageUseCase {
    /// BFS streaming scan.
    ///
    /// Pull-based tree: emits `ScanTreeStarted` at start, `ScanTreeChildren`
    /// as children are discovered. Frontend calls `get_scan_tree_children` on expand.
    /// Leaf folders are sized in parallel (10 threads) as they're discovered.
    pub fn run(
        window: tauri::WebviewWindow,
        path: String,
        _max_depth: u32, // deprecated — we scan everything now
        cancel: Arc<AtomicBool>,
        start: Instant,
        scan_id: String,
        tree_state: Arc<Mutex<HashMap<String, HashMap<String, Vec<ScanTreeChild>>>>>,
    ) -> impl std::future::Future<Output = ()> {
        let (done_tx, done_rx) = oneshot::channel();
        let (tx, mut rx) = mpsc::unbounded_channel::<ScanStep>();

        let path_walk = path.clone();
        let scan_id_blocking = scan_id.clone();
        let cancel_blocking = cancel.clone();
        let tree_state_blocking = tree_state.clone();

        tokio::task::spawn_blocking(move || {
            let base = Path::new(&path_walk);

            // ─── Shared state ───

            // Track children per parent for rollup + tree lookup
            let parent_children: Arc<Mutex<HashMap<String, Vec<String>>>> =
                Arc::new(Mutex::new(HashMap::new()));

            // Leaf sizing results for rollup
            let leaf_results: Arc<Mutex<HashMap<String, FolderUsage>>> =
                Arc::new(Mutex::new(HashMap::new()));

            // Track which folders we've already emitted ChildrenReady for
            let emitted_children: Arc<Mutex<std::collections::HashSet<String>>> =
                Arc::new(Mutex::new(std::collections::HashSet::new()));

            // ─── Seed the root ───

            let root_name = base
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path_walk.clone());
            let root_path = normalize_path(base);

            // BFS queue
            let mut queue: VecDeque<String> = VecDeque::new();
            queue.push_back(root_path.clone());

            // ─── Sizing work queue (10 threads) ───

            let (work_tx, work_rx) = std::sync::mpsc::channel::<String>();
            let work_rx = Arc::new(std::sync::Mutex::new(work_rx));

            let tx_clone = tx.clone();
            let leaf_results_clone = leaf_results.clone();
            let cancel_size = cancel_blocking.clone();

            let mut sizing_handles = Vec::new();
            for _ in 0..10 {
                let rx = work_rx.clone();
                let tx_t = tx_clone.clone();
                let results = leaf_results_clone.clone();
                let cancel_t = cancel_size.clone();

                let handle = std::thread::spawn(move || {
                    loop {
                        if cancel_t.load(Ordering::Relaxed) {
                            break;
                        }

                        let folder_path = {
                            let lock = rx.lock().unwrap();
                            match lock.recv() {
                                Ok(p) => p,
                                Err(_) => break,
                            }
                        };

                        if cancel_t.load(Ordering::Relaxed) {
                            break;
                        }

                        let result = size_folder(Path::new(&folder_path));

                        // Store for rollup
                        results.lock().unwrap().insert(result.path.clone(), result.clone());

                        // Emit to frontend
                        let _ = tx_t.send(ScanStep::Folder(result));
                    }
                });

                sizing_handles.push(handle);
            }

            // ─── BFS Discovery Loop ───

            // Emit Started so frontend knows the root
            let _ = tx.send(ScanStep::Started(ScanTreeStarted {
                scan_id: scan_id_blocking.clone(),
                root_path: root_path.clone(),
                root_name: root_name.clone(),
            }));

            const BATCH_SIZE: usize = 200;

            // Track which folders had children discovered in this batch
            let mut batch_parent_children: Vec<(String, Vec<ScanTreeChild>)> = Vec::new();

            loop {
                if cancel_blocking.load(Ordering::Relaxed) {
                    break;
                }

                let mut batch_leaves: Vec<String> = Vec::new();
                batch_parent_children.clear();

                // Process a batch from the BFS queue
                for _ in 0..BATCH_SIZE {
                    let current_path = match queue.pop_front() {
                        Some(p) => p,
                        None => break,
                    };

                    let children =
                        readdir_children(Path::new(&current_path), &cancel_blocking);

                    if children.is_empty() {
                        // Leaf — queue for sizing
                        batch_leaves.push(current_path);
                    } else {
                        // Record parent→children relationship for rollup
                        let child_paths: Vec<String> = children.iter().map(|c| c.path.clone()).collect();
                        {
                            let mut pc = parent_children.lock().unwrap();
                            pc.insert(current_path.clone(), child_paths);
                        }

                        // Track for ChildrenReady emission
                        let child_infos: Vec<ScanTreeChild> = children.into_iter().map(|c| ScanTreeChild {
                            path: c.path,
                            name: c.name,
                        }).collect();

                        batch_parent_children.push((current_path, child_infos));

                        // Add children to queue
                        for child in &batch_parent_children.last().unwrap().1 {
                            queue.push_back(child.path.clone());
                        }
                    }
                }

                // If nothing processed and queue is empty, we're done discovering
                if batch_parent_children.is_empty() && batch_leaves.is_empty() && queue.is_empty() {
                    break;
                }

                // Emit ChildrenReady for folders whose children were discovered
                for (parent, children) in &batch_parent_children {
                    // Store in tree state for get_scan_tree_children command
                    {
                        let mut tree = tree_state_blocking.lock().unwrap();
                        tree.entry(scan_id_blocking.clone())
                            .or_insert_with(HashMap::new)
                            .insert(parent.clone(), children.clone());
                    }

                    let _ = tx.send(ScanStep::ChildrenReady(ScanTreeChildren {
                        scan_id: scan_id_blocking.clone(),
                        parent_path: parent.clone(),
                        children: children.clone(),
                    }));

                    emitted_children.lock().unwrap().insert(parent.clone());
                }

                // Queue leaves for sizing
                for leaf in batch_leaves {
                    if cancel_blocking.load(Ordering::Relaxed) {
                        break;
                    }
                    let _ = work_tx.send(leaf);
                }
            }

            // Close sizing work queue — threads will drain and exit
            drop(work_tx);

            // Wait for all sizing threads
            for handle in sizing_handles {
                let _ = handle.join();
            }

            if cancel_blocking.load(Ordering::Relaxed) {
                let _ = tx.send(ScanStep::Cancelled);
                return;
            }

            // ─── Rollup ───

            let pc = parent_children.lock().unwrap().clone();

            // Collect all folder paths (parents + children) for bottom-up rollup
            let mut all_paths: Vec<String> = Vec::new();
            for (parent, children) in &pc {
                all_paths.push(parent.clone());
                all_paths.extend(children.clone());
            }

            let mut stats = leaf_results.lock().unwrap().clone();

            // Roll up: process folders bottom-up (children before parents)
            // Sort by path depth descending as an approximation
            all_paths.sort_by_key(|a| a.chars().filter(|c| *c == '/').count());
            all_paths.reverse();

            for folder_path in &all_paths {
                let children: Vec<String> = pc.get(folder_path).cloned().unwrap_or_default();
                if children.is_empty() {
                    continue; // Leaf — already sized
                }

                let mut total_size: u64 = 0;
                let mut total_files: u64 = 0;
                let mut total_folders: u64 = 0;

                for child_path in &children {
                    if let Some(child_stats) = stats.get(child_path) {
                        total_size += child_stats.size;
                        total_files += child_stats.file_count;
                        total_folders += 1 + child_stats.folder_count;
                    }
                }

                stats.insert(
                    folder_path.clone(),
                    FolderUsage {
                        path: folder_path.clone(),
                        size: total_size,
                        file_count: total_files,
                        folder_count: total_folders,
                    },
                );
            }

            // Emit rolled-up stats for non-leaf folders
            for (folder_path, _children) in &pc {
                if let Some(usage) = stats.get(folder_path) {
                    let _ = tx.send(ScanStep::Folder(usage.clone()));
                }
            }

            let _ = tx.send(ScanStep::Complete);
        });

        let start_clone = start.clone();
        let scan_id_clone = scan_id.clone();
        let cancel_clone = cancel.clone();

        // Async emitter — drains the channel and emits Tauri events
        tokio::spawn(async move {
            let mut total_files: u64 = 0;
            let mut total_size: u64 = 0;
            let mut folder_count: usize = 0;

            while let Some(step) = rx.recv().await {
                if cancel_clone.load(Ordering::Relaxed) {
                    emit_cancelled(&window, &scan_id_clone);
                    break;
                }

                match step {
                    ScanStep::Started(info) => {
                        let _ = window.emit("scan:tree_started", &info);
                    }
                    ScanStep::ChildrenReady(children) => {
                        let _ = window.emit("scan:children_ready", &children);
                    }
                    ScanStep::Folder(usage) => {
                        total_files += usage.file_count;
                        total_size += usage.size;
                        folder_count += 1;

                        let _ = window.emit(
                            "scan:chunk",
                            ScanChunk {
                                scan_id: scan_id_clone.clone(),
                                data: ScanChunkData::FolderUsage { usage },
                            },
                        );
                    }
                    ScanStep::Complete => {
                        let _ = window.emit(
                            "scan:progress",
                            ScanProgress {
                                scan_id: scan_id_clone.clone(),
                                percentage: 100.0,
                                message: format!(
                                    "Scanned {} files across {} folders",
                                    total_files, folder_count
                                ),
                            },
                        );

                        let duration = start_clone.elapsed().as_millis() as u64;
                        let _ = window.emit(
                            "scan:complete",
                            ScanComplete {
                                scan_id: scan_id_clone.clone(),
                                total_items: total_files,
                                total_size,
                                duration_ms: duration,
                            },
                        );
                    }
                    ScanStep::Cancelled => {
                        emit_cancelled(&window, &scan_id_clone);
                        break;
                    }
                }
            }

            let _ = done_tx.send(());
        });

        async { let _ = done_rx.await; }
    }
}

/// Size a single folder by walking all its descendants.
fn size_folder(path: &Path) -> FolderUsage {
    let mut size: u64 = 0;
    let mut file_count: u64 = 0;
    let mut folder_count: u64 = 0;

    let walkdir = walkdir::WalkDir::new(path);
    for entry in walkdir {
        match entry {
            Ok(e) => {
                let meta = match e.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if meta.is_file() {
                    size += meta.len();
                    file_count += 1;
                } else if meta.is_dir() && e.path() != path {
                    folder_count += 1;
                }
            }
            Err(_) => continue,
        }
    }

    FolderUsage {
        path: normalize_path(path),
        size,
        file_count,
        folder_count,
    }
}

/// Read immediate subdirectories of a path.
/// Returns list of discovered child folders.
fn readdir_children(
    dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Vec<DiscoveredFolder> {
    let mut children = Vec::new();

    match dir.read_dir() {
        Ok(entries) => {
            for entry in entries.flatten() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                let path = entry.path();
                if path.is_dir() {
                    let child_path = normalize_path(&path);
                    let child_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();

                    children.push(DiscoveredFolder {
                        path: child_path,
                        name: child_name,
                    });
                }
            }
        }
        Err(_) => {} // Skip inaccessible directories
    }

    children
}

/// Normalize a path to forward slashes for consistent keys.
fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().to_string().replace('\\', "/")
}

fn emit_cancelled(window: &tauri::WebviewWindow, scan_id: &str) {
    let _ = window.emit(
        "scan:error",
        DomainScanError {
            scan_id: scan_id.to_string(),
            message: "Scan cancelled".into(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_tree() -> TempDir {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        fs::create_dir_all(base.join("a/b/c")).unwrap();
        fs::create_dir_all(base.join("d")).unwrap();

        fs::write(base.join("a/file1.txt"), &[0u8; 100]).unwrap();
        fs::write(base.join("a/b/file2.txt"), &[0u8; 200]).unwrap();
        fs::write(base.join("d/file3.txt"), &[0u8; 50]).unwrap();

        temp
    }

    // ─── readdir_children ───

    #[test]
    fn reads_immediate_subdirectories() {
        let temp = create_test_tree();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_children(base, &cancel);

        assert_eq!(children.len(), 2);
        let names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"d"));
    }

    #[test]
    fn nested_directory_has_children() {
        let temp = create_test_tree();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_children(&base.join("a"), &cancel);

        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "b");
    }

    #[test]
    fn leaf_directory_has_no_children() {
        let temp = create_test_tree();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_children(&base.join("a/b/c"), &cancel);

        assert!(children.is_empty());
    }

    #[test]
    fn empty_directory_has_no_children() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_children(base, &cancel);

        assert!(children.is_empty());
    }

    // ─── size_folder ───

    #[test]
    fn sizes_leaf_folder() {
        let temp = create_test_tree();
        let base = temp.path();

        let usage = size_folder(&base.join("d"));
        assert_eq!(usage.size, 50);
        assert_eq!(usage.file_count, 1);
        assert_eq!(usage.folder_count, 0);
    }

    #[test]
    fn sizes_empty_folder() {
        let temp = create_test_tree();
        let base = temp.path();

        let usage = size_folder(&base.join("a/b/c"));
        assert_eq!(usage.size, 0);
        assert_eq!(usage.file_count, 0);
        assert_eq!(usage.folder_count, 0);
    }

    #[test]
    fn sizes_folder_with_subdirs() {
        let temp = create_test_tree();
        let base = temp.path();

        let usage = size_folder(&base.join("a"));
        assert_eq!(usage.size, 300); // 100 + 200
        assert_eq!(usage.file_count, 2);
        assert_eq!(usage.folder_count, 2); // b, b/c
    }
}