//! Normalized usage vocabulary shared by every provider.
//!
//! The whole application works on the types in this module. A provider adapter's
//! only job is to turn a provider-specific structured source into
//! [`UsageRecord`]s; nothing downstream knows where the data came from.
//!
//! # Two rules that shape this file
//!
//! 1. **Unknown is not zero.** Every metric that a provider may omit is an
//!    `Option`. `reasoning_tokens: None` means "the provider does not report
//!    reasoning tokens"; `Some(0)` means "the provider reports zero".
//! 2. **Billing units are not dollars.** Subscription products report usage
//!    units (AI credits, plan allowance), which are modelled separately from
//!    USD so the UI never claims money was spent when it was not.

use serde::{Deserialize, Serialize};

// ──────────────────────────────────────────────────────────────────────
// Provider identity
// ──────────────────────────────────────────────────────────────────────

/// Canonical provider IDs. These are the database identifiers — display names
/// are never used as keys.
pub mod provider_id {
    pub const OPENCODE: &str = "opencode";
    pub const FREEBUFF: &str = "freebuff";
    pub const CLAUDE_CODE: &str = "claude-code";
    pub const CODEX: &str = "codex";
    pub const GEMINI_CLI: &str = "gemini-cli";
    pub const GITHUB_COPILOT: &str = "github-copilot";
}

/// How a provider is paid for. Never inferred from the provider's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentMode {
    /// The user is billed per token for this usage.
    Api,
    /// The usage is covered by a subscription entitlement; per-token spend may
    /// not exist at all.
    Subscription,
    /// Free or promotional access with no per-token charge.
    Promotional,
    /// Not established from provider metadata.
    Unknown,
}

impl PaymentMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            PaymentMode::Api => "api",
            PaymentMode::Subscription => "subscription",
            PaymentMode::Promotional => "promotional",
            PaymentMode::Unknown => "unknown",
        }
    }

    /// Inverse of [`PaymentMode::as_str`], kept so the column encoding lives in
    /// one place. Used by the round-trip tests and by readers of the database.
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "api" => PaymentMode::Api,
            "subscription" => PaymentMode::Subscription,
            "promotional" => PaymentMode::Promotional,
            _ => PaymentMode::Unknown,
        }
    }
}

/// The unit a provider bills in. GitHub Copilot, for example, meters usage in
/// AI credits rather than dollars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BillingUnit {
    Usd,
    AiCredits,
    SubscriptionUnits,
    Unknown,
}

impl BillingUnit {
    pub fn as_str(&self) -> &'static str {
        match self {
            BillingUnit::Usd => "usd",
            BillingUnit::AiCredits => "ai_credits",
            BillingUnit::SubscriptionUnits => "subscription_units",
            BillingUnit::Unknown => "unknown",
        }
    }

    /// Inverse of [`BillingUnit::as_str`], kept so the column encoding lives in
    /// one place. Used by the round-trip tests and by readers of the database.
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "usd" => BillingUnit::Usd,
            "ai_credits" => BillingUnit::AiCredits,
            "subscription_units" => BillingUnit::SubscriptionUnits,
            _ => BillingUnit::Unknown,
        }
    }

    /// Canonical name of the metric this unit meters, used when the source
    /// itself names no metric. `None` for plain USD, which needs no name.
    pub fn metric_name(&self) -> Option<&'static str> {
        match self {
            BillingUnit::Usd => None,
            BillingUnit::AiCredits => Some("AI credits"),
            BillingUnit::SubscriptionUnits => Some("subscription units"),
            BillingUnit::Unknown => None,
        }
    }

    /// Short suffix for compact table cells, e.g. `14 cr`.
    #[allow(dead_code)]
    pub fn suffix(&self) -> &'static str {
        match self {
            BillingUnit::Usd => "usd",
            BillingUnit::AiCredits => "cr",
            BillingUnit::SubscriptionUnits => "units",
            BillingUnit::Unknown => "",
        }
    }
}

/// Which structured source a record came from. Preserved for debugging and for
/// provider-specific deduplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    OpenCodeDatabase,
    FreebuffDatabase,
    ClaudeSessionFile,
    CodexSessionFile,
    GeminiSessionFile,
    CopilotUsageSource,
}

