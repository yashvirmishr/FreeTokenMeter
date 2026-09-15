//! Gemini CLI provider.
//!
//! # Source
//!
//! Gemini CLI persists each session as a JSON document under
//! `~/.gemini/tmp/<project-hash>/chats/session-*.json`:
//!
//! ```json
//! {
//!   "sessionId": "…",
//!   "startTime": "…", "lastUpdated": "…",
//!   "messages": [
//!     {
//!       "id": "…", "timestamp": "…", "type": "gemini",
//!       "model": "gemini-2.5-pro",
//!       "tokens": { "input": 1200, "output": 340, "cached": 0,
//!                   "thoughts": 120, "tool": 0, "total": 1660 }
//!     }
//!   ]
//! }
//! ```
//!
//! The interactive `/stats` screen is deliberately **not** scraped: the session
//! files are the structured source, and they survive whether or not the CLI is
//! running.
//!
//! # Cached tokens are often absent
//!
//! Gemini only reports cached-token detail for API-key / Vertex users; OAuth
//! (free-tier) sessions omit it. Those fields are preserved as `null` rather
//! than being flattened to `0`.
//!
//! # Cost semantics
//!
//! Access method decides the money story, so the provider reads it from
//! configuration instead of guessing:
//!
//! * Google-account sign-in (`oauth_creds.json`) → promotional/free tier, so
//!   `ACTUAL COST: $0.00` with a reference API value
//! * otherwise the payment mode is `UNKNOWN` and actual cost stays `N/A`
//!
//! Gemini is never assumed to be free because of its name.

use std::path::{Path, PathBuf};

use chrono::DateTime;
use log::{info, warn};
use serde_json::Value;

use crate::database::UsageRecord;
use crate::pricing::{value_usage, PricingInput};
use crate::provider::{
    collect_files_with_extension, rfc3339, FetchOutcome, FetchRequest, UsageProvider,
};
use crate::usage::{BillingUnit, PaymentMode, ProviderAvailability, SourceType};

pub struct GeminiCliProvider {
    config_dir: PathBuf,
}

impl GeminiCliProvider {
    pub const ID: &'static str = crate::usage::provider_id::GEMINI_CLI;

    pub fn new() -> Self {
        Self::with_config_dir(dirs::home_dir().unwrap_or_default().join(".gemini"))
    }

    pub fn with_config_dir(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    /// Directory holding per-project session state.
    pub fn tmp_dir(&self) -> PathBuf {
        self.config_dir.join("tmp")
    }

    /// Every `chats/session-*.json`, oldest path first.
    pub fn session_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        collect_files_with_extension(&self.tmp_dir(), "json", &mut files);
        files.retain(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("session-"))
                .unwrap_or(false)
        });
        files.sort();
        files
    }

    /// Google-account sign-in means the free promotional tier, which is the
    /// only local evidence of how this usage is paid for.
    fn has_google_sign_in(&self) -> bool {
        self.config_dir.join("oauth_creds.json").exists()
    }
}

impl Default for GeminiCliProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn timestamp_ms(value: &Value) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn token_field(tokens: &Value, key: &str) -> Option<i64> {
    tokens.get(key).and_then(|v| v.as_i64())
}

/// Result of parsing one session document.
#[derive(Debug, Default)]
pub struct SessionParse {
    pub records: Vec<UsageRecord>,
    pub scanned: usize,
    pub skipped: usize,
}

