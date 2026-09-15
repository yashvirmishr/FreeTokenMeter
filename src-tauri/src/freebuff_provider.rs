//! Freebuff Desktop provider.
//!
//! # Source
//!
//! Freebuff stores one SQLite database per project at
//! `~/.config/freebuff-desktop/projects/<project>/desktop-v2.db`. Assistant
//! messages carry a `metrics_json` column with the provider's own accounting:
//!
//! ```json
//! {
//!   "usage": {
//!     "inputTokens": 5011758,
//!     "cachedInputTokens": 4785280,
//!     "outputTokens": 8529,
//!     "reasoningOutputTokens": 0,
//!     "totalTokens": 5020287
//!   },
//!   "costUsd": 0
//! }
//! ```
//!
//! `cachedInputTokens` is a subset of `inputTokens` and `totalTokens` is
//! `input + output`, so cache traffic is never counted twice.
//!
//! # Cost semantics
//!
//! Freebuff's models are promotional: `costUsd` is zero, and the value of the
//! usage is expressed through reference pricing rather than a charge. A missing
//! `costUsd` key stays absent rather than becoming zero.
//!
//! # Incremental sync
//!
//! Each project database keeps its own watermark (`ts:<path>` in the cursor), so
//! only messages newer than the last pass are read. One unreadable project never
//! stops the others.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use log::{error, info, warn};
use rusqlite::Connection;
use serde::Deserialize;

use crate::database::UsageRecord;
use crate::pricing::{value_usage, PricingInput};
use crate::provider::{FetchOutcome, FetchRequest, UsageProvider};
use crate::usage::{BillingUnit, PaymentMode, ProviderAvailability, SourceType};

/// `metrics_json` payload, using Freebuff's camelCase keys.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FreebuffMetrics {
    #[serde(default)]
    usage: Option<FreebuffUsage>,
    #[serde(default)]
    cost_usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FreebuffUsage {
    #[serde(default)]
    input_tokens: Option<i64>,
    #[serde(default)]
    cached_input_tokens: Option<i64>,
    #[serde(default)]
    output_tokens: Option<i64>,
    #[serde(default)]
    reasoning_output_tokens: Option<i64>,
    #[serde(default)]
    total_tokens: Option<i64>,
}

/// A raw assistant message row.
#[derive(Debug, Clone)]
pub(crate) struct FreebuffRawRecord {
    pub seq: i64,
    pub thread_id: String,
    pub model: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub total_tokens: i64,
    pub cost_usd: Option<f64>,
    pub ts: i64,
    pub db_path: PathBuf,
}

pub struct FreebuffProvider {
    config_dir: PathBuf,
}

impl FreebuffProvider {
    pub const ID: &'static str = crate::usage::provider_id::FREEBUFF;

    pub fn new() -> Self {
        Self::with_config_dir(
            dirs::home_dir()
                .unwrap_or_default()
                .join(".config")
                .join("freebuff-desktop")
                .join("projects"),
        )
    }

