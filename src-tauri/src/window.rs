//! Desktop window behaviour.
//!
//! Owns everything that touches the native window: show/hide, always-on-top,
//! geometry restore + debounced persistence, and application shutdown.
//!
//! The native window is the single source of truth for state such as
//! always-on-top; the UI only mirrors what the window reports.

use std::sync::atomic::Ordering;
use std::time::Duration;

use log::{info, warn};
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow,
};

use crate::preferences::{resolve_geometry, MonitorRect};
use crate::AppState;

/// Label of the main (only) window, as defined in `tauri.conf.json`.
pub const MAIN_WINDOW_LABEL: &str = "main";

/// How long the window must stay still before geometry is written to disk.
const GEOMETRY_DEBOUNCE: Duration = Duration::from_millis(750);

/// Tick interval of the geometry persister thread.
const GEOMETRY_POLL: Duration = Duration::from_millis(200);

/// Terminal-style state of the desktop shell, mirrored by the UI.
#[derive(Debug, Clone, Serialize)]
pub struct WindowState {
    pub always_on_top: bool,
    pub visible: bool,
    pub width: u32,
    pub height: u32,
    pub x: Option<i32>,
    pub y: Option<i32>,
}

pub fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(MAIN_WINDOW_LABEL)
}

/// Show, unminimize and focus the main window (tray "Show", tray icon click).
pub fn show_main_window(app: &AppHandle) {
    let Some(window) = main_window(app) else {
        warn!("Main window not available");
        return;
    };
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

/// Hide the main window to the tray. Background synchronization keeps running.
pub fn hide_main_window(app: &AppHandle) {
    if let Some(window) = main_window(app) {
        let _ = window.hide();
        let _ = app.emit("window://hidden", ());
    }
}

/// Current native always-on-top state.
pub fn is_always_on_top(app: &AppHandle) -> bool {
    main_window(app)
        .and_then(|w| w.is_always_on_top().ok())
        .unwrap_or(false)
}

/// Current native window visibility.
pub fn is_window_visible(app: &AppHandle) -> bool {
    main_window(app)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// Snapshot of the desktop shell state for the UI.
pub fn window_state(app: &AppHandle) -> WindowState {
    let prefs = app
        .state::<AppState>()
        .prefs
        .lock()
        .map(|p| p.data().clone())
        .unwrap_or_default();

    WindowState {
        always_on_top: is_always_on_top(app),
        visible: is_window_visible(app),
        width: prefs.window_width,
        height: prefs.window_height,
        x: prefs.window_x,
        y: prefs.window_y,
    }
}

/// Apply always-on-top to the native window, persist it, and reflect the real
/// state in the tray menu and the UI.
///
/// The persisted preference is only updated after the native call succeeds, so
/// preferences can never disagree with the actual window.
pub fn apply_always_on_top(app: &AppHandle, enabled: bool) -> Result<bool, String> {
    let window = main_window(app).ok_or_else(|| "Main window is not available".to_string())?;
    window
        .set_always_on_top(enabled)
        .map_err(|e| format!("Failed to set always-on-top: {}", e))?;

    // Read back the native state instead of trusting the request.
    let actual = window.is_always_on_top().unwrap_or(enabled);

    {
        let state = app.state::<AppState>();
        let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
        prefs.set_always_on_top(actual)?;
    }

    crate::tray::set_always_on_top_checked(app, actual);
    let _ = app.emit("prefs://changed", window_state(app));
    info!("Always on top set to {}", actual);
    Ok(actual)
}

/// Flip always-on-top based on the real native state.
pub fn toggle_always_on_top(app: &AppHandle) -> Result<bool, String> {
    let current = is_always_on_top(app);
    apply_always_on_top(app, !current)
}

fn monitor_rect(monitor: &tauri::Monitor) -> MonitorRect {
    MonitorRect {
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
    }
}

/// Restore the saved window geometry, validating it against the monitors that
/// are actually connected right now.
pub fn restore_geometry(app: &AppHandle) {
    let Some(window) = main_window(app) else {
        return;
    };

    let prefs = app
        .state::<AppState>()
        .prefs
        .lock()
        .map(|p| p.data().clone())
        .unwrap_or_default();

    let monitors: Vec<MonitorRect> = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(monitor_rect)
        .collect();
    let primary = window
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| monitor_rect(&m))
        .or_else(|| monitors.first().copied());

    let geo = resolve_geometry(&prefs, &monitors, primary);

    if let Err(e) = window.set_size(PhysicalSize::new(geo.width, geo.height)) {
        warn!("Failed to restore window size: {}", e);
    }

    // Only force a position when one was saved; otherwise let the platform
    // place the (first-run) window.
    if prefs.window_x.is_some() && prefs.window_y.is_some() {
        if let Err(e) = window.set_position(PhysicalPosition::new(geo.x, geo.y)) {
            warn!("Failed to restore window position: {}", e);
        }
    }

    info!(
        "Restored window geometry: {}x{} at ({}, {})",
        geo.width, geo.height, geo.x, geo.y
    );
}

/// Read the current window geometry as (width, height, x, y).
fn read_geometry(window: &WebviewWindow) -> Option<(u32, u32, Option<i32>, Option<i32>)> {
    let size = window.inner_size().ok()?;
    let position = window.outer_position().ok();
    Some((
        size.width,
        size.height,
        position.map(|p| p.x),
        position.map(|p| p.y),
    ))
}

/// Persist the window geometry immediately (used at shutdown).
pub fn persist_geometry(app: &AppHandle) {
    let Some(window) = main_window(app) else {
        return;
    };
    let Some((width, height, x, y)) = read_geometry(&window) else {
        return;
    };

    let state = app.state::<AppState>();
    let Ok(mut prefs) = state.prefs.lock() else {
        return;
    };
    match prefs.set_geometry(width, height, x, y) {
        Ok(true) => info!("Persisted window geometry {}x{} at ({:?}, {:?})", width, height, x, y),
        Ok(false) => {}
        Err(e) => warn!("Failed to persist window geometry: {}", e),
    }
}

/// Debounced geometry persistence.
///
/// `Resized`/`Moved` events only bump a counter; this thread waits until the
/// window has been still for [`GEOMETRY_DEBOUNCE`] before writing to disk, so
/// dragging a window never causes a write per pixel.
pub fn spawn_geometry_persister(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_saved_generation = 0u64;

        loop {
            std::thread::sleep(GEOMETRY_POLL);

            let state = app.state::<AppState>();
            if state.shutdown.load(Ordering::SeqCst) {
                persist_geometry(&app);
                return;
            }

            let generation = state.geometry_events.load(Ordering::SeqCst);
            if generation == last_saved_generation {
                continue;
            }

            // Wait out the debounce window; if the window moves again we simply
            // loop around and wait again.
            std::thread::sleep(GEOMETRY_DEBOUNCE);
            if app.state::<AppState>().geometry_events.load(Ordering::SeqCst) != generation {
                continue;
            }

            last_saved_generation = generation;
            persist_geometry(&app);
        }
    });
}

/// Mark the window geometry as changed (called from window events).
pub fn mark_geometry_dirty(app: &AppHandle) {
    app.state::<AppState>()
        .geometry_events
        .fetch_add(1, Ordering::SeqCst);
}

/// `true` once the user asked the application to quit.
pub fn is_shutting_down(app: &AppHandle) -> bool {
    app.state::<AppState>().shutdown.load(Ordering::SeqCst)
}

/// Terminate the application: stop background loops, persist geometry, exit.
pub fn quit_application(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.shutdown.store(true, Ordering::SeqCst);
    // Fold the very latest geometry into the preferences before exiting.
    persist_geometry(app);
    info!("Quit requested; shutting down");
    app.exit(0);
}
