//! Persistent application preferences.
//!
//! Preferences are stored as a small JSON document next to the usage database
//! (`<data-dir>/freetokenmeter/preferences.json`). SQLite is reserved for
//! usage history; UI/desktop settings stay in this lightweight file.
//!
//! The file is written atomically (temp file + rename) and is debounced by the
//! caller so window dragging never causes a write per pixel.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Default window size used on first launch.
pub const DEFAULT_WINDOW_WIDTH: u32 = 720;
pub const DEFAULT_WINDOW_HEIGHT: u32 = 640;

/// Smallest usable widget footprint. The terminal UI stays readable here.
pub const MIN_WINDOW_WIDTH: u32 = 360;
pub const MIN_WINDOW_HEIGHT: u32 = 300;

/// Guard against absurd values in a hand-edited preferences file.
const MAX_WINDOW_DIMENSION: u32 = 16_384;

/// User preferences for the desktop shell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPreferences {
    /// Keep the window above other windows.
    pub always_on_top: bool,

    /// Persisted window width in physical pixels.
    pub window_width: u32,

    /// Persisted window height in physical pixels.
    pub window_height: u32,

    /// Persisted window x position in physical pixels (`None` = let the OS place it).
    pub window_x: Option<i32>,

    /// Persisted window y position in physical pixels (`None` = let the OS place it).
    pub window_y: Option<i32>,

    /// Providers the user has switched off. Empty means "nothing disabled", so
    /// a provider added in a later release starts enabled by default.
    pub disabled_providers: Vec<String>,

    /// How much history to import the first time a provider syncs.
    /// `0` means all available history.
    pub backfill_days: i64,
}

/// Default first-import window. Deliberately bounded: a full history import on
/// first launch should not stall the dashboard.
pub const DEFAULT_BACKFILL_DAYS: i64 = 30;

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            always_on_top: false,
            window_width: DEFAULT_WINDOW_WIDTH,
            window_height: DEFAULT_WINDOW_HEIGHT,
            window_x: None,
            window_y: None,
            disabled_providers: Vec::new(),
            backfill_days: DEFAULT_BACKFILL_DAYS,
        }
    }
}

impl AppPreferences {
    /// Clamp values that could make the window unusable before they are used.
    pub fn sanitize(&mut self) {
        if self.window_width == 0 {
            self.window_width = DEFAULT_WINDOW_WIDTH;
        }
        if self.window_height == 0 {
            self.window_height = DEFAULT_WINDOW_HEIGHT;
        }
        self.window_width = self.window_width.clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_DIMENSION);
        self.window_height = self.window_height.clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_DIMENSION);

        if self.backfill_days < 0 {
            self.backfill_days = DEFAULT_BACKFILL_DAYS;
        }

        // Keep the disabled list canonical so two equivalent preferences files
        // never compare as different.
        self.disabled_providers.sort();
        self.disabled_providers.dedup();
    }

    /// Whether a provider should synchronize.
    pub fn is_provider_enabled(&self, provider: &str) -> bool {
        !self
            .disabled_providers
            .iter()
            .any(|disabled| disabled == provider)
    }
}

/// A monitor rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl MonitorRect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + self.width as i32
            && y < self.y + self.height as i32
    }

    fn right(&self) -> i32 {
        self.x.saturating_add(self.width as i32)
    }

    fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }
}

/// A validated window geometry that is guaranteed to be reachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedGeometry {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
}

/// Fallback screen area when the platform reports no monitors at all.
const FALLBACK_MONITOR: MonitorRect = MonitorRect {
    x: 0,
    y: 0,
    width: 1280,
    height: 720,
};

/// How much of the saved rectangle lands on a given monitor.
fn overlap_area(monitors: &MonitorRect, x: i32, y: i32, w: u32, h: u32) -> i64 {
    let left = x.max(monitors.x);
    let top = y.max(monitors.y);
    let right = (x.saturating_add(w as i32)).min(monitors.right());
    let bottom = (y.saturating_add(h as i32)).min(monitors.bottom());
    let w = (right - left).max(0) as i64;
    let h = (bottom - top).max(0) as i64;
    w * h
}

