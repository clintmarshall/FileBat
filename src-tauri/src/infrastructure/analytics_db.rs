use crate::domain::{AnalyticsRepo, AppError, UsageSnapshot};
use sqlx::{Row, SqlitePool};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// SQLite-backed analytics storage with lazy initialization.
///
/// The database is not created until the first write/read operation,
/// keeping cold start fast. Uses `Arc<SqlitePool>` so the pool can be
/// cloned out of the mutex lock.
pub struct SqliteAnalytics {
    pool: Mutex<Option<Arc<SqlitePool>>>,
    db_path: PathBuf,
}

impl SqliteAnalytics {
    pub fn new(app_data_dir: PathBuf) -> Self {
        let db_path = app_data_dir.join("analytics.db");
        Self {
            pool: Mutex::new(None),
            db_path,
        }
    }

    /// Lazily initialize the SQLite connection pool.
    /// Returns a cloneable `Arc<SqlitePool>` that outlives the mutex guard.
    async fn ensure_pool(&self) -> Result<Arc<SqlitePool>, AppError> {
        let mut pool_guard = self.pool.lock().await;

        if pool_guard.is_none() {
            let pool = self.create_pool().await?;
            *pool_guard = Some(Arc::new(pool));
        }

        // Clone the Arc — cheap, just increments the ref count
        Ok(pool_guard.as_ref().unwrap().clone())
    }

    async fn create_pool(&self) -> Result<SqlitePool, AppError> {
        // Ensure parent directory exists
        if let Some(parent) = self.db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Database(format!("Failed to create db dir: {}", e)))?;
        }

        let db_url = self.db_path.to_string_lossy().to_string();
        let pool = SqlitePool::connect(&db_url)
            .await
            .map_err(|e| AppError::Database(format!("Failed to connect to sqlite: {}", e)))?;

        // Create tables if they don't exist
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS usage_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL,
                total_size INTEGER NOT NULL,
                file_count INTEGER NOT NULL,
                folder_count INTEGER NOT NULL,
                top_folders TEXT,
                scanned_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_snapshots_path ON usage_snapshots(path);
            CREATE INDEX IF NOT EXISTS idx_snapshots_scanned_at ON usage_snapshots(scanned_at);
            "#,
        )
        .execute(&pool)
        .await
        .map_err(|e| AppError::Database(format!("Failed to create tables: {}", e)))?;

        Ok(pool)
    }
}

impl AnalyticsRepo for SqliteAnalytics {
    async fn save_snapshot(
        &self,
        path: &str,
        total_size: u64,
        file_count: u64,
        folder_count: u64,
        top_folders: &str,
    ) -> Result<UsageSnapshot, AppError> {
        let pool = self.ensure_pool().await?;
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

        let row = sqlx::query(
            r#"
            INSERT INTO usage_snapshots (path, total_size, file_count, folder_count, top_folders, scanned_at)
            VALUES (?, ?, ?, ?, ?, ?)
            RETURNING id, path, total_size, file_count, folder_count, top_folders, scanned_at;
            "#,
        )
        .bind(path)
        .bind(total_size as i64)
        .bind(file_count as i64)
        .bind(folder_count as i64)
        .bind(top_folders)
        .bind(&now)
        .fetch_one(&*pool)
        .await
        .map_err(|e| AppError::Database(format!("Failed to save snapshot: {}", e)))?;

        Ok(UsageSnapshot {
            id: row.try_get("id").map_err(|e| AppError::Database(e.to_string()))?,
            path: row.try_get("path").map_err(|e| AppError::Database(e.to_string()))?,
            total_size: row.try_get("total_size").map(|v: i64| v as u64).map_err(|e| AppError::Database(e.to_string()))?,
            file_count: row.try_get("file_count").map(|v: i64| v as u64).map_err(|e| AppError::Database(e.to_string()))?,
            folder_count: row.try_get("folder_count").map(|v: i64| v as u64).map_err(|e| AppError::Database(e.to_string()))?,
            top_folders: row.try_get("top_folders").map_err(|e| AppError::Database(e.to_string()))?,
            scanned_at: row.try_get("scanned_at").map_err(|e| AppError::Database(e.to_string()))?,
        })
    }

    async fn query_history(
        &self,
        path: &str,
        start: &str,
        end: &str,
    ) -> Result<Vec<UsageSnapshot>, AppError> {
        let pool = self.ensure_pool().await?;

        let rows = sqlx::query(
            r#"
            SELECT id, path, total_size, file_count, folder_count, top_folders, scanned_at
            FROM usage_snapshots
            WHERE path = ?
              AND scanned_at >= ?
              AND scanned_at <= ?
            ORDER BY scanned_at DESC;
            "#,
        )
        .bind(path)
        .bind(start)
        .bind(end)
        .fetch_all(&*pool)
        .await
        .map_err(|e| AppError::Database(format!("Failed to query history: {}", e)))?;

        rows.into_iter()
            .map(|row| {
                Ok(UsageSnapshot {
                    id: row.try_get("id").map_err(|e| AppError::Database(e.to_string()))?,
                    path: row.try_get("path").map_err(|e| AppError::Database(e.to_string()))?,
                    total_size: row.try_get("total_size").map(|v: i64| v as u64).map_err(|e| AppError::Database(e.to_string()))?,
                    file_count: row.try_get("file_count").map(|v: i64| v as u64).map_err(|e| AppError::Database(e.to_string()))?,
                    folder_count: row.try_get("folder_count").map(|v: i64| v as u64).map_err(|e| AppError::Database(e.to_string()))?,
                    top_folders: row.try_get("top_folders").map_err(|e| AppError::Database(e.to_string()))?,
                    scanned_at: row.try_get("scanned_at").map_err(|e| AppError::Database(e.to_string()))?,
                })
            })
            .collect()
    }

    async fn get_snapshot_paths(&self) -> Result<Vec<String>, AppError> {
        let pool = self.ensure_pool().await?;

        let rows = sqlx::query(
            r#"SELECT DISTINCT path FROM usage_snapshots ORDER BY path"#,
        )
        .fetch_all(&*pool)
        .await
        .map_err(|e| AppError::Database(format!("Failed to query paths: {}", e)))?;

        rows.into_iter()
            .map(|row| {
                row.try_get::<String, _>("path").map_err(|e| AppError::Database(e.to_string()))
            })
            .collect()
    }
}
