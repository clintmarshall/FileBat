use crate::domain::{
    FolderStructure, FolderUsage, ScanChunk, ScanChunkData, ScanComplete,
    ScanError as DomainScanError, ScanProgress, ScanStructure,
};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};

/// Orchestrates disk usage scans.
///
/// BFS streaming approach:
/// - Discover folders in batches (readdir only, no file stats)
/// - After each batch, emit accumulated structure → frontend renders growing tree
/// - Size leaf folders in parallel (10 threads) as they're discovered
/// - Stream chunk events as sizing completes
/// - After all discovery, roll up parent totals from children
pub struct DiskUsageUseCase;

/// Inter-thread orchestration enum.
enum ScanStep {
    /// Structure data — may be emitted multiple times with growing data.
    Structure(ScanStructure),
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
    /// Discovers folders breadth-first in batches, emitting structure to the
    /// frontend after each batch so the tree renders immediately and grows.
    /// Leaf folders are sized in parallel (10 threads) as they're discovered.
    pub fn run(
        window: tauri::WebviewWindow,
        path: String,
        _max_depth: u32, // deprecated — we scan everything now
        cancel: Arc<AtomicBool>,
        start: Instant,
        scan_id: String,
    ) -> impl std::future::Future<Output = ()> {
        let (done_tx, done_rx) = oneshot::channel();
        let (tx, mut rx) = mpsc::unbounded_channel::<ScanStep>();

        let path_walk = path.clone();
        let scan_id_blocking = scan_id.clone();
        let cancel_blocking = cancel.clone();

        tokio::task::spawn_blocking(move || {
            let base = Path::new(&path_walk);

            // ─── Shared state ───

            // Accumulated structure — grows with each BFS batch
            let all_folders: Arc<Mutex<Vec<FolderStructure>>> =
                Arc::new(Mutex::new(Vec::new()));

            // Track children per parent for rollup later
            let parent_children: Arc<Mutex<HashMap<String, Vec<String>>>> =
                Arc::new(Mutex::new(HashMap::new()));

            // Leaf sizing results for rollup
            let leaf_results: Arc<Mutex<HashMap<String, FolderUsage>>> =
                Arc::new(Mutex::new(HashMap::new()));

            // ─── Seed the root ───

            let root_name = base
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path_walk.clone());
            let root_path = normalize_path(base);

            let root_structure = FolderStructure {
                path: root_path.clone(),
                name: root_name,
                children: Vec::new(),
            };

            all_folders.lock().unwrap().push(root_structure);

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

            const BATCH_SIZE: usize = 200;

            // Emit initial structure (root only) so UI appears instantly
            let initial = ScanStructure {
                scan_id: scan_id_blocking.clone(),
                root_path: root_path.clone(),
                folders: all_folders.lock().unwrap().clone(),
                total_folders: 1,
            };
            let _ = tx.send(ScanStep::Structure(initial));

            loop {
                if cancel_blocking.load(Ordering::Relaxed) {
                    break;
                }

                let mut batch_new: Vec<DiscoveredFolder> = Vec::with_capacity(BATCH_SIZE);
                let mut batch_leaves: Vec<String> = Vec::new();

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
                        {
                            let mut pc = parent_children.lock().unwrap();
                            pc.entry(current_path.clone())
                                .or_default()
                                .extend(children.iter().map(|c| c.path.clone()));
                        }

                        // Add children to queue and batch
                        for mut child in children {
                            let child_path = child.path.clone();
                            queue.push_back(child_path);
                            batch_new.push(child);
                        }
                    }
                }

                // If nothing new and queue is empty, we're done discovering
                if batch_new.is_empty() && queue.is_empty() {
                    // Size any remaining leaves from this final iteration
                    for leaf in batch_leaves {
                        let _ = work_tx.send(leaf);
                    }
                    break;
                }

                // Add new folders to accumulated structure
                {
                    let mut folders = all_folders.lock().unwrap();
                    for f in &batch_new {
                        folders.push(FolderStructure {
                            path: f.path.clone(),
                            name: f.name.clone(),
                            children: Vec::new(),
                        });
                    }
                }

                // Emit accumulated structure — frontend re-renders growing tree
                let total_estimated = all_folders.lock().unwrap().len() + queue.len();
                let structure = ScanStructure {
                    scan_id: scan_id_blocking.clone(),
                    root_path: root_path.clone(),
                    folders: all_folders.lock().unwrap().clone(),
                    total_folders: total_estimated,
                };
                let _ = tx.send(ScanStep::Structure(structure));

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

            // Build complete parent→children map from all_folders
            let all_folders_snapshot = all_folders.lock().unwrap().clone();

            // We need the full tree structure for rollup.
            // parent_children was populated during BFS, but children Vec in
            // FolderStructure is empty. Rebuild from parent_children.
            let pc = parent_children.lock().unwrap().clone();

            // Roll up: process folders bottom-up (deepest first).
            // Since BFS discovered top-down, reverse the all_folders list
            // as an approximation of bottom-up order.
            let mut stats = leaf_results.lock().unwrap().clone();

            for folder in all_folders_snapshot.iter().rev() {
                let children: Vec<String> = pc.get(&folder.path).cloned().unwrap_or_default();
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
                    folder.path.clone(),
                    FolderUsage {
                        path: folder.path.clone(),
                        size: total_size,
                        file_count: total_files,
                        folder_count: total_folders,
                    },
                );
            }

            // Emit rolled-up stats for non-leaf folders
            for folder in all_folders_snapshot.iter() {
                let children: Vec<String> = pc.get(&folder.path).cloned().unwrap_or_default();
                if children.is_empty() {
                    continue; // Already emitted as leaf
                }
                if let Some(usage) = stats.get(&folder.path) {
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
                    ScanStep::Structure(structure) => {
                        let _ = window.emit("scan:structure", &structure);
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