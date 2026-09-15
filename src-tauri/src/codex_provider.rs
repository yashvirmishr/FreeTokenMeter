//! OpenAI Codex CLI provider.
//!
//! # Source
//!
//! Codex writes one rollout per session to
//! `~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl`. Each line is a JSON
//! object with a `type` and a `payload`. The structured usage records have
//! `"type": "token_usage_record"`, whose payload contains the provider's own
//! per-response token accounting:
//!
//! ```json
//! {
//!   "type": "token_usage_record",
//!   "payload": {
//!     "session_id": "...", "turn_id": "...",
//!     "response_id": "resp_...",
//!     "usage": {
//!       "input_tokens": 26580,
//!       "cached_input_tokens": 17152,
//!       "cache_write_input_tokens": 0,
//!       "output_tokens": 271,
//!       "reasoning_output_tokens": 103,
//!       "total_tokens": 26851
//!     }
//!   }
//! }
//! ```
//!
//! The model for a turn comes from the preceding `turn_context` line. The
//! `event_msg` / `token_count` lines carry a running total and a plan type
//! (`free`, `premium`, ...) which tells us this is entitlement usage.
//!
//! # Cost semantics
//!
//! There is no monetary cost anywhere in the rollout. Codex reports usage
//! against a subscription/entitlement, so:
//!
//! ```text
//! ACTUAL COST          N/A          (no per-token invoice exists locally)
//! REFERENCE API VALUE  $X.XX        (from the shared pricing registry)
//! ```
//!
//! FreeTokenMeter never translates "reference value" into "you saved $X" for a
//! flat-fee plan, because the alternative expenditure is not known.
//!
//! # Deduplication
//!
//! One record per `token_usage_record`, keyed on the API `response_id`. That is
//! the provider's own event identity. Note that `cached_input_tokens` is a
//! *subset* of `input_tokens`, so it is never added again on top of the total.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::DateTime;
use log::{info, warn};
use serde_json::Value;

use crate::database::UsageRecord;
use crate::pricing::{value_usage, PricingInput};
use crate::provider::{
    collect_files_with_extension, read_appended, rfc3339, FetchOutcome, FetchRequest, UsageProvider,
};
use crate::usage::{BillingUnit, PaymentMode, ProviderAvailability, SourceType};

/// Number of malformed lines tolerated before a file is abandoned.
const MAX_MALFORMED_LINES: usize = 100;

pub struct CodexProvider {
    sessions_dir: PathBuf,
}

impl CodexProvider {
    pub fn new() -> Self {
        Self::with_sessions_dir(
            dirs::home_dir()
                .unwrap_or_default()
                .join(".codex")
                .join("sessions"),
        )
    }

    /// Point the provider at an explicit rollout directory (tests, non-standard
    /// installs).
    pub fn with_sessions_dir(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    /// All rollout files, oldest first so incremental offsets are stable.
    pub fn session_files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = Vec::new();
        collect_files_with_extension(&self.sessions_dir, "jsonl", &mut files);
        files.sort();
        files
    }
}

impl Default for CodexProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Continuation state carried across passes for one rollout file.
#[derive(Debug, Clone, Default)]
pub(crate) struct RolloutState {
    /// Model of the most recent `turn_context`.
    pub model: Option<String>,
    /// Plan type observed in a `token_count` event (free, premium, ...).
    pub plan_type: Option<String>,
    /// Codex CLI version from the rollout's `session_meta` line.
    pub cli_version: Option<String>,
    /// Highest event timestamp seen so far.
    pub last_ts_ms: Option<i64>,
}

/// Result of parsing a chunk of rollout lines.
#[derive(Debug, Default)]
pub struct RolloutParse {
    pub records: Vec<UsageRecord>,
    /// Lines examined.
    pub scanned: usize,
    /// Lines skipped: malformed, not usage, or outside the backfill window.
    pub skipped: usize,
    /// Continuation state after the chunk.
    pub state: RolloutState,
}

fn timestamp_ms(raw: &Value) -> Option<i64> {
    let text = raw.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn int_field(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|v| v.as_i64())
}

