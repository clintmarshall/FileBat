//! Single-pass disk usage scan using the `ignore` crate.
//!
//! Replaces the two-phase BFS + crossbeam + WalkDir approach with
//! `ignore::WalkParallel` — one parallel walk, metadata is free from readdir.
//!
//! Feature-gated behind `ignore-walker`. Toggle with:
//! ```bash
//! cargo run --features ignore-walker  # new implementation
//! cargo run                           # original implementation
//! ```

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
/// Files contribute size to their parent folder. Folders contribute to their parent.
/// Atomic operations allow concurrent updates from multiple walker threads.
struct FolderAccum {
    size: AtomicU64,
    file_count: AtomicU64,
    folder_count: AtomicU64,
}

impl FolderAccum {
    fn new() -> Self {
        Self {
            size: AtomicU64::new(0),
            file_count: AtomicU64::new(0),
            folder_count: AtomicU64::new(0),
        }
    }
}

enum ScanStep {
    Started(ScanTreeStarted),
    ChildrenReady(ScanTreeChildren),
    Folder(FolderUsage),
    Complete,
    Cancelled,
}

/// Orchestrates disk usage scans using the ignore crate.
pub struct DiskUsageUseCase;

impl DiskUsageUseCase {
    /// Single-pass scan using `ignore::WalkParallel`.
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
            let folders_ref = folders.clone();

            // Initialize root
            {
                let mut f = folders.lock().unwrap();
                f.insert(root_path.clone(), (root_id, FolderAccum::new()));
            }

            // Walk the tree in parallel using ignore crate
            // The run method takes a factory that creates a closure for each worker thread.
            // Each closure processes entries on its dedicated thread.
            // Disable .gitignore respect — we're scanning disk usage, not source code.
            let walker = ignore::WalkBuilder::new(&path_walk)
                .ignore(false)
                .build_parallel();
            let folders_clone = folders_ref.clone();
            let arena_arc = Arc::new(std::sync::Mutex::new(arena));
            let arena_clone = arena_arc.clone();
            let cancel_clone = cancel_blocking.clone();
            let root_path_clone = root_path.clone();

            walker.run(|| {
                let folders = folders_clone.clone();
                let arena = arena_clone.clone();
                let cancel = cancel_clone.clone();
                let root_path = root_path_clone.clone();

                Box::new(move |entry: Result<ignore::DirEntry, ignore::Error>| {
                    if cancel.load(Ordering::Relaxed) {
                        return ignore::WalkState::Quit;
                    }

                    match entry {
                        Ok(entry) => {
                            let path = entry.path();

                            match entry.file_type() {
                                Some(ft) if ft.is_file() => {
                                    // File — add size to parent folder
                                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                                    let parent = path.parent().map(|p| normalize_path(p));

                                    if let Some(parent_path) = parent {
                                        let f = folders.lock().unwrap();
                                        if let Some((_, accum)) = f.get(&parent_path) {
                                            accum.size.fetch_add(size, Ordering::Relaxed);
                                            accum.file_count.fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                }
                                Some(ft) if ft.is_dir() => {
                                    // Directory — record it. Parent linking happens in a post-walk pass
                                    // because parallel traversal doesn't guarantee parent-first order.
                                    // Skip the root directory — it was added before the walk.
                                    let normalized = normalize_path(path);
                                    if normalized == root_path {
                                        return ignore::WalkState::Continue;
                                    }

                                    let name = path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default();

                                    {
                                        let mut a = arena.lock().unwrap();
                                        let node_id = a.alloc_folder(&name);

                                        // Record this folder — linking happens later
                                        {
                                            let mut f = folders.lock().unwrap();
                                            f.insert(normalized, (node_id, FolderAccum::new()));
                                        }
                                    }
                                }
                                _ => {}
                            }

                            ignore::WalkState::Continue
                        }
                        Err(_) => ignore::WalkState::Continue, // Skip inaccessible entries
                    }
                })
            });

            // Walk complete — collect results and emit events
            // Drop the walk's arena reference before post-processing
            drop(arena_clone);
            drop(cancel_clone);
            drop(folders_clone);

            // Post-walk: link parents and children using path relationships.
            // Parallel traversal doesn't guarantee parent-first order, so we do this now.
            // Collect parent-child pairs first, then apply to arena (avoids lock ordering issues).
            let folders_map = folders_ref.lock().unwrap();

            let mut parent_child_pairs: Vec<(NodeId, NodeId)> = Vec::new();

            for (path, (node_id, _)) in folders_map.iter() {
                if let Some(parent_path) = path.rsplit_once('/').map(|(p, _)| p) {
                    if let Some((parent_id, _)) = folders_map.get(parent_path) {
                        parent_child_pairs.push((*parent_id, *node_id));
                    }
                }
            }

            // Apply links to arena
            {
                let mut arena = arena_arc.lock().unwrap();
                for (parent_id, child_id) in &parent_child_pairs {
                    arena.add_child(*parent_id, *child_id);
                }
            }

             // Take ownership of arena for frontend queries
            let arena = std::mem::replace(
                &mut *arena_arc.lock().unwrap(),
                FolderArena::new(),
            );

            let mut final_arena = arena;
            let _structural = final_arena.freeze_structural();

            // Store arena for frontend queries
            let arena_arc = Arc::new(std::sync::Mutex::new(final_arena));
            {
                let mut tree_map = tree_blocking.lock().unwrap();
                tree_map.insert(scan_id_blocking.clone(), arena_arc.clone());
            }

            // Emit started event
            let _ = tx.send(ScanStep::Started(ScanTreeStarted {
                scan_id: scan_id_blocking.clone(),
                root_id,
                root_path: root_path.clone(),
                root_name: root_name.clone(),
            }));

            // Emit children_ready events for folders with children
            // Count children per parent from the pairs we collected
            let mut child_counts: std::collections::HashMap<NodeId, u32> =
                std::collections::HashMap::new();
            for (parent_id, _) in &parent_child_pairs {
                *child_counts.entry(*parent_id).or_insert(0) += 1;
            }

            for (parent_id, count) in child_counts {
                let _ = tx.send(ScanStep::ChildrenReady(ScanTreeChildren {
                    scan_id: scan_id_blocking.clone(),
                    parent_id,
                    child_count: count,
                }));
            }

            // Brief pause to let the frontend process children_ready events
            // and fetch children via get_scan_tree_children before folder_usage arrives.
            std::thread::sleep(std::time::Duration::from_millis(500));

            // Emit folder usage for all discovered folders
            for (_path, (node_id, accum)) in folders_map.iter() {
                let size = accum.size.load(Ordering::SeqCst);
                let file_count = accum.file_count.load(Ordering::SeqCst);
                let folder_count_val = accum.folder_count.load(Ordering::SeqCst);

                let _ = tx.send(ScanStep::Folder(FolderUsage {
                    node_id: *node_id,
                    size,
                    file_count,
                    folder_count: folder_count_val,
                }));
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
