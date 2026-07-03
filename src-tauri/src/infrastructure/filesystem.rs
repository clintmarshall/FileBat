use crate::domain::{AppError, Entry, EntryType, FileSystemRepo, Volume};
use std::fs;
use std::path::Path;

/// Concrete filesystem implementation using `std::fs`.
pub struct StdFileSystem;

impl FileSystemRepo for StdFileSystem {
    fn list_directory(&self, path: &str) -> Result<Vec<Entry>, AppError> {
        let entries = fs::read_dir(path)?;

        let mut result = Vec::new();

        for entry in entries {
            let entry = entry?;
            let metadata = entry.metadata()?;
            let path_buf = entry.path();

            let name = path_buf
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let is_symlink = path_buf.is_symlink();
            let is_dir = if is_symlink {
                false
            } else {
                metadata.is_dir()
            };

            let entry_type = if is_symlink {
                EntryType::Symlink
            } else if is_dir {
                EntryType::Folder
            } else {
                EntryType::File
            };

            let extension = if entry_type == EntryType::File {
                path_buf.extension().map(|e| e.to_string_lossy().to_string())
            } else {
                None
            };

            let modified = metadata
                .modified()
                .map(|t| {
                    let dt = chrono::DateTime::<chrono::Local>::from(t);
                    dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
                })
                .unwrap_or_default();

            result.push(Entry {
                name,
                path: path_buf.to_string_lossy().to_string(),
                size: metadata.len(),
                modified,
                entry_type,
                extension,
            });
        }

        Ok(result)
    }

    fn get_volumes(&self) -> Vec<Volume> {
        #[cfg(target_os = "windows")]
        {
            let mut volumes = Vec::new();
            for letter in b'A'..=b'Z' {
                let drive_path = format!("{}:\\", letter as char);
                if Path::new(&drive_path).exists() {
                    volumes.push(Volume {
                        name: format!("{}:", letter as char),
                        path: drive_path,
                    });
                }
            }
            volumes
        }

        #[cfg(not(target_os = "windows"))]
        {
            vec![Volume {
                name: "/".to_string(),
                path: "/".to_string(),
            }]
        }
    }

    fn rename(&self, old_path: &str, new_path: &str) -> Result<(), AppError> {
        fs::rename(old_path, new_path)?;
        Ok(())
    }

    fn delete(&self, paths: &[&str]) -> Result<Vec<String>, AppError> {
        let mut deleted = Vec::new();

        for path_str in paths {
            let path = Path::new(path_str);
            if !path.exists() {
                deleted.push(format!("Skipped (not found): {}", path_str));
                continue;
            }

            if path.is_dir() {
                // TODO: Windows Recycle Bin via SHFileOperation
                fs::remove_dir_all(path_str)
                    .map_err(|e| AppError::Other(format!("Failed to delete '{}': {}", path_str, e)))?;
            } else {
                fs::remove_file(path_str)
                    .map_err(|e| AppError::Other(format!("Failed to delete '{}': {}", path_str, e)))?;
            }
            deleted.push(format!("Deleted: {}", path_str));
        }

        Ok(deleted)
    }

    fn create_folder(&self, path: &str) -> Result<(), AppError> {
        fs::create_dir(path)
            .map_err(|e| AppError::Other(format!("Failed to create folder '{}': {}", path, e)))?;
        Ok(())
    }

    fn copy(&self, sources: &[&str], dest_dir: &str) -> Result<Vec<String>, AppError> {
        let mut copied = Vec::new();
        let dest = Path::new(dest_dir);

        if !dest.is_dir() {
            return Err(AppError::Other(format!("Destination '{}' is not a directory", dest_dir)));
        }

        for src_path in sources {
            let src = Path::new(src_path);
            let new_path = dest.join(
                src.file_name()
                    .ok_or_else(|| AppError::Other(format!("Invalid source path: {}", src_path)))?,
            );

            if src.is_dir() {
                // Recursive directory copy
                copy_dir(src, &new_path)
                    .map_err(|e| AppError::Other(format!("Failed to copy '{}': {}", src_path, e)))?;
            } else {
                fs::copy(src_path, &new_path)
                    .map_err(|e| AppError::Other(format!("Failed to copy '{}': {}", src_path, e)))?;
            }
            copied.push(format!("Copied: {} → {}", src_path, new_path.display()));
        }

        Ok(copied)
    }

    fn move_to(&self, sources: &[&str], dest_dir: &str) -> Result<Vec<String>, AppError> {
        let mut moved = Vec::new();
        let dest = Path::new(dest_dir);

        if !dest.is_dir() {
            return Err(AppError::Other(format!("Destination '{}' is not a directory", dest_dir)));
        }

        for src_path in sources {
            let src = Path::new(src_path);
            let new_path = dest.join(
                src.file_name()
                    .ok_or_else(|| AppError::Other(format!("Invalid source path: {}", src_path)))?,
            );

            fs::rename(src_path, &new_path)
                .map_err(|e| AppError::Other(format!("Failed to move '{}': {}", src_path, e)))?;
            moved.push(format!("Moved: {} → {}", src_path, new_path.display()));
        }

        Ok(moved)
    }
}

/// Recursively copy a directory.
fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}
