use rusqlite::Connection;
use std::path::PathBuf;
use chrono::{Utc, TimeZone};
use serde::{Deserialize, Serialize};
use log::{info, warn, error};

use crate::database::UsageRecord;
use crate::pricing::estimate_model_cost;

/// Raw session row as stored in OpenCode's SQLite database.
#[derive(Debug, Clone)]
pub(crate) struct OpenCodeSession {
    pub(crate) id: String,
    pub(crate) model_json: Option<String>,
    pub(crate) tokens_input: i64,
    pub(crate) tokens_output: i64,
    pub(crate) tokens_reasoning: i64,
    pub(crate) tokens_cache_read: i64,
    pub(crate) tokens_cache_write: i64,
    pub(crate) cost: f64,
    pub(crate) time_created: i64,
}

/// Parsed model info from OpenCode's JSON model field.
/// Uses serde rename to match the actual camelCase JSON keys.
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
    pub fn new() -> Self {
        let db_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".local")
            .join("share")
            .join("opencode")
            .join("opencode.db");
        
        Self { db_path }
    }

    pub fn is_available(&self) -> bool {
        self.db_path.exists()
    }

    pub fn get_status(&self) -> (bool, String, Option<String>) {
        if self.is_available() {
            (true, "CONNECTED".to_string(), Some(format!("Database: {}", self.db_path.display())))
        } else {
            (false, "NOT FOUND".to_string(), Some("OpenCode installation was not detected.".to_string()))
        }
    }

    /// Fetch all sessions newer than `last_sync_time`, or all sessions if None.
    /// Returns raw sessions before normalization — the sync layer handles dedup.
    pub fn fetch_sessions(&self, last_sync_time: Option<i64>) -> Result<Vec<OpenCodeSession>, String> {
        if !self.is_available() {
            return Err("OpenCode database not found".to_string());
        }

        let conn = Connection::open(&self.db_path)
            .map_err(|e| {
                error!("Failed to open OpenCode database: {}", e);
                format!("Failed to open OpenCode database: {}", e)
            })?;

        // Select all token columns including reasoning, and time_updated for staleness detection
        let sql = if last_sync_time.is_some() {
            "SELECT id, model, tokens_input, tokens_output, tokens_reasoning,
                    tokens_cache_read, tokens_cache_write,
                    cost, time_created, time_updated
             FROM session
             WHERE time_updated > ?1
             ORDER BY time_created ASC"
        } else {
            "SELECT id, model, tokens_input, tokens_output, tokens_reasoning,
                    tokens_cache_read, tokens_cache_write,
                    cost, time_created, time_updated
             FROM session
             ORDER BY time_created ASC"
        };

        let mut stmt = conn.prepare(sql)
            .map_err(|e| {
                error!("Failed to prepare session query: {}", e);
                format!("Failed to prepare query: {}", e)
            })?;

        let row_mapper = |row: &rusqlite::Row| {
            // Read time_updated for SQL filtering but don't store it
            let _time_updated: i64 = row.get(9)?;
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

        let sessions: Vec<OpenCodeSession> = if let Some(sync_time) = last_sync_time {
            stmt.query_map(rusqlite::params![sync_time], row_mapper)
        } else {
            stmt.query_map([], row_mapper)
        }
        .map_err(|e| {
            error!("Failed to query sessions: {}", e);
            format!("Failed to query sessions: {}", e)
        })?
        .filter_map(|r| match r {
            Ok(s) => Some(s),
            Err(e) => {
                warn!("Skipping malformed session row: {}", e);
                None
            }
        })
        .collect();

        info!("Fetched {} sessions from OpenCode database", sessions.len());
        Ok(sessions)
    }

    /// Parse the model JSON and normalize to (model_id, provider_id, variant).
    /// Returns ("unknown", "opencode", None) on parse failure.
    pub fn parse_model(model_json: &Option<String>) -> (String, String, Option<String>) {
        match model_json {
            Some(json) => match serde_json::from_str::<OpenCodeModel>(json) {
                Ok(m) => (m.id, m.provider_id, m.variant),
                Err(e) => {
                    warn!("Failed to parse model JSON '{}': {}", json, e);
                    ("unknown".to_string(), "opencode".to_string(), None)
                }
            },
            None => ("unknown".to_string(), "opencode".to_string(), None),
        }
    }

    /// Convert a raw OpenCode session into our normalized UsageRecord.
    /// This is a pure transformation — no side effects, no DB access.
    pub fn normalize_session(session: &OpenCodeSession) -> UsageRecord {
        let (model_name, _provider_id, _variant) = Self::parse_model(&session.model_json);

        // OpenCode's total = input + output + reasoning + cache_read + cache_write
        // We store the individual breakdown and let the caller decide what "total" means.
        let total_tokens = session.tokens_input
            + session.tokens_output
            + session.tokens_reasoning
            + session.tokens_cache_read
            + session.tokens_cache_write;

        let timestamp = Utc.timestamp_millis_opt(session.time_created)
            .single()
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();

        // Use OpenCode's authoritative cost when available.
        // Only fall back to our pricing estimate when cost = $0.
        let estimated_cost = if session.cost > 0.0 {
            session.cost
        } else {
            estimate_model_cost(
                &model_name,
                session.tokens_input,
                session.tokens_output,
                session.tokens_cache_read,
            )
        };

        UsageRecord {
            id: format!("oc_{}", session.id),
            provider: "opencode".to_string(),
            model: model_name,
            timestamp,
            input_tokens: session.tokens_input,
            output_tokens: session.tokens_output,
            reasoning_tokens: session.tokens_reasoning,
            cached_tokens: session.tokens_cache_read,
            total_tokens,
            estimated_cost,
            session_id: Some(session.id.clone()),
        }
    }
}
