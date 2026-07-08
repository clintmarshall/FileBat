use crate::domain::{
    FolderUsage, NodeId, ScanChunk, ScanChunkData, ScanComplete,
    ScanError as DomainScanError, ScanProgress, ScanTreeChildren, ScanTreeStarted,
};
use crate::usecases::analytics::arena::FolderArena;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};

/// Orchestrates disk usage scans.
///
/// Approach: SINGLE-PASS BFS
/// - One readdir per folder: discovers subdirectories AND sizes immediate files
///   (file sizes are free from WIN32_FIND_DATA on Windows, no extra syscall)
/// - Rollup: bottom-up propagation after BFS completes
/// - Visual: tree structure emitted immediately, sizes fill in during rollup
///
/// This replaces the old two-phase approach (BFS + WalkDir per leaf) which was
/// O(files) slower because WalkDir re-walked every file separately.
pub struct DiskUsageUseCase;

/// Inter-thread orchestration enum.
enum ScanStep {
    /// Scan started — emit root info to frontend.
    Started(ScanTreeStarted),
    /// Children discovered for a folder — thin event (id + count).
    ChildrenReady(ScanTreeChildren),
    /// One folder sized (leaf or rolled-up parent).
    Folder(FolderUsage),
    /// Scan finished.
    Complete,
    /// Scan was cancelled.
    Cancelled,
}

/// Result of reading a single directory: immediate file sizes + subdirectory children.
struct DirEntry {
    /// Subdirectories: (name, normalized_path)
    subdirs: Vec<(String, String)>,
    /// Sum of immediate file sizes (not recursive)
    file_size: u64,
    /// Count of immediate files
    file_count: u64,
}

