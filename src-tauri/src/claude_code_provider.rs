//! Claude Code provider.
//!
//! # Source
//!
//! Claude Code appends one JSONL transcript per session under
//! `~/.claude/projects/<project-slug>/<session-uuid>.jsonl`. Assistant turns
//! carry the API's own token accounting:
//!
//! ```json
//! {
//!   "type": "assistant",
//!   "uuid": "…", "sessionId": "…", "timestamp": "…",
//!   "message": {
//!     "id": "msg_…",
//!     "model": "claude-sonnet-4-5-20250929",
//!     "usage": {
//!       "input_tokens": 4,
//!       "cache_creation_input_tokens": 12345,
//!       "cache_read_input_tokens": 67890,
//!       "output_tokens": 512
//!     }
//!   }
//! }
//! ```
//!
//! Field names vary between Claude Code versions, so every metric is read
//! defensively and stays `null` when the version does not report it.
//!
//! # Cost semantics
//!
//! Claude Code's own cost figures are **list-price calculations**, and the
//! documentation is explicit that they are not the user's contracted rate.
//! FreeTokenMeter therefore:
//!
//! * never labels a transcript cost as the user's actual bill
//! * keeps it as `provider_reported_cost_usd`
//! * reports `ACTUAL COST: N/A` for subscription usage while still showing the
//!   reference API value
//!
//! # Status
//!
//! When `~/.claude/projects` does not exist — or exists without transcripts —
//! the provider reports `USAGE UNAVAILABLE` rather than inventing a parser for
//! the interactive `/usage` screen.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::DateTime;
use log::{info, warn};
use serde_json::Value;

use crate::database::UsageRecord;
use crate::pricing::{value_usage, PricingInput, PricingSource, PricingStatus};
use crate::provider::{
    collect_files_with_extension, read_appended, rfc3339, FetchOutcome, FetchRequest, UsageProvider,
};
use crate::usage::{BillingUnit, PaymentMode, ProviderAvailability, SourceType};

pub struct ClaudeCodeProvider {
    config_dir: PathBuf,
}

impl ClaudeCodeProvider {
    pub const ID: &'static str = crate::usage::provider_id::CLAUDE_CODE;

    pub fn new() -> Self {
        Self::with_config_dir(
            dirs::home_dir()
                .unwrap_or_default()
                .join(".claude"),
        )
    }

    pub fn with_config_dir(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.config_dir.join("projects")
    }

    /// All transcript files, oldest path first.
    pub fn transcript_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        collect_files_with_extension(&self.projects_dir(), "jsonl", &mut files);
        files.sort();
        files
    }
}

impl Default for ClaudeCodeProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn usage_int(usage: &Value, key: &str) -> Option<i64> {
    usage.get(key).and_then(|v| v.as_i64())
}

fn timestamp_ms(value: &Value) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Result of parsing transcript lines.
#[derive(Debug, Default)]
pub struct TranscriptParse {
    pub records: Vec<UsageRecord>,
    pub scanned: usize,
    pub skipped: usize,
    pub last_ts_ms: Option<i64>,
}

