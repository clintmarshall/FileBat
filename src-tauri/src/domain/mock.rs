//! Mock FileSystemRepo for integration testing.
//!
//! In-memory filesystem that mimics real behavior without touching disk.
use super::{AppError, Entry, EntryType, FileSystemRepo, Volume};

pub struct MockFileSystem {
    pub entries: Vec<Entry>,
    pub volumes: Vec<Volume>,
}

impl MockFileSystem {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            volumes: vec![Volume {
                name: "C:".to_string(),
                path: "C:\\".to_string(),
            }],
        }
    }

    pub fn with_entries(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            volumes: Self::default_volumes(),
        }
    }

    fn default_volumes() -> Vec<Volume> {
        vec![Volume {
            name: "C:".to_string(),
            path: "C:\\".to_string(),
        }]
    }
}

impl Default for MockFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystemRepo for MockFileSystem {
    fn list_directory(&self, _path: &str) -> Result<Vec<Entry>, AppError> {
        Ok(self.entries.clone())
    }

    fn get_volumes(&self) -> Vec<Volume> {
        self.volumes.clone()
    }

    fn rename(&self, _old_path: &str, _new_path: &str) -> Result<(), AppError> {
        Ok(())
    }

    fn delete(&self, paths: &[&str]) -> Result<Vec<String>, AppError> {
        Ok(paths.iter().map(|p| format!("Deleted: {}", p)).collect())
    }

    fn create_folder(&self, _path: &str) -> Result<(), AppError> {
        Ok(())
    }

    fn copy(&self, sources: &[&str], _dest_dir: &str) -> Result<Vec<String>, AppError> {
        Ok(sources.iter().map(|s| format!("Copied: {}", s)).collect())
    }

    fn move_to(&self, sources: &[&str], _dest_dir: &str) -> Result<Vec<String>, AppError> {
        Ok(sources.iter().map(|s| format!("Moved: {}", s)).collect())
    }
}
