//! Provider abstraction and registry.
//!
//! # Contract
//!
//! ```text
//! provider source
//!       ↓  (provider module only)
//! adapter / parser
//!       ↓
//! normalized UsageRecord
//!       ↓  (shared, provider-agnostic)
//! dedup → SQLite → pricing → aggregation → terminal UI
//! ```
//!
//! A provider module may know where its own data lives and how to parse it.
//! Nothing else in the application does. Adding a provider means writing one
//! module that implements [`UsageProvider`] and registering it here — the sync
//! manager, database, pricing, aggregation and UI stay untouched.
//!
//! # No UI scraping
//!
//! Providers may only read machine-readable sources, in this order of
//! preference:
//!
//! 1. official structured local database
//! 2. official local API
//! 3. official structured CLI output
//! 4. official telemetry / export format
//! 5. stable local session/history files
//!
//! If none exists, the provider must report [`ProviderAvailability::Unsupported`]
//! or [`ProviderAvailability::UsageUnavailable`] rather than invent a parser.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::database::UsageRecord;
use crate::registry::RegistryState;
use crate::usage::ProviderAvailability;

/// Incremental sync position for one provider.
///
/// Persisted as JSON in the `sync_state` table. Providers that read append-only
/// JSONL keep a byte offset per file; providers backed by a database keep the
/// last source timestamp. Both are cheap to advance and never re-import history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderCursor {
    /// Highest event timestamp already imported (milliseconds since epoch).
    pub last_timestamp_ms: Option<i64>,

    /// Provider-native id of the most recent imported event.
    pub last_event_id: Option<String>,

    /// Append-only text sources: file path → bytes already consumed.
    pub offsets: HashMap<String, u64>,

    /// Provider-specific continuation state (for example the last model seen in
    /// a session file, so a resumed read still attributes usage correctly).
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

/// Input handed to a provider for one synchronization pass.
pub struct FetchRequest<'a> {
    /// Cursor from the previous pass, when the provider has synced before.
    pub cursor: Option<&'a ProviderCursor>,
    /// Only import events at or after this timestamp (backfill window).
    /// `None` means "all available history".
    pub since_ms: Option<i64>,
    /// The shared dynamic pricing registry. Providers never build their own.
    pub registry: Option<&'a RegistryState>,
    /// Upper bound on records to return in this pass.
    pub limit: usize,
}

/// What a provider produced for one pass.
pub struct FetchOutcome {
    /// Normalized records, already deduplicated within the pass.
    pub records: Vec<UsageRecord>,
    /// The cursor to persist for the next pass.
    pub cursor: ProviderCursor,
    /// How many source events were examined.
    pub scanned: usize,
    /// How many were skipped (unparseable, no tokens, before the window).
    pub skipped: usize,
}

/// One provider integration.
///
/// Implementations must be cheap to construct, must never panic on malformed
/// local data, and must never return records for a source they cannot read.
pub trait UsageProvider: Send + Sync {
    /// Canonical provider id, e.g. `claude-code`.
    fn id(&self) -> &'static str;

    /// Display name, e.g. `Claude Code`.
    fn display_name(&self) -> &'static str;

    /// Short uppercase label for narrow UI columns, e.g. `CLAUDE`.
    fn short_label(&self) -> &'static str;

    /// Structured source kind recorded on every record, e.g. `codex_session_file`.
    fn source_type(&self) -> &'static str;

    /// How this provider's usage is paid for.
    fn payment_mode(&self) -> crate::usage::PaymentMode;

    /// The unit this provider meters in.
    fn billing_unit(&self) -> crate::usage::BillingUnit {
        crate::usage::BillingUnit::Usd
    }

    /// Detect the provider and locate a usable structured source.
    fn availability(&self) -> ProviderAvailability;

    /// Human explanation for the detected state.
    fn detail(&self) -> Option<String>;

