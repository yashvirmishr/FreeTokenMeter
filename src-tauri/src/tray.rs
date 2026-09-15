//! System tray integration.
//!
//! The tray icon itself is declared in `tauri.conf.json` (`app.trayIcon`), which
//! is how Tauri 2 bundles and registers it. This module attaches the menu,
//! handlers and dynamic state to that icon.
//!
//! Menu:
//! ```text
//! FreeTokenMeter
//! ────────────────
//! Show
//! Hide
//! Sync Now
//! ✓ Always on Top
//! ────────────────
//! Quit
//! ```
//!
//! Behaviour: left-clicking the icon shows/focuses the window, right-clicking
//! opens the menu (`showMenuOnLeftClick: false` in the config). Everything the
//! menu triggers runs the same code paths as the UI.

use log::warn;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconEvent},
    AppHandle, Runtime,
};

use crate::sync::SyncReport;

/// Tray id, matching `app.trayIcon.id` in `tauri.conf.json`.
pub const TRAY_ID: &str = "main-tray";

pub const MENU_TITLE: &str = "tray-title";
pub const MENU_SHOW: &str = "tray-show";
pub const MENU_HIDE: &str = "tray-hide";
pub const MENU_SYNC: &str = "tray-sync-now";
pub const MENU_ALWAYS_ON_TOP: &str = "tray-always-on-top";
pub const MENU_QUIT: &str = "tray-quit";

/// Unfocused tray icon menu entry.
struct TrayMenu<R: Runtime> {
    title: MenuItem<R>,
    show: MenuItem<R>,
    hide: MenuItem<R>,
    sync_now: MenuItem<R>,
    always_on_top: CheckMenuItem<R>,
    separator_one: PredefinedMenuItem<R>,
    separator_two: PredefinedMenuItem<R>,
    quit: MenuItem<R>,
}

fn build_items<R: Runtime>(app: &AppHandle<R>, always_on_top: bool) -> tauri::Result<TrayMenu<R>> {
    Ok(TrayMenu {
        title: MenuItem::with_id(app, MENU_TITLE, "FreeTokenMeter", false, None::<&str>)?,
        show: MenuItem::with_id(app, MENU_SHOW, "Show", true, None::<&str>)?,
        hide: MenuItem::with_id(app, MENU_HIDE, "Hide", true, None::<&str>)?,
        sync_now: MenuItem::with_id(app, MENU_SYNC, "Sync Now", true, None::<&str>)?,
        always_on_top: CheckMenuItem::with_id(
            app,
            MENU_ALWAYS_ON_TOP,
            "Always on Top",
            true,
            always_on_top,
            None::<&str>,
        )?,
        separator_one: PredefinedMenuItem::separator(app)?,
        separator_two: PredefinedMenuItem::separator(app)?,
        quit: MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?,
    })
}

/// Build the tray menu for the given always-on-top state.
pub fn build_menu<R: Runtime>(app: &AppHandle<R>, always_on_top: bool) -> tauri::Result<Menu<R>> {
    let items = build_items(app, always_on_top)?;
    Menu::with_items(
        app,
        &[
            &items.title,
            &items.separator_one,
            &items.show,
            &items.hide,
            &items.sync_now,
            &items.always_on_top,
            &items.separator_two,
            &items.quit,
        ],
    )
}

/// Attach the menu and handlers to the tray icon declared in the Tauri config.
///
/// Tray setup failures are logged, never fatal: FreeTokenMeter must still start
/// (and keep syncing) if the platform has no usable tray.
pub fn setup(app: &AppHandle) {
    if let Err(e) = setup_inner(app) {
        warn!("System tray setup failed ({}); continuing without tray", e);
    }
}

fn setup_inner(app: &AppHandle) -> tauri::Result<()> {
    let always_on_top = crate::window::is_always_on_top(app);

    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        warn!("Tray icon '{}' was not created by Tauri; tray menu skipped", TRAY_ID);
        return Ok(());
    };

    tray.set_menu(Some(build_menu(app, always_on_top)?))?;
    tray.set_tooltip(Some(DEFAULT_TOOLTIP))?;

    // Menu events are global in Tauri 2; filter by our item ids.
    tray.on_menu_event(|app, event| {
        handle_menu_event(app, event.id().as_ref());
    });

    // Left click shows/focuses the window, right click opens the menu.
    tray.on_tray_icon_event(|tray, event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            crate::window::show_main_window(tray.app_handle());
        }
    });

    Ok(())
}

pub const DEFAULT_TOOLTIP: &str = "FreeTokenMeter — token usage monitor";

/// Rebuild the menu so the checkmark reflects the real native state.
pub fn set_always_on_top_checked<R: Runtime>(app: &AppHandle<R>, enabled: bool) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    match build_menu(app, enabled) {
        Ok(menu) => {
            if let Err(e) = tray.set_menu(Some(menu)) {
                warn!("Failed to update tray menu: {}", e);
            }
        }
        Err(e) => warn!("Failed to rebuild tray menu: {}", e),
    }
}

/// Show the most recent sync result in the tray tooltip — visible without
/// needing OS notification permissions.
pub fn set_sync_status_tooltip<R: Runtime>(app: &AppHandle<R>, report: &SyncReport) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    let mut tooltip = String::from("FreeTokenMeter — last sync\n");
    tooltip.push_str(&report.summary_lines().join("\n"));
    if let Err(e) = tray.set_tooltip(Some(tooltip)) {
        warn!("Failed to update tray tooltip: {}", e);
    }
}

/// Route a tray menu selection to the shared application logic.
fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        MENU_SHOW => crate::window::show_main_window(app),
        MENU_HIDE => crate::window::hide_main_window(app),
        // Exactly the same synchronization path as the UI's Sync button.
        MENU_SYNC => crate::sync::spawn_sync_now(app.clone()),
        MENU_ALWAYS_ON_TOP => {
            if let Err(e) = crate::window::toggle_always_on_top(app) {
                warn!("Failed to toggle always-on-top from tray: {}", e);
            }
        }
        MENU_QUIT => crate::window::quit_application(app),
        _ => {}
    }
}
