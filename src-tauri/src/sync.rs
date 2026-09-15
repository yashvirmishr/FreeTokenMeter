//! Provider synchronization.
//!
//! This module owns the single synchronization path used by:
//!   - the UI (`sync_all` command + Sync button)
//!   - the tray "Sync Now" action
//!   - the background scheduler that keeps collecting data while the window is
//!     hidden to the tray
//!
//! # How a pass works
//!
//! ```text
//! registered providers
//!        ↓  filter: enabled by the user + a readable source
//!   bounded parallel fetch          (one slow provider cannot block the rest)
//!        ↓
//!   serialized dedup + write        (one SQLite connection)
//!        ↓
//!   per-provider cursor persisted
//!        ↓
//!   SyncReport → UI + tray
//! ```
//!
//! Providers are isolated twice over: their `fetch` runs inside
//! `catch_unwind`, and any error is reported per provider. A failing (or
//! panicking) provider never prevents another provider from syncing and never
//! takes down the application.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::database::UsageRecord;
use crate::provider::{FetchOutcome, FetchRequest, ProviderCursor, UsageProvider};
use crate::AppState;

/// Maximum number of providers fetched at the same time.
const MAX_PARALLEL_FETCHES: usize = 4;

/// Upper bound on records imported from one provider in a single pass, so a
/// first-time backfill of a huge history cannot stall the dashboard.
pub const SYNC_RECORD_LIMIT: usize = 20_000;

/// Result of one provider within a pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderSyncResult {
    /// Canonical provider id.
    pub provider: String,
    /// Display name used by the terminal-style banner.
    pub name: String,
    /// `true` when the provider synced without errors.
    pub ok: bool,
    /// `true` when the provider was not synced at all (disabled or no source).
    pub skipped: bool,
    /// Human-readable status or error.
    pub message: String,
    /// Records written or updated (an upsert of an unchanged session still
    /// counts here, but adds no tokens).
    pub records_synced: usize,
    /// Net new tokens added to the local history by this sync.
    pub new_tokens: i64,
    /// Source events examined.
    pub events_scanned: usize,
    /// Source events skipped (malformed, outside the window, no tokens).
    pub events_skipped: usize,
    /// Wall-clock duration of this provider's fetch and write.
    pub duration_ms: i64,
}

/// Result of a full pass across every registered provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    pub providers: Vec<ProviderSyncResult>,
    pub started_at: i64,
    pub finished_at: i64,
    /// Total new tokens across every provider.
    pub new_tokens: i64,
}

impl SyncReport {
    /// `true` when every provider that was actually attempted succeeded.
    pub fn all_ok(&self) -> bool {
        self.providers.iter().all(|p| p.ok || p.skipped)
    }

    /// Look up one provider's result. Used by the test suite.
    #[cfg(test)]
    pub fn get(&self, provider: &str) -> Option<&ProviderSyncResult> {
        self.providers.iter().find(|p| p.provider == provider)
    }

    /// Lines for the terminal-style status banner, e.g.
    /// `OpenCode: +12,482 tokens`.
    pub fn summary_lines(&self) -> Vec<String> {
        self.providers
            .iter()
            .map(|result| {
                if result.skipped {
                    format!("{}: {}", result.name, result.message)
                } else if result.ok {
                    format!(
                        "{}: +{} tokens",
                        result.name,
                        format_thousands(result.new_tokens)
                    )
                } else {
                    format!("{}: FAILED - {}", result.name, result.message)
                }
            })
            .collect()
    }
}

/// Group digits with commas so token counts read like the rest of the UI.
fn format_thousands(n: i64) -> String {
    let negative = n < 0;
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if negative {
        format!("-{}", out)
    } else {
        out
    }
}

/// The event-time window used the first time a provider syncs.
fn backfill_cutoff(days: i64) -> Option<i64> {
    if days <= 0 {
        return None;
    }
    Some(Utc::now().timestamp_millis() - days * 24 * 60 * 60 * 1000)
}

/// A provider's fetch, as produced by the parallel phase.
struct FetchedOutcome {
    index: usize,
    provider: String,
    name: String,
    outcome: Result<(FetchOutcome, i64), String>,
}

