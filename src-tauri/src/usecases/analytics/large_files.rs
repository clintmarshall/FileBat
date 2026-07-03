use crate::domain::{
    Entry, EntryType, ScanChunk, ScanChunkData, ScanComplete,
    ScanError as DomainScanError, ScanProgress,
};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};
use walkdir::WalkDir;

/// Orchestrates large file detection scans.
///
/// Uses an MPSC channel so `window.emit()` calls happen on the async executor,
/// not inside the blocking thread pool.
pub struct LargeFilesUseCase;

/// Inter-thread orchestration enum for large file scans.
enum ScanStep {
    File(Entry),
    Progress { scanned: u64, found: usize },
    Complete {
        total_items: u64,
        total_size: u64,
    },
    Cancelled,
}

impl LargeFilesUseCase {
    /// Find large files in a directory tree.
    ///
    /// Returns a future that resolves when the scan (and emission) is fully complete.
    pub fn run(
        window: tauri::WebviewWindow,
        path: String,
        min_size: u64,
        max_results: usize,
        cancel: Arc<AtomicBool>,
        start: Instant,
        scan_id: String,
    ) -> impl std::future::Future<Output = ()> {
        let (done_tx, done_rx) = oneshot::channel();
        let (tx, mut rx) = mpsc::unbounded_channel();

        let cancel_walk = cancel.clone();
        let path_walk = path.clone();

        tokio::task::spawn_blocking(move || {
            let mut results: Vec<Entry> = Vec::new();
            let mut scanned: u64 = 0;

            for entry in WalkDir::new(&path_walk) {
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

                        if !meta.is_file() {
                            continue;
                        }

                        scanned += 1;
                        let file_size = meta.len();

                        if file_size >= min_size {
                            let p = e.path();
                            let name = p
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();

                            let modified = meta
                                .modified()
                                .map(|t| {
                                    let dt = chrono::DateTime::<chrono::Local>::from(t);
                                    dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
                                })
                                .unwrap_or_default();

                            let entry = Entry {
                                name,
                                path: p.to_string_lossy().to_string(),
                                size: file_size,
                                modified,
                                entry_type: EntryType::File,
                                extension: p
                                    .extension()
                                    .map(|ext| ext.to_string_lossy().to_string()),
                            };

                            results.push(entry.clone());

                            if tx.send(ScanStep::File(entry)).is_err() {
                                return;
                            }

                            if results.len() >= max_results {
                                break;
                            }
                        }

                        if scanned % 5000 == 0 {
                            let _ = tx.send(ScanStep::Progress {
                                scanned,
                                found: results.len(),
                            });
                        }
                    }
                    Err(_) => continue,
                }
            }

            results.sort_by(|a, b| b.size.cmp(&a.size));

            let total_size: u64 = results.iter().map(|e| e.size).sum();

            let _ = tx.send(ScanStep::Complete {
                total_items: results.len() as u64,
                total_size,
            });
        });

        let start_clone = start.clone();
        let scan_id_clone = scan_id.clone();
        let cancel_clone = cancel.clone();

        tokio::spawn(async move {
            while let Some(step) = rx.recv().await {
                if cancel_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    emit_cancelled(&window, &scan_id_clone);
                    break;
                }

                match step {
                    ScanStep::File(entry) => {
                        let _ = window.emit(
                            "scan:chunk",
                            ScanChunk {
                                scan_id: scan_id_clone.clone(),
                                data: ScanChunkData::LargeFile { entry },
                            },
                        );
                    }
                    ScanStep::Progress { scanned, found } => {
                        let _ = window.emit(
                            "scan:progress",
                            ScanProgress {
                                scan_id: scan_id_clone.clone(),
                                percentage: 100.0,
                                message: format!(
                                    "Scanned {} files, found {} large files",
                                    scanned, found
                                ),
                            },
                        );
                    }
                    ScanStep::Complete {
                        total_items,
                        total_size,
                    } => {
                        let duration = start_clone.elapsed().as_millis() as u64;
                        let _ = window.emit(
                            "scan:complete",
                            ScanComplete {
                                scan_id: scan_id_clone.clone(),
                                total_items,
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

fn emit_cancelled(window: &tauri::WebviewWindow, scan_id: &str) {
    let _ = window.emit(
        "scan:error",
        DomainScanError {
            scan_id: scan_id.to_string(),
            message: "Scan cancelled".into(),
        },
    );
}