/// Parse a Gemini CLI session document into normalized records.
///
/// Only `gemini` turns that carry a `tokens` block produce records.
pub fn parse_session(
    document: &str,
    source_id: &str,
    since_ms: Option<i64>,
    registry: Option<&crate::registry::RegistryState>,
    payment_mode: PaymentMode,
) -> SessionParse {
    let mut out = SessionParse::default();

    let Ok(root) = serde_json::from_str::<Value>(document) else {
        out.skipped += 1;
        return out;
    };

    let session_id = root
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let Some(messages) = root.get("messages").and_then(|m| m.as_array()) else {
        out.skipped += 1;
        return out;
    };

    for message in messages {
        out.scanned += 1;

        let kind = message.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        if kind != "gemini" {
            out.skipped += 1;
            continue;
        }

        let Some(tokens) = message.get("tokens").filter(|t| t.is_object()) else {
            out.skipped += 1;
            continue;
        };

        let input = token_field(tokens, "input");
        let output = token_field(tokens, "output");
        // Present for API-key / Vertex users; absent for OAuth sessions.
        let cached = token_field(tokens, "cached");
        let thoughts = token_field(tokens, "thoughts");

        if input.is_none() && output.is_none() && cached.is_none() && thoughts.is_none() {
            out.skipped += 1;
            continue;
        }

        let Some(event_id) = message
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        else {
            out.skipped += 1;
            continue;
        };

        let Some(ts) = message
            .get("timestamp")
            .and_then(timestamp_ms)
            .or_else(|| root.get("lastUpdated").and_then(timestamp_ms))
        else {
            out.skipped += 1;
            continue;
        };

        if let Some(cutoff) = since_ms {
            if ts < cutoff {
                out.skipped += 1;
                continue;
            }
        }

        let model = message
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let value = value_usage(
            &PricingInput {
                provider: GeminiCliProvider::ID,
                model: &model,
                provider_cost_usd: None,
                billing_units: None,
                billing_unit: BillingUnit::Usd,
                payment_mode,
                input_tokens: input,
                output_tokens: output,
                cached_tokens: cached,
                cache_write_tokens: None,
            },
            registry,
        );

        // `total` from the source when present, otherwise the sum of what the
        // session actually reported (thoughts are billed as output).
        let total = token_field(tokens, "total").unwrap_or_else(|| {
            input.unwrap_or(0) + output.unwrap_or(0) + thoughts.unwrap_or(0)
        });

        out.records.push(UsageRecord {
            id: format!("gemini_{}", event_id),
            provider: GeminiCliProvider::ID.to_string(),
            model,
            timestamp: rfc3339(ts),
            input_tokens: input,
            output_tokens: output,
            cached_tokens: cached,
            cache_write_tokens: None,
            reasoning_tokens: thoughts,
            total_tokens: total,
            actual_cost_usd: value.actual_cost_usd,
            reference_value: value.reference_value,
            free_value: value.free_value,
            billing_unit: value.billing_unit.as_str().to_string(),
            billing_units: value.billing_units,
            // The session document names no metered unit or tool version.
            billing_metric_name: None,
            payment_mode: value.payment_mode.as_str().to_string(),
            pricing_status: value.pricing_status.as_str().to_string(),
            pricing_source: Some(value.pricing_source.as_str().to_string()),
            pricing_version: Some(value.pricing_version),
            reference_model: value.reference_model,
            reference_input_rate: value.reference_input_rate,
            reference_output_rate: value.reference_output_rate,
            reference_cached_rate: value.reference_cached_rate,
            session_id: session_id.clone(),
            event_id: Some(event_id),
            source_type: SourceType::GeminiSessionFile.as_str().to_string(),
            source_id: Some(source_id.to_string()),
            source_version: None,
        });
    }

    out
}

/// File modification time in milliseconds, used as a cheap "has this session
/// changed?" check. A session document is rewritten as the session grows.
fn modified_ms(path: &Path) -> Option<u128> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
}

