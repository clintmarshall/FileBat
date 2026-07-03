#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod domain;
mod infrastructure;
mod usecases;

use std::sync::Arc;

use infrastructure::{SqliteAnalytics, StdFileSystem};
use usecases::{AnalyticsUseCase, FileOperationsUseCase, NavigationUseCase};

fn main() {
    // Build the dependency graph:
    //   Infrastructure (StdFileSystem, SqliteAnalytics) → Use Cases → Commands (Tauri IPC)
    //
    // StdFileSystem is a zero-sized unit struct — creating instances is free.
    // SqliteAnalytics is lazy-init — DB opens only when Analytics panel first used.
    let nav = Arc::new(NavigationUseCase::new(StdFileSystem));
    let ops = Arc::new(FileOperationsUseCase::new(StdFileSystem));
    let analytics = Arc::new(AnalyticsUseCase::new());

    // SqliteAnalytics needs the app data directory for the database file.
    // We create it with a placeholder path; the real path is resolved at startup.
    // Tauri provides the app data dir via AppHandle, but we need it at Builder time.
    // Workaround: use a default location that gets overridden.
    let app_data_dir = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("filebitch");
    let db = Arc::new(SqliteAnalytics::new(app_data_dir));

    tauri::Builder::default()
        .manage(nav)
        .manage(ops)
        .manage(analytics)
        .manage(db)
        .invoke_handler(tauri::generate_handler![
            // Navigation
            commands::navigation::list_dir,
            commands::navigation::get_volumes,
            // File Operations
            commands::fileops::rename,
            commands::fileops::delete,
            commands::fileops::create_folder,
            commands::fileops::copy_items,
            commands::fileops::move_items,
            // Analytics
            commands::scan::start_scan_usage,
            commands::scan::start_find_large_files,
            commands::scan::start_find_duplicates,
            commands::scan::cancel_scan,
            commands::scan::snapshot_usage,
            commands::scan::usage_history,
        ])
        .run(tauri::generate_context!())
        .expect("Failed to build FileBitch app");
}