/// Synchronize every registered provider. Never returns an error: failures are
/// reported per provider inside the [`SyncReport`].
pub fn sync_all(state: &AppState) -> SyncReport {
    let started_at = Utc::now().timestamp_millis();

    let prefs = state
        .prefs
        .lock()
        .map(|p| p.data().clone())
        .unwrap_or_default();

    let providers: Vec<&dyn UsageProvider> = state.providers.iter().collect();

    // ── Phase 1: fetch in parallel, bounded ────────────────────────────────
    let index = AtomicUsize::new(0);
    let fetched: Mutex<Vec<FetchedOutcome>> = Mutex::new(Vec::new());
    let index_ref = &index;
    let fetched_ref = &fetched;
    let providers_ref = &providers;
    let prefs_ref = &prefs;
    let state_ref: &AppState = state;

    let worker_count = providers.len().min(MAX_PARALLEL_FETCHES).max(1);
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(move || loop {
                let position = index_ref.fetch_add(1, Ordering::SeqCst);
                let Some(provider) = providers_ref.get(position) else {
                    break;
                };

                let id = provider.id().to_string();
                let name = provider.display_name().to_string();

                if !prefs_ref.is_provider_enabled(&id) {
                    fetched_ref.lock().unwrap().push(FetchedOutcome {
                        index: position,
                        provider: id,
                        name,
                        outcome: Err("provider disabled in settings".to_string()),
                    });
                    continue;
                }

                if !provider.availability().is_connected() {
                    fetched_ref.lock().unwrap().push(FetchedOutcome {
                        index: position,
                        provider: id,
                        name,
                        outcome: Err(format!(
                            "no readable source ({})",
                            provider
                                .detail()
                                .unwrap_or_else(|| provider.availability().label().to_string())
                        )),
                    });
                    continue;
                }

                let started = Instant::now();
                let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    fetch_one(state_ref, *provider, prefs_ref)
                }));
                let elapsed = started.elapsed().as_millis() as i64;

                let outcome = match result {
                    Ok(Ok(outcome)) => Ok((outcome, elapsed)),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err("provider sync panicked".to_string()),
                };

                fetched_ref.lock().unwrap().push(FetchedOutcome {
                    index: position,
                    provider: id,
                    name,
                    outcome,
                });
            });
        }
    });

    // ── Phase 2: dedup + write serially, one connection ───────────────────
    let mut outcomes = fetched.into_inner().unwrap_or_default();
    outcomes.sort_by_key(|outcome| outcome.index);

    let mut results = Vec::with_capacity(outcomes.len());
    for entry in outcomes {
        results.push(persist_outcome(state, entry));
    }

    let finished_at = Utc::now().timestamp_millis();
    let report = SyncReport {
        new_tokens: results.iter().map(|r| r.new_tokens).sum(),
        providers: results,
        started_at,
        finished_at,
    };

    let history_size = state
        .db
        .lock()
        .ok()
        .and_then(|db| db.count_records().ok())
        .unwrap_or(0);

    if report.all_ok() {
        info!(
            "Sync pass finished in {}ms ({} new tokens, {} records in history)",
            report.finished_at - report.started_at,
            report.new_tokens,
            history_size
        );
    } else {
        warn!(
            "Sync pass finished with provider errors in {}ms ({} new tokens)",
            report.finished_at - report.started_at,
            report.new_tokens
        );
    }

    report
}

/// Load a provider's cursor and run its fetch.
fn fetch_one(
    state: &AppState,
    provider: &dyn UsageProvider,
    prefs: &crate::preferences::AppPreferences,
) -> Result<FetchOutcome, String> {
    let cursor_json = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_sync_cursor(provider.id()).map_err(|e| e.to_string())?
    };

    let cursor: Option<ProviderCursor> = cursor_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok());

    // Incremental when a cursor exists; otherwise a bounded backfill.
    let since_ms = if cursor.is_some() {
        None
    } else {
        backfill_cutoff(prefs.backfill_days)
    };

    let request = FetchRequest {
        cursor: cursor.as_ref(),
        since_ms,
        registry: Some(state.pricing_registry.as_ref()),
        limit: SYNC_RECORD_LIMIT,
    };

    provider.fetch(&request)
}

