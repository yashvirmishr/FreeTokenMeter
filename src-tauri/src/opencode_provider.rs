//! OpenCode provider.
//!
//! # Source
//!
//! OpenCode stores its own SQLite database at
//! `~/.local/share/opencode/opencode.db`. The `session` table already carries
//! per-session token accounting and, for paid models, a real cost:
//!
//! ```text
//! session(id, model, tokens_input, tokens_output, tokens_reasoning,
//!         tokens_cache_read, tokens_cache_write, cost, time_created, time_updated)
//! ```
//!
//! `model` is a JSON object (`{"id":"mimo-v2.5-free","providerID":"opencode"}`).
//!
//! # Cost semantics
//!
//! OpenCode reports a non-zero `cost` only for paid usage, so the payment mode
//! is derived per session: a positive cost means an API-style charge, zero cost
//! means a promotional/free model whose value is expressed through reference
//! pricing instead.
//!
//! # Incremental sync
//!
//! `time_updated` is the change marker: sessions touched since the last pass are
//! re-read and upserted, so a session that grows after its first sync replaces
//! its own earlier row instead of double counting.

use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use log::{error, info, warn};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::database::UsageRecord;
use crate::pricing::{value_usage, PricingInput};
use crate::provider::{FetchOutcome, FetchRequest, ProviderCursor, UsageProvider};
use crate::usage::{BillingUnit, PaymentMode, ProviderAvailability, SourceType};

/// A row from OpenCode's `session` table.
#[derive(Debug, Clone)]
pub(crate) struct OpenCodeSession {
    pub(crate) id: String,
    pub(crate) model_json: Option<String>,
    pub(crate) tokens_input: Option<i64>,
    pub(crate) tokens_output: Option<i64>,
    pub(crate) tokens_reasoning: Option<i64>,
    pub(crate) tokens_cache_read: Option<i64>,
    pub(crate) tokens_cache_write: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) time_created: i64,
}

/// Parsed model info from OpenCode's JSON model field.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeModel {
    id: String,
    #[serde(rename = "providerID")]
    provider_id: String,
    variant: Option<String>,
}

pub struct OpenCodeProvider {
    db_path: PathBuf,
}

impl OpenCodeProvider {
    pub const ID: &'static str = crate::usage::provider_id::OPENCODE;

    pub fn new() -> Self {
        Self::with_path(
            dirs::home_dir()
                .unwrap_or_default()
                .join(".local")
                .join("share")
                .join("opencode")
                .join("opencode.db"),
        )
    }

