use crate::infrastructure::StdFileSystem;
use crate::usecases::FileOperationsUseCase;
use std::sync::Arc;

/// Tauri command wrappers for file operations.
/// Thin IPC bridge — no business logic.

type FileOpsState = Arc<FileOperationsUseCase<StdFileSystem>>;

/// Rename a file or folder.
#[tauri::command]
pub async fn rename(
    path: String,
    new_name: String,
    ops: tauri::State<'_, FileOpsState>,
) -> Result<(), String> {
    ops.rename(&path, &new_name).map_err(|e| e.to_user_message())
}

/// Delete files/folders.
#[tauri::command]
pub async fn delete(
    paths: Vec<String>,
    ops: tauri::State<'_, FileOpsState>,
) -> Result<Vec<String>, String> {
    ops.delete(&paths).map_err(|e| e.to_user_message())
}

/// Create a new folder.
#[tauri::command]
pub async fn create_folder(
    parent_path: String,
    name: String,
    ops: tauri::State<'_, FileOpsState>,
) -> Result<(), String> {
    ops.create_folder(&parent_path, &name).map_err(|e| e.to_user_message())
}

/// Copy files/folders to a destination directory.
#[tauri::command]
pub async fn copy_items(
    sources: Vec<String>,
    dest_dir: String,
    ops: tauri::State<'_, FileOpsState>,
) -> Result<Vec<String>, String> {
    ops.copy(&sources, &dest_dir).map_err(|e| e.to_user_message())
}

/// Move files/folders to a destination directory.
#[tauri::command]
pub async fn move_items(
    sources: Vec<String>,
    dest_dir: String,
    ops: tauri::State<'_, FileOpsState>,
) -> Result<Vec<String>, String> {
    ops.move_to(&sources, &dest_dir).map_err(|e| e.to_user_message())
}