/// Write one provider's records and record its cursor/health.
fn persist_outcome(state: &AppState, entry: FetchedOutcome) -> ProviderSyncResult {
    let now = Utc::now().timestamp_millis();

    match entry.outcome {
        Err(message) => {
            let skipped = message.contains("disabled in settings") || message.contains("no readable source");
            if let Ok(db) = state.db.lock() {
                let _ = db.update_sync_state(
                    &entry.provider,
                    now,
                    None,
                    if skipped { "unavailable" } else { "error" },
                    Some(&message),
                );
            }
            if !skipped {
                warn!("{} sync failed: {}", entry.name, message);
            }
            ProviderSyncResult {
                provider: entry.provider,
                name: entry.name,
                ok: false,
                skipped,
                message,
                ..Default::default()
            }
        }
        Ok((outcome, duration_ms)) => {
            let tokens_before = state
                .db
                .lock()
                .ok()
                .and_then(|db| db.get_provider_token_total(&entry.provider).ok())
                .unwrap_or(0);

            let records: &Vec<UsageRecord> = &outcome.records;
            let written = match state.db.lock() {
                Ok(db) => db.upsert_usage_records(records).unwrap_or(0),
                Err(e) => {
                    error!("{}: database unavailable: {}", entry.name, e);
                    0
                }
            };

            let cursor_json = serde_json::to_string(&outcome.cursor).ok();
            if let Ok(db) = state.db.lock() {
                let _ = db.update_sync_state(
                    &entry.provider,
                    now,
                    cursor_json.as_deref(),
                    "connected",
                    None,
                );
            }

            let tokens_after = state
                .db
                .lock()
                .ok()
                .and_then(|db| db.get_provider_token_total(&entry.provider).ok())
                .unwrap_or(0);

            ProviderSyncResult {
                provider: entry.provider,
                name: entry.name,
                ok: true,
                skipped: false,
                message: format!("Synced {} record(s)", written),
                records_synced: written,
                new_tokens: (tokens_after - tokens_before).max(0),
                events_scanned: outcome.scanned,
                events_skipped: outcome.skipped,
                duration_ms,
            }
        }
    }
}

/// Store the latest report, show it in the tray tooltip and notify the UI.
pub fn publish(app: &AppHandle, report: &SyncReport) {
    let state = app.state::<AppState>();
    if let Ok(mut slot) = state.last_sync_report.lock() {
        *slot = Some(report.clone());
    }
    crate::tray::set_sync_status_tooltip(app, report);
    if let Err(e) = app.emit("sync://complete", report.clone()) {
        warn!("Failed to emit sync report: {}", e);
    }
}

/// Run a sync pass on the current thread and publish the result.
///
/// Concurrent calls are coalesced, so a queued tray request cannot stack up
/// work while a pass is already running.
pub fn sync_and_publish(app: &AppHandle) {
    if app.state::<AppState>().syncing.swap(true, Ordering::SeqCst) {
        info!("Sync already in progress; request ignored");
        return;
    }

    let report = {
        let state = app.state::<AppState>();
        sync_all(&state)
    };

    publish(app, &report);
    app.state::<AppState>().syncing.store(false, Ordering::SeqCst);
}

/// Run a sync pass on a background thread (used by the tray "Sync Now" item).
pub fn spawn_sync_now(app: AppHandle) {
    std::thread::spawn(move || sync_and_publish(&app));
}

/// How long the scheduler waits before its first pass.
const BACKGROUND_SYNC_STARTUP_DELAY: Duration = Duration::from_secs(3);

/// How often the window-independent synchronization runs.
pub const BACKGROUND_SYNC_INTERVAL_SECS: u64 = 30;
const BACKGROUND_SYNC_INTERVAL: Duration = Duration::from_secs(BACKGROUND_SYNC_INTERVAL_SECS);

/// Start the window-independent synchronization loop.
///
/// This is what keeps SQLite history up to date while the window is hidden to
/// the tray — the user does not need to keep the window open for data
/// collection to work.
pub fn spawn_background_scheduler(app: AppHandle) {
    std::thread::spawn(move || {
        if !sleep_interruptible(&app, BACKGROUND_SYNC_STARTUP_DELAY) {
            return;
        }

        loop {
            sync_and_publish(&app);
            if !sleep_interruptible(&app, BACKGROUND_SYNC_INTERVAL) {
                return;
            }
        }
    });
}