    /// Point the provider at an explicit database (tests, portable installs).
    pub fn with_path(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    /// Parse the model JSON into `(model_id, provider_id, variant)`.
    pub fn parse_model(model_json: &Option<String>) -> (String, String, Option<String>) {
        match model_json {
            Some(json) => match serde_json::from_str::<OpenCodeModel>(json) {
                Ok(m) => (m.id, m.provider_id, m.variant),
                Err(e) => {
                    warn!("Failed to parse OpenCode model JSON '{}': {}", json, e);
                    ("unknown".to_string(), "opencode".to_string(), None)
                }
            },
            None => ("unknown".to_string(), "opencode".to_string(), None),
        }
    }

    /// Normalize one session row into the shared usage model.
    pub fn normalize_session(
        session: &OpenCodeSession,
        registry: Option<&crate::registry::RegistryState>,
    ) -> UsageRecord {
        let (model_name, _provider_id, _variant) = Self::parse_model(&session.model_json);
        let cost = session.cost.filter(|c| *c > 0.0);

        // OpenCode's own total counts every token class it tracks.
        let total_tokens = session.tokens_input.unwrap_or(0)
            + session.tokens_output.unwrap_or(0)
            + session.tokens_reasoning.unwrap_or(0)
            + session.tokens_cache_read.unwrap_or(0)
            + session.tokens_cache_write.unwrap_or(0);

        let payment_mode = if cost.is_some() {
            PaymentMode::Api
        } else {
            PaymentMode::Promotional
        };

        let value = value_usage(
            &PricingInput {
                provider: Self::ID,
                model: &model_name,
                provider_cost_usd: cost,
                billing_units: None,
                billing_unit: BillingUnit::Usd,
                payment_mode,
                input_tokens: session.tokens_input,
                output_tokens: session.tokens_output,
                cached_tokens: session.tokens_cache_read,
                cache_write_tokens: session.tokens_cache_write,
            },
            registry,
        );

        UsageRecord {
            id: format!("oc_{}", session.id),
            provider: Self::ID.to_string(),
            model: model_name,
            timestamp: Utc
                .timestamp_millis_opt(session.time_created)
                .single()
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default(),
            input_tokens: session.tokens_input,
            output_tokens: session.tokens_output,
            cached_tokens: session.tokens_cache_read,
            cache_write_tokens: session.tokens_cache_write,
            reasoning_tokens: session.tokens_reasoning,
            total_tokens,
            actual_cost_usd: value.actual_cost_usd,
            reference_value: value.reference_value,
            free_value: value.free_value,
            billing_unit: value.billing_unit.as_str().to_string(),
            billing_units: value.billing_units,
            // OpenCode bills in plain USD and records no tool version.
            billing_metric_name: None,
            payment_mode: value.payment_mode.as_str().to_string(),
            pricing_status: value.pricing_status.as_str().to_string(),
            pricing_source: Some(value.pricing_source.as_str().to_string()),
            pricing_version: Some(value.pricing_version),
            reference_model: value.reference_model,
            reference_input_rate: value.reference_input_rate,
            reference_output_rate: value.reference_output_rate,
            reference_cached_rate: value.reference_cached_rate,
            session_id: Some(session.id.clone()),
            event_id: Some(session.id.clone()),
            source_type: SourceType::OpenCodeDatabase.as_str().to_string(),
            source_id: Some(session.id.clone()),
            source_version: None,
        }
    }

    /// Sessions touched since `since_ms` (all sessions when `None`).
    fn fetch_sessions(&self, since_ms: Option<i64>) -> Result<Vec<OpenCodeSession>, String> {
        let conn = Connection::open(&self.db_path).map_err(|e| {
            error!("Failed to open OpenCode database: {}", e);
            format!("Failed to open OpenCode database: {}", e)
        })?;

        let base = "SELECT id, model, tokens_input, tokens_output, tokens_reasoning,
                           tokens_cache_read, tokens_cache_write, cost, time_created
                    FROM session";
        let sql = if since_ms.is_some() {
            format!("{} WHERE time_updated > ?1 ORDER BY time_created ASC", base)
        } else {
            format!("{} ORDER BY time_created ASC", base)
        };

        let mut stmt = conn.prepare(&sql).map_err(|e| {
            error!("Failed to prepare OpenCode session query: {}", e);
            format!("Failed to prepare session query: {}", e)
        })?;

        let row_mapper = |row: &rusqlite::Row| {
            Ok(OpenCodeSession {
                id: row.get(0)?,
                model_json: row.get(1)?,
                tokens_input: row.get(2)?,
                tokens_output: row.get(3)?,
                tokens_reasoning: row.get(4)?,
                tokens_cache_read: row.get(5)?,
                tokens_cache_write: row.get(6)?,
                cost: row.get(7)?,
                time_created: row.get(8)?,
            })
        };

        let sessions: Vec<OpenCodeSession> = if let Some(sync_time) = since_ms {
            stmt.query_map(rusqlite::params![sync_time], row_mapper)
        } else {
            stmt.query_map([], row_mapper)
        }
        .map_err(|e| {
            error!("Failed to query OpenCode sessions: {}", e);
            format!("Failed to query sessions: {}", e)
        })?
        .filter_map(|row| match row {
            Ok(session) => Some(session),
            Err(e) => {
                warn!("Skipping malformed OpenCode session row: {}", e);
                None
            }
        })
        .collect();

        info!("Fetched {} session(s) from OpenCode", sessions.len());
        Ok(sessions)
    }

    /// Highest `time_updated` in the database.
    ///
    /// This is the correct incremental watermark because it comes from the
    /// database's own clock rather than the application's: a wall-clock marker
    /// would skip sessions whose rows are stamped earlier than "now".
    fn max_time_updated(&self) -> Result<i64, String> {
        let conn = Connection::open(&self.db_path)
            .map_err(|e| format!("Failed to open OpenCode database: {}", e))?;
        conn.query_row(
            "SELECT COALESCE(MAX(time_updated), 0) FROM session",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| format!("Failed to read the OpenCode session watermark: {}", e))
    }
}

impl Default for OpenCodeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageProvider for OpenCodeProvider {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn short_label(&self) -> &'static str {
        "OPENCODE"
    }

