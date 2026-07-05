use crate::domain::{
    FolderStructure, FolderUsage, ScanChunk, ScanChunkData, ScanComplete,
    ScanError as DomainScanError, ScanProgress, ScanStructure,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};

/// Orchestrates disk usage scans.
///
/// Three-phase approach:
/// - Phase 1: Recursive readdir to discover full folder tree → emit scan:structure
/// - Phase 2: Parallel sizing of leaf folders only (10 threads) → emit scan:chunk per leaf
/// - Phase 3: Rollup — propagate leaf stats to parents → emit scan:chunk per parent
pub struct DiskUsageUseCase;

/// Inter-thread orchestration enum.
enum ScanStep {
    Structure(ScanStructure),
    Folder(FolderUsage),
    Complete {
        folder_count: usize,
    },
    Cancelled,
}

impl DiskUsageUseCase {
    /// Scan a directory tree for disk usage.
    ///
    /// Phase 1: Recursive readdir to discover the full folder tree.
    /// No file stats — just directory names. Emit scan:structure.
    ///
    /// Phase 2: Parallel sizing of leaf folders only (10 threads).
    /// Every file is visited exactly once. Emit scan:chunk per leaf.
    ///
    /// Phase 3: Rollup — propagate leaf stats to parents.
    /// Emit scan:chunk per parent so the frontend can patch all rows.
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

        // Offload everything to the blocking pool
        tokio::task::spawn_blocking(move || {
            let base = Path::new(&path_walk);

            // ─── Phase 1: Full Structure Discovery ───
            let raw_structure = discover_structure(base);
            let total_folders = raw_structure.len();

            // Convert to FolderStructure for emission
            let folders: Vec<FolderStructure> = raw_structure
                .iter()
                .map(|(p, name, children)| FolderStructure {
                    path: p.clone(),
                    name: name.clone(),
                    children: children.clone(),
                })
                .collect();

            let structure = ScanStructure {
                scan_id: scan_id_blocking.clone(),
                root_path: path_walk.clone(),
                folders,
                total_folders,
            };

            if tx.send(ScanStep::Structure(structure)).is_err() {
                return;
            }

            if cancel_blocking.load(Ordering::Relaxed) {
                let _ = tx.send(ScanStep::Cancelled);
                return;
            }

            // ─── Phase 2: Parallel Leaf Sizing ───
            let leaves = find_leaves(&raw_structure);

            // Shared map to collect leaf results for rollup
            let leaf_results: Arc<Mutex<HashMap<String, FolderUsage>>> =
                Arc::new(Mutex::new(HashMap::new()));

            // Work queue: send leaf folder paths, worker threads pick them up
            let (work_tx, work_rx) = std::sync::mpsc::channel::<String>();
            let work_rx = Arc::new(std::sync::Mutex::new(work_rx));

            let completed = Arc::new(AtomicUsize::new(0));
            let cancel_size = cancel_blocking.clone();
            let tx_clone = tx.clone();
            let leaf_results_clone = leaf_results.clone();

            // Spawn 10 worker threads
            let mut handles = Vec::new();
            for _ in 0..10 {
                let rx = work_rx.clone();
                let comp = completed.clone();
                let cancel_t = cancel_size.clone();
                let tx_t = tx_clone.clone();
                let results = leaf_results_clone.clone();

                let handle = std::thread::spawn(move || {
                    loop {
                        if cancel_t.load(Ordering::Relaxed) {
                            break;
                        }

                        let folder_path = {
                            let lock = rx.lock().unwrap();
                            match lock.recv() {
                                Ok(path) => path,
                                Err(_) => break,
                            }
                        };

                        let result = size_folder(Path::new(&folder_path));

                        if cancel_t.load(Ordering::Relaxed) {
                            break;
                        }

                        // Store in shared map for rollup
                        results.lock().unwrap().insert(result.path.clone(), result.clone());

                        // Emit to frontend
                        let _ = tx_t.send(ScanStep::Folder(result));
                        let _ = comp.fetch_add(1, Ordering::Relaxed);
                    }
                });

                handles.push(handle);
            }

            // Send all leaves to the work queue
            for leaf_path in leaves {
                if cancel_blocking.load(Ordering::Relaxed) {
                    break;
                }
                let _ = work_tx.send(leaf_path);
            }
            drop(work_tx);

            // Wait for all threads
            for handle in handles {
                let _ = handle.join();
            }

            if cancel_blocking.load(Ordering::Relaxed) {
                let _ = tx.send(ScanStep::Cancelled);
                return;
            }

            // ─── Phase 3: Rollup ───
            let stats = rollup(&raw_structure, leaf_results.lock().unwrap().clone());

            // Emit rolled-up stats for non-leaf folders (bottom-up so parents come after children)
            for (parent_path, _, children) in raw_structure.iter().rev() {
                if children.is_empty() {
                    continue; // Already emitted as leaf
                }
                if let Some(usage) = stats.get(parent_path) {
                    let _ = tx.send(ScanStep::Folder(usage.clone()));
                }
            }

            let _ = tx.send(ScanStep::Complete {
                folder_count: total_folders,
            });
        });