/// Sleep in small steps so shutdown is quick. Returns `false` when quitting.
fn sleep_interruptible(app: &AppHandle, total: Duration) -> bool {
    const STEP: Duration = Duration::from_millis(250);
    let mut slept = Duration::ZERO;

    while slept < total {
        let step = STEP.min(total - slept);
        std::thread::sleep(step);
        slept += step;
        if app.state::<AppState>().shutdown.load(Ordering::SeqCst) {
            return false;
        }
    }

    true
}

/// The last sync report, if any.
pub fn last_report(state: &AppState) -> Option<SyncReport> {
    state.last_sync_report.lock().ok().and_then(|r| r.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_code_provider::ClaudeCodeProvider;
    use crate::codex_provider::CodexProvider;
    use crate::database::Database;
    use crate::freebuff_provider::FreebuffProvider;
    use crate::opencode_provider::OpenCodeProvider;
    use crate::preferences::PreferencesStore;
    use crate::provider::ProviderRegistry;
    use crate::registry::RegistryState;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ftm-sync-test-{}-{}-{}",
            label,
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// A one-provider state, so each test exercises one integration.
    fn state_with(provider: Box<dyn UsageProvider>, label: &str) -> AppState {
        let dir = temp_dir(label);
        let mut providers = ProviderRegistry::new();
        providers.register(provider);

        AppState {
            db: Mutex::new(Database::new(&dir.join("freetokenmeter.db")).expect("test database")),
            providers,
            prefs: Mutex::new(PreferencesStore::load(dir.join("preferences.json"))),
            last_sync_report: Mutex::new(None),
            pricing_registry: Arc::new(RegistryState::new(&dir)),
            syncing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            geometry_events: AtomicU64::new(0),
        }
    }

    const CODEX_ROLLOUT: &str = r#"{"timestamp":"2026-09-07T17:09:31.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}
{"timestamp":"2026-09-07T17:09:32.000Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"plan_type":"free"}}}
{"timestamp":"2026-09-07T17:09:49.000Z","type":"token_usage_record","payload":{"session_id":"s1","response_id":"resp_1","usage":{"input_tokens":1000000,"output_tokens":600000,"total_tokens":1600000}}}
"#;

    fn codex_dir(root: &PathBuf) -> PathBuf {
        let dir = root.join("codex-sessions");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rollout-1.jsonl"), CODEX_ROLLOUT).unwrap();
        dir
    }

    fn fake_freebuff_project(dir: &PathBuf, project: &str) -> PathBuf {
        let project_dir = dir.join(project);
        std::fs::create_dir_all(&project_dir).unwrap();
        let db_path = project_dir.join("desktop-v2.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, model TEXT);
             CREATE TABLE messages (
                seq INTEGER, thread_id TEXT, role TEXT, metrics_json TEXT, ts INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads VALUES (?1, ?2)",
            rusqlite::params!["thread-1", "mimo/mimo-v2.5"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                1,
                "thread-1",
                "assistant",
                r#"{"usage":{"inputTokens":2000000,"cachedInputTokens":0,"outputTokens":0,"reasoningOutputTokens":0,"totalTokens":2000000},"costUsd":0}"#,
                1_700_000_100_000i64
            ],
        )
        .unwrap();
        db_path
    }

    #[test]
    fn syncs_codex_rollouts_and_reports_deltas() {
        let root = temp_dir("codex");
        let state = state_with(
            Box::new(CodexProvider::with_sessions_dir(codex_dir(&root))),
            "codex-state",
        );

        let report = sync_all(&state);
        let result = report.get("codex").expect("codex result");

        assert!(result.ok, "codex should sync: {}", result.message);
        assert_eq!(result.records_synced, 1);
        assert_eq!(result.new_tokens, 1_600_000);
        assert_eq!(report.new_tokens, 1_600_000);
        assert!(report.all_ok());
        assert_eq!(report.summary_lines(), vec!["Codex: +1,600,000 tokens"]);
    }

    #[test]
    fn repeated_syncs_do_not_double_count() {
        let root = temp_dir("codex-repeat");
        let state = state_with(
            Box::new(CodexProvider::with_sessions_dir(codex_dir(&root))),
            "codex-repeat-state",
        );

        let first = sync_all(&state);
        let second = sync_all(&state);

        assert_eq!(first.get("codex").unwrap().new_tokens, 1_600_000);
        assert_eq!(second.get("codex").unwrap().new_tokens, 0);
        assert_eq!(
            state
                .db
                .lock()
                .unwrap()
                .get_provider_token_total("codex")
                .unwrap(),
            1_600_000
        );
    }

    #[test]
    fn a_missing_provider_does_not_stop_the_others() {
        let root = temp_dir("isolation");
        let mut providers = ProviderRegistry::new();
        providers.register(Box::new(CodexProvider::with_sessions_dir(codex_dir(&root))));
        // Claude Code has no transcripts here, so it must report unavailable.
        providers.register(Box::new(ClaudeCodeProvider::with_config_dir(
            root.join("empty-claude"),
        )));
        // Freebuff has one healthy and one unreadable project database.
        let freebuff_dir = root.join("freebuff");
        fake_freebuff_project(&freebuff_dir, "Healthy");
        std::fs::create_dir_all(freebuff_dir.join("Broken")).unwrap();
        std::fs::write(freebuff_dir.join("Broken/desktop-v2.db"), b"not a database").unwrap();
        providers.register(Box::new(FreebuffProvider::with_config_dir(freebuff_dir)));

        let dir = temp_dir("isolation-state");
        let state = AppState {
            db: Mutex::new(Database::new(&dir.join("freetokenmeter.db")).unwrap()),
            providers,
            prefs: Mutex::new(PreferencesStore::load(dir.join("preferences.json"))),
            last_sync_report: Mutex::new(None),
            pricing_registry: Arc::new(RegistryState::new(&dir)),
            syncing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            geometry_events: AtomicU64::new(0),
        };

        let report = sync_all(&state);

        assert!(report.get("codex").unwrap().ok);
        assert!(report.get("freebuff").unwrap().ok);
        assert_eq!(report.get("freebuff").unwrap().records_synced, 1);
        assert_eq!(report.get("freebuff").unwrap().new_tokens, 2_000_000);

        // An unavailable provider is reported as skipped, not as a fatal error.
        let claude = report.get("claude-code").unwrap();
        assert!(!claude.ok);
        assert!(claude.skipped);
        assert!(claude.message.contains("no readable source"));
        assert!(report.all_ok());
    }

    #[test]
    fn every_registered_provider_appears_in_the_report() {
        let root = temp_dir("coverage");
        let mut providers = ProviderRegistry::new();
        providers.register(Box::new(OpenCodeProvider::with_path(
            root.join("missing-opencode.db"),
        )));
        providers.register(Box::new(CodexProvider::with_sessions_dir(
            root.join("missing-sessions"),
        )));

        let dir = temp_dir("coverage-state");
        let state = AppState {
            db: Mutex::new(Database::new(&dir.join("freetokenmeter.db")).unwrap()),
            providers,
            prefs: Mutex::new(PreferencesStore::load(dir.join("preferences.json"))),
            last_sync_report: Mutex::new(None),
            pricing_registry: Arc::new(RegistryState::new(&dir)),
            syncing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            geometry_events: AtomicU64::new(0),
        };

        let report = sync_all(&state);
        assert_eq!(report.providers.len(), 2);
        assert!(report.get("opencode").is_some());
        assert!(report.get("codex").is_some());
        assert_eq!(report.new_tokens, 0);
    }

    #[test]
    fn disabled_providers_do_not_synchronize() {
        let root = temp_dir("disabled");
        let state = state_with(
            Box::new(CodexProvider::with_sessions_dir(codex_dir(&root))),
            "disabled-state",
        );

        {
            let mut prefs = state.prefs.lock().unwrap();
            prefs.set_provider_enabled("codex", false).unwrap();
        }

        let report = sync_all(&state);
        let result = report.get("codex").unwrap();
        assert!(result.skipped);
        assert!(result.message.contains("disabled"));
        assert_eq!(
            state
                .db
                .lock()
                .unwrap()
                .get_provider_token_total("codex")
                .unwrap(),
            0
        );

        // Re-enabling brings it back.
        {
            let mut prefs = state.prefs.lock().unwrap();
            prefs.set_provider_enabled("codex", true).unwrap();
        }
        let report = sync_all(&state);
        assert!(report.get("codex").unwrap().ok);
    }

    #[test]
    fn a_panicking_provider_is_isolated() {
        struct PanickingProvider;
        impl UsageProvider for PanickingProvider {
            fn id(&self) -> &'static str {
                "boom"
            }
            fn display_name(&self) -> &'static str {
                "Boom"
            }
            fn short_label(&self) -> &'static str {
                "BOOM"
            }
            fn source_type(&self) -> &'static str {
                "boom_source"
            }
            fn payment_mode(&self) -> crate::usage::PaymentMode {
                crate::usage::PaymentMode::Unknown
            }
            fn availability(&self) -> crate::usage::ProviderAvailability {
                crate::usage::ProviderAvailability::Connected
            }
            fn detail(&self) -> Option<String> {
                None
            }
            fn fetch(&self, _request: &FetchRequest<'_>) -> Result<FetchOutcome, String> {
                panic!("provider exploded");
            }
        }

        let root = temp_dir("panic");
        let mut providers = ProviderRegistry::new();
        providers.register(Box::new(PanickingProvider));
        providers.register(Box::new(CodexProvider::with_sessions_dir(codex_dir(&root))));

        let dir = temp_dir("panic-state");
        let state = AppState {
            db: Mutex::new(Database::new(&dir.join("freetokenmeter.db")).unwrap()),
            providers,
            prefs: Mutex::new(PreferencesStore::load(dir.join("preferences.json"))),
            last_sync_report: Mutex::new(None),
            pricing_registry: Arc::new(RegistryState::new(&dir)),
            syncing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            geometry_events: AtomicU64::new(0),
        };

        let report = sync_all(&state);
        assert_eq!(report.get("boom").unwrap().message, "provider sync panicked");
        assert!(report.get("codex").unwrap().ok, "the healthy provider still ran");
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(999), "999");
        assert_eq!(format_thousands(1_000), "1,000");
        assert_eq!(format_thousands(12_482), "12,482");
        assert_eq!(format_thousands(-1_500), "-1,500");
    }

    #[test]
    fn summary_lines_surface_failures_and_skips() {
        let report = SyncReport {
            providers: vec![
                ProviderSyncResult {
                    provider: "opencode".into(),
                    name: "OpenCode".into(),
                    ok: true,
                    new_tokens: 12_482,
                    ..Default::default()
                },
                ProviderSyncResult {
                    provider: "codex".into(),
                    name: "Codex".into(),
                    message: "no readable source (No Codex sessions directory)".into(),
                    skipped: true,
                    ..Default::default()
                },
                ProviderSyncResult {
                    provider: "freebuff".into(),
                    name: "Freebuff".into(),
                    message: "database locked".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let lines = report.summary_lines();
        assert_eq!(lines[0], "OpenCode: +12,482 tokens");
        assert!(lines[1].starts_with("Codex: no readable source"));
        assert!(lines[2].contains("FAILED"));
        assert!(!report.all_ok());
    }

    /// End-to-end verification against this machine's real installations.
    ///
    /// Ignored by default because the result depends on what is installed. Run
    /// explicitly with:
    ///
    /// ```text
    /// cargo test --lib -- --ignored --nocapture real_local_data
    /// ```
    ///
    /// It asserts the two rules that matter in the real world: every provider
    /// with a readable source imports records, and every provider without one is
    /// reported as unavailable instead of as zero usage.
    #[test]
    #[ignore = "reads the developer machine's real provider data"]
    fn real_local_data_smoke() {
        let dir = temp_dir("real");
        // Use the application's real (cached) pricing registry, read-only, so
        // reference values are resolved exactly as they are at runtime.
        let registry = Arc::new(RegistryState::new(&crate::get_data_dir()));
        registry.load_cache();
        println!(
            "pricing registry: {} model(s), updated {:?}",
            registry.status().model_count,
            registry.status().last_updated
        );

        let state = AppState {
            db: Mutex::new(Database::new(&dir.join("freetokenmeter.db")).unwrap()),
            providers: crate::provider::default_registry(),
            prefs: Mutex::new(PreferencesStore::load(dir.join("preferences.json"))),
            last_sync_report: Mutex::new(None),
            pricing_registry: registry,
            syncing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            geometry_events: AtomicU64::new(0),
        };

        println!("\n── detected providers ─────────────────────────────────");
        for descriptor in state.providers.descriptors() {
            println!(
                "{:<16} {:<20} {}",
                descriptor.name,
                descriptor.status,
                descriptor.detail.unwrap_or_default()
            );
        }

        let report = sync_all(&state);
        println!("\n── first pass ─────────────────────────────────────────");
        for result in &report.providers {
            println!(
                "{:<16} ok={:<5} skipped={:<5} records={:<5} new_tokens={:<10} scanned={:<6} skipped_events={:<5} {}ms  {}",
                result.name,
                result.ok,
                result.skipped,
                result.records_synced,
                result.new_tokens,
                result.events_scanned,
                result.events_skipped,
                result.duration_ms,
                result.message
            );
        }

        // A provider that is detected as connected must have produced records.
        for descriptor in state.providers.descriptors() {
            let result = report
                .providers
                .iter()
                .find(|r| r.provider == descriptor.id)
                .expect("every provider is in the report");
            if descriptor.state == "connected" {
                assert!(result.ok, "{} is connected but failed: {}", descriptor.name, result.message);
                assert!(
                    result.records_synced > 0,
                    "{} is connected but imported nothing",
                    descriptor.name
                );
            } else {
                assert!(!result.ok, "{} must not report success", descriptor.name);
            }
        }

        // A second pass must not import the same events again. Codex rollout
        // files are append-only and nothing is appending to them here, so that
        // provider is the deterministic proof of deduplication. (OpenCode's
        // database is written by a live CLI on a working machine, so it is
        // allowed to report genuinely new sessions.)
        let second = sync_all(&state);
        println!("\n── second pass (deduplication) ────────────────────────");
        for result in &second.providers {
            println!(
                "{:<16} ok={:<5} records={:<5} scanned={:<6} new_tokens={}",
                result.name,
                result.ok,
                result.records_synced,
                result.events_scanned,
                result.new_tokens
            );
        }

        let codex = second
            .providers
            .iter()
            .find(|r| r.provider == "codex")
            .unwrap();
        assert_eq!(
            codex.records_synced, 0,
            "an unchanged rollout must not be re-imported"
        );
        assert_eq!(codex.new_tokens, 0);

        // No provider may import more on the second pass than on the first.
        for result in &second.providers {
            let first_pass = report
                .providers
                .iter()
                .find(|r| r.provider == result.provider)
                .unwrap();
            assert!(
                result.records_synced <= first_pass.records_synced.max(1),
                "{} re-imported more on the second pass than the first",
                result.name
            );
        }

        // What the provider table shows at runtime.
        let usage = state
            .db
            .lock()
            .unwrap()
            .get_provider_usage(None, None)
            .unwrap();
        println!("\n── provider table ─────────────────────────────────────");
        for row in &usage {
            println!(
                "{:<16} tokens={:<12} actual={:<10} reference={:<10} billing={:?} {}",
                row.provider,
                row.tokens,
                row.actual_cost
                    .map(|c| format!("${:.4}", c))
                    .unwrap_or_else(|| "N/A".to_string()),
                row.reference_value
                    .map(|c| format!("${:.4}", c))
                    .unwrap_or_else(|| "N/A".to_string()),
                row.billing_units,
                row.billing_unit
            );
        }

        let unpriced_models = state.db.lock().unwrap().get_unknown_models().unwrap();
        println!("\n── unpriced models (PRICING: UNKNOWN) ─────────────────");
        if unpriced_models.is_empty() {
            println!("(none)");
        }
        for (provider, model, tokens, records) in &unpriced_models {
            println!(
                "{:<16} {:<28} {} tokens · {} record(s)",
                provider, model, tokens, records
            );
        }

        // History survives: the summary must reflect what was imported.
        let summary = state.db.lock().unwrap().get_usage_summary(None, None).unwrap();
        println!(
            "\nhistory: {} records, {} tokens, actual ${:.4}, reference ${:.4}, unpriced {}",
            summary.request_count,
            summary.total_tokens,
            summary.actual_cost,
            summary.reference_value,
            summary.unpriced_records
        );
        assert!(summary.request_count > 0, "history should not be empty");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backfill_window_is_bounded_and_optional() {
        assert!(backfill_cutoff(0).is_none());
        let cutoff = backfill_cutoff(7).unwrap();
        let expected = Utc::now().timestamp_millis() - 7 * 24 * 60 * 60 * 1000;
        assert!((cutoff - expected).abs() < 5_000);
    }
}
