use crate::domain::{AppError, Entry, Volume};
use crate::infrastructure::StdFileSystem;
use crate::usecases::NavigationUseCase;
use std::sync::Arc;

/// Tauri command wrappers — thin layer that bridges IPC to Use Cases.
///
/// These functions receive serialized args from the frontend,
/// delegate to the use case, and return serializable results.
/// No business logic lives here.

type NavState = Arc<NavigationUseCase<StdFileSystem>>;

#[tauri::command]
pub async fn list_dir(path: String, nav: tauri::State<'_, NavState>) -> Result<Vec<Entry>, String> {
    nav.list_directory(&path).map_err(|e: AppError| e.to_user_message())
}

#[tauri::command]
pub async fn get_volumes(nav: tauri::State<'_, NavState>) -> Result<Vec<Volume>, String> {
    Ok(nav.get_volumes())
}
