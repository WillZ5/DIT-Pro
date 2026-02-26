// DIT System — Bulletproof Card Offload Engine
// Core modules for the DIT material management system

pub mod checkpoint;
pub mod commands;
pub mod config;
pub mod copy_engine;
pub mod db;
pub mod debug_bundle;
pub mod error;
pub mod error_log;
pub mod hash_engine;
pub mod io_scheduler;
pub mod mhl;
pub mod notify;
pub mod preset;
pub mod report;
pub mod tray;
pub mod version;
pub mod volume;
pub mod workflow;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Emitter, Manager};

use commands::AppState;
use io_scheduler::IoScheduler;

/// Monotonic millisecond timestamp for quit hold detection
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Timestamp of the first ⌘Q press in the current sequence
static QUIT_FIRST_MS: AtomicU64 = AtomicU64::new(0);
/// Timestamp of the most recent ⌘Q event (updated on every key repeat)
static QUIT_LAST_MS: AtomicU64 = AtomicU64::new(0);

/// Tauri command: get application version info
#[tauri::command]
fn get_app_version() -> version::VersionInfo {
    version::VersionInfo::current()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // Always enable logging — Warn in release, Info in debug
            let log_level = if cfg!(debug_assertions) {
                log::LevelFilter::Info
            } else {
                log::LevelFilter::Warn
            };
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(log_level)
                    .build(),
            )?;

            // Initialize database
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let db_path = app_data_dir.join("dit-system.db");
            let conn = db::init_database(&db_path.to_string_lossy())?;
            log::info!("Database initialized at {:?}", db_path);

            // Load settings
            let settings = config::load_settings(&app_data_dir)
                .unwrap_or_default();
            log::info!("Settings loaded (offload defaults: src_verify={}, post_verify={})",
                settings.offload.source_verify, settings.offload.post_verify);

            // Initialize app state
            let state = AppState {
                db: Arc::new(Mutex::new(conn)),
                io_scheduler: Mutex::new(IoScheduler::new()),
                app_data_dir: app_data_dir.clone(),
                settings: Mutex::new(settings),
                active_workflows: Arc::new(Mutex::new(HashMap::new())),
            };
            app.manage(state);

            // Intercept window close (red X) — hide to tray instead of quitting
            if let Some(main_window) = app.get_webview_window("main") {
                let app_handle_close = app.handle().clone();
                main_window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        // Hide window — user can reopen via tray or dock icon
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

            // Custom application menu — ⌘Q accelerator with Rust-side hold detection
            let quit_item = MenuItem::with_id(app, "app-quit", "Quit DIT System", true, Some("CmdOrCtrl+Q"))?;

            let app_submenu = Submenu::with_items(app, "DIT System", true, &[
                &PredefinedMenuItem::about(app, Some("About DIT System"), None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::hide(app, Some("Hide DIT System"))?,
                &PredefinedMenuItem::hide_others(app, None)?,
                &PredefinedMenuItem::show_all(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &quit_item,
            ])?;

            let edit_submenu = Submenu::with_items(app, "Edit", true, &[
                &PredefinedMenuItem::undo(app, None)?,
                &PredefinedMenuItem::redo(app, None)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::cut(app, None)?,
                &PredefinedMenuItem::copy(app, None)?,
                &PredefinedMenuItem::paste(app, None)?,
                &PredefinedMenuItem::select_all(app, None)?,
            ])?;

            let window_submenu = Submenu::with_items(app, "Window", true, &[
                &PredefinedMenuItem::minimize(app, None)?,
                &PredefinedMenuItem::close_window(app, None)?,
            ])?;

            let menu = Menu::with_items(app, &[&app_submenu, &edit_submenu, &window_submenu])?;
            app.set_menu(menu)?;

            // ⌘Q hold-to-quit: Rust-side detection using atomic timestamps.
            // - Hold ⌘Q for 1s (key repeat events stream in) → quit
            // - Tap ⌘Q once → show toast → tap again within 3s → quit
            // - Menu click "Quit" → same as tap (shows toast, click again to quit)
            app.on_menu_event(move |app_handle, event| {
                if event.id().as_ref() == "app-quit" {
                    let now = now_ms();
                    let first = QUIT_FIRST_MS.load(Ordering::Relaxed);
                    let last = QUIT_LAST_MS.load(Ordering::Relaxed);

                    if first == 0 || now.saturating_sub(last) > 500 {
                        // Gap > 500ms from last event → new press (not key repeat)
                        if first > 0 && now.saturating_sub(first) < 3000 {
                            // Toast is still showing — deliberate second press → quit
                            std::process::exit(0);
                        }
                        // First press — show toast
                        QUIT_FIRST_MS.store(now, Ordering::Relaxed);
                        QUIT_LAST_MS.store(now, Ordering::Relaxed);
                        if let Some(w) = app_handle.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                        app_handle.emit("quit-hint-show", ()).ok();

                        // Reset state after 3s if no further events
                        let handle = app_handle.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_secs(3));
                            let last = QUIT_LAST_MS.load(Ordering::Relaxed);
                            if now_ms().saturating_sub(last) > 500 {
                                QUIT_FIRST_MS.store(0, Ordering::Relaxed);
                                handle.emit("quit-hint-hide", ()).ok();
                            }
                        });
                    } else {
                        // Rapid event (< 500ms gap = key repeat from holding)
                        QUIT_LAST_MS.store(now, Ordering::Relaxed);
                        if now.saturating_sub(first) >= 1000 {
                            // Held for 1+ seconds → quit
                            std::process::exit(0);
                        }
                    }
                }
            });

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
            commands::reveal_in_finder,
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
            commands::pause_offload,
            commands::resume_paused_offload,
            commands::terminate_offload,
            commands::batch_pause,
            commands::batch_terminate,
            commands::delete_job,
            commands::batch_delete,
            commands::clear_logs,
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
            // Error Log & Diagnostics
            commands::get_error_log,
            commands::get_error_log_summary,
            commands::resolve_error_entry,
            commands::clear_error_log_entries,
            commands::export_debug_bundle,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match &event {
                tauri::RunEvent::ExitRequested { api, .. } => {
                    // Prevent unexpected exit — show window so user can hold ⌘Q
                    api.prevent_exit();
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
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