    pub fn with_config_dir(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    /// Every project database, in stable order.
    pub fn find_databases(&self) -> Vec<PathBuf> {
        let mut databases = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.config_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let db_path = path.join("desktop-v2.db");
                    if db_path.exists() {
                        databases.push(db_path);
                    }
                }
            }
        }
        databases.sort();
        databases
    }

    /// Read assistant messages newer than `since_ts` from one database.
    fn read_database(&self, db_path: &Path, since_ts: i64) -> Result<Vec<FreebuffRawRecord>, String> {
        let conn = Connection::open(db_path).map_err(|e| {
            error!("Failed to open Freebuff database {}: {}", db_path.display(), e);
            format!("Failed to open database: {}", e)
        })?;

        let mut threads: HashMap<String, Option<String>> = HashMap::new();
        {
            let mut stmt = conn
                .prepare("SELECT id, model FROM threads")
                .map_err(|e| format!("Failed to prepare threads query: {}", e))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })
                .map_err(|e| format!("Failed to query threads: {}", e))?;
            for row in rows.flatten() {
                threads.insert(row.0, row.1);
            }
        }

        let mut stmt = conn
            .prepare(
                "SELECT seq, thread_id, metrics_json, ts
                 FROM messages
                 WHERE role = 'assistant'
                   AND metrics_json IS NOT NULL
                   AND metrics_json != '{}'
                   AND ts > ?1
                 ORDER BY ts ASC",
            )
            .map_err(|e| format!("Failed to prepare messages query: {}", e))?;

        let rows = stmt
            .query_map([since_ts], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| format!("Failed to query messages: {}", e))?;

        let mut records = Vec::new();
        for row in rows {
            let Ok((seq, thread_id, metrics_json, ts)) = row else {
                warn!("Skipping malformed Freebuff message row in {}", db_path.display());
                continue;
            };

            let metrics: FreebuffMetrics = match serde_json::from_str(&metrics_json) {
                Ok(metrics) => metrics,
                Err(e) => {
                    warn!("Failed to parse metrics_json for seq={}: {}", seq, e);
                    continue;
                }
            };

            let Some(usage) = metrics.usage else {
                continue;
            };

            let has_metric = usage.input_tokens.is_some()
                || usage.output_tokens.is_some()
                || usage.cached_input_tokens.is_some()
                || usage.reasoning_output_tokens.is_some()
                || usage.total_tokens.is_some();
            if !has_metric {
                continue;
            }

            let total = usage
                .total_tokens
                .unwrap_or_else(|| usage.input_tokens.unwrap_or(0) + usage.output_tokens.unwrap_or(0));
            if total == 0 {
                continue;
            }

            let model = threads
                .get(&thread_id)
                .and_then(|m| m.clone())
                .unwrap_or_else(|| "unknown".to_string());

            records.push(FreebuffRawRecord {
                seq,
                thread_id,
                model,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_tokens: usage.cached_input_tokens,
                reasoning_tokens: usage.reasoning_output_tokens,
                total_tokens: total,
                cost_usd: metrics.cost_usd.filter(|c| *c > 0.0),
                ts,
                db_path: db_path.to_path_buf(),
            });
        }

        Ok(records)
    }
}

impl Default for FreebuffProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize a raw Freebuff record into the shared usage model.
pub fn normalize_record(
    record: &FreebuffRawRecord,
    registry: Option<&crate::registry::RegistryState>,
) -> UsageRecord {
    let project_id = record
        .db_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let value = value_usage(
        &PricingInput {
            provider: FreebuffProvider::ID,
            model: &record.model,
            provider_cost_usd: record.cost_usd,
            billing_units: None,
            billing_unit: BillingUnit::Usd,
            payment_mode: PaymentMode::Promotional,
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            cached_tokens: record.cached_tokens,
            cache_write_tokens: None,
        },
        registry,
    );

    UsageRecord {
        id: format!("fb_{}_{}", project_id, record.seq),
        provider: FreebuffProvider::ID.to_string(),
        model: record.model.clone(),
        timestamp: chrono::DateTime::from_timestamp_millis(record.ts)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default(),
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cached_tokens: record.cached_tokens,
        cache_write_tokens: None,
        reasoning_tokens: record.reasoning_tokens,
        total_tokens: record.total_tokens,
        actual_cost_usd: value.actual_cost_usd,
        reference_value: value.reference_value,
        free_value: value.free_value,
        billing_unit: value.billing_unit.as_str().to_string(),
        billing_units: value.billing_units,
        // Freebuff bills in plain USD; its records carry no tool version.
        billing_metric_name: None,
        payment_mode: value.payment_mode.as_str().to_string(),
        pricing_status: value.pricing_status.as_str().to_string(),
        pricing_source: Some(value.pricing_source.as_str().to_string()),
        pricing_version: Some(value.pricing_version),
        reference_model: value.reference_model,
        reference_input_rate: value.reference_input_rate,
        reference_output_rate: value.reference_output_rate,
        reference_cached_rate: value.reference_cached_rate,
        session_id: Some(record.thread_id.clone()),
        event_id: Some(format!("{}:{}", project_id, record.seq)),
        source_type: SourceType::FreebuffDatabase.as_str().to_string(),
        source_id: Some(record.db_path.to_string_lossy().to_string()),
        source_version: None,
    }
}