impl UsageProvider for GeminiCliProvider {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn display_name(&self) -> &'static str {
        "Gemini CLI"
    }

    fn short_label(&self) -> &'static str {
        "GEMINI"
    }

    fn source_type(&self) -> &'static str {
        SourceType::GeminiSessionFile.as_str()
    }

    fn payment_mode(&self) -> PaymentMode {
        if self.has_google_sign_in() {
            PaymentMode::Promotional
        } else {
            PaymentMode::Unknown
        }
    }

    fn availability(&self) -> ProviderAvailability {
        if !self.config_dir.exists() {
            return ProviderAvailability::NotInstalled;
        }
        if self.session_files().is_empty() {
            return ProviderAvailability::UsageUnavailable;
        }
        ProviderAvailability::Connected
    }

    fn detail(&self) -> Option<String> {
        match self.availability() {
            ProviderAvailability::Connected => Some(format!(
                "{} session file(s) in {}",
                self.session_files().len(),
                self.tmp_dir().display()
            )),
            ProviderAvailability::NotInstalled => {
                Some(format!("No Gemini CLI installation at {}", self.config_dir.display()))
            }
            ProviderAvailability::UsageUnavailable => Some(format!(
                "No Gemini CLI session files under {}",
                self.tmp_dir().display()
            )),
            other => Some(other.label().to_string()),
        }
    }

    fn fetch(&self, request: &FetchRequest<'_>) -> Result<FetchOutcome, String> {
        let files = self.session_files();
        if files.is_empty() {
            return Err("No Gemini CLI session files found".to_string());
        }

        let cursor = request.cursor.cloned().unwrap_or_default();
        let mut next = cursor.clone();
        let mut records = Vec::new();
        let mut scanned = 0usize;
        let mut skipped = 0usize;
        let payment_mode = self.payment_mode();

        for path in files {
            if records.len() >= request.limit {
                break;
            }
            let key = path.to_string_lossy().to_string();

            // Session documents are rewritten rather than appended, so the
            // modification time is the change marker.
            let Some(mtime) = modified_ms(&path) else {
                continue;
            };
            if cursor
                .extra
                .get(&format!("mtime:{}", key))
                .and_then(|v| v.parse::<u128>().ok())
                == Some(mtime)
            {
                continue;
            }

            let document = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) => {
                    warn!("Gemini CLI: cannot read {}: {}", path.display(), e);
                    continue;
                }
            };

            if let Some(cutoff) = request.since_ms {
                if let Some(last_updated) = root_timestamp(&document) {
                    if last_updated < cutoff {
                        next.extra.insert(format!("mtime:{}", key), mtime.to_string());
                        continue;
                    }
                }
            }

            let parsed = parse_session(&document, &key, request.since_ms, request.registry, payment_mode);
            scanned += parsed.scanned;
            skipped += parsed.skipped;

            for record in parsed.records {
                if let Some(ts) = DateTime::parse_from_rfc3339(&record.timestamp)
                    .ok()
                    .map(|dt| dt.timestamp_millis())
                {
                    next.last_timestamp_ms = Some(next.last_timestamp_ms.unwrap_or(0).max(ts));
                }
                next.last_event_id = record.event_id.clone();
                records.push(record);
            }

            next.extra.insert(format!("mtime:{}", key), mtime.to_string());
        }

        records.truncate(request.limit);
        info!(
            "Gemini CLI: scanned {} turn(s), imported {} record(s)",
            scanned,
            records.len()
        );

        Ok(FetchOutcome {
            records,
            cursor: next,
            scanned,
            skipped,
        })
    }

    fn default_backfill_days(&self) -> i64 {
        30
    }
}

