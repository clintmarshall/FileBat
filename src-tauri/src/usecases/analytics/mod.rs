mod aggregator;
mod disk_usage;
mod duplicates;
mod large_files;
mod snapshot;

use snapshot::SnapshotUseCase;

use crate::domain::{AppError, ScanTreeChild, UsageSnapshot};
use crate::infrastructure::SqliteAnalytics;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Orchestrates analytics operations: disk usage scans, large file detection,
/// duplicate finding, and usage snapshots.
///
/// Heavy operations spawn background threads and stream results via Tauri events.
/// The domain layer stays pure — Tauri coupling is confined to this use case layer.
///
/// This struct manages scan lifecycle (register/cancel/unregister) and delegates
/// the actual scan logic to dedicated use case structs.
pub struct AnalyticsUseCase {
    /// Shared state for tracking active scans (cancel flags).
    scans: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    /// Single tree store — parent_path → list of children per scan.
    /// Populated by disk_usage scan, queried by get_scan_tree_children.
    /// Replaces the old tree_state + parent_children duplication.
    tree: Arc<Mutex<HashMap<String, HashMap<String, Vec<ScanTreeChild>>>>>,
}

impl AnalyticsUseCase {
    pub fn new() -> Self {
        Self {
            scans: Arc::new(Mutex::new(HashMap::new())),
            tree: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get children for a folder in the tree.
    pub fn get_children(&self, scan_id: &str, parent_path: &str) -> Option<Vec<ScanTreeChild>> {
        let t = self.tree.lock().unwrap();
        t.get(scan_id)?
            .get(parent_path)
            .cloned()
    }

    /// Generate a unique scan id.
    fn scan_id() -> String {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        format!("scan_{}_{}", n, t)
    }

    /// Register a new cancellable scan.
    fn register(&self, id: &str) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        self.scans.lock().unwrap().insert(id.to_string(), cancel.clone());
        cancel
    }

    /// Signal a scan to cancel. Returns true if the scan was found.
    pub fn cancel_scan(&self, scan_id: &str) -> bool {
        if let Some(cancel) = self.scans.lock().unwrap().get(scan_id) {
            cancel.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    // ═══════════════════════════════════════════════════════════
    //  Disk Usage Scan
    // ═══════════════════════════════════════════════════════════

    /// Scan a directory tree for disk usage.
    ///
    /// **Events:** `scan:progress`, `scan:chunk`, `scan:complete`, `scan:error`
    pub async fn scan_usage(
        &self,
        window: tauri::WebviewWindow,
        path: String,
        max_depth: u32,
    ) -> Result<String, AppError> {
        if !Path::new(&path).exists() {
            return Err(AppError::NotFound(path));
        }

        let id = Self::scan_id();
        let cancel = self.register(&id);
        let start = Instant::now();
        let id_run = id.clone();
        let id_unregister = id.clone();
        let cancel_clone = cancel.clone();

        // Initialize tree for this scan
        {
            let mut t = self.tree.lock().unwrap();
            t.entry(id_run.clone()).or_insert_with(HashMap::new);
        }

        // run() returns a future that resolves when the scan + emission is fully done
        disk_usage::DiskUsageUseCase::run(
            window,
            path,
            max_depth,
            cancel_clone,
            start,
            id_run.clone(),
            self.tree.clone(),
        )
        .await;

        // Unregister after the scan is done (completed or cancelled)
        self.scans.lock().unwrap().remove(&id_unregister);

        Ok(id)
    }

    // ═══════════════════════════════════════════════════════════
    //  Large Files
    // ═══════════════════════════════════════════════════════════

    /// Find large files in a directory tree.
    ///
    /// **Events:** `scan:progress`, `scan:chunk`, `scan:complete`, `scan:error`
    pub async fn find_large_files(
        &self,
        window: tauri::WebviewWindow,
        path: String,
        min_size: u64,
        max_results: usize,
    ) -> Result<String, AppError> {
        if !Path::new(&path).exists() {
            return Err(AppError::NotFound(path.clone()));
        }

        let id = Self::scan_id();
        let cancel = self.register(&id);
        let start = Instant::now();
        let id_run = id.clone();
        let id_unregister = id.clone();
        let cancel_clone = cancel.clone();

        large_files::LargeFilesUseCase::run(
            window, path, min_size, max_results, cancel_clone, start, id_run,
        )
        .await;

        self.scans.lock().unwrap().remove(&id_unregister);

        Ok(id)
    }

    // ═══════════════════════════════════════════════════════════
    //  Duplicate Detection
    // ═══════════════════════════════════════════════════════════

    /// Find duplicate files using the 3-stage funnel.
    ///
    /// **Events:** `scan:progress`, `scan:chunk`, `scan:complete`, `scan:error`
    pub async fn find_duplicates(
        &self,
        window: tauri::WebviewWindow,
        path: String,
    ) -> Result<String, AppError> {
        if !Path::new(&path).exists() {
            return Err(AppError::NotFound(path.clone()));
        }

        let id = Self::scan_id();
        let cancel = self.register(&id);
        let start = Instant::now();
        let id_run = id.clone();
        let id_unregister = id.clone();
        let cancel_clone = cancel.clone();

        duplicates::DuplicatesUseCase::run(window, path, cancel_clone, start, id_run).await;

        self.scans.lock().unwrap().remove(&id_unregister);

        Ok(id)
    }

    // ═══════════════════════════════════════════════════════════
    //  Usage Snapshots
    // ═══════════════════════════════════════════════════════════

    /// Save a usage snapshot to the analytics database.
    pub async fn save_snapshot(
        analytics: &SqliteAnalytics,
        path: String,
        total_size: u64,
        file_count: u64,
        folder_count: u64,
        top_folders: String,
    ) -> Result<UsageSnapshot, AppError> {
        SnapshotUseCase::save(
            analytics,
            path,
            total_size,
            file_count,
            folder_count,
            top_folders,
        )
        .await
    }

    /// Query usage history for a path within a date range.
    pub async fn query_history(
        analytics: &SqliteAnalytics,
        path: String,
        start: String,
        end: String,
    ) -> Result<Vec<UsageSnapshot>, AppError> {
        SnapshotUseCase::query(analytics, path, start, end).await
    }
}
