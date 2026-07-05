use crate::domain::{
    FolderUsage, ScanChunk, ScanChunkData, ScanComplete,
    ScanError as DomainScanError, ScanProgress, ScanTreeChild, ScanTreeChildren, ScanTreeStarted,
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

impl DiskUsageUseCase {
    /// BFS streaming scan.
    ///
    /// Pull-based tree: emits `ScanTreeStarted` at start, `ScanTreeChildren`
    /// as children are discovered. Frontend calls `get_scan_tree_children` on expand.
    /// Leaf folders are sized in parallel (10 threads) as they're discovered.
    ///
    /// Memory: single tree store (passed in). No duplicate copies.
    pub fn run(
        window: tauri::WebviewWindow,
        path: String,
        _max_depth: u32, // deprecated — we scan everything now
        cancel: Arc<AtomicBool>,
        start: Instant,
        scan_id: String,
        // Persistent tree — the single store for parent→children.
        // Rollup reads paths from it. get_children queries it.
        tree: Arc<Mutex<HashMap<String, HashMap<String, Vec<ScanTreeChild>>>>>,
    ) -> impl std::future::Future<Output = ()> {
        let (done_tx, done_rx) = oneshot::channel();
        let (tx, mut rx) = mpsc::unbounded_channel::<ScanStep>();

        let path_walk = path.clone();
        let scan_id_blocking = scan_id.clone();
        let cancel_blocking = cancel.clone();
        let tree_blocking = tree.clone();

        tokio::task::spawn_blocking(move || {
            let base = Path::new(&path_walk);

            // ─── Shared state ───

            // Leaf sizing results for rollup — HashMap<path, FolderUsage>
            let leaf_results: Arc<Mutex<HashMap<String, FolderUsage>>> =
                Arc::new(Mutex::new(HashMap::new()));

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

            // Notification channel — sizing threads signal when a leaf is done
            // so the rollup thread can check for incremental rollup opportunities.
            let (notify_tx, notify_rx) = std::sync::mpsc::channel::<()>();

            // Counter — sizing threads increment as they exit; when it reaches 10,
           // rollup knows all sizing is done.
            let sizing_exit_count: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));

            let mut sizing_handles = Vec::new();
            for _ in 0..10 {
                let rx = work_rx.clone();
                let tx_t = tx_clone.clone();
                let results = leaf_results_clone.clone();
                let cancel_t = cancel_size.clone();
                let notify = notify_tx.clone();
                let exit_count = sizing_exit_count.clone();

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

                        // Notify rollup thread to check for rollup opportunities
                        let _ = notify.send(());
                    }
                    // Increment exit count — last thread out triggers rollup to stop
                    exit_count.fetch_add(1, Ordering::Release);
                });

                sizing_handles.push(handle);
            }

          // Signal — BFS tells rollup when discovery is complete so it can
           // finish rolling up any remaining parents.
            let bfs_done: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

            // ─── Incremental Rollup Thread ───
            // Runs concurrently with BFS. After each leaf completes, checks which
            // parents now have all children sized → rolls up → emits → cascades upward.
            // Reads children from the shared tree (single store, no duplication).
            let tree_for_rollup = tree_blocking.clone();
            let scan_id_for_rollup = scan_id_blocking.clone();
            let leaf_results_for_rollup = leaf_results.clone();
            let tx_for_rollup = tx.clone();
            let cancel_for_rollup = cancel_blocking.clone();
            let sizing_exit_for_rollup = sizing_exit_count.clone();
            let bfs_done_for_rollup = bfs_done.clone();

            let rollup_handle = std::thread::spawn(move || {
                // Pending parents: parents not yet rolled up. New parents are added
                // as BFS discovers them. Removed when rolled up.
                let mut pending: std::collections::HashSet<String> = std::collections::HashSet::new();

                loop {
                    if cancel_for_rollup.load(Ordering::Relaxed) {
                        break;
                    }

                    match notify_rx.recv_timeout(std::time::Duration::from_millis(500)) {
                        Ok(()) => {
                            // Pick up new parents discovered since last time
                            {
                                let tree = tree_for_rollup.lock().unwrap();
                                if let Some(scan_tree) = tree.get(&scan_id_for_rollup) {
                                    for parent in scan_tree.keys() {
                                        pending.insert(parent.clone());
                                    }
                                }
                            }

                            // Cascade: roll up parents whose children are all sized
                            let mut changed = true;
                            while changed {
                                changed = false;

                                let to_check: Vec<String> = pending.iter().cloned().collect();

                                for parent in to_check {
                                    let children: Vec<ScanTreeChild> = {
                                        let tree = tree_for_rollup.lock().unwrap();
                                        tree.get(&scan_id_for_rollup)
                                            .and_then(|s| s.get(&parent))
                                            .cloned()
                                            .unwrap_or_default()
                                    };
                                    if children.is_empty() {
                                        continue; // Not yet discovered
                                    }

                                    let all_sized = {
                                        let lr = leaf_results_for_rollup.lock().unwrap();
                                        children.iter().all(|c| lr.contains_key(&c.path))
                                    };

                                    if all_sized {
                                        let (total_size, total_files, total_folders) = {
                                            let lr = leaf_results_for_rollup.lock().unwrap();
                                            let mut s: u64 = 0;
                                            let mut f: u64 = 0;
                                            let mut d: u64 = 0;
                                            for c in &children {
                                                if let Some(cs) = lr.get(&c.path) {
                                                    s += cs.size;
                                                    f += cs.file_count;
                                                    d += 1 + cs.folder_count;
                                                }
                                            }
                                            (s, f, d)
                                        };

                                        let usage = FolderUsage {
                                            path: parent.clone(),
                                            size: total_size,
                                            file_count: total_files,
                                            folder_count: total_folders,
                                        };

                                        {
                                            let mut lr = leaf_results_for_rollup.lock().unwrap();
                                            lr.insert(parent.clone(), usage.clone());
                                        }
                                        pending.remove(&parent);
                                        let _ = tx_for_rollup.send(ScanStep::Folder(usage));
                                        changed = true;
                                    }
                                }
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            // Check if BFS is done AND all sizing threads have exited
                            let bfs_finished = bfs_done_for_rollup.load(Ordering::Acquire);
                            let sizing_done = sizing_exit_for_rollup.load(Ordering::Acquire) >= 10;
                            if bfs_finished && sizing_done {
                                // Final cascade — roll up any remaining parents
                                let mut changed = true;
                                while changed {
                                    changed = false;
                                    let to_check: Vec<String> = pending.iter().cloned().collect();
                                    for parent in to_check {
                                        let children: Vec<ScanTreeChild> = {
                                            let tree = tree_for_rollup.lock().unwrap();
                                            tree.get(&scan_id_for_rollup)
                                                .and_then(|s| s.get(&parent))
                                                .cloned()
                                                .unwrap_or_default()
                                        };
                                        if children.is_empty() {
                                            continue;
                                        }
                                        let all_sized = {
                                            let lr = leaf_results_for_rollup.lock().unwrap();
                                            children.iter().all(|c| lr.contains_key(&c.path))
                                        };
                                        if all_sized {
                                            let (total_size, total_files, total_folders) = {
                                                let lr = leaf_results_for_rollup.lock().unwrap();
                                                let mut s: u64 = 0;
                                                let mut f: u64 = 0;
                                                let mut d: u64 = 0;
                                                for c in &children {
                                                    if let Some(cs) = lr.get(&c.path) {
                                                        s += cs.size;
                                                        f += cs.file_count;
                                                        d += 1 + cs.folder_count;
                                                    }
                                                }
                                                (s, f, d)
                                            };
                                            let usage = FolderUsage {
                                                path: parent.clone(),
                                                size: total_size,
                                                file_count: total_files,
                                                folder_count: total_folders,
                                            };
                                            {
                                                let mut lr = leaf_results_for_rollup.lock().unwrap();
                                                lr.insert(parent.clone(), usage.clone());
                                            }
                                            pending.remove(&parent);
                                            let _ = tx_for_rollup.send(ScanStep::Folder(usage));
                                            changed = true;
                                        }
                                    }
                                }
                                break;
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            break;
                        }
                    }
                }
            });

            // ─── BFS Discovery Loop ───

            // Emit Started so frontend knows the root
            let _ = tx.send(ScanStep::Started(ScanTreeStarted {
                scan_id: scan_id_blocking.clone(),
                root_path: root_path.clone(),
                root_name: root_name.clone(),
            }));

            const BATCH_SIZE: usize = 200;

            // Track which folders had children discovered in this batch
            let mut batch_discovered: Vec<(String, Vec<ScanTreeChild>)> = Vec::new();

            loop {
                if cancel_blocking.load(Ordering::Relaxed) {
                    break;
                }

                let mut batch_leaves: Vec<String> = Vec::new();
                batch_discovered.clear();

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
                        // Store in the shared tree (single store — rollup + get_children both read this)
                        batch_discovered.push((current_path, children.clone()));

                        // Add children to queue
                        for child in &children {
                            queue.push_back(child.path.clone());
                        }
                    }
                }

                // If nothing processed and queue is empty, we're done discovering
                if batch_discovered.is_empty() && batch_leaves.is_empty() && queue.is_empty() {
                    break;
                }

                // Write to tree + emit ChildrenReady for folders whose children were discovered
                for (parent, children) in &batch_discovered {
                    // Store in the shared tree
                    {
                        let mut tree = tree_blocking.lock().unwrap();
                        tree.entry(scan_id_blocking.clone())
                            .or_insert_with(HashMap::new)
                            .insert(parent.clone(), children.clone());
                    }

                    let _ = tx.send(ScanStep::ChildrenReady(ScanTreeChildren {
                        scan_id: scan_id_blocking.clone(),
                        parent_path: parent.clone(),
                        children: children.clone(),
                    }));
                }

                // Queue leaves for sizing
                for leaf in batch_leaves {
                    if cancel_blocking.load(Ordering::Relaxed) {
                        break;
                    }
                    let _ = work_tx.send(leaf);
                }
            }

            // Signal rollup that BFS discovery is complete — it can now see all parents
            bfs_done.store(true, Ordering::Release);

            // Close sizing work queue — threads will drain and exit
            drop(work_tx);

            // Wait for all sizing threads
            for handle in sizing_handles {
                let _ = handle.join();
            }

            // Wait for rollup thread
            let _ = rollup_handle.join();

            if cancel_blocking.load(Ordering::Relaxed) {
                let _ = tx.send(ScanStep::Cancelled);
                return;
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
/// Returns list of discovered child folders as ScanTreeChild (the single shared type).
fn readdir_children(
    dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Vec<ScanTreeChild> {
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

                    children.push(ScanTreeChild {
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

    // ─── Real filesystem (E:\projects) ───

    #[test]
    fn real_projects_folder_has_children() {
        let path = std::path::Path::new("E:/projects");
        assert!(path.is_dir(), "E:\\projects must exist");

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_children(path, &cancel);

        // E:\projects has at least filebitch
        assert!(!children.is_empty(), "E:\\projects should have subdirectories");
        let names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"filebitch"), "E:\\projects should contain filebitch");

        println!("E:\\projects top-level folders: {:?}", names);
    }

    #[test]
    fn real_projects_filebitch_has_subdirs() {
        let path = std::path::Path::new("E:/projects/filebitch");
        assert!(path.is_dir(), "E:\\projects\\filebitch must exist");

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_children(path, &cancel);

        assert!(!children.is_empty(), "filebitch should have subdirectories");
        let names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"src-tauri"), "filebitch should contain src-tauri");

        println!("filebitch subdirs ({}) : {:?}", children.len(), names);
    }

    #[test]
    fn real_size_filebitch_src_tauri() {
        let path = std::path::Path::new("E:/projects/filebitch/src-tauri");
        assert!(path.is_dir(), "E:\\projects\\filebitch\\src-tauri must exist");

        let usage = size_folder(path);

        assert!(usage.size > 0, "src-tauri should have files");
        assert!(usage.file_count > 0, "src-tauri should have files");
        assert!(usage.folder_count >= 1, "src-tauri should have subfolders (at least src/)");

        println!(
            "src-tauri: {} bytes, {} files, {} folders",
            usage.size, usage.file_count, usage.folder_count
        );
    }

    #[test]
    fn real_full_bfs_discovery_projects() {
        let path = std::path::Path::new("E:/projects");
        assert!(path.is_dir(), "E:\\projects must exist");

        let cancel = Arc::new(AtomicBool::new(false));
        let root = normalize_path(path);

        // BFS — count folders discovered (same logic as the scan)
        let mut queue: VecDeque<String> = VecDeque::new();
        queue.push_back(root);
        let mut total_folders: usize = 0;

        while let Some(current) = queue.pop_front() {
            total_folders += 1;
            let children = readdir_children(Path::new(&current), &cancel);
            for child in &children {
                queue.push_back(child.path.clone());
            }
        }

        assert!(total_folders > 10, "E:\\projects should have at least 10 folders, found {}", total_folders);

        println!("E:\\projects BFS discovered {} folders", total_folders);
    }

    #[test]
    fn real_size_folder_filebitch_root() {
        let path = std::path::Path::new("E:/projects/filebitch");
        assert!(path.is_dir(), "E:\\projects\\filebitch must exist");

        let usage = size_folder(path);

        // filebitch is a real project — should have meaningful content
        assert!(usage.file_count > 50, "filebitch should have >50 files, found {}", usage.file_count);
        assert!(usage.folder_count > 5, "filebitch should have >5 folders, found {}", usage.folder_count);
        assert!(usage.size > 10_000, "filebitch should be >10KB, found {} bytes", usage.size);

        println!(
            "filebitch root: {} bytes, {} files, {} folders",
            usage.size, usage.file_count, usage.folder_count
        );
    }
}