    fn source_type(&self) -> &'static str {
        SourceType::OpenCodeDatabase.as_str()
    }

    fn payment_mode(&self) -> PaymentMode {
        PaymentMode::Promotional
    }

    fn availability(&self) -> ProviderAvailability {
        if self.db_path.exists() {
            ProviderAvailability::Connected
        } else {
            ProviderAvailability::NotInstalled
        }
    }

    fn detail(&self) -> Option<String> {
        match self.availability() {
            ProviderAvailability::Connected => Some(format!("Database: {}", self.db_path.display())),
            _ => Some("OpenCode installation was not detected.".to_string()),
        }
    }

    fn fetch(&self, request: &FetchRequest<'_>) -> Result<FetchOutcome, String> {
        if !self.db_path.exists() {
            return Err("OpenCode database not found".to_string());
        }

        let cursor = request.cursor.cloned().unwrap_or_default();
        let sessions = self.fetch_sessions(cursor.last_timestamp_ms)?;
        let watermark = self.max_time_updated()?;

        // If the pass hit the record ceiling, leave the watermark where it was
        // so the remaining sessions are picked up next time instead of skipped.
        let truncated = sessions.len() > request.limit;

        let records: Vec<UsageRecord> = sessions
            .iter()
            .take(request.limit)
            .map(|session| Self::normalize_session(session, request.registry))
            .collect();

        let scanned = records.len();
        let mut next = ProviderCursor {
            last_event_id: cursor.last_event_id.clone(),
            ..cursor.clone()
        };
        if !truncated {
            next.last_timestamp_ms =
                Some(watermark.max(cursor.last_timestamp_ms.unwrap_or(0)));
        }
        if let Some(last) = records.last() {
            next.last_event_id = last.event_id.clone();
        }

        Ok(FetchOutcome {
            records,
            cursor: next,
            scanned,
            skipped: 0,
        })
    }

    fn default_backfill_days(&self) -> i64 {
        30
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn session(cost: f64, input: i64) -> OpenCodeSession {
        OpenCodeSession {
            id: "ses_1".to_string(),
            model_json: Some(
                r#"{"id":"mimo-v2.5-free","providerID":"opencode","variant":null}"#.to_string(),
            ),
            tokens_input: Some(input),
            tokens_output: Some(500),
            tokens_reasoning: Some(0),
            tokens_cache_read: Some(100),
            tokens_cache_write: Some(0),
            cost: Some(cost),
            time_created: 1_700_000_000_000,
        }
    }

    #[test]
    fn parses_the_model_object() {
        let (model, provider, variant) = OpenCodeProvider::parse_model(&Some(
            r#"{"id":"mimo-v2.5-free","providerID":"opencode","variant":"x"}"#.to_string(),
        ));
        assert_eq!(model, "mimo-v2.5-free");
        assert_eq!(provider, "opencode");
        assert_eq!(variant.as_deref(), Some("x"));
    }

    #[test]
    fn unparseable_model_json_does_not_invent_a_model() {
        let (model, _, _) = OpenCodeProvider::parse_model(&Some("not json".to_string()));
        assert_eq!(model, "unknown");
    }

    #[test]
    fn free_session_is_promotional_with_a_known_zero_cost() {
        let record = OpenCodeProvider::normalize_session(&session(0.0, 1_000_000), None);
        assert_eq!(record.payment_mode, "promotional");
        assert_eq!(record.actual_cost_usd, Some(0.0));
        assert_eq!(record.total_tokens, 1_000_500 + 100);
        assert_eq!(record.source_type, "opencode_database");
    }

    #[test]
    fn paid_session_uses_the_provider_reported_cost() {
        let record = OpenCodeProvider::normalize_session(&session(1.25, 1_000_000), None);
        assert_eq!(record.payment_mode, "api");
        assert_eq!(record.actual_cost_usd, Some(1.25));
        assert_eq!(record.pricing_status, "actual_cost_known");
    }

    #[test]
    fn missing_token_classes_stay_null() {
        let mut raw = session(0.0, 10);
        raw.tokens_cache_write = None;
        raw.tokens_reasoning = None;
        let record = OpenCodeProvider::normalize_session(&raw, None);
        assert!(record.cache_write_tokens.is_none());
        assert!(record.reasoning_tokens.is_none());
    }

    #[test]
    fn reports_not_installed_without_a_database() {
        let provider = OpenCodeProvider::with_path(PathBuf::from("/definitely/missing/opencode.db"));
        assert_eq!(provider.availability(), ProviderAvailability::NotInstalled);
    }

    /// The session identity is stable, so repeated syncs upsert the same row.
    #[test]
    fn record_id_is_derived_from_the_session_id() {
        let record = OpenCodeProvider::normalize_session(&session(0.0, 1), None);
        assert_eq!(record.id, "oc_ses_1");
        assert_eq!(record.event_id.as_deref(), Some("ses_1"));
    }

    /// A cursor advanced by a pass must not re-import unchanged sessions.
    #[test]
    fn cursor_uses_the_database_watermark() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-opencode-inc-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("opencode.db");

        let create = |updated: i64| {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS session (
                    id TEXT PRIMARY KEY, model TEXT,
                    tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER,
                    tokens_cache_read INTEGER, tokens_cache_write INTEGER, cost REAL,
                    time_created INTEGER, time_updated INTEGER);",
            )
            .unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO session VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    "ses_1",
                    r#"{"id":"mimo-v2.5-free","providerID":"opencode","variant":null}"#,
                    1_000i64,
                    500i64,
                    0i64,
                    100i64,
                    0i64,
                    0.0f64,
                    1_700_000_000_000i64,
                    updated
                ],
            )
            .unwrap();
        };

        create(1_700_000_000_000);
        let provider = OpenCodeProvider::with_path(db_path.clone());
        let first = provider
            .fetch(&FetchRequest {
                cursor: None,
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert_eq!(first.records.len(), 1);

        let second = provider
            .fetch(&FetchRequest {
                cursor: Some(&first.cursor),
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert!(second.records.is_empty(), "unchanged session must not be re-read");

        create(1_700_000_100_000);
        let third = provider
            .fetch(&FetchRequest {
                cursor: Some(&second.cursor),
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert_eq!(third.records.len(), 1, "a touched session is re-read");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