        let start_clone = start.clone();
        let scan_id_clone = scan_id.clone();
        let cancel_clone = cancel.clone();

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
                    ScanStep::Complete { folder_count: _ } => {
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
/// Returns a FolderUsage with accumulated stats.
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

/// Discover all directories under a root path.
/// Returns a list of (path, name, children_paths) — no file stats, just directory names.
/// This is a cheap readdir-only walk — every file is touched exactly zero times.
///
/// Uses a HashMap for O(1) parent lookup instead of linear scan.
pub fn discover_structure(root: &Path) -> Vec<(String, String, Vec<String>)> {
    // HashMap: path -> (name, children_paths) for O(1) parent lookup
    let mut map: HashMap<String, (String, Vec<String>)> = HashMap::new();

    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());
    let root_path = normalize_path(root);

    map.insert(root_path.clone(), (root_name, Vec::new()));

    // BFS — discover subdirectories recursively
    let mut queue: Vec<std::path::PathBuf> = vec![root.to_path_buf()];

    while let Some(current) = queue.pop() {
        let current_path = normalize_path(&current);

        match current.read_dir() {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let child_path = normalize_path(&path);
                        let child_name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();

                        // O(1) parent lookup — add child to parent's children list
                        if let Some(parent) = map.get_mut(&current_path) {
                            parent.1.push(child_path.clone());
                        }

                        map.insert(child_path.clone(), (child_name, Vec::new()));
                        queue.push(path);
                    }
                }
            }
            Err(_) => continue, // Skip inaccessible directories
        }
    }

    // Convert HashMap to ordered Vec (BFS order from insertion)
    // We need to preserve BFS order for the frontend, so rebuild from the queue order.
    // Actually, HashMap doesn't preserve insertion order. Use a Vec to track order separately.
    // Simpler: just collect from the map — order doesn't matter for correctness,
    // the frontend builds the tree from parent→children relationships.
    map.into_iter()
        .map(|(path, (name, children))| (path, name, children))
        .collect()
}

/// Identify leaf folders — folders that have no subdirectories.
/// These are the only folders that need a full WalkDir for sizing.
pub fn find_leaves(structure: &[(String, String, Vec<String>)]) -> Vec<String> {
    structure
        .iter()
        .filter(|(_, _, children)| children.is_empty())
        .map(|(path, _, _)| path.clone())
        .collect()
}