/// Parse Claude Code transcript lines into normalized records.
///
/// Only `assistant` turns that actually carry a `message.usage` block produce a
/// record; user turns, tool results and system lines are counted as skipped.
pub fn parse_transcript_lines<'a>(
    lines: impl Iterator<Item = &'a str>,
    since_ms: Option<i64>,
    registry: Option<&crate::registry::RegistryState>,
) -> TranscriptParse {
    let mut out = TranscriptParse::default();
    let mut seen: HashSet<String> = HashSet::new();

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        out.scanned += 1;

        let Ok(root) = serde_json::from_str::<Value>(line) else {
            out.skipped += 1;
            continue;
        };

        if root.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            out.skipped += 1;
            continue;
        }

        let message = root.get("message").unwrap_or(&Value::Null);
        let Some(usage) = message.get("usage").filter(|u| u.is_object()) else {
            out.skipped += 1;
            continue;
        };

        let input = usage_int(usage, "input_tokens");
        let output = usage_int(usage, "output_tokens");
        let cache_read = usage_int(usage, "cache_read_input_tokens");
        // Older versions nest the cache-write breakdown.
        let cache_write = usage_int(usage, "cache_creation_input_tokens").or_else(|| {
            usage.get("cache_creation").and_then(|c| {
                let ephemeral_5m = c.get("ephemeral_5m_input_tokens").and_then(|v| v.as_i64());
                let ephemeral_1h = c.get("ephemeral_1h_input_tokens").and_then(|v| v.as_i64());
                match (ephemeral_5m, ephemeral_1h) {
                    (None, None) => None,
                    (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
                }
            })
        });

        if input.is_none() && output.is_none() && cache_read.is_none() && cache_write.is_none() {
            out.skipped += 1;
            continue;
        }

        let event_id = message
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| root.get("uuid").and_then(|v| v.as_str()).map(str::to_string))
            .or_else(|| root.get("requestId").and_then(|v| v.as_str()).map(str::to_string));

        let Some(event_id) = event_id else {
            out.skipped += 1;
            continue;
        };
        if !seen.insert(event_id.clone()) {
            out.skipped += 1;
            continue;
        }

        let ts = root
            .get("timestamp")
            .and_then(timestamp_ms)
            .or(out.last_ts_ms);
        let Some(ts) = ts else {
            out.skipped += 1;
            continue;
        };
        out.last_ts_ms = Some(out.last_ts_ms.unwrap_or(0).max(ts));

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

        // A transcript cost is the provider's own list-price calculation, not
        // the user's bill. It is carried separately from `actual_cost_usd`.
        let reported_cost = root
            .get("costUSD")
            .and_then(|v| v.as_f64())
            .filter(|c| *c > 0.0);

        let mut value = value_usage(
            &PricingInput {
                provider: ClaudeCodeProvider::ID,
                model: &model,
                // Not the user's contracted rate, so it must not become actual cost.
                provider_cost_usd: None,
                billing_units: None,
                billing_unit: BillingUnit::Usd,
                payment_mode: PaymentMode::Subscription,
                input_tokens: input,
                output_tokens: output,
                cached_tokens: cache_read,
                cache_write_tokens: cache_write,
            },
            registry,
        );
        value.provider_reported_cost_usd = reported_cost;

        // If the shared registry cannot price this model, fall back to the
        // provider's own list-price figure rather than reporting nothing.
        if value.reference_value.is_none() {
            if let Some(cost) = reported_cost {
                value.reference_value = Some(cost);
                value.pricing_status = PricingStatus::ReferenceApplied;
                value.pricing_source = PricingSource::ProviderReported;
            }
        }

        // Claude reports input and output separately from cache traffic:
        // all four contribute to the tokens the model processed.
        let total = input.unwrap_or(0)
            + output.unwrap_or(0)
            + cache_read.unwrap_or(0)
            + cache_write.unwrap_or(0);

        out.records.push(UsageRecord {
            id: format!("claude_{}", event_id),
            provider: ClaudeCodeProvider::ID.to_string(),
            model,
            timestamp: rfc3339(ts),
            input_tokens: input,
            output_tokens: output,
            cached_tokens: cache_read,
            cache_write_tokens: cache_write,
            reasoning_tokens: None,
            total_tokens: total,
            actual_cost_usd: value.actual_cost_usd,
            reference_value: value.reference_value,
            free_value: value.free_value,
            billing_unit: value.billing_unit.as_str().to_string(),
            billing_units: value.billing_units,
            // The transcript exposes no entitlement name; only the reference
            // (list-price) value is knowable for a subscription plan.
            billing_metric_name: None,
            payment_mode: value.payment_mode.as_str().to_string(),
            pricing_status: value.pricing_status.as_str().to_string(),
            pricing_source: Some(value.pricing_source.as_str().to_string()),
            pricing_version: Some(value.pricing_version),
            reference_model: value.reference_model,
            reference_input_rate: value.reference_input_rate,
            reference_output_rate: value.reference_output_rate,
            reference_cached_rate: value.reference_cached_rate,
            session_id: root
                .get("sessionId")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            event_id: Some(event_id),
            source_type: SourceType::ClaudeSessionFile.as_str().to_string(),
            source_id: None,
            // Claude Code stamps its own version on each transcript line.
            source_version: root
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        });
    }

    out
}

