//! System tray (macOS menu bar) for DIT Pro.
//!
//! Provides a menu bar icon with:
//! - Left-click: show/focus main window
//! - Right-click: context menu with status and quick actions
//! - Dynamic icon: idle, active (copying), error

use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

/// Initialize the system tray icon and menu.
pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let version = crate::version::VersionInfo::current()
        .full_string
        .split('+')
        .next()
        .unwrap_or("unknown")
        .to_string();

    // Build context menu items
    let title_item = MenuItemBuilder::new(format!("DIT Pro v{}", version))
        .enabled(false)
        .build(app)?;

    let show_item = MenuItemBuilder::with_id("show", "Show Window").build(app)?;

    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&title_item)
        .separator()
        .item(&show_item)
        .separator()
        .item(&quit_item)
        .build()?;

    // Load idle icon
    let icon_bytes = include_bytes!("../icons/tray-idle.png");
    let icon = Image::from_bytes(icon_bytes)?;

    let mut tray_builder = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("DIT Pro")
        .menu(&menu)
        .show_menu_on_left_click(false);

    // macOS: use template icons for monochrome menu bar appearance
    #[cfg(target_os = "macos")]
    {
        tray_builder = tray_builder.icon_as_template(true);
    }

    let _tray = tray_builder
        .on_tray_icon_event({
            let app_handle = app.clone();
            move |_tray, event| {
                if let tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    ..
                } = event
                {
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                }
            }
        })
        .on_menu_event({
            let app_handle = app.clone();
            move |_app, event: tauri::menu::MenuEvent| {
                match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        // Tray "Quit" click → direct exit
                        std::process::exit(0);
                    }
                    _ => {}
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Tray icon state variants
pub enum TrayState {
    Idle,
    Active,
    Error,
}

/// Update the tray icon based on current state
pub fn update_tray_icon(app: &AppHandle, state: TrayState) {
    let icon_bytes: &[u8] = match state {
        TrayState::Idle => include_bytes!("../icons/tray-idle.png"),
        TrayState::Active => include_bytes!("../icons/tray-active.png"),
        TrayState::Error => include_bytes!("../icons/tray-error.png"),
    };

    if let Ok(icon) = Image::from_bytes(icon_bytes) {
        if let Some(tray) = app.tray_by_id("main-tray") {
            let _ = tray.set_icon(Some(icon));
            // macOS: only use template for idle (monochrome menu bar);
            // colored icons should not be templates. Windows ignores this.
            #[cfg(target_os = "macos")]
            {
                let is_template = matches!(state, TrayState::Idle);
                let _ = tray.set_icon_as_template(is_template);
            }
        }
    }
}
