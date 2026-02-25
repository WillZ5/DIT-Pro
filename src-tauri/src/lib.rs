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
pub mod preset;
pub mod report;
pub mod tray;
pub mod volume;
pub mod workflow;

use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

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
        .plugin(tauri_plugin_process::init())
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

            // Intercept window close (red X) — hide window instead of closing
            if let Some(main_window) = app.get_webview_window("main") {
                let app_handle_close = app.handle().clone();
                main_window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        // Hide instead of close — app stays in background via tray
                        if let Some(w) = app_handle_close.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                });
            } else {
                log::warn!("Main window not found during setup");
            }

            // Initialize system tray
            if let Err(e) = tray::setup_tray(app.handle()) {
                log::warn!("Failed to setup system tray: {}", e);
            }

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
            commands::resume_offload,
            // Settings
            commands::get_settings,
            commands::save_settings,
            // Presets
            commands::list_presets,
            commands::create_preset,
            commands::update_preset,
            commands::delete_preset,
            // Reports
            commands::get_report_dates,
            commands::get_day_report,
            commands::get_job_report,
            commands::export_day_report,
            commands::export_job_report,
            // Notifications
            commands::send_test_email,
            commands::save_smtp_password,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match &event {
                tauri::RunEvent::ExitRequested { api, .. } => {
                    // Cmd+Q or last window closed — prevent default exit,
                    // show window and let frontend handle quit confirmation
                    api.prevent_exit();
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                    app_handle.emit("quit-requested", ()).ok();
                }
                tauri::RunEvent::Reopen { .. } => {
                    // macOS dock icon clicked — show/focus window
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                }
                _ => {}
            }
        });
}