impl UsageProvider for ClaudeCodeProvider {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn short_label(&self) -> &'static str {
        "CLAUDE"
    }

    fn source_type(&self) -> &'static str {
        SourceType::ClaudeSessionFile.as_str()
    }

    fn payment_mode(&self) -> PaymentMode {
        PaymentMode::Subscription
    }

    fn availability(&self) -> ProviderAvailability {
        if !self.config_dir.exists() {
            return ProviderAvailability::NotInstalled;
        }
        if self.transcript_files().is_empty() {
            // Claude Code is installed but exposes no local usage we can read.
            // The interactive `/usage` screen is deliberately not scraped.
            return ProviderAvailability::UsageUnavailable;
        }
        ProviderAvailability::Connected
    }

    fn detail(&self) -> Option<String> {
        match self.availability() {
            ProviderAvailability::Connected => Some(format!(
                "{} transcript file(s) in {}",
                self.transcript_files().len(),
                self.projects_dir().display()
            )),
            ProviderAvailability::NotInstalled => Some(format!(
                "No Claude Code installation at {}",
                self.config_dir.display()
            )),
            ProviderAvailability::UsageUnavailable => Some(format!(
                "No session transcripts in {} — list-price figures shown by `/usage` are not used",
                self.projects_dir().display()
            )),
            other => Some(other.label().to_string()),
        }
    }

    fn fetch(&self, request: &FetchRequest<'_>) -> Result<FetchOutcome, String> {
        let files = self.transcript_files();
        if files.is_empty() {
            return Err(format!(
                "No Claude Code transcripts found in {}",
                self.projects_dir().display()
            ));
        }
        if !self.config_dir.exists() {
            return Err("Claude Code is not installed".to_string());
        }

        let cursor = request.cursor.cloned().unwrap_or_default();
        let mut next = cursor.clone();
        let mut records = Vec::new();
        let mut scanned = 0usize;
        let mut skipped = 0usize;

        for path in files {
            if records.len() >= request.limit {
                break;
            }
            let key = path.to_string_lossy().to_string();
            let previous_offset = cursor.offsets.get(&key).copied().unwrap_or(0);

            let (chunk, new_offset) = match read_appended(&path, previous_offset) {
                Ok(pair) => pair,
                Err(e) => {
                    warn!("Claude Code: cannot read {}: {}", path.display(), e);
                    continue;
                }
            };

            let parsed = parse_transcript_lines(chunk.lines(), request.since_ms, request.registry);
            scanned += parsed.scanned;
            skipped += parsed.skipped;

            if let Some(ts) = parsed.last_ts_ms {
                next.last_timestamp_ms = Some(next.last_timestamp_ms.unwrap_or(0).max(ts));
            }

            for mut record in parsed.records {
                record.source_id = Some(key.clone());
                if let Some(event_id) = record.event_id.clone() {
                    next.last_event_id = Some(event_id);
                }
                records.push(record);
            }

            next.offsets.insert(key, new_offset);
        }

        records.truncate(request.limit);
        info!(
            "Claude Code: scanned {} line(s), imported {} record(s)",
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

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSCRIPT: &str = r#"{"type":"user","uuid":"u1","sessionId":"s1","timestamp":"2026-09-01T10:00:00.000Z","message":{"role":"user","content":"hi"}}
{"type":"assistant","uuid":"a1","sessionId":"s1","timestamp":"2026-09-01T10:00:05.000Z","version":"1.0.128","costUSD":0.0123,"message":{"id":"msg_1","model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":4,"cache_creation_input_tokens":12345,"cache_read_input_tokens":67890,"output_tokens":512}}}
{"type":"assistant","uuid":"a2","sessionId":"s1","timestamp":"2026-09-01T10:01:00.000Z","message":{"id":"msg_2","model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":10,"output_tokens":20}}}
"#;

    fn parse(text: &str, since: Option<i64>) -> TranscriptParse {
        parse_transcript_lines(text.lines(), since, None)
    }

    #[test]
    fn parses_assistant_usage_only() {
        let parsed = parse(TRANSCRIPT, None);
        assert_eq!(parsed.records.len(), 2);
        assert_eq!(parsed.scanned, 3);
        assert_eq!(parsed.skipped, 1);
    }

    #[test]
    fn does_not_label_list_price_as_actual_cost() {
        let parsed = parse(TRANSCRIPT, None);
        let first = &parsed.records[0];
        // The transcript's own figure stays separate from actual cost.
        assert!(first.actual_cost_usd.is_none());
        assert_eq!(first.payment_mode, "subscription");
        assert_eq!(first.source_type, "claude_session_file");
        // The tool version is carried through from the transcript line.
        assert_eq!(first.source_version.as_deref(), Some("1.0.128"));
    }

    #[test]
    fn reports_null_not_zero_for_missing_cache_metrics() {
        let parsed = parse(TRANSCRIPT, None);
        let second = &parsed.records[1];
        assert_eq!(second.input_tokens, Some(10));
        assert!(second.cached_tokens.is_none());
        assert!(second.cache_write_tokens.is_none());
        assert_eq!(second.total_tokens, 30);
    }

    #[test]
    fn claude_token_semantics_add_cache_traffic_to_the_total() {
        let parsed = parse(TRANSCRIPT, None);
        let first = &parsed.records[0];
        assert_eq!(first.cached_tokens, Some(67_890));
        assert_eq!(first.cache_write_tokens, Some(12_345));
        // input + output + cache read + cache write
        assert_eq!(first.total_tokens, 4 + 512 + 67_890 + 12_345);
    }

    #[test]
    fn deduplicates_retried_turns() {
        let duplicated = format!("{}{}\n", TRANSCRIPT, TRANSCRIPT.lines().nth(1).unwrap());
        let parsed = parse(&duplicated, None);
        assert_eq!(parsed.records.len(), 2);
    }

    #[test]
    fn backfill_window_filters_old_turns() {
        let cutoff = DateTime::parse_from_rfc3339("2026-09-01T10:00:30.000Z")
            .unwrap()
            .timestamp_millis();
        let parsed = parse(TRANSCRIPT, Some(cutoff));
        assert_eq!(parsed.records.len(), 1);
        assert_eq!(parsed.records[0].event_id.as_deref(), Some("msg_2"));
    }

    #[test]
    fn falls_back_to_provider_reported_value_when_the_registry_is_empty() {
        let parsed = parse(TRANSCRIPT, None);
        let first = &parsed.records[0];
        // No registry available in tests, but the transcript had a figure.
        assert_eq!(first.reference_value, Some(0.0123));
        assert_eq!(first.pricing_source.as_deref(), Some("provider_reported"));
    }

    #[test]
    fn nested_cache_creation_breakdown_is_summed() {
        let text = r#"{"type":"assistant","uuid":"a1","sessionId":"s1","timestamp":"2026-09-01T10:00:05.000Z","message":{"id":"msg_n","model":"claude-sonnet-4-5","usage":{"input_tokens":1,"output_tokens":2,"cache_creation":{"ephemeral_5m_input_tokens":100,"ephemeral_1h_input_tokens":200}}}}
"#;
        let parsed = parse(text, None);
        assert_eq!(parsed.records.len(), 1);
        assert_eq!(parsed.records[0].cache_write_tokens, Some(300));
        assert_eq!(parsed.records[0].total_tokens, 303);
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let text = format!("{}{}\n", TRANSCRIPT, "{\"type\":");
        let parsed = parse(&text, None);
        assert_eq!(parsed.records.len(), 2);
        assert_eq!(parsed.skipped, 2);
    }

    #[test]
    fn reports_not_installed_when_config_dir_is_absent() {
        let provider = ClaudeCodeProvider::with_config_dir(PathBuf::from("/definitely/missing/claude"));
        assert_eq!(provider.availability(), ProviderAvailability::NotInstalled);
    }

    #[test]
    fn reports_usage_unavailable_when_installed_without_transcripts() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-claude-empty-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(dir.join("projects")).unwrap();

        let provider = ClaudeCodeProvider::with_config_dir(dir.clone());
        assert_eq!(provider.availability(), ProviderAvailability::UsageUnavailable);
        assert!(provider
            .detail()
            .unwrap()
            .contains("transcripts"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incremental_read_only_imports_new_turns() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-claude-inc-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let project = dir.join("projects").join("demo");
        std::fs::create_dir_all(&project).unwrap();
        let file = project.join("session.jsonl");
        std::fs::write(&file, format!("{}\n", TRANSCRIPT.lines().next().unwrap())).unwrap();

        let provider = ClaudeCodeProvider::with_config_dir(dir.clone());
        let first = provider
            .fetch(&FetchRequest {
                cursor: None,
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert!(first.records.is_empty());

        std::fs::write(&file, TRANSCRIPT).unwrap();
        let second = provider
            .fetch(&FetchRequest {
                cursor: Some(&first.cursor),
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert_eq!(second.records.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