impl DiskUsageUseCase {
    /// Single-pass BFS scan.
    pub fn run(
        window: tauri::WebviewWindow,
        path: String,
        _max_depth: u32,
        cancel: Arc<AtomicBool>,
        start: Instant,
        scan_id: String,
        tree: Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::Mutex<FolderArena>>>>>,
    ) -> impl std::future::Future<Output = ()> {
        let (done_tx, done_rx) = oneshot::channel();
        let (tx, mut rx) = mpsc::unbounded_channel::<ScanStep>();

        let path_walk = path.clone();
        let scan_id_blocking = scan_id.clone();
        let cancel_blocking = cancel.clone();
        let tree_blocking = tree.clone();

        tokio::task::spawn_blocking(move || {
            let base = Path::new(&path_walk);

            // ─── Arena ───

            let mut arena = FolderArena::new();
            let root_name = base
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path_walk.clone());
            let root_path = normalize_path(base);

            let root_id = arena.alloc_folder(&root_name);

            // Per-folder accumulated stats (set during BFS for immediate files,
            // updated during rollup for totals)
            let mut folder_size: HashMap<NodeId, u64> = HashMap::new();
            let mut folder_file_count: HashMap<NodeId, u64> = HashMap::new();
            folder_size.insert(root_id, 0);
            folder_file_count.insert(root_id, 0);

            // BFS queue — stores (NodeId, normalized_path)
            let mut queue: VecDeque<(NodeId, String)> = VecDeque::new();
            queue.push_back((root_id, root_path.clone()));

            // Completed folders in reverse BFS order for bottom-up rollup
            let mut completed: Vec<NodeId> = Vec::new();

            // ─── Single-Pass BFS ───
            // Each iteration: readdir one folder, discover children, size immediate files.

            let mut batch_discovered: Vec<(NodeId, u32)> = Vec::new();

            while let Some((current_id, current_path)) = queue.pop_front() {
                if cancel_blocking.load(Ordering::Relaxed) {
                    break;
                }

                let entries = read_dir_usage(Path::new(&current_path), &cancel_blocking);
                let child_count = entries.subdirs.len();

                // Record immediate file stats for this folder
                *folder_size.entry(current_id).or_insert(0) += entries.file_size;
                *folder_file_count.entry(current_id).or_insert(0) += entries.file_count;

                // Allocate children in arena, link to parent
                for (child_name, child_path) in entries.subdirs {
                    let child_id = arena.alloc_folder(&child_name);
                    arena.add_child(current_id, child_id);
                    folder_size.insert(child_id, 0);
                    folder_file_count.insert(child_id, 0);
                    queue.push_back((child_id, child_path));
                }

                if child_count > 0 {
                    batch_discovered.push((current_id, child_count as u32));
                }

                completed.push(current_id);
            }

            // Sort queue alphabetically (already processed, but for consistency)
            // Not needed since we process in BFS order.

            // Freeze structural data
            let structural = arena.freeze_structural();
            arena.init_pending(&structural);

            // Store arena in tree for frontend queries
            let arena_arc = Arc::new(std::sync::Mutex::new(arena));
            {
                let mut tree_map = tree_blocking.lock().unwrap();
                tree_map.insert(scan_id_blocking.clone(), arena_arc.clone());
            }

            // ─── Emit Started + ChildrenReady events ───

            let _ = tx.send(ScanStep::Started(ScanTreeStarted {
                scan_id: scan_id_blocking.clone(),
                root_id,
                root_path: root_path.clone(),
                root_name: root_name.clone(),
            }));

            for (parent_id, child_count) in &batch_discovered {
                let _ = tx.send(ScanStep::ChildrenReady(ScanTreeChildren {
                    scan_id: scan_id_blocking.clone(),
                    parent_id: *parent_id,
                    child_count: *child_count,
                }));
            }

            // ─── Bottom-Up Rollup ───
            // Process completed folders in reverse BFS order (leaves first).
            // Each folder already has immediate file sizes from BFS.
            // Rollup adds children's totals to get the final size.
            //
            // Process in batches to avoid holding the arena lock for too long.
            // The get_scan_tree_children command needs the arena lock — if we hold it
            // for the entire rollup, the command blocks and the frontend hangs.

            const ROLLUP_BATCH: usize = 500;
            let mut rollup_results: Vec<(NodeId, u64, u64, u64)> = Vec::with_capacity(ROLLUP_BATCH);

            let mut folder_count: usize = 0;

            // Reverse BFS order: leaves first, root last
            let reversed: Vec<NodeId> = completed.iter().copied().rev().collect();

            for chunk in reversed.chunks(ROLLUP_BATCH) {
                if cancel_blocking.load(Ordering::Relaxed) {
                    break;
                }

                // Compute rollup for this batch without holding arena lock
                // (folder_size and folder_file_count are HashMaps, no lock needed)
                rollup_results.clear();
                for &id in chunk {
                    let mut size = *folder_size.get(&id).unwrap_or(&0);
                    let mut file_count = *folder_file_count.get(&id).unwrap_or(&0);

                    // Sum children's already-computed sizes using structural data (Arc, no lock)
                    let mut sib = structural.first_child[id.0 as usize];
                    let mut folder_count_local: u64 = 0;
                    while let Some(child_id) = sib {
                        size += *folder_size.get(&child_id).unwrap_or(&0);
                        file_count += *folder_file_count.get(&child_id).unwrap_or(&0);
                        folder_count_local += 1;
                        sib = structural.next_sibling[child_id.0 as usize];
                    }

                    rollup_results.push((id, size, file_count, folder_count_local));
                }

                // Write results to arena (brief lock)
                {
                    let mut arena = arena_arc.lock().unwrap();
                    for &(id, size, file_count, folder_count_local) in &rollup_results {
                        arena.write_size(id, size, file_count, folder_count_local);
                    }
                }

                // Emit folder usage events (no lock needed)
                for &(id, size, file_count, folder_count_local) in &rollup_results {
                    let _ = tx.send(ScanStep::Folder(FolderUsage {
                        node_id: id,
                        size,
                        file_count,
                        folder_count: folder_count_local,
                    }));
                    folder_count += 1;
                }

                // Brief sleep to force thread context switch so other tasks
                // (e.g., get_scan_tree_children) can acquire the arena lock.
                // yield_now() is just a hint on Windows and doesn't guarantee a switch.
                std::thread::sleep(std::time::Duration::from_millis(1));
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
            let mut emit_total_files: u64 = 0;
            let mut emit_total_size: u64 = 0;
            let mut emit_folder_count: usize = 0;

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
                        emit_total_files += usage.file_count;
                        emit_total_size += usage.size;
                        emit_folder_count += 1;

                        let _ = window.emit(
                            "scan:chunk",
                            ScanChunk {
                                scan_id: scan_id_clone.clone(),
                                data: ScanChunkData::FolderUsage { usage },
                            },
                        );
                    }
                    ScanStep::Complete => {
                        // Observability
                        {
                            let tree = tree_for_emit.lock().unwrap();
                            let scan_arena = tree.get(&scan_id_clone);
                            let tree_folders = scan_arena
                                .and_then(|a| a.lock().ok().map(|g| g.len()))
                                .unwrap_or(0);
                            println!(
                                "[BACKEND MEMORY] scan={} | arena_folders={} | sized_folders={} | total_files={} | total_size={}",
                                scan_id_clone, tree_folders, emit_folder_count, emit_total_files, format_bytes(emit_total_size),
                            );
                        }

                        let _ = window.emit(
                            "scan:progress",
                            ScanProgress {
                                scan_id: scan_id_clone.clone(),
                                percentage: 100.0,
                                message: format!(
                                    "Scanned {} files across {} folders",
                                    emit_total_files, emit_folder_count
                                ),
                            },
                        );

                        let duration = start_clone.elapsed().as_millis() as u64;
                        let _ = window.emit(
                            "scan:complete",
                            ScanComplete {
                                scan_id: scan_id_clone.clone(),
                                total_items: emit_total_files,
                                total_size: emit_total_size,
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

/// Read a single directory: discover subdirectories and size immediate files.
///
/// On Windows, `read_dir` uses FindFirstFileEx which returns WIN32_FIND_DATA
/// containing file sizes — no extra syscall needed for `entry.metadata()`.
fn read_dir_usage(dir: &Path, cancel: &Arc<AtomicBool>) -> DirEntry {
    let mut subdirs = Vec::new();
    let mut file_size: u64 = 0;
    let mut file_count: u64 = 0;

    match dir.read_dir() {
        Ok(entries) => {
            for entry in entries {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                match entry {
                    Ok(e) => {
                        let path = e.path();
                        let meta = match e.metadata() {
                            Ok(m) => m,
                            Err(_) => continue,
                        };

                        if meta.is_file() {
                            file_size += meta.len();
                            file_count += 1;
                        } else if meta.is_dir() {
                            let name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let child_path = normalize_path(&path);
                            subdirs.push((name, child_path));
                        }
                    }
                    Err(_) => continue,
                }
            }
        }
        Err(_) => {} // Skip inaccessible directories
    }

    // Sort subdirs alphabetically for consistent rendering
    subdirs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

    DirEntry {
        subdirs,
        file_size,
        file_count,
    }
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

    // ─── read_dir_usage ───

    #[test]
    fn reads_subdirs_and_file_sizes() {
        let temp = create_test_tree();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let entries = read_dir_usage(base, &cancel);

        assert_eq!(entries.subdirs.len(), 2);
        let names: Vec<&str> = entries.subdirs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"d"));
        // Root has no immediate files
        assert_eq!(entries.file_size, 0);
        assert_eq!(entries.file_count, 0);
    }

    #[test]
    fn sizes_immediate_files() {
        let temp = create_test_tree();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let entries = read_dir_usage(&base.join("a"), &cancel);

        // "a" has one subdir "b" and one file "file1.txt" (100 bytes)
        assert_eq!(entries.subdirs.len(), 1);
        assert_eq!(entries.subdirs[0].0, "b");
        assert_eq!(entries.file_size, 100);
        assert_eq!(entries.file_count, 1);
    }

    #[test]
    fn leaf_directory_has_no_subdirs() {
        let temp = create_test_tree();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let entries = read_dir_usage(&base.join("a/b/c"), &cancel);

        assert!(entries.subdirs.is_empty());
        assert_eq!(entries.file_size, 0);
        assert_eq!(entries.file_count, 0);
    }

    #[test]
    fn empty_directory() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        let cancel = Arc::new(AtomicBool::new(false));
        let entries = read_dir_usage(base, &cancel);

        assert!(entries.subdirs.is_empty());
        assert_eq!(entries.file_size, 0);
        assert_eq!(entries.file_count, 0);
    }

    // ─── Real filesystem (E:\projects) ───

    #[test]
    fn real_projects_folder_has_children() {
        let path = std::path::Path::new("E:/projects");
        assert!(path.is_dir(), "E:\\projects must exist");

        let cancel = Arc::new(AtomicBool::new(false));
        let entries = read_dir_usage(path, &cancel);

        assert!(!entries.subdirs.is_empty(), "E:\\projects should have subdirectories");
        let names: Vec<&str> = entries.subdirs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"filebitch"), "E:\\projects should contain filebitch");

        println!("E:\\projects top-level folders: {:?}, files: {}, size: {}", names, entries.file_count, format_bytes(entries.file_size));
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
        let mut total_files: u64 = 0;
        let mut total_size: u64 = 0;

        while let Some(current) = queue.pop_front() {
            total_folders += 1;
            let entries = read_dir_usage(Path::new(&current), &cancel);
            total_files += entries.file_count;
            total_size += entries.file_size;
            for (_, child_path) in entries.subdirs {
                queue.push_back(child_path);
            }
        }

        assert!(
            total_folders > 10,
            "E:\\projects should have at least 10 folders, found {}",
            total_folders
        );

        println!(
            "E:\\projects BFS: {} folders, {} files, {}",
            total_folders, total_files, format_bytes(total_size)
        );
    }
}