impl SourceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceType::OpenCodeDatabase => "opencode_database",
            SourceType::FreebuffDatabase => "freebuff_database",
            SourceType::ClaudeSessionFile => "claude_session_file",
            SourceType::CodexSessionFile => "codex_session_file",
            SourceType::GeminiSessionFile => "gemini_session_file",
            SourceType::CopilotUsageSource => "copilot_usage_source",
        }
    }
}

/// Distinct availability states. `Connected` is the only state in which a
/// provider is allowed to contribute usage records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderAvailability {
    /// Installed and a readable structured usage source was found.
    Connected,
    /// No installation detected at all.
    NotInstalled,
    /// Installed, but no supported machine-readable usage source exists.
    UsageUnavailable,
    /// Detected and supported, but switched off by the user.
    Disabled,
    /// No reliable machine-readable source exists for this product.
    Unsupported,
}

impl ProviderAvailability {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderAvailability::Connected => "connected",
            ProviderAvailability::NotInstalled => "not_installed",
            ProviderAvailability::UsageUnavailable => "usage_unavailable",
            ProviderAvailability::Disabled => "disabled",
            ProviderAvailability::Unsupported => "unsupported",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ProviderAvailability::Connected => "CONNECTED",
            ProviderAvailability::NotInstalled => "NOT INSTALLED",
            ProviderAvailability::UsageUnavailable => "USAGE UNAVAILABLE",
            ProviderAvailability::Disabled => "DISABLED",
            ProviderAvailability::Unsupported => "UNSUPPORTED",
        }
    }

    /// `true` only for `Connected`: the state that yields records.
    pub fn is_connected(&self) -> bool {
        matches!(self, ProviderAvailability::Connected)
    }
}

// ──────────────────────────────────────────────────────────────────────
// Normalized usage record
// ──────────────────────────────────────────────────────────────────────

/// One normalized usage event.
///
/// Providers fill in what their source actually exposes; everything else stays
/// `None` so the UI can distinguish "not reported" from "zero".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// Stable, provider-scoped primary key. The database uniqueness rule.
    pub id: String,

    /// Canonical provider id (`opencode`, `codex`, ...).
    pub provider: String,

    /// Model identifier exactly as the provider reported it.
    pub model: String,

    /// RFC 3339 timestamp of the event.
    pub timestamp: String,

    // ── token metrics (None = not reported by this source) ──
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,

    /// Always present: a record with no billable tokens is not a record.
    pub total_tokens: i64,

    // ── value ──
    /// What the provider actually charged in USD. `None` when the provider
    /// reports no monetary cost (subscription/entitlement usage).
    pub actual_cost_usd: Option<f64>,

    /// Equivalent cost at reference (list) rates. `None` when no reference
    /// pricing exists — never fabricated.
    pub reference_value: Option<f64>,

    /// `reference_value - actual_cost`, i.e. the value delivered above what was
    /// paid. `None` when either side is unknown.
    pub free_value: Option<f64>,

    /// The unit the provider meters this usage in.
    pub billing_unit: String,

    /// Billed units in `billing_unit` (e.g. 14.0 AI credits).
    pub billing_units: Option<f64>,

    /// Human name of the billing metric, as the provider reports it — the
    /// entitlement or metered unit this usage was counted against (a Codex plan
    /// type, a Claude service tier, `AI credits`). `None` when the source names
    /// no metric, or when the unit is plain USD.
    pub billing_metric_name: Option<String>,

    /// How this provider's usage is paid for.
    pub payment_mode: String,

    /// Pricing resolution status (`actual_cost_known`, `reference_applied`,
    /// `reference_unavailable`, `unknown`, `no_tokens`).
    pub pricing_status: String,

    /// Where the pricing came from (`ftm_override`, `litellm_registry`, ...).
    pub pricing_source: Option<String>,

    /// Version of the pricing registry used, for historical reproducibility.
    pub pricing_version: Option<String>,

    /// The model whose rates were used as the reference.
    pub reference_model: Option<String>,

    /// Reference rates per million tokens at valuation time.
    pub reference_input_rate: Option<f64>,
    pub reference_output_rate: Option<f64>,
    pub reference_cached_rate: Option<f64>,

    // ── provenance ──
    /// Provider session/thread identifier.
    pub session_id: Option<String>,

    /// Provider-native event identifier (request/response/turn id). Used for
    /// provider-specific deduplication.
    pub event_id: Option<String>,

    /// Which structured source produced this record.
    pub source_type: String,

    /// Concrete source locator (file path, database path) where available.
    pub source_id: Option<String>,

    /// Version of the tool that produced the source data (a CLI version, an app
    /// version). Kept for debugging and for interpreting source differences.
    /// `None` when the source does not record one.
    pub source_version: Option<String>,
}

