use crate::domain::{
    DuplicateGroup, ScanChunk, ScanChunkData, ScanComplete,
    ScanError as DomainScanError, ScanProgress,
};
use crate::usecases::analytics::aggregator::{calculate_wasted_space, HashGrouping, SizeGrouping};
use sha2::Digest;
use std::io::Read;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::{mpsc, oneshot};
use walkdir::WalkDir;

/// Orchestrates duplicate file detection.
///
/// Uses the 3-stage funnel:
/// 1. Group by exact size (O(1) metadata — eliminates 99% of candidates)
/// 2. Hash first 8KB (cheap filter)
/// 3. Full SHA-256 (expensive but rare by this point)
///
/// MPSC channel ensures `window.emit()` runs on the async executor.
pub struct DuplicatesUseCase;

/// Inter-thread orchestration enum for duplicate scans.
enum ScanStep {
    Progress { percentage: f64, message: String },
    Group(DuplicateGroup),
    Complete {
        group_count: u64,
        total_wasted: u64,
    },
    Cancelled,
}

impl DuplicatesUseCase {
    /// Find duplicate files using the 3-stage funnel.
    ///
    /// Returns a future that resolves when the scan (and emission) is fully complete.
    pub fn run(
        window: tauri::WebviewWindow,
        path: String,
        cancel: Arc<AtomicBool>,
        start: Instant,
        scan_id: String,
    ) -> impl std::future::Future<Output = ()> {
        let (done_tx, done_rx) = oneshot::channel();
        let (tx, mut rx) = mpsc::unbounded_channel();

        let cancel_walk = cancel.clone();
        let path_walk = path.clone();

        tokio::task::spawn_blocking(move || {
            // ── Stage 1: Group files by exact size ──
            let mut size_grouping = SizeGrouping::new();

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

                        if !meta.is_file() || meta.len() == 0 {
                            continue;
                        }

                        let file_path = e.path().to_string_lossy().to_string();
                        size_grouping.record(file_path, meta.len());
                    }
                    Err(_) => continue,
                }
            }

            let (potential, total_files) = size_grouping.into_candidates();
            let potential_count = SizeGrouping::candidate_file_count(&potential);

            let _ = tx.send(ScanStep::Progress {
                percentage: 33.0,
                message: format!(
                    "Stage 1: {} files scanned, {} size matches ({:.1}% reduction)",
                    total_files,
                    potential_count,
                    if total_files > 0 {
                        (potential_count as f64 / total_files as f64) * 100.0
                    } else {
                        0.0
                    }
                ),
            });

            // ── Stage 2: Hash first 8KB ──
            let mut prefix_grouping = HashGrouping::new();

            for (_, paths) in potential {
                if cancel_walk.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send(ScanStep::Cancelled);
                    return;
                }

