use crate::domain::{
    FolderUsage, NodeId, ScanChunk, ScanChunkData, ScanComplete,
    ScanError as DomainScanError, ScanProgress, ScanTreeChildren, ScanTreeStarted,
};
use crate::usecases::analytics::arena::FolderArena;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};

/// Orchestrates disk usage scans.
///
/// Approach:
/// - Phase 1: BFS discovers all folders (readdir only, no file stats)
/// - Phase 2: Leaf folders sized in parallel (10 threads)
/// - Phase 3: Rollup propagates totals bottom-up via parent pointers
/// - Visual: emits `FolderStarted` for top-level folders when their first leaf is sized
///
/// Arena-based: folders stored in FolderArena (SoA, first-child/next-sibling).
/// Pull-based tree: emits `ScanTreeStarted` at start, thin `ScanTreeChildren`
/// (id + count) as children are discovered. Frontend pulls children on expand.
pub struct DiskUsageUseCase;

/// Inter-thread orchestration enum.
enum ScanStep {
    /// Scan started — emit root info to frontend.
    Started(ScanTreeStarted),
    /// Children discovered for a folder — thin event (id + count only).
    ChildrenReady(ScanTreeChildren),
    /// One folder sized (leaf or rolled-up parent).
    Folder(FolderUsage),
    /// Scan finished.
    Complete,
    /// Scan was cancelled.
    Cancelled,
}

