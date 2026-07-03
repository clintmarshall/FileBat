use super::models::{Entry, Volume};

/// Abstraction over the operating system filesystem.
///
/// Defined in the domain layer so use cases depend on an interface,
/// not a concrete OS binding. This enables testing and future extensions
/// (network drives, virtual filesystems, etc.).
pub trait FileSystemRepo: Send + Sync {
    // ── Navigation ──

    /// List the immediate children of a directory.
    fn list_directory(&self, path: &str) -> Result<Vec<Entry>, AppError>;

    /// Enumerate mounted volumes / drives.
    fn get_volumes(&self) -> Vec<Volume>;

    // ── File Operations ──

    /// Rename or move a file/folder to a new path within the same filesystem.
    fn rename(&self, old_path: &str, new_path: &str) -> Result<(), AppError>;

    /// Delete files/folders.
    /// On Windows, sends to Recycle Bin where possible.
    /// On Unix, permanent delete.
    fn delete(&self, paths: &[&str]) -> Result<Vec<String>, AppError>;

    /// Create a new empty folder at the given path.
    fn create_folder(&self, path: &str) -> Result<(), AppError>;

    /// Copy files/folders from source paths to a destination directory.
    fn copy(&self, sources: &[&str], dest_dir: &str) -> Result<Vec<String>, AppError>;

    /// Move files/folders from source paths to a destination directory.
    fn move_to(&self, sources: &[&str], dest_dir: &str) -> Result<Vec<String>, AppError>;
}

/// Domain-level error type.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Scan cancelled")]
    ScanCancelled,

    #[error("{0}")]
    Other(String),
}

impl AppError {
    /// Convert to a user-facing string safe for IPC.
    pub fn to_user_message(&self) -> String {
        match self {
            AppError::PermissionDenied(path) => {
                format!("Access denied: {}", path)
            }
            AppError::NotFound(path) => {
                format!("Path not found: {}", path)
            }
            AppError::Io(err) => {
                format!("Filesystem error: {}", err)
            }
            AppError::Database(err) => {
                format!("Database error: {}", err)
            }
            AppError::ScanCancelled => "Scan was cancelled".to_string(),
            AppError::Other(msg) => msg.clone(),
        }
    }
}

/// Analytics persistence — SQLite storage for usage snapshots.
///
/// Defined in the domain layer so use cases depend on an interface.
/// The concrete implementation uses SQLite with lazy initialization.
/// Methods are async because sqlx is async.
pub trait AnalyticsRepo: Send + Sync {
    /// Save a usage snapshot to the database.
    /// Lazy-initializes the database if it hasn't been opened yet.
    async fn save_snapshot(
        &self,
        path: &str,
        total_size: u64,
        file_count: u64,
        folder_count: u64,
        top_folders: &str,
    ) -> Result<UsageSnapshot, AppError>;

    /// Query historical snapshots for a path within a date range.
    async fn query_history(
        &self,
        path: &str,
        start: &str,
        end: &str,
    ) -> Result<Vec<UsageSnapshot>, AppError>;

    /// Get all distinct paths that have snapshots.
    async fn get_snapshot_paths(&self) -> Result<Vec<String>, AppError>;
}

// Re-export analytics models used by the trait
use super::models::UsageSnapshot;
