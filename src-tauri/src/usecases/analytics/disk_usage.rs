use crate::domain::{
    FolderUsage, ScanChunk, ScanChunkData, ScanComplete,
    ScanError as DomainScanError, ScanProgress,
};
use crate::usecases::analytics::aggregator::FolderUsageAccumulator;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};
use walkdir::WalkDir;

/// Orchestrates disk usage scans.
///
/// Responsible for: walkdir iteration, cancel checks, event emission.
/// The heavy accumulation math is delegated to [FolderUsageAccumulator].
pub struct DiskUsageUseCase;

/// Inter-thread orchestration enum.
/// Used to stream data back out of the `spawn_blocking` worker thread into the async main event pump.
enum ScanStep {
    Folder(FolderUsage),
    Complete {
        total_files: u64,
        folder_count: usize,
        total_size: u64,
    },
    Cancelled,
}

impl DiskUsageUseCase {
    /// Scan a directory tree for disk usage.
    ///
    /// **Tauri v2 Fix Note:** Instead of calling `window.emit()` continuously inside a blocking thread context
    /// (which starves the underlying Tokio executor loop and causes a complete freeze on the IPC bus),
    /// this implementation uses an unbounded MPSC channel. The blocking task iterates synchronously and pushes
    /// items to the channel, while a lightweight async task processes the channel back on the safe thread
    /// pool to emit events smoothly.
    ///
    /// Returns a future that resolves when the scan (and emission) is fully complete,
    /// so the caller can perform cleanup (unregister) at the right time.
    pub fn run(
        window: tauri::WebviewWindow,
        path: String,
        max_depth: u32,
        cancel: Arc<AtomicBool>,
        start: Instant,
        scan_id: String,
    ) -> impl std::future::Future<Output = ()> {

        let (done_tx, done_rx) = oneshot::channel();
        let (tx, mut rx) = mpsc::unbounded_channel();

        let cancel_walk = cancel.clone();
        let path_walk = path.clone();

        // 2. Offload the synchronous, heavy I/O filesystem work to Tokio's dedicated blocking pool.
        tokio::task::spawn_blocking(move || {
            let mut acc = FolderUsageAccumulator::new(&path_walk, max_depth);

            for entry in WalkDir::new(&path_walk) {
                // Cooperative Cancellation check during filesystem walking
                if cancel_walk.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send(ScanStep::Cancelled);
                    return;
                }

                match entry {
                    Ok(ref e) => {
                        let meta = match e.metadata() {
                            Ok(m) => m,
                            Err(_) => continue,
                        };
                        let p = e.path();

                        if meta.is_file() {
                            acc.record_file(p, meta.len());
                        } else if meta.is_dir() {
                            acc.record_directory(p);
                        }
                    }
                    Err(_) => continue, // skip permission or broken node errors safely
                }
            }

            // Consolidate aggregated figures from the accumulator
            let (folders, total_files) = acc.finalize();
            let total_size: u64 = folders.iter().map(|f| f.size).sum();
            let folder_count = folders.len();

            // Stream aggregated folders out through the channel.
            for folder in folders {
                if cancel_walk.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send(ScanStep::Cancelled);
                    return;
                }

                // If the frontend unmounted or crashed, the receiver (rx) breaks, and we abort.
                if tx.send(ScanStep::Folder(folder)).is_err() {
                    return;
                }
            }

            // Signal to the receiver that the hard iteration logic has finished successfully.
            let _ = tx.send(ScanStep::Complete {
                total_files,
                folder_count,
                total_size,
            });
        });

        let start_clone = start.clone();
        let scan_id_clone = scan_id.clone();
        let cancel_clone = cancel.clone();

        // 3. Spawn a lightweight async task on Tauri's core executor loop to drain the channel.
        // This is where window.emit calls are handled safely without freezing the interface.
        tokio::spawn(async move {
            while let Some(step) = rx.recv().await {
                // Secondary fallback cancellation safety gate
                if cancel_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    emit_cancelled(&window, &scan_id_clone);
                    break;
                }

                match step {
                    ScanStep::Folder(folder) => {
                        let _ = window.emit(
                            "scan:chunk",
                            ScanChunk {
                                scan_id: scan_id_clone.clone(),
                                data: ScanChunkData::FolderUsage { usage: folder },
                            },
                        );
                    }
                    ScanStep::Complete {
                        total_files,
                        folder_count,
                        total_size,
                    } => {
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
                                total_items: total_files as u64,
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
            // Signal that the receiver task is done (scan complete or cancelled)
            let _ = done_tx.send(());
        });

        // Return a future that resolves when the receiver task finishes
        async { let _ = done_rx.await; }
    }
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