impl DiskUsageUseCase {
    /// BFS streaming scan with arena-based tree storage.
    ///
    /// Memory: single FolderArena per scan. No duplicate copies.
    /// Rollup: parent-pointer walk after each leaf completes.
    pub fn run(
        window: tauri::WebviewWindow,
        path: String,
        _max_depth: u32, // deprecated — we scan everything now
        cancel: Arc<AtomicBool>,
        start: Instant,
        scan_id: String,
        // Persistent arena — the single store for the folder tree.
        tree: Arc<Mutex<HashMap<String, Arc<Mutex<FolderArena>>>>>,
    ) -> impl std::future::Future<Output = ()> {
        let (done_tx, done_rx) = oneshot::channel();
        let (tx, mut rx) = mpsc::unbounded_channel::<ScanStep>();

        let path_walk = path.clone();
        let scan_id_blocking = scan_id.clone();
        let cancel_blocking = cancel.clone();
        let tree_blocking = tree.clone();

        tokio::task::spawn_blocking(move || {
            let base = Path::new(&path_walk);

            // ─── Arena (wrapped in Arc<Mutex<>> for concurrent frontend queries) ───

            let mut arena = FolderArena::new();
            let root_name = base
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path_walk.clone());
            let root_path = normalize_path(base);

            let root_id = arena.alloc_folder(&root_name);

            // Store arena in tree BEFORE BFS so the frontend can query children
            // as they are discovered. Arc<Mutex<>> lets BFS write and frontend
            // read concurrently without blocking each other.
            let arena_arc = Arc::new(std::sync::Mutex::new(arena));
            let arena_bfs = arena_arc.clone();

            {
                let mut tree = tree_blocking.lock().unwrap();
                tree.insert(scan_id_blocking.clone(), arena_arc);
            }

            // Path map: NodeId → normalized path string (for IPC emission)
            let mut path_map: std::collections::HashMap<NodeId, String> =
                std::collections::HashMap::new();
            path_map.insert(root_id, root_path.clone());

            // BFS queue — stores (NodeId, path)
            let mut queue: VecDeque<(NodeId, String)> = VecDeque::new();
            queue.push_back((root_id, root_path.clone()));

            // ─── Sizing work queue (10 threads, leaf-based) ───
            // Each thread sizes a leaf folder, writes to arena, and does parent-pointer rollup.
            // Events stream to the frontend as each leaf completes.
            let (work_tx, work_rx) = std::sync::mpsc::channel::<(NodeId, String)>();
            let work_rx = Arc::new(std::sync::Mutex::new(work_rx));

            let tx_clone = tx.clone();
            let cancel_size = cancel_blocking.clone();
            let arena_threads = arena_bfs.clone();

            let mut sizing_handles = Vec::new();
            for _ in 0..50 {
                let rx = work_rx.clone();
                let tx_t = tx_clone.clone();
                let cancel_t = cancel_size.clone();
                let arena_t = arena_threads.clone();

                let handle = std::thread::spawn(move || {
                    loop {
                        if cancel_t.load(Ordering::Relaxed) {
                            break;
                        }

                        let (node_id, folder_path) = {
                            let lock = rx.lock().unwrap();
                            match lock.recv() {
                                Ok(item) => item,
                                Err(_) => break, // Channel closed
                            }
                        };

                        if cancel_t.load(Ordering::Relaxed) {
                            break;
                        }

                        let (size, file_count, folder_count) = size_folder_stats(Path::new(&folder_path));

                        // Write stats to arena and do parent-pointer rollup
                        {
                            let mut arena = arena_t.lock().unwrap();
                            let idx = node_id.0 as usize;

                            // Write leaf stats
                            arena.size[idx] = size;
                            arena.file_count[idx] = file_count;
                            arena.folder_count[idx] = folder_count;
                            arena.sized[idx] = true;

                            // Emit leaf stats
                            let _ = tx_t.send(ScanStep::Folder(FolderUsage {
                                node_id,
                                size,
                                file_count,
                                folder_count,
                            }));

                            // Walk parent chain — roll up parents whose children are all sized
                            let mut current = arena.parent[idx];
                            while let Some(parent_id) = current {
                                let p = parent_id.0 as usize;
                                if arena.all_children_sized(parent_id) {
                                    let (s, f, d) = arena.sum_children(parent_id);
                                    arena.size[p] = s;
                                    arena.file_count[p] = f;
                                    arena.folder_count[p] = d;
                                    arena.sized[p] = true;

                                    let _ = tx_t.send(ScanStep::Folder(FolderUsage {
                                        node_id: parent_id,
                                        size: s,
                                        file_count: f,
                                        folder_count: d,
                                    }));

                                    current = arena.parent[p];
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                });

                sizing_handles.push(handle);
            }

            // ─── BFS Discovery Loop ───

            // Emit Started so frontend knows the root
            let _ = tx.send(ScanStep::Started(ScanTreeStarted {
                scan_id: scan_id_blocking.clone(),
                root_id,
                root_path: root_path.clone(),
                root_name: root_name.clone(),
            }));

            const BATCH_SIZE: usize = 200;

            let mut batch_leaves: Vec<(NodeId, String)> = Vec::new();

            loop {
                if cancel_blocking.load(Ordering::Relaxed) {
                    break;
                }

                let mut batch_discovered: Vec<(NodeId, u32)> = Vec::new();

                // Process a batch from the BFS queue
                for _ in 0..BATCH_SIZE {
                    let (current_id, current_path) = match queue.pop_front() {
                        Some(item) => item,
                        None => break,
                    };

                    let children = readdir_names(Path::new(&current_path), &cancel_blocking);
                    let child_count = children.len();

                    if child_count == 0 {
                        // Leaf — queue for sizing
                        batch_leaves.push((current_id, current_path));
                    } else {
                        // Allocate children in arena, link to parent
                        let mut arena = arena_bfs.lock().unwrap();
                        for (child_name, child_path) in children {
                            let child_id = arena.alloc_folder(&child_name);
                            arena.add_child(current_id, child_id);
                            path_map.insert(child_id, child_path.clone());
                            queue.push_back((child_id, child_path));
                        }
                        // arena lock dropped here

                        // Track for thin event emission
                        batch_discovered.push((current_id, child_count as u32));
                    }
                }

                // Sort queue alphabetically so the next batch processes in name order
                queue.make_contiguous().sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

                // If nothing processed and queue is empty, we're done discovering
                if batch_discovered.is_empty() && batch_leaves.is_empty() && queue.is_empty() {
                    break;
                }

                // Emit thin ChildrenReady events
                for (parent_id, child_count) in &batch_discovered {
                    let _ = tx.send(ScanStep::ChildrenReady(ScanTreeChildren {
                        scan_id: scan_id_blocking.clone(),
                        parent_id: *parent_id,
                        child_count: *child_count,
                    }));
                }

                // Queue leaves for sizing
                for (node_id, leaf_path) in &batch_leaves {
                    if cancel_blocking.load(Ordering::Relaxed) {
                        break;
                    }
                    let _ = work_tx.send((*node_id, leaf_path.clone()));
                }
                batch_leaves.clear();
            }

            // Close sizing work queue — threads will drain and exit
            drop(work_tx);

            // Wait for all sizing threads
            for handle in sizing_handles {
                let _ = handle.join();
            }

            // ─── Final Rollup Pass ───
            // Sizing threads do parent-pointer rollup as they complete, so most nodes
            // are already sized. This pass catches any remaining unsized nodes.
            {
                let mut arena = arena_bfs.lock().unwrap();
                for idx in (0..arena.len()).rev() {
                    let id = NodeId(idx as u32);

                    // Skip if already sized during the scan
                    if arena.sized[idx] {
                        continue;
                    }

                    let node_path = match path_map.get(&id) {
                        Some(p) => p.clone(),
                        None => continue,
                    };

                    let children: Vec<NodeId> = arena.children(id).collect();
                    if children.is_empty() {
                        // Leaf — size it now
                        let (size, file_count, folder_count) =
                            size_folder_stats(Path::new(&node_path));
                        arena.size[idx] = size;
                        arena.file_count[idx] = file_count;
                        arena.folder_count[idx] = folder_count;
                        arena.sized[idx] = true;

                        let _ = tx.send(ScanStep::Folder(FolderUsage {
                            node_id: id,
                            size,
                            file_count,
                            folder_count,
                        }));
                    } else {
                        // Internal node — roll up from children
                        if arena.all_children_sized(id) {
                            let (total_size, total_files, total_folders) = arena.sum_children(id);
                            arena.size[idx] = total_size;
                            arena.file_count[idx] = total_files;
                            arena.folder_count[idx] = total_folders;
                            arena.sized[idx] = true;

                            let _ = tx.send(ScanStep::Folder(FolderUsage {
                                node_id: id,
                                size: total_size,
                                file_count: total_files,
                                folder_count: total_folders,
                            }));
                        }
                    }
                }
            }

            if cancel_blocking.load(Ordering::Relaxed) {
                let _ = tx.send(ScanStep::Cancelled);
                return;
            }

            let _ = tx.send(ScanStep::Complete);
        });

        let start_clone = start.clone();
        let scan_id_clone = scan_id.clone();
        let cancel_clone = cancel.clone();
        let tree_for_emit = tree.clone();

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
                        // Observability — log backend memory state
                        {
                            let tree = tree_for_emit.lock().unwrap();
                            let scan_arena = tree.get(&scan_id_clone);
                            let tree_folders = scan_arena.and_then(|a| a.lock().ok().map(|g| g.len())).unwrap_or(0);
                            println!(
                                "[BACKEND MEMORY] scan={} | arena_folders={} | sized_folders={} | total_files={} | total_size={}",
                                scan_id_clone, tree_folders, folder_count, total_files, format_bytes(total_size),
                            );
                        }

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

/// Size a single folder by walking all its descendants. Returns stats only.
fn size_folder_stats(path: &Path) -> (u64, u64, u64) {
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

    (size, file_count, folder_count)
}

/// Size an entire subtree with a single WalkDir pass.
/// For each file, adds its size to all ancestor folders up to the root.
/// For each directory, increments the folder_count of all ancestors.
/// Returns FolderUsage for every folder in the subtree, sorted by depth (children first).
/// Read immediate subdirectories of a path.
/// Returns list of (name, normalized_path) tuples.
fn readdir_names(
    dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Vec<(String, String)> {
    let mut children = Vec::new();

    match dir.read_dir() {
        Ok(entries) => {
            for entry in entries.flatten() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                let path = entry.path();
                if path.is_dir() {
                    let child_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let child_path = normalize_path(&path);

                    children.push((child_name, child_path));
                }
            }
        }
        Err(_) => {} // Skip inaccessible directories
    }

    // Sort alphabetically so BFS discovers folders in name order
    children.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    children
}

/// Format bytes as human-readable size (for observability logging).
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000_000 {
        format!("{:.1} TB", bytes as f64 / 1_000_000_000_000.0)
    } else if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.1} KB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{} B", bytes)
    }
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

    // ─── readdir_names ───

    #[test]
    fn reads_immediate_subdirectories() {
        let temp = create_test_tree();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_names(base, &cancel);

        assert_eq!(children.len(), 2);
        let names: Vec<&str> = children.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"d"));
    }

