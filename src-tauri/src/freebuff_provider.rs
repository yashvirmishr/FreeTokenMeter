/// Freebuff provider — reads real usage data from Freebuff Desktop's local SQLite databases.
///
/// Freebuff stores per-project databases at:
///   ~/.config/freebuff-desktop/projects/<project-hash>/desktop-v2.db
///
/// Each database contains a `messages` table with a `metrics_json` column
/// that records token usage for assistant messages:
///
/// ```json
/// {
///   "usage": {
///     "inputTokens": 5011758,
///     "cachedInputTokens": 4785280,
///     "outputTokens": 8529,
///     "reasoningOutputTokens": 0,
///     "totalTokens": 5020287
///   },
///   "costUsd": 0
/// }
/// ```
///
/// All Freebuff models are free (costUsd = 0). Reference pricing is configured
/// separately in pricing.rs for models with known paid equivalents.

use rusqlite::Connection;
use std::path::{PathBuf, Path};
use serde::Deserialize;
use log::{info, warn, error};

use crate::database::UsageRecord;
use crate::pricing::calculate_usage_value;

/// Raw metrics from Freebuff's metrics_json column.
/// Freebuff uses camelCase JSON keys (e.g., `inputTokens`, `costUsd`).
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

/// A raw message row from Freebuff's database.
struct FreebuffMessageRow {
    seq: i64,
    thread_id: String,
    metrics_json: String,
    ts: i64,
}

/// Thread info from Freebuff's database.
struct FreebuffThreadInfo {
    id: String,
    model: Option<String>,
}

pub struct FreebuffProvider {
    /// Base directory where Freebuff stores project databases.
    config_dir: PathBuf,
}

impl FreebuffProvider {
    pub fn new() -> Self {
        let config_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".config")
            .join("freebuff-desktop")
            .join("projects");

