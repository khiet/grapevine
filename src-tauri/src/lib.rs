use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};
#[cfg(not(target_os = "linux"))]
use tauri_plugin_positioner::{Position, WindowExt};

mod commands;
mod github;
mod keychain;
mod launch_at_login;
#[cfg(any(target_os = "linux", test))]
mod linux_popover;
mod merged;
mod pr_state;
mod settings;
mod snooze;
mod store;
mod sync;
mod turns;
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
    // Before the window shows, so a row whose snooze ran out overnight is
    // awake the moment the popover opens, whatever the sync loop is waiting on.
    sync::wake_expired_snoozes(app);
    place_popover(&window);
    let _ = window.show();
    let _ = window.set_focus();
}

/// Under the tray icon, where the last tray event said it is.
#[cfg(not(target_os = "linux"))]
fn place_popover(window: &tauri::WebviewWindow) {
    let _ = window.move_window(Position::TrayBottomCenter);
}

/// Logical pixels between the popover and the top and right edges of the
/// work area.
#[cfg(target_os = "linux")]
const POPOVER_GAP: f64 = 8.0;

/// Top-right of the primary monitor: the Linux tray never says where its icon
/// is, so there is nothing to anchor under.
#[cfg(target_os = "linux")]
fn place_popover(window: &tauri::WebviewWindow) {
    // The inner size, which for this undecorated window is the whole window:
    // the outer size is not known until the window has been shown once, and
    // would put the first open in the wrong place.
    let (Ok(Some(monitor)), Ok(size)) = (window.primary_monitor(), window.inner_size()) else {
        return;
    };
    let gap = (POPOVER_GAP * monitor.scale_factor()).round() as i32;
    let _ = window.set_position(linux_popover::top_right(
        monitor.work_area(),
        size.width,
        gap,
    ));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before the builder below initialises GTK, which reads the backend once.
    // Nothing else is running yet, so changing the environment is safe.
    #[cfg(target_os = "linux")]
    if let Some(backend) =
        linux_popover::backend_to_force(std::env::var_os(linux_popover::BACKEND_VAR).as_deref())
    {
        std::env::set_var(linux_popover::BACKEND_VAR, backend);
    }

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
            commands::list_affiliated_repos,
            commands::get_prs,
            commands::mark_read,
            commands::mark_all_read,
            commands::dismiss_merged,
            commands::clear_merged,
            commands::snooze,
            commands::wake,
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
            // Managed before anything below can run a command or a sync, both
            // of which fetch it from app state.
            app.manage(store::Store::new(
                app.path()
                    .app_data_dir()
                    .map_err(|e| format!("cannot resolve app data dir: {e}")),
            ));

            // Menubar-only app: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let quit = MenuItemBuilder::with_id("quit", "Quit Grapevine").build(app)?;
            let menu = MenuBuilder::new(app);
            // The Linux tray reports no clicks and opens its menu on every
            // click, so the menu is the only way to reach the popover.
            #[cfg(target_os = "linux")]
            let open = MenuItemBuilder::with_id("open", "Open Grapevine").build(app)?;
            #[cfg(target_os = "linux")]
            let menu = menu.item(&open);
            let menu = menu.item(&quit).build()?;

            // A dedicated glyph, not the app icon: template icons are
            // alpha-only, so the colored app icon would flatten into an
            // illegible block.
            #[cfg(not(target_os = "linux"))]
            let glyph = tauri::include_image!("icons/tray.png");
            // The same glyph in white: Linux has no template icons to recolor
            // it, and Ubuntu's panel is dark in both light and dark styles.
            #[cfg(target_os = "linux")]
            let glyph = tauri::include_image!("icons/tray-white.png");
            let tray = TrayIconBuilder::with_id("main").icon(glyph);
            #[cfg(target_os = "macos")]
            let tray = tray.icon_as_template(true);
            tray.menu(&menu)
                // Left click toggles the popover; the menu stays on right
                // click. Linux ignores this and opens the menu on every click.
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => toggle_popover(app),
                    "quit" => app.exit(0),
                    _ => {}
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
