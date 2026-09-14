use rusqlite::{Connection, params};
use std::path::PathBuf;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use log::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub timestamp: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cached_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost: f64,
    pub reference_value: Option<f64>,
    pub free_value: Option<f64>,
    pub pricing_status: String,
    pub session_id: Option<String>,
    pub pricing_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub status: String,
    pub detail: Option<String>,
    pub last_sync: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSummary {
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cached_tokens: i64,
    pub request_count: i64,
    pub actual_cost: f64,
    pub reference_value: f64,
    pub free_value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider: String,
    pub tokens: i64,
    pub percentage: f64,
    pub actual_cost: f64,
    pub reference_value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsage {
    pub model: String,
    pub tokens: i64,
    pub percentage: f64,
    pub actual_cost: f64,
    pub reference_value: Option<f64>,
    pub pricing_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub timestamp: String,
    pub provider: String,
    pub model: String,
    pub tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardData {
    pub summary: UsageSummary,
    pub provider_usage: Vec<ProviderUsage>,
    pub model_usage: Vec<ModelUsage>,
    pub activity: Vec<ActivityEntry>,
    pub providers: Vec<ProviderStatus>,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(db_path: &PathBuf) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;

        // Schema v3: added reference_value, free_value, pricing_status, pricing_version.
        // Uses INSERT OR REPLACE so updated sessions overwrite stale data.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS usage_records (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                reasoning_tokens INTEGER NOT NULL DEFAULT 0,
                cached_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                estimated_cost REAL NOT NULL DEFAULT 0.0,
                reference_value REAL,
                free_value REAL,
                pricing_status TEXT NOT NULL DEFAULT 'unknown',
                session_id TEXT,
                time_created INTEGER NOT NULL,
                pricing_version TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_usage_timestamp ON usage_records(timestamp);
            CREATE INDEX IF NOT EXISTS idx_usage_provider ON usage_records(provider);
            CREATE INDEX IF NOT EXISTS idx_usage_model ON usage_records(model);
            CREATE INDEX IF NOT EXISTS idx_usage_session ON usage_records(session_id);
            CREATE INDEX IF NOT EXISTS idx_usage_time_created ON usage_records(time_created);

            CREATE TABLE IF NOT EXISTS sync_state (
                provider TEXT PRIMARY KEY,
                last_sync_time INTEGER NOT NULL,
                last_session_id TEXT
            );"
        )?;

        // Migration v1→v2: add reasoning_tokens
        let has_reasoning = {
            conn.prepare("SELECT reasoning_tokens FROM usage_records LIMIT 1").is_ok()
        };
        if !has_reasoning {
            info!("Migrating schema v1→v2: adding reasoning_tokens column");
            conn.execute_batch(
                "ALTER TABLE usage_records ADD COLUMN reasoning_tokens INTEGER NOT NULL DEFAULT 0;
                 DROP TABLE IF EXISTS sync_state;
                 CREATE TABLE IF NOT EXISTS sync_state (
                     provider TEXT PRIMARY KEY,
                     last_sync_time INTEGER NOT NULL,
                     last_session_id TEXT
                 );"
            )?;
        }

        // Migration v2→v3: add reference pricing columns
        let has_reference = {
            conn.prepare("SELECT reference_value FROM usage_records LIMIT 1").is_ok()
        };
        if !has_reference {
            info!("Migrating schema v2→v3: adding reference_value, free_value, pricing_status, pricing_version");
            conn.execute_batch(
                "ALTER TABLE usage_records ADD COLUMN reference_value REAL;
                 ALTER TABLE usage_records ADD COLUMN free_value REAL;
                 ALTER TABLE usage_records ADD COLUMN pricing_status TEXT NOT NULL DEFAULT 'unknown';
                 ALTER TABLE usage_records ADD COLUMN pricing_version TEXT;
                 DROP TABLE IF EXISTS sync_state;
                 CREATE TABLE IF NOT EXISTS sync_state (
                     provider TEXT PRIMARY KEY,
                     last_sync_time INTEGER NOT NULL,
                     last_session_id TEXT
                 );"
            )?;
        }

        Ok(Self { conn })
    }

    /// Insert or replace a usage record.
    /// Uses INSERT OR REPLACE so that if a session's tokens increase after
    /// initial sync, the updated record overwrites the stale one.
    pub fn upsert_usage_record(&self, record: &UsageRecord) -> Result<(), rusqlite::Error> {
        let now = Utc::now().timestamp_millis();
        self.conn.execute(
            "INSERT OR REPLACE INTO usage_records
             (id, provider, model, timestamp, input_tokens, output_tokens, reasoning_tokens,
              cached_tokens, total_tokens, estimated_cost, reference_value, free_value,
              pricing_status, session_id, time_created, pricing_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                record.id,
                record.provider,
                record.model,
                record.timestamp,
                record.input_tokens,
                record.output_tokens,
                record.reasoning_tokens,
                record.cached_tokens,
                record.total_tokens,
                record.estimated_cost,
                record.reference_value,
                record.free_value,
                record.pricing_status,
                record.session_id,
                now,
                record.pricing_version,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_usage_records(&self, records: &[UsageRecord]) -> Result<usize, rusqlite::Error> {
        let mut count = 0;
        for record in records {
            self.upsert_usage_record(record)?;
            count += 1;
        }
        Ok(count)
    }

    pub fn get_last_sync_time(&self, provider: &str) -> Result<Option<i64>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT last_sync_time FROM sync_state WHERE provider = ?1"
        )?;
        let mut rows = stmt.query_map(params![provider], |row| {
            row.get::<_, i64>(0)
        })?;
        
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn update_sync_state(&self, provider: &str, last_sync: i64) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT OR REPLACE INTO sync_state (provider, last_sync_time, last_session_id) 
             VALUES (?1, ?2, NULL)",
            params![provider, last_sync],
        )?;
        Ok(())
    }

    pub fn get_usage_summary(&self, days: Option<i64>) -> Result<UsageSummary, rusqlite::Error> {
        let (where_clause, query_params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match days {
            Some(d) => {
                let cutoff = Utc::now().timestamp_millis() - (d * 24 * 60 * 60 * 1000);
                ("WHERE time_created >= ?1".to_string(), vec![Box::new(cutoff)])
            }
            None => ("WHERE 1=1".to_string(), vec![]),
        };

        let query = format!(
            "SELECT 
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(reasoning_tokens), 0),
                COALESCE(SUM(cached_tokens), 0),
                COUNT(*),
                COALESCE(SUM(estimated_cost), 0.0),
                COALESCE(SUM(CASE WHEN reference_value IS NOT NULL THEN reference_value ELSE 0.0 END), 0.0),
                COALESCE(SUM(CASE WHEN free_value IS NOT NULL THEN free_value ELSE 0.0 END), 0.0)
             FROM usage_records {}",
            where_clause
        );

        let mut stmt = self.conn.prepare(&query)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = query_params.iter().map(|p| p.as_ref()).collect();
        let mut rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, f64>(8)?,
            ))
        })?;

        if let Some(row) = rows.next() {
            let (total, input, output, reasoning, cached, count, est_cost, ref_val, free_val) = row?;
            Ok(UsageSummary {
                total_tokens: total,
                input_tokens: input,
                output_tokens: output,
                reasoning_tokens: reasoning,
                cached_tokens: cached,
                request_count: count,
                actual_cost: est_cost,
                reference_value: ref_val,
                free_value: free_val,
            })
        } else {
            Ok(UsageSummary {
                total_tokens: 0,
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
                cached_tokens: 0,
                request_count: 0,
                actual_cost: 0.0,
                reference_value: 0.0,
                free_value: 0.0,
            })
        }
    }

    pub fn get_provider_usage(&self, days: Option<i64>) -> Result<Vec<ProviderUsage>, rusqlite::Error> {
        let (where_clause, query_params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match days {
            Some(d) => {
                let cutoff = Utc::now().timestamp_millis() - (d * 24 * 60 * 60 * 1000);
                ("WHERE time_created >= ?1".to_string(), vec![Box::new(cutoff)])
            }
            None => ("WHERE 1=1".to_string(), vec![]),
        };

        let query = format!(
            "SELECT provider,
                    COALESCE(SUM(total_tokens), 0) as tokens,
                    COALESCE(SUM(estimated_cost), 0.0) as cost,
                    COALESCE(SUM(CASE WHEN reference_value IS NOT NULL THEN reference_value ELSE 0.0 END), 0.0) as ref_val
             FROM usage_records {} GROUP BY provider ORDER BY tokens DESC",
            where_clause
        );

        let mut stmt = self.conn.prepare(&query)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = query_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })?;

        let raw: Vec<(String, i64, f64, f64)> = rows.filter_map(|r| r.ok()).collect();
        let total_tokens: i64 = raw.iter().map(|(_, t, _, _)| *t).sum();
        
        Ok(raw.into_iter().map(|(provider, tokens, cost, ref_val)| {
            let percentage = if total_tokens > 0 {
                (tokens as f64 / total_tokens as f64) * 100.0
            } else {
                0.0
            };
            ProviderUsage {
                provider,
                tokens,
                percentage,
                actual_cost: cost,
                reference_value: ref_val,
            }
        }).collect())
    }

    pub fn get_model_usage(&self, days: Option<i64>) -> Result<Vec<ModelUsage>, rusqlite::Error> {
        let (where_clause, query_params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match days {
            Some(d) => {
                let cutoff = Utc::now().timestamp_millis() - (d * 24 * 60 * 60 * 1000);
                ("WHERE time_created >= ?1".to_string(), vec![Box::new(cutoff)])
            }
            None => ("WHERE 1=1".to_string(), vec![]),
        };

        // Use the most common pricing_status per model for display
        let query = format!(
            "SELECT model,
                    COALESCE(SUM(total_tokens), 0) as tokens,
                    COALESCE(SUM(estimated_cost), 0.0) as cost,
                    COALESCE(SUM(CASE WHEN reference_value IS NOT NULL THEN reference_value ELSE 0.0 END), 0.0) as ref_val,
                    pricing_status
             FROM usage_records {} GROUP BY model ORDER BY tokens DESC",
            where_clause
        );

        let mut stmt = self.conn.prepare(&query)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = query_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let raw: Vec<(String, i64, f64, f64, String)> = rows.filter_map(|r| r.ok()).collect();
        let total_tokens: i64 = raw.iter().map(|(_, t, _, _, _)| *t).sum();
        
        Ok(raw.into_iter().map(|(model, tokens, cost, ref_val, status)| {
            let percentage = if total_tokens > 0 {
                (tokens as f64 / total_tokens as f64) * 100.0
            } else {
                0.0
            };
            // If ref_val > 0, show it; otherwise None for "N/A"
            let reference_value = if ref_val > 0.0 { Some(ref_val) } else { None };
            ModelUsage {
                model,
                tokens,
                percentage,
                actual_cost: cost,
                reference_value,
                pricing_status: status,
            }
        }).collect())
    }

    pub fn get_activity(&self, limit: usize) -> Result<Vec<ActivityEntry>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, provider, model, total_tokens 
             FROM usage_records ORDER BY time_created DESC LIMIT ?1"
        )?;
        
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(ActivityEntry {
                timestamp: row.get(0)?,
                provider: row.get(1)?,
                model: row.get(2)?,
                tokens: row.get(3)?,
            })
        })?;

        Ok(rows.filter_map(|r| {
            r.map_err(|e| warn!("Skipping malformed activity row: {}", e)).ok()
        }).collect())
    }

    /// Count total records, for diagnostics.
    pub fn count_records(&self) -> Result<usize, rusqlite::Error> {
        let mut stmt = self.conn.prepare("SELECT COUNT(*) FROM usage_records")?;
        let mut rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        rows.next().unwrap_or(Ok(0)).map(|n| n as usize)
    }
}