    /// Import usage. Returns `Err` for provider-level failures; individual
    /// malformed events are skipped and counted instead.
    fn fetch(&self, request: &FetchRequest<'_>) -> Result<FetchOutcome, String>;

    /// Sensible default backfill window when the provider is first enabled.
    fn default_backfill_days(&self) -> i64 {
        30
    }
}

/// Detection result for one provider, without running a sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: String,
    pub name: String,
    pub short_label: String,
    pub source_type: String,
    pub payment_mode: String,
    pub billing_unit: String,
    pub state: String,
    pub status: String,
    pub detail: Option<String>,
    pub default_backfill_days: i64,
}

// ──────────────────────────────────────────────────────────────────────
// Shared file-source helpers
// ──────────────────────────────────────────────────────────────────────

/// Collect every file under `dir` with the given extension, recursively.
pub(crate) fn collect_files_with_extension(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_extension(&path, extension, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}

/// Read only the bytes appended since `offset`.
///
/// Stops at the last complete line so a partially flushed write is retried on
/// the next pass instead of being parsed as a truncated event. Returns the new
/// byte offset to persist.
pub(crate) fn read_appended(path: &Path, offset: u64) -> std::io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    // The file was rewritten or truncated: start over.
    let start = if offset > length { 0 } else { offset };
    file.seek(SeekFrom::Start(start))?;

    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    let text = String::from_utf8_lossy(&buffer).into_owned();

    let consumed = text.rfind('\n').map(|i| i + 1).unwrap_or(0);
    Ok((text[..consumed].to_string(), start + consumed as u64))
}

