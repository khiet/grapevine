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
mod fix_requests;
mod github;
mod keychain;
mod launch_at_login;
#[cfg(any(target_os = "linux", test))]
mod linux_popover;
mod merged;
mod popover;
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

/// Set when a second launch arrived before `setup` could show the popover
/// for it; `setup` shows it on the way out.
#[cfg(target_os = "linux")]
struct EarlySecondLaunch(std::sync::atomic::AtomicBool);

fn request_popover(app: &tauri::AppHandle, request: popover::Request) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let visible = on_screen(&window);
    let recently_auto_hidden = app
        .state::<LastAutoHide>()
        .0
        .lock()
        .unwrap()
        .is_some_and(|t| t.elapsed() < Duration::from_millis(300));
    match popover::decide(request, visible, recently_auto_hidden) {
        popover::Action::Nothing => {}
        popover::Action::Hide => {
            let _ = put_away(&window);
        }
        popover::Action::Show => {
            // Before the window shows, so a row whose snooze ran out
            // overnight is awake, and one Copilot has had its hour with is
            // the viewer's again, the moment the popover opens, whatever the
            // sync loop is waiting on.
            sync::expire_by_clock(app);
            place_popover(&window);
            let _ = window.show();
            #[cfg(target_os = "linux")]
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn on_screen(window: &tauri::WebviewWindow) -> bool {
    window.is_visible().unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn on_screen(window: &tauri::WebviewWindow) -> bool {
    linux_popover::on_screen(
        window.is_visible().unwrap_or(false),
        window.is_minimized().unwrap_or(false),
    )
}

#[cfg(not(target_os = "linux"))]
fn put_away(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.hide()
}

/// Takes the popover off screen by minimizing it. GNOME's app switcher and
/// dock list only windows that exist on screen or minimized, so a hidden
/// popover could not be switched to.
#[cfg(target_os = "linux")]
fn put_away(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.minimize()
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

    let builder = tauri::Builder::default();
    // Registered first, as the plugin requires: a second launch must hand over
    // to the running copy and exit before it builds a tray icon of its own.
    // macOS needs none of this; it does not start a second copy.
    #[cfg(target_os = "linux")]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        // The plugin starts answering second launches before `setup` below
        // has run, and showing the popover needs the Store that `setup`
        // manages. A launch landing in that gap is left for `setup` to honor
        // once everything it needs exists.
        if app.try_state::<store::Store>().is_some() {
            request_popover(app, popover::Request::SecondLaunch);
        } else {
            app.state::<EarlySecondLaunch>()
                .0
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }));
    #[cfg(target_os = "linux")]
    let builder = builder.manage(EarlySecondLaunch(false.into()));
    builder
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
            commands::fix_conflict,
            commands::get_poll_interval,
            commands::set_poll_interval,
            commands::get_launch_at_login,
            commands::set_launch_at_login,
        ])
        .on_window_event(|window, event| {
            let Some(popover) = window.get_webview_window(window.label()) else {
                return;
            };
            if let WindowEvent::Focused(false) = event {
                if put_away(&popover).is_ok() {
                    *window.state::<LastAutoHide>().0.lock().unwrap() = Some(Instant::now());
                }
            }
            // The app switcher or the dock can bring the popover back without
            // going through `request_popover`, so what that does before
            // showing is repeated here.
            #[cfg(target_os = "linux")]
            if let WindowEvent::Focused(true) = event {
                sync::expire_by_clock(window.app_handle());
                place_popover(&popover);
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
            // click, so the menu is the tray's only way to reach the popover.
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
                    "open" => request_popover(app, popover::Request::TrayClick),
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
                        request_popover(tray.app_handle(), popover::Request::TrayClick);
                    }
                })
                .build(app)?;

            sync::start(app.handle());

            // Shown from the start as far as GNOME is concerned, but
            // minimized: the app switcher and the dock list Grapevine before
            // the popover has ever been opened. Minimizing before showing
            // keeps the popover from flashing up at launch. The window config
            // skips the taskbar, which only macOS wants now.
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_skip_taskbar(false);
                place_popover(&window);
                let _ = window.minimize();
                let _ = window.show();
            }

            #[cfg(target_os = "linux")]
            if app
                .state::<EarlySecondLaunch>()
                .0
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                request_popover(app.handle(), popover::Request::SecondLaunch);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
