// DIT System — Bulletproof Card Offload Engine
// Core modules for the DIT material management system

pub mod checkpoint;
pub mod copy_engine;
pub mod db;
pub mod hash_engine;
pub mod io_scheduler;
pub mod mhl;
pub mod notify;
pub mod report;
pub mod volume;

use tauri::Manager;

/// Tauri command: get application version
#[tauri::command]
fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Initialize database
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let db_path = app_data_dir.join("dit-system.db");
            let _conn = db::init_database(db_path.to_str().unwrap_or("dit-system.db"))?;
            log::info!("Database initialized at {:?}", db_path);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_app_version])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
