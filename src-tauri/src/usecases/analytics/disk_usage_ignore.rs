//! Single-pass disk usage scan using `jwalk` for parallel traversal.
//!
//! `jwalk` uses Rayon internally for parallelism and doesn't filter files based on .gitignore,
//! making it suitable for disk usage scanning where we need to see ALL files.

use crate::domain::{
    FolderUsage, NodeId, ScanChunk, ScanChunkData, ScanComplete,
    ScanError as DomainScanError, ScanProgress, ScanTreeChildren, ScanTreeStarted,
};
use crate::usecases::analytics::arena::FolderArena;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};

/// Thread-safe folder accumulator.
struct FolderAccum {
    size: AtomicU64,
    file_count: AtomicU64,
}

impl FolderAccum {
    fn new() -> Self {
        Self {
            size: AtomicU64::new(0),
            file_count: AtomicU64::new(0),
        }
    }
}

enum ScanStep {
    Started(ScanTreeStarted),
    ChildrenReady(ScanTreeChildren),
    Folder(FolderUsage),
    Complete {
        scan_id: String,
        total_files: u64,
        total_size: u64,
        folder_count: usize,
    },
    Cancelled,
}

/// Orchestrates disk usage scans using jwalk for parallel traversal.
pub struct DiskUsageUseCase;