/// Render a millisecond timestamp as RFC 3339, or an empty string when the
/// value is not representable.
pub(crate) fn rfc3339(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

/// The set of providers the application knows about.
///
/// Providers are registered dynamically: the sync manager, commands and UI all
/// iterate this list rather than naming providers individually.
pub struct ProviderRegistry {
    providers: Vec<Box<dyn UsageProvider>>,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn UsageProvider>) {
        self.providers.push(provider);
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn UsageProvider> {
        self.providers.iter().map(|p| p.as_ref())
    }

    /// Look up one provider by its canonical id. Used by the test suite to
    /// assert that a registry contains exactly what it should.
    #[cfg(test)]
    pub fn get(&self, id: &str) -> Option<&dyn UsageProvider> {
        self.providers.iter().find(|p| p.id() == id).map(|p| p.as_ref())
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Detection snapshot for every registered provider. Used by the UI and by
    /// `providers health`.
    pub fn descriptors(&self) -> Vec<ProviderDescriptor> {
        self.iter()
            .map(|p| {
                let state = p.availability();
                ProviderDescriptor {
                    id: p.id().to_string(),
                    name: p.display_name().to_string(),
                    short_label: p.short_label().to_string(),
                    source_type: p.source_type().to_string(),
                    payment_mode: p.payment_mode().as_str().to_string(),
                    billing_unit: p.billing_unit().as_str().to_string(),
                    state: state.as_str().to_string(),
                    status: state.label().to_string(),
                    detail: p.detail(),
                    default_backfill_days: p.default_backfill_days(),
                }
            })
            .collect()
    }
}

/// The shipped provider set.
///
/// Order matters only for presentation: the UI lists providers in this order.
pub fn default_registry() -> ProviderRegistry {
    let mut registry = ProviderRegistry::new();
    registry.register(Box::new(crate::opencode_provider::OpenCodeProvider::new()));
    registry.register(Box::new(crate::freebuff_provider::FreebuffProvider::new()));
    registry.register(Box::new(crate::claude_code_provider::ClaudeCodeProvider::new()));
    registry.register(Box::new(crate::codex_provider::CodexProvider::new()));
    registry.register(Box::new(crate::gemini_cli_provider::GeminiCliProvider::new()));
    registry.register(Box::new(crate::copilot_provider::GitHubCopilotProvider::new()));
    registry
}

/// Providers that are known to exist but have no supported local source yet.
/// Reported in the UI as FUTURE/UNSUPPORTED so users are not left guessing.
pub const FUTURE_PROVIDERS: &[(&str, &str, &str)] = &[
    (
        "cline",
        "Cline",
        "Cline's token/cost fields are part of its extension API; no stable local usage store found.",
    ),
    (
        "kiro",
        "Kiro",
        "Kiro's /usage reports context-window occupancy, not historical token consumption.",
    ),
    (
        "cursor",
        "Cursor",
        "Cursor publishes usage in its web dashboard; no supported local machine-readable source.",
    ),
    (
        "windsurf",
        "Windsurf",
        "Windsurf exposes plan usage in its web dashboard only; no supported local source.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::{BillingUnit, PaymentMode};

    struct FakeProvider {
        id: &'static str,
        state: ProviderAvailability,
    }

    impl UsageProvider for FakeProvider {
        fn id(&self) -> &'static str {
            self.id
        }
        fn display_name(&self) -> &'static str {
            "Fake"
        }
        fn short_label(&self) -> &'static str {
            "FAKE"
        }
        fn source_type(&self) -> &'static str {
            "fake_source"
        }
        fn payment_mode(&self) -> PaymentMode {
            PaymentMode::Unknown
        }
        fn billing_unit(&self) -> BillingUnit {
            BillingUnit::Usd
        }
        fn availability(&self) -> ProviderAvailability {
            self.state
        }
        fn detail(&self) -> Option<String> {
            None
        }
        fn fetch(&self, _request: &FetchRequest<'_>) -> Result<FetchOutcome, String> {
            Ok(FetchOutcome {
                records: Vec::new(),
                cursor: ProviderCursor::default(),
                scanned: 0,
                skipped: 0,
            })
        }
    }

    #[test]
    fn registry_registers_and_resolves_providers() {
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(FakeProvider {
            id: "alpha",
            state: ProviderAvailability::Connected,
        }));
        registry.register(Box::new(FakeProvider {
            id: "beta",
            state: ProviderAvailability::NotInstalled,
        }));

        assert_eq!(registry.len(), 2);
        assert!(registry.get("alpha").is_some());
        assert!(registry.get("gamma").is_none());
        assert_eq!(registry.get("beta").unwrap().availability(), ProviderAvailability::NotInstalled);
    }

    #[test]
    fn descriptors_expose_every_state() {
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(FakeProvider {
            id: "alpha",
            state: ProviderAvailability::Connected,
        }));
        registry.register(Box::new(FakeProvider {
            id: "beta",
            state: ProviderAvailability::UsageUnavailable,
        }));

        let descriptors = registry.descriptors();
        assert_eq!(descriptors.len(), 2);
        assert_eq!(descriptors[0].state, "connected");
        assert_eq!(descriptors[1].status, "USAGE UNAVAILABLE");
    }

    #[test]
    fn default_registry_has_six_primary_providers() {
        let registry = default_registry();
        assert_eq!(registry.len(), 6);
        for id in [
            "opencode",
            "freebuff",
            "claude-code",
            "codex",
            "gemini-cli",
            "github-copilot",
        ] {
            assert!(registry.get(id).is_some(), "missing provider {}", id);
        }
    }

    #[test]
    fn cursor_round_trips_through_json() {
        let mut cursor = ProviderCursor {
            last_timestamp_ms: Some(1_700_000_000_000),
            last_event_id: Some("resp_1".to_string()),
            ..Default::default()
        };
        cursor.offsets.insert("/tmp/a.jsonl".to_string(), 4096);

        let json = serde_json::to_string(&cursor).unwrap();
        let back: ProviderCursor = serde_json::from_str(&json).unwrap();
        assert_eq!(back.offsets.get("/tmp/a.jsonl"), Some(&4096));
        assert_eq!(back.last_timestamp_ms, Some(1_700_000_000_000));
    }

    #[test]
    fn future_providers_are_documented_not_implemented() {
        let registry = default_registry();
        for (id, _, _) in FUTURE_PROVIDERS {
            assert!(
                registry.get(id).is_none(),
                "{} must not be registered until it has a supported source",
                id
            );
        }
    }

}
