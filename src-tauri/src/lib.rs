use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};
use tauri_plugin_positioner::{Position, WindowExt};

mod commands;
mod github;
mod keychain;
mod merged;
mod settings;
mod sync;
mod unread;

/// When the popover was last hidden because it lost focus. Clicking the tray
/// icon while the popover is open blurs it first, then delivers the click;
/// without this timestamp the click would instantly re-show the window,
/// turning "click to dismiss" into a no-op.
struct LastAutoHide(Mutex<Option<Instant>>);

fn toggle_popover(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    let recently_auto_hidden = app
        .state::<LastAutoHide>()
        .0
        .lock()
        .unwrap()
        .is_some_and(|t| t.elapsed() < Duration::from_millis(300));
    if recently_auto_hidden {
        return;
    }
    let _ = window.move_window(Position::TrayBottomCenter);
    let _ = window.show();
    let _ = window.set_focus();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(LastAutoHide(Mutex::new(None)))
        .manage(sync::SyncState::default())
        .invoke_handler(tauri::generate_handler![
            commands::token_status,
            commands::save_token,
            commands::clear_token,
            commands::list_repos,
            commands::add_repo,
            commands::remove_repo,
            commands::get_prs,
            commands::mark_read,
            commands::mark_all_read,
            commands::dismiss_merged,
            commands::clear_merged,
            commands::get_poll_interval,
            commands::set_poll_interval,
            commands::get_launch_at_login,
            commands::set_launch_at_login,
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(false) = event {
                if window.hide().is_ok() {
                    *window.state::<LastAutoHide>().0.lock().unwrap() = Some(Instant::now());
                }
            }
        })
        .setup(|app| {
            // Menubar-only app: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let quit = MenuItemBuilder::with_id("quit", "Quit Grapevine").build(app)?;
            let menu = MenuBuilder::new(app).item(&quit).build()?;

            TrayIconBuilder::with_id("main")
                // A dedicated glyph, not the app icon: template icons are
                // alpha-only, so the colored app icon would flatten into an
                // illegible block.
                .icon(tauri::include_image!("icons/tray.png"))
                .icon_as_template(true)
                .menu(&menu)
                // Left click toggles the popover; the menu stays on right click.
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_popover(tray.app_handle());
                    }
                })
                .build(app)?;

            sync::start(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