impl DiskUsageUseCase {
    /// Single-pass scan using `jwalk` parallel walker.
    pub async fn run(
        window: tauri::WebviewWindow,
        path: String,
        _max_depth: u32,
        cancel: Arc<AtomicBool>,
        start: Instant,
        scan_id: String,
        tree: Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::Mutex<FolderArena>>>>>,
    ) {
        let (done_tx, done_rx) = oneshot::channel();
        let (tx, mut rx) = mpsc::unbounded_channel::<ScanStep>();

        let path_walk = path.clone();
        let scan_id_blocking = scan_id.clone();
        let cancel_blocking = cancel.clone();
        let tree_blocking = tree.clone();

        tokio::task::spawn_blocking(move || {
            let base = Path::new(&path_walk);
            let root_name = base
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path_walk.clone());
            let root_path = normalize_path(base);

            // Build arena
            let mut arena = FolderArena::new();
            let root_id = arena.alloc_folder(&root_name);

            // Track folders: path -> (NodeId, Accumulator)
            let folders: Arc<std::sync::Mutex<HashMap<String, (NodeId, FolderAccum)>>> =
                Arc::new(std::sync::Mutex::new(HashMap::new()));

            // Initialize root
            {
                let mut f = folders.lock().unwrap();
                f.insert(root_path.clone(), (root_id, FolderAccum::new()));
            }

            // Wrap arena in Arc<Mutex> for shared access during parallel walk
            let arena_arc = Arc::new(std::sync::Mutex::new(arena));

            // Debug counters
            let counter_files = Arc::new(AtomicU64::new(0));
            let counter_dirs = Arc::new(AtomicU64::new(0));
            let counter_size = Arc::new(AtomicU64::new(0));
            let counter_parent_found = Arc::new(AtomicU64::new(0));
            let counter_parent_missing = Arc::new(AtomicU64::new(0));
            let counter_errors = Arc::new(AtomicU64::new(0));

            // Parallel walk with jwalk — parallel by default (Rayon internally)
            jwalk::WalkDir::new(&path_walk)
                .into_iter()
                .for_each({
                    let folders = folders.clone();
                    let arena = arena_arc.clone();
                    let root = root_path.clone();
                    let cancel = cancel_blocking.clone();
                    let c_files = counter_files.clone();
                    let c_dirs = counter_dirs.clone();
                    let c_size = counter_size.clone();
                    let c_pf = counter_parent_found.clone();
                    let c_pm = counter_parent_missing.clone();
                    let c_err = counter_errors.clone();
                    move |result: Result<jwalk::DirEntry<((), ())>, jwalk::Error>| {
                        if cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        match result {
                            Ok(ref e) => {
                                let meta = match e.metadata() {
                                    Ok(m) => m,
                                    Err(_) => {
                                        c_err.fetch_add(1, Ordering::Relaxed);
                                        return;
                                    }
                                };
                                let entry_path = normalize_path(&e.path());

                                if meta.is_dir() {
                                    // Skip root — already registered
                                    if entry_path == root {
                                        c_dirs.fetch_add(1, Ordering::Relaxed);
                                        return;
                                    }
                                    // Register new directory in arena and folders map
                                    let name = e.path()
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    let node_id = {
                                        let mut a = arena.lock().unwrap();
                                        a.alloc_folder(&name)
                                    };
                                    {
                                        let mut f = folders.lock().unwrap();
                                        f.insert(entry_path, (node_id, FolderAccum::new()));
                                    }
                                    c_dirs.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    // File — accumulate to parent folder
                                    let size = meta.len();
                                    c_size.fetch_add(size, Ordering::Relaxed);
                                    c_files.fetch_add(1, Ordering::Relaxed);
                                    let parent = match e.path().parent().map(|p| normalize_path(p)) {
                                        Some(p) => p,
                                        None => {
                                            c_pm.fetch_add(1, Ordering::Relaxed);
                                            return;
                                        }
                                    };
                                    let f = folders.lock().unwrap();
                                    if let Some((_, accum)) = f.get(&parent) {
                                        accum.size.fetch_add(size, Ordering::Relaxed);
                                        accum.file_count.fetch_add(1, Ordering::Relaxed);
                                        c_pf.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        c_pm.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                            Err(_) => {
                                c_err.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                });

            // Walk complete — debug stats
            let walk_files = counter_files.load(Ordering::Relaxed);
            let walk_dirs = counter_dirs.load(Ordering::Relaxed);
            let walk_size = counter_size.load(Ordering::Relaxed);
            let walk_pf = counter_parent_found.load(Ordering::Relaxed);
            let walk_pm = counter_parent_missing.load(Ordering::Relaxed);
            let walk_err = counter_errors.load(Ordering::Relaxed);
            let gb = walk_size as f64 / 1024.0 / 1024.0 / 1024.0;
            println!(
                "[SCAN] jwalk: files={} dirs={} size={:.1}GB parent_found={} parent_missing={} errors={}",
                walk_files, walk_dirs, gb, walk_pf, walk_pm, walk_err
            );

            // Post-walk: link parents and children using path relationships
            let folders_map = folders.lock().unwrap();
            let mut parent_child_pairs: Vec<(NodeId, NodeId)> = Vec::new();
            for (path, (node_id, _)) in folders_map.iter() {
                if let Some(parent_path) = path.rsplit_once('/').map(|(p, _)| p) {
                    if let Some((parent_id, _)) = folders_map.get(parent_path) {
                        parent_child_pairs.push((*parent_id, *node_id));
                    }
                }
            }
            let pairs_clone = parent_child_pairs.clone();
            drop(folders_map);

            // Apply links to arena
            {
                let mut arena = arena_arc.lock().unwrap();
                for (parent_id, child_id) in &pairs_clone {
                    arena.add_child(*parent_id, *child_id);
                }
            }

            // Take ownership of arena for frontend queries
            let arena = std::mem::replace(
                &mut *arena_arc.lock().unwrap(),
                FolderArena::new(),
            );
            let mut final_arena = arena;
            final_arena.freeze_structural();

            // Store arena for frontend queries
            let arena_for_tree = Arc::new(std::sync::Mutex::new(final_arena));
            {
                let mut tree_map = tree_blocking.lock().unwrap();
                tree_map.insert(scan_id_blocking.clone(), arena_for_tree.clone());
            }

            // Get structural data for rollup
            let structural = {
                let guard = arena_for_tree.lock().unwrap();
                let arc = guard.structural_ref().unwrap();
                arc.clone()
            };

            // Re-lock folders for rollup
            let folders_map = folders.lock().unwrap();

            // Emit started event
            let _ = tx.send(ScanStep::Started(ScanTreeStarted {
                scan_id: scan_id_blocking.clone(),
                root_id,
                root_path: root_path.clone(),
                root_name: root_name.clone(),
            }));

            // Emit children_ready events
            let mut child_counts: HashMap<NodeId, u32> = HashMap::new();
            for (parent_id, _) in &pairs_clone {
                *child_counts.entry(*parent_id).or_insert(0) += 1;
            }
            for (parent_id, count) in child_counts {
                let _ = tx.send(ScanStep::ChildrenReady(ScanTreeChildren {
                    scan_id: scan_id_blocking.clone(),
                    parent_id,
                    child_count: count,
                }));
            }

            // Brief pause for frontend to process children_ready events
            std::thread::sleep(std::time::Duration::from_millis(500));

            // Bottom-up rollup: propagate children's stats to parents
            let mut rolled_size: HashMap<NodeId, u64> = HashMap::new();
            let mut rolled_files: HashMap<NodeId, u64> = HashMap::new();
            let mut rolled_folders: HashMap<NodeId, u64> = HashMap::new();

            // Initialize with immediate values from the walk
            for (_path, (node_id, accum)) in folders_map.iter() {
                rolled_size.insert(*node_id, accum.size.load(Ordering::SeqCst));
                rolled_files.insert(*node_id, accum.file_count.load(Ordering::SeqCst));
                rolled_folders.insert(*node_id, 0);
            }

            // Iterative post-order DFS
            let mut traversal = vec![root_id];
            let mut post_order = Vec::new();
            while let Some(node_id) = traversal.pop() {
                post_order.push(node_id);
                let mut child = structural.first_child[node_id.0 as usize];
                while let Some(c) = child {
                    traversal.push(c);
                    child = structural.next_sibling[c.0 as usize];
                }
            }

            // Process in reverse traversal order (children before parents)
            for node_id in post_order.into_iter().rev() {
                if let Some(&Some(parent_id)) = structural.parent.get(node_id.0 as usize) {
                    let size = *rolled_size.get(&node_id).unwrap_or(&0);
                    let files = *rolled_files.get(&node_id).unwrap_or(&0);
                    let folders = *rolled_folders.get(&node_id).unwrap_or(&0);
                    *rolled_size.entry(parent_id).or_insert(0) += size;
                    *rolled_files.entry(parent_id).or_insert(0) += files;
                    *rolled_folders.entry(parent_id).or_insert(0) += 1 + folders;
                }
            }

            // Emit folder usage with rolled-up stats
            for (_path, (node_id, _)) in folders_map.iter() {
                let _ = tx.send(ScanStep::Folder(FolderUsage {
                    node_id: *node_id,
                    size: *rolled_size.get(node_id).unwrap_or(&0),
                    file_count: *rolled_files.get(node_id).unwrap_or(&0),
                    folder_count: *rolled_folders.get(node_id).unwrap_or(&0),
                }));
            }

            drop(folders_map);

            if cancel_blocking.load(Ordering::Relaxed) {
                let _ = tx.send(ScanStep::Cancelled);
                return;
            }

            let _ = tx.send(ScanStep::Complete {
                scan_id: scan_id_blocking.clone(),
                total_files: 0,
                total_size: 0,
                folder_count: 0,
            });
        });

        let start_clone = start.clone();
        let scan_id_clone = scan_id.clone();
        let cancel_clone = cancel.clone();

        // Async emitter
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
                    ScanStep::Complete {
                        scan_id,
                        total_files: _,
                        total_size: _,
                        folder_count: _,
                    } => {
                        let _ = window.emit(
                            "scan:progress",
                            ScanProgress {
                                scan_id,
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

        let _ = done_rx.await;
    }
}

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