/// Parse rollout lines into normalized records.
///
/// `initial` carries the model/plan state from the previous chunk so a resumed
/// read still attributes usage to the correct model.
pub fn parse_rollout_lines<'a>(
    lines: impl Iterator<Item = &'a str>,
    initial: &RolloutState,
    since_ms: Option<i64>,
    registry: Option<&crate::registry::RegistryState>,
) -> RolloutParse {
    let mut out = RolloutParse {
        state: initial.clone(),
        ..Default::default()
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut malformed = 0usize;

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        out.scanned += 1;

        let Ok(root) = serde_json::from_str::<Value>(line) else {
            malformed += 1;
            out.skipped += 1;
            if malformed > MAX_MALFORMED_LINES {
                warn!("Codex rollout has too many malformed lines; stopping at {}", out.scanned);
                break;
            }
            continue;
        };

        let event_ts = root.get("timestamp").and_then(timestamp_ms);
        let payload = root.get("payload").unwrap_or(&Value::Null);

        match root.get("type").and_then(|t| t.as_str()) {
            Some("turn_context") => {
                // The active model for subsequent turns.
                let model = payload
                    .get("model")
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
                    .or_else(|| {
                        payload
                            .get("collaboration_mode")
                            .and_then(|c| c.get("settings"))
                            .and_then(|s| s.get("model"))
                            .and_then(|m| m.as_str())
                            .map(str::to_string)
                    });
                if let Some(model) = model {
                    out.state.model = Some(model);
                }
                if let Some(ts) = event_ts {
                    out.state.last_ts_ms = Some(out.state.last_ts_ms.unwrap_or(0).max(ts));
                }
            }
            Some("session_meta") => {
                // The rollout header names the tool that wrote it.
                if let Some(version) = payload.get("cli_version").and_then(|v| v.as_str()) {
                    out.state.cli_version = Some(version.to_string());
                }
                if let Some(ts) = event_ts {
                    out.state.last_ts_ms = Some(out.state.last_ts_ms.unwrap_or(0).max(ts));
                }
                out.skipped += 1;
            }
            Some("event_msg") => {
                // `token_count` carries the entitlement plan type.
                let inner = payload.get("type").and_then(|t| t.as_str());
                if inner == Some("token_count") {
                    if let Some(plan) = payload
                        .get("rate_limits")
                        .and_then(|r| r.get("plan_type"))
                        .and_then(|p| p.as_str())
                    {
                        out.state.plan_type = Some(plan.to_string());
                    }
                    if let Some(ts) = event_ts {
                        out.state.last_ts_ms = Some(out.state.last_ts_ms.unwrap_or(0).max(ts));
                    }
                }
                out.skipped += 1;
            }
            Some("token_usage_record") => {
                let usage = payload.get("usage").unwrap_or(&Value::Null);
                let response_id = payload
                    .get("response_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);

                // Without a response id we cannot deduplicate, so we synthesise
                // one from the session and event ordering.
                let event_id = response_id.clone().unwrap_or_else(|| {
                    format!(
                        "{}-{}",
                        payload
                            .get("turn_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("turn"),
                        out.scanned
                    )
                });

                if !seen.insert(event_id.clone()) {
                    out.skipped += 1;
                    continue;
                }

                let input = int_field(usage, "input_tokens");
                let output = int_field(usage, "output_tokens");
                let cached = int_field(usage, "cached_input_tokens");
                let cache_write = int_field(usage, "cache_write_input_tokens");
                let reasoning = int_field(usage, "reasoning_output_tokens");

                let has_metric = input.is_some()
                    || output.is_some()
                    || cached.is_some()
                    || cache_write.is_some()
                    || reasoning.is_some();
                if !has_metric {
                    out.skipped += 1;
                    continue;
                }

                let ts = event_ts.or(out.state.last_ts_ms);
                let Some(ts) = ts else {
                    out.skipped += 1;
                    continue;
                };
                if let Some(cutoff) = since_ms {
                    if ts < cutoff {
                        out.skipped += 1;
                        out.state.last_ts_ms = Some(out.state.last_ts_ms.unwrap_or(0).max(ts));
                        continue;
                    }
                }

                // Codex reports input and output; cached input is already part
                // of the input count, so the billable total is input + output.
                let total = int_field(usage, "total_tokens").unwrap_or_else(|| {
                    input.unwrap_or(0) + output.unwrap_or(0)
                });

                let model = out
                    .state
                    .model
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());

                let payment_mode = if out.state.plan_type.is_some() {
                    PaymentMode::Subscription
                } else {
                    PaymentMode::Unknown
                };

                let value = value_usage(
                    &PricingInput {
                        provider: CodexProvider::ID,
                        model: &model,
                        // The rollout contains no monetary cost at all.
                        provider_cost_usd: None,
                        billing_units: None,
                        billing_unit: BillingUnit::Usd,
                        payment_mode,
                        input_tokens: input,
                        output_tokens: output,
                        cached_tokens: cached,
                        cache_write_tokens: cache_write,
                    },
                    registry,
                );

                let session_id = payload
                    .get("session_id")
                    .or_else(|| payload.get("thread_id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);

                out.records.push(UsageRecord {
                    id: format!("codex_{}", event_id),
                    provider: CodexProvider::ID.to_string(),
                    model,
                    timestamp: rfc3339(ts),
                    input_tokens: input,
                    output_tokens: output,
                    cached_tokens: cached,
                    cache_write_tokens: cache_write,
                    reasoning_tokens: reasoning,
                    total_tokens: total,
                    actual_cost_usd: value.actual_cost_usd,
                    reference_value: value.reference_value,
                    free_value: value.free_value,
                    billing_unit: value.billing_unit.as_str().to_string(),
                    billing_units: value.billing_units,
                    billing_metric_name: out.state.plan_type.clone(),
                    payment_mode: value.payment_mode.as_str().to_string(),
                    pricing_status: value.pricing_status.as_str().to_string(),
                    pricing_source: Some(value.pricing_source.as_str().to_string()),
                    pricing_version: Some(value.pricing_version),
                    reference_model: value.reference_model,
                    reference_input_rate: value.reference_input_rate,
                    reference_output_rate: value.reference_output_rate,
                    reference_cached_rate: value.reference_cached_rate,
                    session_id,
                    event_id: Some(event_id),
                    source_type: SourceType::CodexSessionFile.as_str().to_string(),
                    source_id: None,
                    source_version: out.state.cli_version.clone(),
                });

                out.state.last_ts_ms = Some(out.state.last_ts_ms.unwrap_or(0).max(ts));
            }
            _ => {
                if let Some(ts) = event_ts {
                    out.state.last_ts_ms = Some(out.state.last_ts_ms.unwrap_or(0).max(ts));
                }
                out.skipped += 1;
            }
        }
    }

    out
}

impl CodexProvider {
    pub(crate) const ID: &'static str = crate::usage::provider_id::CODEX;
}

impl UsageProvider for CodexProvider {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn display_name(&self) -> &'static str {
        "Codex"
    }

    fn short_label(&self) -> &'static str {
        "CODEX"
    }

    fn source_type(&self) -> &'static str {
        SourceType::CodexSessionFile.as_str()
    }

    fn payment_mode(&self) -> PaymentMode {
        PaymentMode::Subscription
    }

    fn availability(&self) -> ProviderAvailability {
        if !self.sessions_dir.exists() {
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
                "{} rollout file(s) in {}",
                self.session_files().len(),
                self.sessions_dir.display()
            )),
            ProviderAvailability::NotInstalled => Some(format!(
                "No Codex sessions directory at {}",
                self.sessions_dir.display()
            )),
            ProviderAvailability::UsageUnavailable => Some(format!(
                "{} exists but contains no rollout files yet",
                self.sessions_dir.display()
            )),
            other => Some(other.label().to_string()),
        }
    }

    fn fetch(&self, request: &FetchRequest<'_>) -> Result<FetchOutcome, String> {
        let files = self.session_files();
        if files.is_empty() {
            return Err(format!(
                "No Codex rollout files found in {}",
                self.sessions_dir.display()
            ));
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

            // Skip files that were already fully read and cannot contain
            // anything newer than the backfill window.
            if let Some(cutoff) = request.since_ms {
                if previous_offset == 0 {
                    let modified = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64);
                    if let Some(modified) = modified {
                        if modified < cutoff {
                            // Remember it as consumed so it is not rescanned.
                            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                            next.offsets.insert(key.clone(), len);
                            continue;
                        }
                    }
                }
            }

            let (chunk, new_offset) = match read_appended(&path, previous_offset) {
                Ok(pair) => pair,
                Err(e) => {
                    warn!("Codex: cannot read {}: {}", path.display(), e);
                    continue;
                }
            };

            let model_key = format!("model:{}", key);
            let plan_key = format!("plan:{}", key);
            let version_key = format!("cli:{}", key);
            let initial = RolloutState {
                model: cursor.extra.get(&model_key).cloned(),
                plan_type: cursor.extra.get(&plan_key).cloned(),
                cli_version: cursor.extra.get(&version_key).cloned(),
                last_ts_ms: cursor.last_timestamp_ms,
            };

            let parsed = parse_rollout_lines(
                chunk.lines(),
                &initial,
                request.since_ms,
                request.registry,
            );

            scanned += parsed.scanned;
            skipped += parsed.skipped;

            if let Some(model) = parsed.state.model {
                next.extra.insert(model_key, model);
            }
            if let Some(plan) = parsed.state.plan_type {
                next.extra.insert(plan_key, plan);
            }
            if let Some(version) = parsed.state.cli_version {
                next.extra.insert(version_key, version);
            }
            if let Some(ts) = parsed.state.last_ts_ms {
                next.last_timestamp_ms = Some(next.last_timestamp_ms.unwrap_or(0).max(ts));
            }

            for mut record in parsed.records {
                record.source_id = Some(key.clone());
                if let Some(response_id) = record.event_id.clone() {
                    next.last_event_id = Some(response_id);
                }
                records.push(record);
            }

            next.offsets.insert(key, new_offset);
        }

        records.truncate(request.limit);
        info!(
            "Codex: scanned {} line(s), imported {} record(s)",
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
    use crate::usage::BillingUnit;

    const ROLLOUT: &str = r#"{"timestamp":"2026-09-07T17:09:30.000Z","type":"session_meta","payload":{"session_id":"sess-1","cwd":"/tmp","model_provider":"openai","cli_version":"0.107.0-alpha.5"}}
{"timestamp":"2026-09-07T17:09:31.000Z","type":"turn_context","payload":{"turn_id":"t1","model":"gpt-5.3-codex"}}
{"timestamp":"2026-09-07T17:09:32.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"plan_type":"free"}}}
{"timestamp":"2026-09-07T17:09:49.203Z","type":"token_usage_record","payload":{"thread_id":"thr-1","turn_id":"t1","session_id":"sess-1","response_id":"resp_1","usage":{"input_tokens":26580,"cached_input_tokens":17152,"cache_write_input_tokens":0,"output_tokens":271,"reasoning_output_tokens":103,"total_tokens":26851}}}
{"timestamp":"2026-09-07T17:10:00.000Z","type":"token_usage_record","payload":{"thread_id":"thr-1","turn_id":"t1","session_id":"sess-1","response_id":"resp_2","usage":{"input_tokens":78446,"cached_input_tokens":74496,"output_tokens":209,"reasoning_output_tokens":107,"total_tokens":78655}}}
"#;

    fn parse(text: &str, since: Option<i64>) -> RolloutParse {
        parse_rollout_lines(
            text.lines(),
            &RolloutState::default(),
            since,
            None,
        )
    }

    #[test]
    fn parses_one_record_per_response() {
        let parsed = parse(ROLLOUT, None);
        assert_eq!(parsed.records.len(), 2);
        assert_eq!(parsed.records[0].event_id.as_deref(), Some("resp_1"));
        assert_eq!(parsed.records[0].session_id.as_deref(), Some("sess-1"));
    }

    #[test]
    fn attributes_the_turn_model_and_token_counts() {
        let parsed = parse(ROLLOUT, None);
        let first = &parsed.records[0];
        assert_eq!(first.model, "gpt-5.3-codex");
        assert_eq!(first.input_tokens, Some(26_580));
        assert_eq!(first.output_tokens, Some(271));
        assert_eq!(first.cached_tokens, Some(17_152));
        assert_eq!(first.reasoning_tokens, Some(103));
        // Cached input is a subset of input: the total must not add it again.
        assert_eq!(first.total_tokens, 26_851);
    }

    #[test]
    fn detects_subscription_usage_and_never_invents_a_cost() {
        let parsed = parse(ROLLOUT, None);
        let first = &parsed.records[0];
        assert_eq!(first.payment_mode, "subscription");
        // No monetary cost exists in a Codex rollout.
        assert!(first.actual_cost_usd.is_none());
        assert!(first.free_value.is_none());
        assert_eq!(first.billing_unit, BillingUnit::Usd.as_str());
        assert_eq!(first.source_type, "codex_session_file");
        // The entitlement plan is the metered metric; the rollout header names
        // the tool version that wrote it.
        assert_eq!(first.billing_metric_name.as_deref(), Some("free"));
        assert_eq!(first.source_version.as_deref(), Some("0.107.0-alpha.5"));
    }

    #[test]
    fn missing_cache_metrics_stay_null() {
        let text = r#"{"timestamp":"2026-09-07T17:09:31.000Z","type":"turn_context","payload":{"model":"gpt-5.3-codex"}}
{"timestamp":"2026-09-07T17:09:49.000Z","type":"token_usage_record","payload":{"turn_id":"t1","response_id":"resp_x","usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}}
"#;
        let parsed = parse(text, None);
        assert_eq!(parsed.records.len(), 1);
        let record = &parsed.records[0];
        assert_eq!(record.input_tokens, Some(100));
        assert!(record.cached_tokens.is_none(), "absent cache data must not become 0");
        assert!(record.cache_write_tokens.is_none());
        assert!(record.reasoning_tokens.is_none());
    }

    #[test]
    fn duplicate_events_are_collapsed() {
        let duplicated = format!("{}{}", ROLLOUT, ROLLOUT.lines().last().unwrap());
        let parsed = parse(&duplicated, None);
        assert_eq!(parsed.records.len(), 2, "resp_2 must not be imported twice");
    }

    #[test]
    fn backfill_window_excludes_older_events() {
        let cutoff = DateTime::parse_from_rfc3339("2026-09-07T17:09:50.000Z")
            .unwrap()
            .timestamp_millis();
        let parsed = parse(ROLLOUT, Some(cutoff));
        assert_eq!(parsed.records.len(), 1);
        assert_eq!(parsed.records[0].event_id.as_deref(), Some("resp_2"));
    }

    #[test]
    fn malformed_lines_are_skipped_not_fatal() {
        let text = format!("{}{}\n", ROLLOUT, "{not json at all}");
        let parsed = parse(&text, None);
        // Both usage records still parse. Skipped counts the two non-usage lines
        // (session_meta, token_count) plus the malformed one.
        assert_eq!(parsed.records.len(), 2);
        assert_eq!(parsed.skipped, 3);
    }

    #[test]
    fn missing_directory_is_reported_as_not_installed() {
        let provider = CodexProvider::with_sessions_dir(PathBuf::from("/definitely/missing/codex"));
        assert_eq!(provider.availability(), ProviderAvailability::NotInstalled);
    }

    #[test]
    fn incremental_read_imports_only_new_events() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-codex-inc-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rollout-1.jsonl");

        // First pass: only the first record exists.
        let first_two: String = ROLLOUT.lines().take(4).map(|l| format!("{}\n", l)).collect();
        std::fs::write(&file, &first_two).unwrap();

        let provider = CodexProvider::with_sessions_dir(dir.clone());
        let request = FetchRequest {
            cursor: None,
            since_ms: None,
            registry: None,
            limit: 1_000,
        };
        let first = provider.fetch(&request).unwrap();
        assert_eq!(first.records.len(), 1);

        // A new turn is appended; the second pass must only see the new event.
        let mut appended = first_two.clone();
        appended.push_str(&format!("{}\n", ROLLOUT.lines().nth(4).unwrap()));
        std::fs::write(&file, appended).unwrap();

        let second = provider.fetch(&FetchRequest {
            cursor: Some(&first.cursor),
            since_ms: None,
            registry: None,
            limit: 1_000,
        })
        .unwrap();
        assert_eq!(second.records.len(), 1);
        assert_eq!(second.records[0].event_id.as_deref(), Some("resp_2"));
        assert_eq!(second.scanned, 1, "only the appended line is re-read");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_rollout_files_do_not_abort_the_provider() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-codex-bad-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jsonl"), "not json\n").unwrap();
        std::fs::write(dir.join("b.jsonl"), ROLLOUT).unwrap();

        let provider = CodexProvider::with_sessions_dir(dir.clone());
        let outcome = provider
            .fetch(&FetchRequest {
                cursor: None,
                since_ms: None,
                registry: None,
                limit: 1_000,
            })
            .unwrap();
        assert_eq!(outcome.records.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