/// Choose the monitor the window should be restored onto.
fn pick_monitor(
    prefs: &AppPreferences,
    monitors: &[MonitorRect],
    primary: Option<MonitorRect>,
) -> MonitorRect {
    // Preferred: the monitor that contains the saved top-left corner.
    if let (Some(x), Some(y)) = (prefs.window_x, prefs.window_y) {
        if let Some(m) = monitors.iter().find(|m| m.contains(x, y)) {
            return *m;
        }
        // Otherwise the monitor that the saved rectangle mostly covers.
        let best = monitors
            .iter()
            .map(|m| {
                (
                    overlap_area(m, x, y, prefs.window_width, prefs.window_height),
                    *m,
                )
            })
            .max_by_key(|(area, _)| *area);
        if let Some((area, m)) = best {
            if area > 0 {
                return m;
            }
        }
    }

    // Nothing usable saved: prefer the real primary monitor, then the first
    // monitor the platform reports.
    primary
        .or_else(|| monitors.first().copied())
        .unwrap_or(FALLBACK_MONITOR)
}

/// Turn saved preferences into a geometry that is always on-screen.
///
/// Handles: monitor removed, resolution changed, off-screen position, window
/// smaller than the minimum, and window larger than the available space.
pub fn resolve_geometry(
    prefs: &AppPreferences,
    monitors: &[MonitorRect],
    primary: Option<MonitorRect>,
) -> ResolvedGeometry {
    let mut prefs = prefs.clone();
    prefs.sanitize();

    let target = pick_monitor(&prefs, monitors, primary);

    // Never exceed the target monitor; never below the widget minimum unless the
    // monitor itself is smaller than that minimum.
    let width = prefs
        .window_width
        .min(target.width)
        .max(MIN_WINDOW_WIDTH.min(target.width));
    let height = prefs
        .window_height
        .min(target.height)
        .max(MIN_WINDOW_HEIGHT.min(target.height));

    // Clamp the position so the window stays fully visible where possible.
    let (x, y) = match (prefs.window_x, prefs.window_y) {
        (Some(x), Some(y)) => (x, y),
        _ => (target.x, target.y),
    };

    let max_x = target.right().saturating_sub(width as i32);
    let max_y = target.bottom().saturating_sub(height as i32);
    let clamped_x = x.clamp(target.x, max_x.max(target.x));
    let clamped_y = y.clamp(target.y, max_y.max(target.y));

    ResolvedGeometry {
        width,
        height,
        x: clamped_x,
        y: clamped_y,
    }
}

/// File-backed preference store.
#[derive(Debug, Clone)]
pub struct PreferencesStore {
    path: PathBuf,
    data: AppPreferences,
}

impl PreferencesStore {
    /// Load preferences from `path`, falling back to defaults when the file is
    /// missing, unreadable, or malformed. Loading never fails the app.
    pub fn load(path: PathBuf) -> Self {
        let data = match std::fs::read_to_string(&path) {
            Ok(raw) => {
                let mut parsed: AppPreferences = serde_json::from_str(&raw).unwrap_or_else(|e| {
                    log::warn!(
                        "Preferences file {} is invalid ({}); using defaults",
                        path.display(),
                        e
                    );
                    AppPreferences::default()
                });
                parsed.sanitize();
                parsed
            }
            Err(_) => AppPreferences::default(),
        };

        Self { path, data }
    }

    pub fn data(&self) -> &AppPreferences {
        &self.data
    }

    /// Enable or disable one provider. Persists across restarts.
    pub fn set_provider_enabled(
        &mut self,
        provider: &str,
        enabled: bool,
    ) -> Result<AppPreferences, String> {
        let mut next = self.data.clone();
        next.disabled_providers
            .retain(|disabled| disabled != provider);
        if !enabled {
            next.disabled_providers.push(provider.to_string());
        }
        next.sanitize();

        if next.disabled_providers == self.data.disabled_providers {
            return Ok(self.data.clone());
        }

        self.data = next;
        self.save()?;
        Ok(self.data.clone())
    }

    /// Change the first-import window. `0` means all available history.
    pub fn set_backfill_days(&mut self, days: i64) -> Result<AppPreferences, String> {
        let mut next = self.data.clone();
        next.backfill_days = days.max(0);
        next.sanitize();

        if next.backfill_days == self.data.backfill_days {
            return Ok(self.data.clone());
        }

        self.data = next;
        self.save()?;
        Ok(self.data.clone())
    }