fn root_timestamp(document: &str) -> Option<i64> {
    let root: Value = serde_json::from_str(document).ok()?;
    root.get("lastUpdated")
        .or_else(|| root.get("startTime"))
        .and_then(timestamp_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = r#"{
      "sessionId": "sess-1",
      "startTime": "2026-08-02T10:00:00.000Z",
      "lastUpdated": "2026-08-02T10:05:00.000Z",
      "messages": [
        {"id":"u1","timestamp":"2026-08-02T10:00:01.000Z","type":"user","content":"hello"},
        {"id":"m1","timestamp":"2026-08-02T10:00:10.000Z","type":"gemini","model":"gemini-2.5-pro",
         "tokens":{"input":1200,"output":340,"cached":0,"thoughts":120,"tool":0,"total":1660}},
        {"id":"m2","timestamp":"2026-08-02T10:01:10.000Z","type":"gemini","model":"gemini-2.5-flash",
         "tokens":{"input":800,"output":200,"thoughts":10,"total":1010}}
      ]
    }"#;

    fn parse(text: &str, since: Option<i64>, mode: PaymentMode) -> SessionParse {
        parse_session(text, "/tmp/session-1.json", since, None, mode)
    }

    #[test]
    fn parses_gemini_turns_only() {
        let parsed = parse(SESSION, None, PaymentMode::Promotional);
        assert_eq!(parsed.records.len(), 2);
        assert_eq!(parsed.scanned, 3);
        assert_eq!(parsed.skipped, 1);
        assert_eq!(parsed.records[0].model, "gemini-2.5-pro");
    }

    #[test]
    fn missing_cached_tokens_stay_null() {
        let parsed = parse(SESSION, None, PaymentMode::Promotional);
        let second = &parsed.records[1];
        assert!(
            second.cached_tokens.is_none(),
            "OAuth sessions omit cache data; it must not become 0"
        );
        assert_eq!(second.reasoning_tokens, Some(10));
        assert_eq!(second.total_tokens, 1_010);
    }

    #[test]
    fn promotional_usage_has_a_known_zero_actual_cost() {
        let parsed = parse(SESSION, None, PaymentMode::Promotional);
        let first = &parsed.records[0];
        assert_eq!(first.actual_cost_usd, Some(0.0));
        assert_eq!(first.payment_mode, "promotional");
    }

    #[test]
    fn unknown_payment_mode_never_claims_a_zero_cost() {
        let parsed = parse(SESSION, None, PaymentMode::Unknown);
        let first = &parsed.records[0];
        assert!(first.actual_cost_usd.is_none());
        assert_eq!(first.payment_mode, "unknown");
    }

    #[test]
    fn backfill_window_filters_old_turns() {
        let cutoff = DateTime::parse_from_rfc3339("2026-08-02T10:01:00.000Z")
            .unwrap()
            .timestamp_millis();
        let parsed = parse(SESSION, Some(cutoff), PaymentMode::Promotional);
        assert_eq!(parsed.records.len(), 1);
        assert_eq!(parsed.records[0].event_id.as_deref(), Some("m2"));
    }

    #[test]
    fn malformed_document_is_reported_not_guessed() {
        let parsed = parse("{not json", None, PaymentMode::Promotional);
        assert!(parsed.records.is_empty());
        assert_eq!(parsed.skipped, 1);
    }

    #[test]
    fn session_without_messages_is_skipped() {
        let parsed = parse("{\"sessionId\":\"s\"}", None, PaymentMode::Promotional);
        assert!(parsed.records.is_empty());
    }

    #[test]
    fn reports_not_installed_when_missing() {
        let provider = GeminiCliProvider::with_config_dir(PathBuf::from("/definitely/missing/gemini"));
        assert_eq!(provider.availability(), ProviderAvailability::NotInstalled);
    }

    #[test]
    fn reports_usage_unavailable_without_session_files() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-gemini-empty-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(dir.join("tmp")).unwrap();

        let provider = GeminiCliProvider::with_config_dir(dir.clone());
        assert_eq!(provider.availability(), ProviderAvailability::UsageUnavailable);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unchanged_sessions_are_not_reparsed() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-gemini-inc-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let chats = dir.join("tmp").join("hash").join("chats");
        std::fs::create_dir_all(&chats).unwrap();
        std::fs::write(chats.join("session-1.json"), SESSION).unwrap();

        let provider = GeminiCliProvider::with_config_dir(dir.clone());
        let request = FetchRequest {
            cursor: None,
            since_ms: None,
            registry: None,
            limit: 1_000,
        };
        let first = provider.fetch(&request).unwrap();
        assert_eq!(first.records.len(), 2);

        let second = provider
            .fetch(&FetchRequest {
                cursor: Some(&first.cursor),
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert!(second.records.is_empty(), "unchanged session must not be re-imported");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
