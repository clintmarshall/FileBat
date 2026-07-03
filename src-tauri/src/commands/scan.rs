use crate::domain::{AppError, UsageSnapshot};
use crate::infrastructure::SqliteAnalytics;
use crate::usecases::AnalyticsUseCase;
use std::sync::Arc;

/// Tauri command wrappers for analytics operations.
/// Thin IPC bridge — scan logic lives in AnalyticsUseCase.

type AnalyticsState = Arc<AnalyticsUseCase>;
type AnalyticsDbState = Arc<SqliteAnalytics>;

// ─── Disk Usage Scan ───

/// Start a disk usage scan.
///
/// Returns a scan_id immediately. Results stream as Tauri events:
/// - `scan:progress` — percentage + message
/// - `scan:chunk` — FolderUsage per directory
/// - `scan:complete` — summary (total items, size, duration)
/// - `scan:error` — error message if scan fails
#[tauri::command]
pub async fn start_scan_usage(
    path: String,
    max_depth: u32,
    window: tauri::WebviewWindow,
    analytics: tauri::State<'_, AnalyticsState>,
) -> Result<String, String> {
    analytics
        .scan_usage(window, path, max_depth)
        .await
        .map_err(|e: AppError| e.to_user_message())
}

// ─── Large Files ───

/// Find large files in a directory tree.
///
/// Returns a scan_id immediately. Results stream as Tauri events.
#[tauri::command]
pub async fn start_find_large_files(
    path: String,
    min_size: u64,
    max_results: usize,
    window: tauri::WebviewWindow,
    analytics: tauri::State<'_, AnalyticsState>,
) -> Result<String, String> {
    analytics
        .find_large_files(window, path, min_size, max_results)
        .await
        .map_err(|e: AppError| e.to_user_message())
}

// ─── Duplicate Detection ───

/// Find duplicate files using the 3-stage funnel.
///
/// Returns a scan_id immediately. Results stream as Tauri events.
#[tauri::command]
pub async fn start_find_duplicates(
    path: String,
    window: tauri::WebviewWindow,
    analytics: tauri::State<'_, AnalyticsState>,
) -> Result<String, String> {
    analytics
        .find_duplicates(window, path)
        .await
        .map_err(|e: AppError| e.to_user_message())
}

// ─── Cancel ───

/// Cancel a running scan.
///
/// Returns true if the scan was found and cancelled.
#[tauri::command]
pub fn cancel_scan(
    scan_id: String,
    analytics: tauri::State<'_, AnalyticsState>,
) -> bool {
    analytics.cancel_scan(&scan_id)
}

// ─── Save Snapshot ───

/// Save a usage snapshot to the database.
#[tauri::command]
pub async fn snapshot_usage(
    path: String,
    total_size: u64,
    file_count: u64,
    folder_count: u64,
    top_folders: String,
    analytics: tauri::State<'_, AnalyticsState>,
    db: tauri::State<'_, AnalyticsDbState>,
) -> Result<UsageSnapshot, String> {
    AnalyticsUseCase::save_snapshot(
        &db, path, total_size, file_count, folder_count, top_folders,
    )
    .await
    .map_err(|e: AppError| e.to_user_message())
}

// ─── Usage History ───

/// Get all saved usage snapshots ordered by date.
#[tauri::command]
pub async fn usage_history(
    path: String,
    start: String,
    end: String,
    analytics: tauri::State<'_, AnalyticsState>,
    db: tauri::State<'_, AnalyticsDbState>,
) -> Result<Vec<UsageSnapshot>, String> {
    AnalyticsUseCase::query_history(&db, path, start, end)
        .await
        .map_err(|e: AppError| e.to_user_message())
}