    pub fn set_always_on_top(&mut self, enabled: bool) -> Result<AppPreferences, String> {
        if self.data.always_on_top == enabled {
            return Ok(self.data.clone());
        }
        self.data.always_on_top = enabled;
        self.save()?;
        Ok(self.data.clone())
    }

    /// Persist window geometry. Returns `false` when nothing changed so callers
    /// can avoid needless disk writes.
    pub fn set_geometry(
        &mut self,
        width: u32,
        height: u32,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<bool, String> {
        let mut next = self.data.clone();
        next.window_width = width;
        next.window_height = height;
        next.window_x = x;
        next.window_y = y;
        next.sanitize();

        if next.window_width == self.data.window_width
            && next.window_height == self.data.window_height
            && next.window_x == self.data.window_x
            && next.window_y == self.data.window_y
        {
            return Ok(false);
        }

        self.data = next;
        self.save()?;
        Ok(true)
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
        }

        let json = serde_json::to_string_pretty(&self.data)
            .map_err(|e| format!("Failed to serialize preferences: {}", e))?;

        // Write to a temp file and rename so a crash can never leave a
        // half-written preferences file behind.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json.as_bytes())
            .map_err(|e| format!("Failed to write {}: {}", tmp.display(), e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("Failed to persist {}: {}", self.path.display(), e)
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_prefs_path(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "ftm-prefs-test-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("preferences.json")
    }

    const PRIMARY: MonitorRect = MonitorRect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    fn prefs_at(width: u32, height: u32, x: Option<i32>, y: Option<i32>) -> AppPreferences {
        AppPreferences {
            always_on_top: false,
            window_width: width,
            window_height: height,
            window_x: x,
            window_y: y,
            ..Default::default()
        }
    }

    /// Resolve against a single primary monitor.
    fn resolve_single(prefs: &AppPreferences) -> ResolvedGeometry {
        resolve_geometry(prefs, &[PRIMARY], Some(PRIMARY))
    }

    // ── Persistence ───────────────────────────────────────────────────

    #[test]
    fn missing_file_yields_defaults() {
        let path = temp_prefs_path("missing");
        let store = PreferencesStore::load(path.clone());
        assert_eq!(store.data(), &AppPreferences::default());
        assert!(!path.exists(), "load must not create the file");
    }

    #[test]
    fn always_on_top_round_trips_across_restart() {
        let path = temp_prefs_path("aot");

        {
            let mut store = PreferencesStore::load(path.clone());
            assert!(!store.data().always_on_top);
            store.set_always_on_top(true).unwrap();
        }

        // Simulate a restart: a brand new store reads the same file.
        let reloaded = PreferencesStore::load(path);
        assert!(reloaded.data().always_on_top, "always-on-top did not persist");
    }

    #[test]
    fn geometry_round_trips_across_restart() {
        let path = temp_prefs_path("geometry");

        {
            let mut store = PreferencesStore::load(path.clone());
            let changed = store.set_geometry(900, 600, Some(120), Some(80)).unwrap();
            assert!(changed);
        }

        let reloaded = PreferencesStore::load(path);
        assert_eq!(reloaded.data().window_width, 900);
        assert_eq!(reloaded.data().window_height, 600);
        assert_eq!(reloaded.data().window_x, Some(120));
        assert_eq!(reloaded.data().window_y, Some(80));
    }

    #[test]
    fn unchanged_geometry_is_not_rewritten() {
        let path = temp_prefs_path("nochange");
        let mut store = PreferencesStore::load(path);
        assert!(store.set_geometry(900, 600, Some(10), Some(20)).unwrap());
        assert!(
            !store.set_geometry(900, 600, Some(10), Some(20)).unwrap(),
            "identical geometry should be a no-op"
        );
    }

    #[test]
    fn corrupt_preferences_file_falls_back_to_defaults() {
        let path = temp_prefs_path("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ this is not json").unwrap();

        let store = PreferencesStore::load(path);
        assert_eq!(store.data(), &AppPreferences::default());
    }

    #[test]
    fn partial_preferences_file_keeps_other_defaults() {
        let path = temp_prefs_path("partial");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, br#"{"always_on_top": true}"#).unwrap();

        let store = PreferencesStore::load(path);
        assert!(store.data().always_on_top);
        assert_eq!(store.data().window_width, DEFAULT_WINDOW_WIDTH);
        assert_eq!(store.data().window_height, DEFAULT_WINDOW_HEIGHT);
    }

    // ── Geometry validation ───────────────────────────────────────────

    #[test]
    fn valid_geometry_is_preserved() {
        let prefs = prefs_at(900, 600, Some(100), Some(100));
        let geo = resolve_single(&prefs);
        assert_eq!(
            geo,
            ResolvedGeometry {
                width: 900,
                height: 600,
                x: 100,
                y: 100
            }
        );
    }

    #[test]
    fn off_screen_position_is_clamped_back_into_view() {
        let prefs = prefs_at(900, 600, Some(9000), Some(8000));
        let geo = resolve_single(&prefs);
        assert!(geo.x >= 0 && geo.x + geo.width as i32 <= 1920, "x={}", geo.x);
        assert!(geo.y >= 0 && geo.y + geo.height as i32 <= 1080, "y={}", geo.y);
    }

    #[test]
    fn disconnected_secondary_monitor_repositions_window() {
        // Saved on a secondary display that no longer exists.
        let prefs = prefs_at(800, 600, Some(2200), Some(300));
        let geo = resolve_single(&prefs);
        assert!(PRIMARY.contains(geo.x, geo.y));
        assert!(geo.x + geo.width as i32 <= PRIMARY.width as i32);
        assert!(geo.y + geo.height as i32 <= PRIMARY.height as i32);
    }

    #[test]
    fn negative_secondary_monitor_is_supported() {
        let left_monitor = MonitorRect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let prefs = prefs_at(800, 600, Some(-1800), Some(200));
        let geo = resolve_geometry(&prefs, &[left_monitor, PRIMARY], Some(PRIMARY));
        assert_eq!(geo.x, -1800);
        assert_eq!(geo.y, 200);
    }

    #[test]
    fn tiny_geometry_is_grown_to_the_minimum() {
        let prefs = prefs_at(50, 40, Some(10), Some(10));
        let geo = resolve_single(&prefs);
        assert_eq!(geo.width, MIN_WINDOW_WIDTH);
        assert_eq!(geo.height, MIN_WINDOW_HEIGHT);
    }

    #[test]
    fn oversized_geometry_is_shrunk_to_the_screen() {
        let prefs = prefs_at(5000, 4000, Some(0), Some(0));
        let geo = resolve_single(&prefs);
        assert!(geo.width <= 1920, "width={}", geo.width);
        assert!(geo.height <= 1080, "height={}", geo.height);
    }

    #[test]
    fn resolution_change_keeps_window_visible() {
        // Saved on a 2560x1440 display, now running on a 1366x768 laptop panel.
        let small = MonitorRect {
            x: 0,
            y: 0,
            width: 1366,
            height: 768,
        };
        let prefs = prefs_at(1400, 1000, Some(1200), Some(700));
        let geo = resolve_geometry(&prefs, &[small], Some(small));
        assert!(geo.width <= 1366);
        assert!(geo.height <= 768);
        assert!(geo.x >= 0 && geo.x + geo.width as i32 <= 1366);
        assert!(geo.y >= 0 && geo.y + geo.height as i32 <= 768);
    }

    #[test]
    fn geometry_without_position_lands_on_the_primary_monitor() {
        let prefs = prefs_at(800, 600, None, None);
        let geo = resolve_single(&prefs);
        assert_eq!(geo.x, PRIMARY.x);
        assert_eq!(geo.y, PRIMARY.y);
    }

    #[test]
    fn no_monitors_reported_still_yields_usable_geometry() {
        let prefs = prefs_at(900, 600, Some(400), Some(300));
        let geo = resolve_geometry(&prefs, &[], None);
        assert_eq!(geo.width, 900);
        assert_eq!(geo.height, 600);
        assert!(geo.x >= 0 && geo.y >= 0);
        assert!((geo.x + geo.width as i32) <= FALLBACK_MONITOR.width as i32);
    }

    #[test]
    fn zero_dimensions_fall_back_to_defaults() {
        let prefs = prefs_at(0, 0, Some(50), Some(50));
        let geo = resolve_single(&prefs);
        // 0 is treated as "unset" and replaced by the default widget size.
        assert_eq!(geo.width, DEFAULT_WINDOW_WIDTH);
        assert_eq!(geo.height, DEFAULT_WINDOW_HEIGHT);
    }
}