                for path_str in &paths {
                    if let Some(hash) = hash_prefix(path_str) {
                        prefix_grouping.record(hash, path_str.clone());
                    }
                }
            }

            let prefix_matches = prefix_grouping.into_matches();
            let prefix_count = HashGrouping::match_file_count(&prefix_matches);

            let _ = tx.send(ScanStep::Progress {
                percentage: 66.0,
                message: format!(
                    "Stage 2: {} prefix matches ({:.1}% of size matches)",
                    prefix_count,
                    if potential_count > 0 {
                        (prefix_count as f64 / potential_count as f64) * 100.0
                    } else {
                        0.0
                    }
                ),
            });

            // ── Stage 3: Full SHA-256 ──
            let mut full_grouping = HashGrouping::new();

            for (_, paths) in prefix_matches {
                if cancel_walk.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send(ScanStep::Cancelled);
                    return;
                }

                for path_str in &paths {
                    if let Some(hash) = hash_full(path_str) {
                        full_grouping.record(hash, path_str.clone());
                    }
                }
            }

            // ── Emit confirmed duplicate groups ──
            let full_matches = full_grouping.into_matches();
            let mut group_count: u64 = 0;
            let mut total_wasted: u64 = 0;

            for (_, paths) in full_matches {
                if paths.len() < 2 {
                    continue;
                }

                let size_each = match std::fs::metadata(&paths[0]) {
                    Ok(m) => m.len(),
                    Err(_) => continue,
                };

                let wasted = calculate_wasted_space(size_each, paths.len());
                total_wasted += wasted;

                let group = DuplicateGroup {
                    hash: String::new(),
                    size_each,
                    files: paths,
                    wasted_space: wasted,
                };

                if tx.send(ScanStep::Group(group)).is_err() {
                    return;
                }

                group_count += 1;
            }

            let _ = tx.send(ScanStep::Complete {
                group_count,
                total_wasted,
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
                    ScanStep::Progress {
                        percentage,
                        message,
                    } => {
                        let _ = window.emit(
                            "scan:progress",
                            ScanProgress {
                                scan_id: scan_id_clone.clone(),
                                percentage,
                                message,
                            },
                        );
                    }
                    ScanStep::Group(group) => {
                        let _ = window.emit(
                            "scan:chunk",
                            ScanChunk {
                                scan_id: scan_id_clone.clone(),
                                data: ScanChunkData::DuplicateGroup { group },
                            },
                        );
                    }
                    ScanStep::Complete {
                        group_count,
                        total_wasted,
                    } => {
                        let duration = start_clone.elapsed().as_millis() as u64;
                        let _ = window.emit(
                            "scan:complete",
                            ScanComplete {
                                scan_id: scan_id_clone.clone(),
                                total_items: group_count,
                                total_size: total_wasted,
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

/// Hash the first 8KB of a file using SHA-256.
/// Returns None if the file cannot be read.
fn hash_prefix(path: &str) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0u8; 8192];
    let bytes_read = file.read(&mut buffer).ok()?;
    hasher.update(&buffer[..bytes_read]);
    let result = hasher.finalize();
    Some(format!("{:x}", result))
}

/// Full SHA-256 hash of a file.
/// Returns None if the file cannot be read.
fn hash_full(path: &str) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let bytes = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return None,
        };
        hasher.update(&buffer[..bytes]);
    }
    let result = hasher.finalize();
    Some(format!("{:x}", result))
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
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_file(content: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        file.write_all(content).expect("Failed to write to temp file");
        file.flush().expect("Failed to flush temp file");
        file
    }

    #[test]
    fn hash_prefix_returns_hash_for_small_file() {
        let file = create_test_file(b"hello world");
        let hash = hash_prefix(file.path().to_str().unwrap());
        assert!(hash.is_some());
        assert_eq!(hash.unwrap().len(), 64);
    }

    #[test]
    fn hash_prefix_returns_hash_for_large_file() {
        let content = vec![0xAB; 16384];
        let file = create_test_file(&content);
        let hash = hash_prefix(file.path().to_str().unwrap());
        assert!(hash.is_some());
    }

    #[test]
    fn hash_prefix_same_content_same_hash() {
        let file1 = create_test_file(b"identical content");
        let file2 = create_test_file(b"identical content");
        assert_eq!(
            hash_prefix(file1.path().to_str().unwrap()),
            hash_prefix(file2.path().to_str().unwrap())
        );
    }

    #[test]
    fn hash_prefix_different_content_different_hash() {
        let file1 = create_test_file(b"content a");
        let file2 = create_test_file(b"content b");
        assert_ne!(
            hash_prefix(file1.path().to_str().unwrap()),
            hash_prefix(file2.path().to_str().unwrap())
        );
    }

    #[test]
    fn hash_prefix_returns_none_for_missing_file() {
        assert!(hash_prefix("/nonexistent/path/file.txt").is_none());
    }

    #[test]
    fn hash_full_returns_hash_for_small_file() {
        let file = create_test_file(b"hello world");
        let hash = hash_full(file.path().to_str().unwrap());
        assert!(hash.is_some());
        assert_eq!(hash.unwrap().len(), 64);
    }

    #[test]
    fn hash_full_same_as_prefix_for_small_file() {
        let file = create_test_file(b"small file content");
        let prefix = hash_prefix(file.path().to_str().unwrap());
        let full = hash_full(file.path().to_str().unwrap());
        assert_eq!(prefix, full);
    }

    #[test]
    fn hash_full_returns_none_for_missing_file() {
        assert!(hash_full("/nonexistent/path/file.txt").is_none());
    }
}