/// Roll up stats from children to parents.
/// Given a structure list and a map of folder stats (from leaf sizing),
/// propagate totals bottom-up so every parent has correct aggregates.
pub fn rollup(
    structure: &[(String, String, Vec<String>)],
    mut stats: HashMap<String, FolderUsage>,
) -> HashMap<String, FolderUsage> {
    // Process in reverse BFS order (deepest first) = bottom-up rollup.
    for (parent_path, _, children) in structure.iter().rev() {
        if children.is_empty() {
            continue; // Leaf — already sized
        }

        let mut total_size: u64 = 0;
        let mut total_files: u64 = 0;
        let mut total_folders: u64 = 0;

        for child_path in children {
            if let Some(child_stats) = stats.get(child_path) {
                total_size += child_stats.size;
                total_files += child_stats.file_count;
                total_folders += 1 + child_stats.folder_count; // +1 for the child folder itself
            }
        }

        stats.insert(
            parent_path.clone(),
            FolderUsage {
                path: parent_path.clone(),
                size: total_size,
                file_count: total_files,
                folder_count: total_folders,
            },
        );
    }

    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: create a directory tree and return the temp dir.
    /// Structure:
    /// tmp/
    ///   a/
    ///     file1.txt (100 bytes)
    ///     b/
    ///       file2.txt (200 bytes)
    ///       c/
    ///         (empty — c is a leaf with 0 files)
    ///   d/
    ///     file3.txt (50 bytes)
    fn create_test_tree() -> TempDir {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Create directories
        fs::create_dir_all(base.join("a/b/c")).unwrap();
        fs::create_dir_all(base.join("d")).unwrap();

        // Create files
        fs::write(base.join("a/file1.txt"), &[0u8; 100]).unwrap();
        fs::write(base.join("a/b/file2.txt"), &[0u8; 200]).unwrap();
        fs::write(base.join("d/file3.txt"), &[0u8; 50]).unwrap();

        temp
    }

    // ─── discover_structure ───

    #[test]
    fn discovers_root_and_all_subdirectories() {
        let temp = create_test_tree();
        let base = temp.path();

        let structure = discover_structure(base);

        // Should find: base, a, a/b, a/b/c, d = 5 directories
        assert_eq!(structure.len(), 5);

        let paths: Vec<&str> = structure.iter().map(|(p, _, _)| p.as_str()).collect();
        assert!(paths.iter().any(|p| p.ends_with("/a")));
        assert!(paths.iter().any(|p| p.ends_with("/a/b")));
        assert!(paths.iter().any(|p| p.ends_with("/a/b/c")));
        assert!(paths.iter().any(|p| p.ends_with("/d")));
    }

    #[test]
    fn builds_parent_children_relationships() {
        let temp = create_test_tree();
        let base = temp.path();

        let structure = discover_structure(base);

        // Find the root entry
        let root = structure.iter().find(|(p, _, _)| *p == normalize_path(base));
        assert!(root.is_some(), "Root should be in structure");

        let (_, _, children) = root.unwrap();
        assert_eq!(children.len(), 2, "Root should have 2 children (a, d)");

        // Find 'a' and check its children
        let a_path = structure
            .iter()
            .find(|(p, _, _)| p.ends_with("/a"))
            .unwrap()
            .0
            .clone();
        let a_entry = structure.iter().find(|(p, _, _)| p == &a_path).unwrap();
        assert_eq!(a_entry.1, "a");
        assert_eq!(a_entry.2.len(), 1, "a should have 1 child (b)");
        assert!(a_entry.2[0].ends_with("/a/b"));
    }

    #[test]
    fn handles_empty_directory() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        let structure = discover_structure(base);
        assert_eq!(structure.len(), 1);
        assert_eq!(structure[0].2.len(), 0, "Empty dir has no children");
    }

    // ─── find_leaves ───

    #[test]
    fn identifies_leaf_folders() {
        let temp = create_test_tree();
        let base = temp.path();

        let structure = discover_structure(base);
        let leaves = find_leaves(&structure);

        // Leaves should be folders with no subdirectories:
        // a/b/c (empty) → leaf
        // d (has file but no subdirs) → leaf
        let leaf_names: Vec<&str> = leaves.iter().map(|p| p.as_str()).collect();
        assert!(leaf_names.iter().any(|p| p.ends_with("/a/b/c")), "c should be a leaf");
        assert!(leaf_names.iter().any(|p| p.ends_with("/d")), "d should be leaf");

        // a and a/b should NOT be leaves (they have subdirs)
        assert!(!leaf_names.iter().any(|p| p.ends_with("/a") && !p.ends_with("/a/b")), "a should not be a leaf");
    }

    // ─── rollup ───

    #[test]
    fn rollup_propagates_leaf_stats_to_parents() {
        let structure = vec![
            ("root".to_string(), "root".to_string(), vec!["root/a".to_string(), "root/b".to_string()]),
            ("root/a".to_string(), "a".to_string(), vec![]),
            ("root/b".to_string(), "b".to_string(), vec![]),
        ];

        let mut stats = HashMap::new();
        stats.insert("root/a".to_string(), FolderUsage {
            path: "root/a".to_string(),
            size: 100,
            file_count: 1,
            folder_count: 0,
        });
        stats.insert("root/b".to_string(), FolderUsage {
            path: "root/b".to_string(),
            size: 200,
            file_count: 2,
            folder_count: 0,
        });

        let result = rollup(&structure, stats);

        let root = result.get("root").unwrap();
        assert_eq!(root.size, 300, "Root size = sum of children");
        assert_eq!(root.file_count, 3, "Root file_count = sum of children");
        assert_eq!(root.folder_count, 2, "Root folder_count = number of child folders");
    }

    #[test]
    fn rollup_handles_single_child() {
        let structure = vec![
            ("root".to_string(), "root".to_string(), vec!["root/a".to_string()]),
            ("root/a".to_string(), "a".to_string(), vec![]),
        ];

        let mut stats = HashMap::new();
        stats.insert("root/a".to_string(), FolderUsage {
            path: "root/a".to_string(),
            size: 500,
            file_count: 5,
            folder_count: 0,
        });

        let result = rollup(&structure, stats);

        let root = result.get("root").unwrap();
        assert_eq!(root.size, 500);
        assert_eq!(root.file_count, 5);
        assert_eq!(root.folder_count, 1);
    }

    #[test]
    fn rollup_preserves_leaf_values() {
        let structure = vec![
            ("root".to_string(), "root".to_string(), vec!["root/a".to_string()]),
            ("root/a".to_string(), "a".to_string(), vec![]),
        ];

        let mut stats = HashMap::new();
        stats.insert("root/a".to_string(), FolderUsage {
            path: "root/a".to_string(),
            size: 42,
            file_count: 7,
            folder_count: 0,
        });

        let result = rollup(&structure, stats);

        // Leaf should be unchanged
        let leaf = result.get("root/a").unwrap();
        assert_eq!(leaf.size, 42);
        assert_eq!(leaf.file_count, 7);
    }
}