impl UsageProvider for FreebuffProvider {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn display_name(&self) -> &'static str {
        "Freebuff"
    }

    fn short_label(&self) -> &'static str {
        "FREEBUFF"
    }

    fn source_type(&self) -> &'static str {
        SourceType::FreebuffDatabase.as_str()
    }

    fn payment_mode(&self) -> PaymentMode {
        PaymentMode::Promotional
    }

    fn availability(&self) -> ProviderAvailability {
        if !self.config_dir.exists() {
            return ProviderAvailability::NotInstalled;
        }
        if self.find_databases().is_empty() {
            return ProviderAvailability::UsageUnavailable;
        }
        ProviderAvailability::Connected
    }

    fn detail(&self) -> Option<String> {
        match self.availability() {
            ProviderAvailability::Connected => Some(format!(
                "{} project database(s) found",
                self.find_databases().len()
            )),
            ProviderAvailability::NotInstalled => {
                Some("Freebuff Desktop installation was not detected.".to_string())
            }
            ProviderAvailability::UsageUnavailable => Some(
                "Freebuff is installed but no project databases were found.".to_string(),
            ),
            other => Some(other.label().to_string()),
        }
    }

    fn fetch(&self, request: &FetchRequest<'_>) -> Result<FetchOutcome, String> {
        let databases = self.find_databases();
        if databases.is_empty() {
            return Err("No Freebuff project databases found".to_string());
        }

        let cursor = request.cursor.cloned().unwrap_or_default();
        let mut next = cursor.clone();
        let mut records = Vec::new();
        let mut scanned = 0usize;

        for db_path in &databases {
            if records.len() >= request.limit {
                break;
            }
            let key = format!("ts:{}", db_path.to_string_lossy());
            let since_ts = cursor
                .extra
                .get(&key)
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0);

            let raw = match self.read_database(db_path, since_ts) {
                Ok(raw) => raw,
                Err(e) => {
                    // One broken project must never stop the others.
                    warn!("Skipping Freebuff database {}: {}", db_path.display(), e);
                    continue;
                }
            };

            scanned += raw.len();
            let mut newest = since_ts;
            for record in &raw {
                newest = newest.max(record.ts);
                next.last_event_id = record.event_id();
                records.push(normalize_record(record, request.registry));
            }
            next.extra.insert(key, newest.to_string());
            if newest > 0 {
                next.last_timestamp_ms = Some(next.last_timestamp_ms.unwrap_or(0).max(newest));
            }
        }

        records.truncate(request.limit);
        info!(
            "Freebuff: scanned {} message(s), imported {} record(s)",
            scanned,
            records.len()
        );

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

