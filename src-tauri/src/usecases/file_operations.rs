use crate::domain::{AppError, FileSystemRepo};

/// Application-level logic for file operations.
///
/// Validates inputs, handles edge cases (name collisions, empty names),
/// and delegates to the repository. No Tauri or UI knowledge.
pub struct FileOperationsUseCase<R: FileSystemRepo> {
    repo: R,
}

impl<R: FileSystemRepo> FileOperationsUseCase<R> {
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    /// Rename a file or folder.
    pub fn rename(&self, old_path: &str, new_name: &str) -> Result<(), AppError> {
        if new_name.is_empty() {
            return Err(AppError::Other("Name cannot be empty".to_string()));
        }

        let parent = std::path::Path::new(old_path)
            .parent()
            .ok_or_else(|| AppError::Other(format!("Cannot determine parent of '{}'", old_path)))?;

        let new_path = parent.join(new_name);

        self.repo.rename(old_path, &new_path.to_string_lossy())
    }

    /// Delete files/folders.
    pub fn delete(&self, paths: &[String]) -> Result<Vec<String>, AppError> {
        let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        self.repo.delete(&path_refs)
    }

    /// Create a new folder.
    pub fn create_folder(&self, parent_path: &str, name: &str) -> Result<(), AppError> {
        if name.is_empty() {
            return Err(AppError::Other("Folder name cannot be empty".to_string()));
        }

        let new_path = std::path::Path::new(parent_path).join(name);
        self.repo.create_folder(&new_path.to_string_lossy())
    }

    /// Copy files/folders to a destination directory.
    pub fn copy(&self, sources: &[String], dest_dir: &str) -> Result<Vec<String>, AppError> {
        self.repo.copy(&sources.iter().map(|s| s.as_str()).collect::<Vec<_>>(), dest_dir)
    }

    /// Move files/folders to a destination directory.
    pub fn move_to(&self, sources: &[String], dest_dir: &str) -> Result<Vec<String>, AppError> {
        self.repo
            .move_to(&sources.iter().map(|s| s.as_str()).collect::<Vec<_>>(), dest_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::mock::MockFileSystem;

    #[test]
    fn rename_rejects_empty_name() {
        let mock = MockFileSystem::new();
        let usecase = FileOperationsUseCase::new(mock);
        let result = usecase.rename("C:\\test\\file.txt", "");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_user_message().contains("cannot be empty"));
    }

    #[test]
    fn rename_succeeds_with_valid_name() {
        let mock = MockFileSystem::new();
        let usecase = FileOperationsUseCase::new(mock);
        let result = usecase.rename("C:\\test\\file.txt", "new_name.txt");

        assert!(result.is_ok());
    }

    #[test]
    fn delete_returns_deleted_paths() {
        let mock = MockFileSystem::new();
        let usecase = FileOperationsUseCase::new(mock);
        let paths = vec!["C:\\test\\file.txt".to_string()];
        let result = usecase.delete(&paths).unwrap();

        assert_eq!(result.len(), 1);
        assert!(result[0].contains("Deleted"));
    }

    #[test]
    fn create_folder_rejects_empty_name() {
        let mock = MockFileSystem::new();
        let usecase = FileOperationsUseCase::new(mock);
        let result = usecase.create_folder("C:\\test", "");

        assert!(result.is_err());
    }

    #[test]
    fn create_folder_succeeds_with_valid_name() {
        let mock = MockFileSystem::new();
        let usecase = FileOperationsUseCase::new(mock);
        let result = usecase.create_folder("C:\\test", "New Folder");

        assert!(result.is_ok());
    }

    #[test]
    fn copy_returns_copied_paths() {
        let mock = MockFileSystem::new();
        let usecase = FileOperationsUseCase::new(mock);
        let sources = vec!["C:\\test\\file.txt".to_string()];
        let result = usecase.copy(&sources, "C:\\dest").unwrap();

        assert_eq!(result.len(), 1);
        assert!(result[0].contains("Copied"));
    }

    #[test]
    fn move_to_returns_moved_paths() {
        let mock = MockFileSystem::new();
        let usecase = FileOperationsUseCase::new(mock);
        let sources = vec!["C:\\test\\file.txt".to_string()];
        let result = usecase.move_to(&sources, "C:\\dest").unwrap();

        assert_eq!(result.len(), 1);
        assert!(result[0].contains("Moved"));
    }
}