// ──────────────────────────────────────────────────────────────────────
// Aggregates sent to the UI
// ──────────────────────────────────────────────────────────────────────

/// Aggregated totals over the selected window/filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSummary {
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cached_tokens: i64,
    pub request_count: i64,

    /// Only records with a known monetary cost contribute here.
    pub actual_cost: f64,
    /// Records that reported no monetary cost at all (subscription/entitlement).
    pub unbilled_records: i64,
    pub reference_value: f64,
    pub free_value: f64,

    /// Total billed units across providers whose unit is not USD.
    pub billing_units: f64,

    /// Records whose model has no pricing entry ("PRICING: UNKNOWN") — excluded
    /// from the cost totals rather than counted as $0.
    pub unpriced_records: i64,
    pub unpriced_tokens: i64,
}

/// Per-provider aggregate for the dashboard's provider table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider: String,
    pub tokens: i64,
    pub percentage: f64,
    pub actual_cost: Option<f64>,
    pub reference_value: Option<f64>,
    pub billing_units: Option<f64>,
    pub billing_unit: String,
    pub billing_metric_name: Option<String>,
    pub records: i64,
}

/// Per-model aggregate for the model table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsage {
    pub provider: String,
    pub model: String,
    pub tokens: i64,
    pub percentage: f64,
    pub actual_cost: Option<f64>,
    pub reference_value: Option<f64>,
    pub billing_units: Option<f64>,
    pub billing_unit: String,
    pub billing_metric_name: Option<String>,
    pub pricing_status: String,
    pub pricing_source: Option<String>,
}

/// One row of the activity feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub timestamp: String,
    pub provider: String,
    pub model: String,
    pub tokens: i64,
    pub billing_units: Option<f64>,
    pub billing_unit: String,
}

/// Everything the dashboard needs in one response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardData {
    pub summary: UsageSummary,
    pub provider_usage: Vec<ProviderUsage>,
    pub model_usage: Vec<ModelUsage>,
    pub activity: Vec<ActivityEntry>,
    pub providers: Vec<ProviderStatus>,
}

/// Detection + health state for one provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatus {
    /// Canonical provider id.
    pub id: String,
    /// Display name ("Claude Code").
    pub name: String,
    /// Short label for narrow columns ("CLAUDE").
    pub short_label: String,
    /// One of the [`ProviderAvailability`] strings.
    pub state: String,
    /// Convenience flag for the UI: `state == "connected"`.
    pub connected: bool,
    /// Terminal-style state label ("CONNECTED", "USAGE UNAVAILABLE", ...).
    pub status: String,
    /// Human explanation, especially for unavailable/error states.
    pub detail: Option<String>,
    /// Whether the user has this provider enabled for synchronization.
    pub enabled: bool,
    /// Structured source kind, e.g. `codex_session_file`.
    pub source_type: String,
    /// Payment mode reported by the provider metadata.
    pub payment_mode: String,
    /// Billing unit for this provider's records.
    pub billing_unit: String,
    /// Name of the billing metric for this provider's records, if any.
    pub billing_metric_name: Option<String>,
    /// Version of the tool that produced this provider's records, if known.
    pub source_version: Option<String>,
    /// Milliseconds since the last successful sync, if any.
    pub last_sync_age_secs: Option<i64>,
    /// Pre-formatted "3s ago" string.
    pub last_sync: Option<String>,
    /// Records stored locally for this provider.
    pub records: i64,
    /// Tokens stored locally for this provider.
    pub tokens: i64,
}