    #[test]
    fn nested_directory_has_children() {
        let temp = create_test_tree();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_names(&base.join("a"), &cancel);

        assert_eq!(children.len(), 1);
        assert_eq!(children[0].0, "b");
    }

    #[test]
    fn leaf_directory_has_no_children() {
        let temp = create_test_tree();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_names(&base.join("a/b/c"), &cancel);

        assert!(children.is_empty());
    }

    #[test]
    fn empty_directory_has_no_children() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_names(base, &cancel);

        assert!(children.is_empty());
    }

    // ─── size_folder_stats ───

    #[test]
    fn sizes_leaf_folder() {
        let temp = create_test_tree();
        let base = temp.path();

        let (size, file_count, folder_count) = size_folder_stats(&base.join("d"));
        assert_eq!(size, 50);
        assert_eq!(file_count, 1);
        assert_eq!(folder_count, 0);
    }

    #[test]
    fn sizes_empty_folder() {
        let temp = create_test_tree();
        let base = temp.path();

        let (size, file_count, folder_count) = size_folder_stats(&base.join("a/b/c"));
        assert_eq!(size, 0);
        assert_eq!(file_count, 0);
        assert_eq!(folder_count, 0);
    }

    #[test]
    fn sizes_folder_with_subdirs() {
        let temp = create_test_tree();
        let base = temp.path();

        let (size, file_count, folder_count) = size_folder_stats(&base.join("a"));
        assert_eq!(size, 300); // 100 + 200
        assert_eq!(file_count, 2);
        assert_eq!(folder_count, 2); // b, b/c
    }

    // ─── Real filesystem (E:\projects) ───

    #[test]
    fn real_projects_folder_has_children() {
        let path = std::path::Path::new("E:/projects");
        assert!(path.is_dir(), "E:\\projects must exist");

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_names(path, &cancel);

        // E:\projects has at least filebitch
        assert!(!children.is_empty(), "E:\\projects should have subdirectories");
        let names: Vec<&str> = children.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"filebitch"), "E:\\projects should contain filebitch");

        println!("E:\\projects top-level folders: {:?}", names);
    }

    #[test]
    fn real_projects_filebitch_has_subdirs() {
        let path = std::path::Path::new("E:/projects/filebitch");
        assert!(path.is_dir(), "E:\\projects\\filebitch must exist");

        let cancel = Arc::new(AtomicBool::new(false));
        let children = readdir_names(path, &cancel);

        assert!(!children.is_empty(), "filebitch should have subdirectories");
        let names: Vec<&str> = children.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"src-tauri"), "filebitch should contain src-tauri");

        println!("filebitch subdirs ({}) : {:?}", children.len(), names);
    }

    #[test]
    fn real_size_filebitch_src_tauri() {
        let path = std::path::Path::new("E:/projects/filebitch/src-tauri");
        assert!(path.is_dir(), "E:\\projects\\filebitch\\src-tauri must exist");

        let (size, file_count, folder_count) = size_folder_stats(path);

        assert!(size > 0, "src-tauri should have files");
        assert!(file_count > 0, "src-tauri should have files");
        assert!(folder_count >= 1, "src-tauri should have subfolders (at least src/)");

        println!(
            "src-tauri: {} bytes, {} files, {} folders",
            size, file_count, folder_count
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
            let children = readdir_names(Path::new(&current), &cancel);
            for (_, child_path) in children {
                queue.push_back(child_path);
            }
        }

        assert!(total_folders > 10, "E:\\projects should have at least 10 folders, found {}", total_folders);

        println!("E:\\projects BFS discovered {} folders", total_folders);
    }

    #[test]
    fn real_size_folder_filebitch_root() {
        let path = std::path::Path::new("E:/projects/filebitch");
        assert!(path.is_dir(), "E:\\projects\\filebitch must exist");

        let (size, file_count, folder_count) = size_folder_stats(path);

        // filebitch is a real project — should have meaningful content
        assert!(file_count > 50, "filebitch should have >50 files, found {}", file_count);
        assert!(folder_count > 5, "filebitch should have >5 folders, found {}", folder_count);
        assert!(size > 10_000, "filebitch should be >10KB, found {} bytes", size);

        println!(
            "filebitch root: {} bytes, {} files, {} folders",
            size, file_count, folder_count
        );
    }
}
