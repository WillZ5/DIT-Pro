// DIT System — Bulletproof Card Offload Engine
// Core modules for the DIT material management system

pub mod checkpoint;
pub mod commands;
pub mod config;
pub mod copy_engine;
pub mod db;
pub mod hash_engine;
pub mod io_scheduler;
pub mod mhl;
pub mod notify;
pub mod report;
pub mod volume;
pub mod workflow;

use std::sync::{Arc, Mutex};
use tauri::Manager;

use commands::AppState;
use io_scheduler::IoScheduler;

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
            let conn = db::init_database(db_path.to_str().unwrap_or("dit-system.db"))?;
            log::info!("Database initialized at {:?}", db_path);

            // Load settings
            let settings = config::load_settings(&app_data_dir)
                .unwrap_or_default();
            log::info!("Settings loaded (hash: {:?})", settings.hash_algorithms);

            // Initialize app state
            let state = AppState {
                db: Arc::new(Mutex::new(conn)),
                io_scheduler: Mutex::new(IoScheduler::new()),
                app_data_dir: app_data_dir.clone(),
                settings: Mutex::new(settings),
            };
            app.manage(state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // App
            get_app_version,
            // Jobs
            commands::create_job,
            commands::get_job_progress,
            commands::list_jobs,
            commands::recover_job,
            // Volumes
            commands::list_volumes,
            commands::get_space_info,
            commands::preflight_check,
            // Hash
            commands::hash_file,
            // IO Scheduler
            commands::register_device,
            commands::get_scheduler_status,
            // MHL
            commands::create_mhl_generation,
            commands::verify_mhl_chain,
            // Workflow
            commands::start_offload,
            // Settings
            commands::get_settings,
            commands::save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
