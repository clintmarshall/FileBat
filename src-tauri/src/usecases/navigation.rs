use crate::domain::{sort_entries, AppError, Entry, FileSystemRepo, Volume};

/// Application-level logic for navigation operations.
///
/// Use Cases orchestrate repositories and apply business rules
/// (sorting, filtering, validation) without knowing about Tauri or the UI.
pub struct NavigationUseCase<R: FileSystemRepo> {
    repo: R,
}

impl<R: FileSystemRepo> NavigationUseCase<R> {
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    /// List a directory, sorted by type then name.
    pub fn list_directory(&self, path: &str) -> Result<Vec<Entry>, AppError> {
        let mut entries = self.repo.list_directory(path)?;
        sort_entries(&mut entries);
        Ok(entries)
    }

    /// Get available volumes/drives.
    pub fn get_volumes(&self) -> Vec<Volume> {
        self.repo.get_volumes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::mock::MockFileSystem;
    use crate::domain::EntryType;

    fn entry(name: &str, entry_type: EntryType, size: u64) -> Entry {
        Entry {
            name: name.to_string(),
            path: format!("C:\\test\\{}", name),
            size,
            modified: String::new(),
            entry_type,
            extension: None,
        }
    }

    #[test]
    fn list_directory_sorts_folders_first() {
        let mock = MockFileSystem::with_entries(vec![
            entry("file.txt", EntryType::File, 100),
            entry("folder", EntryType::Folder, 0),
        ]);
        let usecase = NavigationUseCase::new(mock);
        let entries = usecase.list_directory("C:\\test").unwrap();

        assert_eq!(entries[0].entry_type, EntryType::Folder);
        assert_eq!(entries[1].entry_type, EntryType::File);
    }

    #[test]
    fn list_directory_sorts_by_name_within_type() {
        let mock = MockFileSystem::with_entries(vec![
            entry("Zebra.txt", EntryType::File, 0),
            entry("apple.txt", EntryType::File, 0),
            entry("Banana.txt", EntryType::File, 0),
        ]);
        let usecase = NavigationUseCase::new(mock);
        let entries = usecase.list_directory("C:\\test").unwrap();

        assert_eq!(entries[0].name, "apple.txt");
        assert_eq!(entries[1].name, "Banana.txt");
        assert_eq!(entries[2].name, "Zebra.txt");
    }

    #[test]
    fn get_volumes_returns_drives() {
        let mock = MockFileSystem::new();
        let usecase = NavigationUseCase::new(mock);
        let volumes = usecase.get_volumes();

        assert!(!volumes.is_empty());
        assert_eq!(volumes[0].name, "C:");
    }

    #[test]
    fn list_directory_empty_returns_empty() {
        let mock = MockFileSystem::new();
        let usecase = NavigationUseCase::new(mock);
        let entries = usecase.list_directory("C:\\empty").unwrap();

        assert!(entries.is_empty());
    }
}