impl FreebuffRawRecord {
    /// Provider-native event id, used for the cursor watermark.
    pub(crate) fn event_id(&self) -> Option<String> {
        let project = self
            .db_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        Some(format!("{}:{}", project, self.seq))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(cost: Option<f64>) -> FreebuffRawRecord {
        FreebuffRawRecord {
            seq: 42,
            thread_id: "abc-123".to_string(),
            model: "mimo/mimo-v2.5".to_string(),
            input_tokens: Some(1_000),
            output_tokens: Some(500),
            cached_tokens: Some(200),
            reasoning_tokens: Some(0),
            total_tokens: 1_500,
            cost_usd: cost,
            ts: 1_700_000_000_000,
            db_path: PathBuf::from(
                "/home/user/.config/freebuff-desktop/projects/MyProject-abc123/desktop-v2.db",
            ),
        }
    }

    #[test]
    fn normalize_generates_a_stable_project_scoped_id() {
        let normalized = normalize_record(&raw(None), None);
        assert_eq!(normalized.id, "fb_MyProject-abc123_42");
        assert_eq!(normalized.provider, "freebuff");
        assert_eq!(normalized.model, "mimo/mimo-v2.5");
        assert_eq!(normalized.source_type, "freebuff_database");
    }

    #[test]
    fn promotional_usage_is_a_known_zero_cost_with_reference_value() {
        let normalized = normalize_record(&raw(None), None);
        assert_eq!(normalized.payment_mode, "promotional");
        assert_eq!(normalized.actual_cost_usd, Some(0.0));
        // mimo/mimo-v2.5 has a configured reference rate.
        assert!(normalized.reference_value.is_some());
    }

    #[test]
    fn missing_metrics_stay_null() {
        let mut record = raw(None);
        record.cached_tokens = None;
        record.reasoning_tokens = None;
        let normalized = normalize_record(&record, None);
        assert!(normalized.cached_tokens.is_none());
        assert!(normalized.reasoning_tokens.is_none());
    }

    #[test]
    fn reports_not_installed_without_the_config_directory() {
        let provider = FreebuffProvider::with_config_dir(PathBuf::from("/definitely/missing/freebuff"));
        assert_eq!(provider.availability(), ProviderAvailability::NotInstalled);
    }

    #[test]
    fn reports_usage_unavailable_when_installed_without_projects() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-freebuff-empty-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let provider = FreebuffProvider::with_config_dir(dir.clone());
        assert_eq!(provider.availability(), ProviderAvailability::UsageUnavailable);

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn write_project(dir: &Path, project: &str, seq: i64, ts: i64) -> PathBuf {
        let project_dir = dir.join(project);
        std::fs::create_dir_all(&project_dir).unwrap();
        let db_path = project_dir.join("desktop-v2.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS threads (id TEXT PRIMARY KEY, model TEXT);
             CREATE TABLE IF NOT EXISTS messages (
                seq INTEGER, thread_id TEXT, role TEXT, metrics_json TEXT, ts INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO threads VALUES (?1, ?2)",
            rusqlite::params!["thread-1", "mimo/mimo-v2.5"],
        )
        .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO messages VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                seq,
                "thread-1",
                "assistant",
                r#"{"usage":{"inputTokens":2000000,"cachedInputTokens":0,"outputTokens":0,"reasoningOutputTokens":0,"totalTokens":2000000},"costUsd":0}"#,
                ts
            ],
        )
        .unwrap();
        db_path
    }

    #[test]
    fn incremental_read_imports_only_new_messages() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-freebuff-inc-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        write_project(&dir, "Alpha", 1, 1_700_000_000_000);
        let provider = FreebuffProvider::with_config_dir(dir.clone());

        let first = provider
            .fetch(&FetchRequest {
                cursor: None,
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert_eq!(first.records.len(), 1);

        // Nothing new yet.
        let second = provider
            .fetch(&FetchRequest {
                cursor: Some(&first.cursor),
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert!(second.records.is_empty());

        // A newer message arrives.
        write_project(&dir, "Alpha", 2, 1_700_000_100_000);
        let third = provider
            .fetch(&FetchRequest {
                cursor: Some(&second.cursor),
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert_eq!(third.records.len(), 1);
        assert_eq!(third.records[0].id, "fb_Alpha_2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_broken_project_does_not_stop_the_others() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-freebuff-isolation-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        write_project(&dir, "Healthy", 1, 1_700_000_000_000);
        std::fs::create_dir_all(dir.join("Broken")).unwrap();
        std::fs::write(dir.join("Broken/desktop-v2.db"), b"not a database").unwrap();

        let provider = FreebuffProvider::with_config_dir(dir.clone());
        let outcome = provider
            .fetch(&FetchRequest {
                cursor: None,
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert_eq!(outcome.records.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