        Self { config_dir }
    }

    /// Check if the Freebuff config directory exists.
    pub fn is_available(&self) -> bool {
        self.config_dir.exists()
    }

    /// Get provider status.
    pub fn get_status(&self) -> (bool, String, Option<String>) {
        if !self.is_available() {
            return (false, "NOT FOUND".to_string(), Some("Freebuff Desktop installation was not detected.".to_string()));
        }

        let db_count = self.find_databases().len();
        if db_count == 0 {
            return (false, "NO DATA".to_string(), Some("Freebuff is installed but no project databases were found.".to_string()));
        }

        (true, "CONNECTED".to_string(), Some(format!("{} project database(s) found", db_count)))
    }

    /// Find all Freebuff desktop-v2.db files.
    fn find_databases(&self) -> Vec<PathBuf> {
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

        databases
    }

    /// Fetch all usage records from all Freebuff databases.
    /// Returns raw records before normalization — the sync layer handles dedup.
    pub fn fetch_all_records(&self) -> Result<Vec<FreebuffRawRecord>, String> {
        let databases = self.find_databases();

        if databases.is_empty() {
            return Err("No Freebuff project databases found".to_string());
        }

        let mut all_records = Vec::new();

        for db_path in &databases {
            match self.read_database(db_path) {
                Ok(mut records) => {
                    all_records.append(&mut records);
                }
                Err(e) => {
                    warn!("Failed to read Freebuff database {}: {}", db_path.display(), e);
                }
            }
        }

        info!("Fetched {} raw Freebuff records from {} databases", all_records.len(), databases.len());
        Ok(all_records)
    }

    /// Read usage records from a single Freebuff database.
    fn read_database(&self, db_path: &Path) -> Result<Vec<FreebuffRawRecord>, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| {
                error!("Failed to open Freebuff database {}: {}", db_path.display(), e);
                format!("Failed to open database: {}", e)
            })?;

        // Get all threads with their models
        let threads = self.read_threads(&conn)?;

        // Get assistant messages with non-empty metrics_json
        let messages = self.read_messages(&conn)?;

        // Also get user messages for context (they have empty metrics but we need timestamps)
        let _user_messages = self.read_user_messages(&conn)?;

        let mut records = Vec::new();

        // Process assistant messages with usage data
        for msg in &messages {
            let metrics: FreebuffMetrics = match serde_json::from_str(&msg.metrics_json) {
                Ok(m) => m,
                Err(e) => {
                    warn!("Failed to parse metrics_json for seq={}: {}", msg.seq, e);
                    continue;
                }
            };

            let usage = match metrics.usage {
                Some(u) => u,
                None => continue, // No usage data in this message
            };

            let input_tokens = usage.input_tokens.unwrap_or(0);
            let output_tokens = usage.output_tokens.unwrap_or(0);
            let cached_tokens = usage.cached_input_tokens.unwrap_or(0);
            let reasoning_tokens = usage.reasoning_output_tokens.unwrap_or(0);
            let total_tokens = usage.total_tokens.unwrap_or(input_tokens + output_tokens);

            // Skip messages with zero tokens
            if total_tokens == 0 {
                continue;
            }

            // Look up thread model
            let model = threads.get(&msg.thread_id)
                .and_then(|t| t.model.clone())
                .unwrap_or_else(|| "unknown".to_string());

            records.push(FreebuffRawRecord {
                seq: msg.seq,
                thread_id: msg.thread_id.clone(),
                model,
                input_tokens,
                output_tokens,
                cached_tokens,
                reasoning_tokens,
                total_tokens,
                cost_usd: metrics.cost_usd.unwrap_or(0.0),
                ts: msg.ts,
                db_path: db_path.to_path_buf(),
            });
        }

        Ok(records)
    }

    /// Read threads from a database.
    fn read_threads(&self, conn: &Connection) -> Result<std::collections::HashMap<String, FreebuffThreadInfo>, String> {
        let mut stmt = conn.prepare(
            "SELECT id, model FROM threads"
        ).map_err(|e| format!("Failed to prepare threads query: {}", e))?;

        let rows = stmt.query_map([], |row| {
            Ok(FreebuffThreadInfo {
                id: row.get(0)?,
                model: row.get(1)?,
            })
        }).map_err(|e| format!("Failed to query threads: {}", e))?;

        let mut map = std::collections::HashMap::new();
        for row in rows.flatten() {
            map.insert(row.id.clone(), row);
        }
        Ok(map)
    }

    /// Read assistant messages with non-empty metrics_json.
    fn read_messages(&self, conn: &Connection) -> Result<Vec<FreebuffMessageRow>, String> {
        let mut stmt = conn.prepare(
            "SELECT seq, thread_id, metrics_json, ts
             FROM messages
             WHERE role = 'assistant' AND metrics_json != '{}' AND metrics_json IS NOT NULL
             ORDER BY ts ASC"
        ).map_err(|e| format!("Failed to prepare messages query: {}", e))?;

        let rows = stmt.query_map([], |row| {
            Ok(FreebuffMessageRow {
                seq: row.get(0)?,
                thread_id: row.get(1)?,
                metrics_json: row.get(2)?,
                ts: row.get(3)?,
            })
        }).map_err(|e| format!("Failed to query messages: {}", e))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Read user messages (for activity feed context).
    fn read_user_messages(&self, conn: &Connection) -> Result<Vec<FreebuffMessageRow>, String> {
        let mut stmt = conn.prepare(
            "SELECT seq, thread_id, metrics_json, ts
             FROM messages
             WHERE role = 'user'
             ORDER BY ts ASC"
        ).map_err(|e| format!("Failed to prepare user messages query: {}", e))?;

        let rows = stmt.query_map([], |row| {
            Ok(FreebuffMessageRow {
                seq: row.get(0)?,
                thread_id: row.get(1)?,
                metrics_json: row.get(2)?,
                ts: row.get(3)?,
            })
        }).map_err(|e| format!("Failed to query user messages: {}", e))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

/// A raw Freebuff record before normalization.
pub struct FreebuffRawRecord {
    pub seq: i64,
    pub thread_id: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
    pub ts: i64,
    pub db_path: PathBuf,
}

/// Convert a raw Freebuff record into our normalized UsageRecord.
/// Uses the shared pricing engine for cost calculation.
pub fn normalize_record(record: &FreebuffRawRecord) -> UsageRecord {
    // Generate a stable ID: fb_<db_hash>_<seq>
    // Use the parent directory name as a project identifier
    let project_id = record.db_path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let usage_value = calculate_usage_value(
        &record.model,
        record.cost_usd,
        record.input_tokens,
        record.output_tokens,
        record.cached_tokens,
    );

    let timestamp = chrono::DateTime::from_timestamp_millis(record.ts)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default();

    UsageRecord {
        id: format!("fb_{}_{}", project_id, record.seq),
        provider: "freebuff".to_string(),
        model: record.model.clone(),
        timestamp,
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        reasoning_tokens: record.reasoning_tokens,
        cached_tokens: record.cached_tokens,
        total_tokens: record.total_tokens,
        estimated_cost: usage_value.actual_cost,
        reference_value: usage_value.reference_value,
        free_value: usage_value.free_value,
        pricing_status: usage_value.pricing_status.as_str().to_string(),
        session_id: Some(record.thread_id.clone()),
        pricing_version: Some(usage_value.pricing_version.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_record_generates_stable_id() {
        let record = FreebuffRawRecord {
            seq: 42,
            thread_id: "abc-123".to_string(),
            model: "mimo/mimo-v2.5".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cached_tokens: 200,
            reasoning_tokens: 0,
            total_tokens: 1500,
            cost_usd: 0.0,
            ts: 1700000000000,
            db_path: PathBuf::from("/home/user/.config/freebuff-desktop/projects/MyProject-abc123/desktop-v2.db"),
        };

        let normalized = normalize_record(&record);
        assert!(normalized.id.starts_with("fb_"));
        assert!(normalized.id.contains("42"));
        assert_eq!(normalized.provider, "freebuff");
        assert_eq!(normalized.model, "mimo/mimo-v2.5");
    }

    #[test]
    fn normalize_record_uses_pricing_engine() {
        let record = FreebuffRawRecord {
            seq: 1,
            thread_id: "test".to_string(),
            model: "mimo/mimo-v2.5".to_string(),
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            cached_tokens: 0,
            reasoning_tokens: 0,
            total_tokens: 1_500_000,
            cost_usd: 0.0,
            ts: 1700000000000,
            db_path: PathBuf::from("/test/desktop-v2.db"),
        };

        let normalized = normalize_record(&record);
        // mimo/mimo-v2.5 should have reference pricing
        assert!(normalized.reference_value.is_some());
        assert!(normalized.reference_value.unwrap() > 0.0);
    }
}
