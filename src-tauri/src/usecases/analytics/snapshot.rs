use crate::domain::{AnalyticsRepo, AppError, UsageSnapshot};

/// Orchestrates usage snapshot persistence.
///
/// Thin delegation to the AnalyticsRepo — the business logic is just
/// "save this snapshot" and "query history for this path."
pub struct SnapshotUseCase;

impl SnapshotUseCase {
    /// Save a usage snapshot to the analytics database.
    pub async fn save<R: AnalyticsRepo>(
        repo: &R,
        path: String,
        total_size: u64,
        file_count: u64,
        folder_count: u64,
        top_folders: String,
    ) -> Result<UsageSnapshot, AppError> {
        repo.save_snapshot(&path, total_size, file_count, folder_count, &top_folders)
            .await
    }

    /// Query usage history for a path within a date range.
    pub async fn query<R: AnalyticsRepo>(
        repo: &R,
        path: String,
        start: String,
        end: String,
    ) -> Result<Vec<UsageSnapshot>, AppError> {
        repo.query_history(&path, &start, &end).await
    }
}
