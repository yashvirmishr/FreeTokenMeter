//! Pricing registry — separates actual provider cost from reference API value.
//!
//! FreeTokenMeter distinguishes between:
//!   - What the user actually paid (actual cost, reported by the provider)
//!   - What equivalent usage would cost at a paid/reference rate (reference value)
//!   - The difference (free value = reference - actual)
//!
//! All prices are per million tokens in USD.
//!
//! ## Pricing lookup order
//!
//! 1. **FTM overrides** — explicit FreeTokenMeter promotional/free model mappings
//! 2. **Dynamic registry** — LiteLLM model cost map (local cache, refreshed daily)
//! 3. **Unknown** — pricing is unavailable (not assumed $0)
//!
//! Adding a new model normally requires zero code changes if the LiteLLM//! registry already knows its pricing.

use crate::registry::{DynamicPricingEntry, PricingRates as RegistryRates, RegistryState};
use crate::usage::{BillingUnit, PaymentMode};

// ──────────────────────────────────────────────────────────────────────
// Pricing rates (per million tokens, USD)
// ──────────────────────────────────────────────────────────────────────

/// Pricing rates for a single model (per million tokens, USD).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PricingRates {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cached_per_million: f64,
}

impl PricingRates {
    pub const FREE: PricingRates = PricingRates {
        input_per_million: 0.0,
        output_per_million: 0.0,
        cached_per_million: 0.0,
    };

    /// True when every rate is exactly zero (an explicitly free model).
    pub fn is_free(&self) -> bool {
        self.input_per_million == 0.0
            && self.output_per_million == 0.0
            && self.cached_per_million == 0.0
    }
}

impl From<&RegistryRates> for PricingRates {
    fn from(r: &RegistryRates) -> Self {
        Self {
            input_per_million: r.input_per_million,
            output_per_million: r.output_per_million,
            cached_per_million: r.cached_per_million,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// Pricing status
// ──────────────────────────────────────────────────────────────────────

/// Status of the pricing calculation for a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PricingStatus {
    /// The provider reported a real (non-zero) cost for this usage.
    ActualCostKnown,
    /// The provider reported $0 and reference pricing was applied.
    ReferenceApplied,
    /// No reference value could be established for this record: either the model
    /// is a known free model with no reference configured, or none of the
    /// record's tokens are rateable (cache-write only).
    ReferenceUnavailable,
    /// The model is not in any pricing registry — pricing is unknown.
    /// Unknown is *not* the same as free: no misleading $0 is reported.
    Unknown,
    /// No tokens to price.
    NoTokens,
}

impl PricingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PricingStatus::ActualCostKnown => "actual_cost_known",
            PricingStatus::ReferenceApplied => "reference_applied",
            PricingStatus::ReferenceUnavailable => "reference_unavailable",
            PricingStatus::Unknown => "unknown",
            PricingStatus::NoTokens => "no_tokens",
        }
    }

    /// Inverse of [`PricingStatus::as_str`], kept so the column encoding lives
    /// in one place. Used by the round-trip tests.
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "actual_cost_known" => PricingStatus::ActualCostKnown,
            "reference_applied" => PricingStatus::ReferenceApplied,
            "reference_unavailable" => PricingStatus::ReferenceUnavailable,
            "no_tokens" => PricingStatus::NoTokens,
            _ => PricingStatus::Unknown,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// Pricing source
// ──────────────────────────────────────────────────────────────────────

/// Where the pricing data came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PricingSource {
    /// Explicit FreeTokenMeter override (promotional/free model).
    FtmOverride,
    /// LiteLLM dynamic registry.
    LitellmRegistry,
    /// LiteLLM registry, but served from local cache.
    LitellmCached,
    /// Provider-reported cost only (no reference pricing).
    ProviderReported,
    /// No pricing available.
    Unknown,
}

impl PricingSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            PricingSource::FtmOverride => "ftm_override",
            PricingSource::LitellmRegistry => "litellm_registry",
            PricingSource::LitellmCached => "litellm_cached",
            PricingSource::ProviderReported => "provider_reported",
            PricingSource::Unknown => "unknown",
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// Complete pricing profile
// ──────────────────────────────────────────────────────────────────────

/// Complete pricing profile for a model (FTM override entries).
///
/// Serialize-only: profiles are emitted to the UI, never read back from disk.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelPricingProfile {
    /// Provider this entry applies to (`"opencode"`, `"freebuff"`, or `"api"`
    /// for provider-agnostic paid API rates).
    pub provider: &'static str,

    /// The model identifier as reported by the provider.
    pub model_id: &'static str,

    /// Additional identifiers that resolve to this same profile.
    #[serde(default)]
    pub aliases: &'static [&'static str],

    /// Human-readable name.
    pub display_name: &'static str,

    /// What the user actually pays through the provider.
    /// All zeros means the provider confirmed the model is free.
    pub actual: PricingRates,

    /// Reference/equivalent pricing — what this usage would cost at a paid rate.
    /// `None` means no reference is configured (unknown != free).
    pub reference: Option<PricingRates>,

    /// The model id whose pricing is used as reference (e.g., "mimo-v2.5").
    pub reference_model: Option<&'static str>,

    /// Where the pricing was sourced from.
    pub source: &'static str,

    /// Version of this pricing entry.
    pub version: &'static str,

    /// Date the pricing was last verified (`YYYY-MM` or `YYYY-MM-DD`).
    pub verified_at: &'static str,
}

// ──────────────────────────────────────────────────────────────────────
// FTM overrides — promotional and free models
// ──────────────────────────────────────────────────────────────────────

/// Current pricing registry version.
/// Bump this when the static override list changes.
pub const PRICING_VERSION: &str = "ftm-pricing-v3";

const OPCODE_FREE_SOURCE: &str = "OpenCode Zen free-model listing";
const FREEBUFF_FREE_SOURCE: &str = "Freebuff Desktop free-model listing";
const VERIFIED_OPCODE: &str = "2025-07";
const VERIFIED_FREEBUFF: &str = "2025-07";
const MIMO_REFERENCE_SOURCE: &str = "OpenRouter / Xiaomi MiMo-V2.5 paid API pricing";

/// FreeTokenMeter-specific overrides for promotional/free models.
///
/// Only models that are genuinely free through a specific provider belong here.
/// Paid models are resolved from the dynamic LiteLLM registry instead.
pub const OVERRIDES: &[ModelPricingProfile] = &[
    // ──────────────────────────────────────────────────────────────────
    // OpenCode — free models
    // ──────────────────────────────────────────────────────────────────
    ModelPricingProfile {
        provider: "opencode",
        model_id: "mimo-v2.5-free",
        aliases: &[],
        display_name: "MiMo-V2.5 Free",
        actual: PricingRates::FREE,
        reference: Some(PricingRates {
            input_per_million: 0.14,
            output_per_million: 0.28,
            cached_per_million: 0.0028,
        }),
        reference_model: Some("mimo-v2.5"),
        source: MIMO_REFERENCE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_OPCODE,
    },
    ModelPricingProfile {
        provider: "opencode",
        model_id: "nemotron-3.5-lightning-free",
        aliases: &[],
        display_name: "Nemotron 3.5 Lightning Free",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: OPCODE_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_OPCODE,
    },
    ModelPricingProfile {
        provider: "opencode",
        model_id: "nemotron-3-ultra-free",
        aliases: &[],
        display_name: "Nemotron 3 Ultra Free",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: OPCODE_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_OPCODE,
    },
    ModelPricingProfile {
        provider: "opencode",
        model_id: "big-pickle",
        aliases: &[],
        display_name: "Big Pickle",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: OPCODE_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_OPCODE,
    },
    // ──────────────────────────────────────────────────────────────────
    // Freebuff — free models
    // ──────────────────────────────────────────────────────────────────
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "mimo/mimo-v2.5",
        aliases: &[],
        display_name: "MiMo 2.5",
        actual: PricingRates::FREE,
        reference: Some(PricingRates {
            input_per_million: 0.14,
            output_per_million: 0.28,
            cached_per_million: 0.0028,
        }),
        reference_model: Some("mimo-v2.5"),
        source: MIMO_REFERENCE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "z-ai/glm-5.3-flash",
        aliases: &[],
        display_name: "GLM 5.3 Flash",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "z-ai/glm-5.2",
        aliases: &[],
        display_name: "GLM 5.2",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "deepseek/deepseek-v4-flash",
        aliases: &[],
        display_name: "DeepSeek V4 Flash",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "openai/gpt-5.6-luna",
        aliases: &[],
        display_name: "GPT-5.6 Luna",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "openai/gpt-5.6-luna-es",
        aliases: &[],
        display_name: "GPT-5.6 Luna ES",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "upstage/solar-pro4",
        aliases: &[],
        display_name: "Solar Pro 4",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "minimax/minimax-m3",
        aliases: &[],
        display_name: "MiniMax M3",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "meta/muse-spark-1.2-contributor",
        aliases: &[],
        display_name: "Muse Spark 1.2",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "meta/muse-spark-1.3-contributor",
        aliases: &[],
        display_name: "Muse Spark 1.3",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "stealth/ox-alpha",
        aliases: &[],
        display_name: "Ox Alpha",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "google/gemini-3.8-flash",
        aliases: &[],
        display_name: "Gemini 3.8 Flash",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "crof/kimi-k3-eco",
        aliases: &[],
        display_name: "Kimi K3 Eco",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
    ModelPricingProfile {
        provider: "freebuff",
        model_id: "fable/fable-5",
        aliases: &[],
        display_name: "Fable 5",
        actual: PricingRates::FREE,
        reference: None,
        reference_model: None,
        source: FREEBUFF_FREE_SOURCE,
        version: "2025-07.1",
        verified_at: VERIFIED_FREEBUFF,
    },
];

// ──────────────────────────────────────────────────────────────────────
// Static override lookup
// ──────────────────────────────────────────────────────────────────────

/// Look up a profile in the static override list.
///
/// Resolution order:
///   1. exact (provider, model id/alias) match — provider-specific pricing wins
///   2. provider-agnostic entry (`provider == "api"` or `"any"`) for the model
///   3. any entry with a matching model id (shared free models)
fn resolve_override<'a>(
    registry: &'a [ModelPricingProfile],
    provider: &str,
    model: &str,
) -> Option<&'a ModelPricingProfile> {
    let matches_model = |p: &ModelPricingProfile| {
        p.model_id.eq_ignore_ascii_case(model)
            || p.aliases.iter().any(|a| a.eq_ignore_ascii_case(model))
    };

    // 1. provider-specific
    if let Some(p) = registry
        .iter()
        .find(|p| p.provider.eq_ignore_ascii_case(provider) && matches_model(p))
    {
        return Some(p);
    }

    // 2. provider-agnostic
    if let Some(p) = registry
        .iter()
        .find(|p| (p.provider == "api" || p.provider == "any") && matches_model(p))
    {
        return Some(p);
    }

    // 3. shared model id
    registry.iter().find(|p| matches_model(p))
}

// ──────────────────────────────────────────────────────────────────────
// Resolved pricing profile (unified view)
// ──────────────────────────────────────────────────────────────────────

/// A resolved pricing profile from any source.
#[derive(Debug, Clone)]
pub struct ResolvedProfile {
    pub display_name: String,
    pub actual: PricingRates,
    pub reference: Option<PricingRates>,
    pub reference_model: Option<String>,
    pub source: PricingSource,
    pub version: String,
}

// ──────────────────────────────────────────────────────────────────────
// Layered pricing resolution
// ──────────────────────────────────────────────────────────────────────

/// Look a model up in the dynamic registry: exact key, then a stripped provider
/// prefix, then a known alias.
fn lookup_dynamic(registry: Option<&RegistryState>, model: &str) -> Option<DynamicPricingEntry> {
    let reg_state = registry?;

    // Exact model key.
    if let Some(entry) = reg_state.lookup(&model.to_lowercase()) {
        return Some(entry);
    }

    // A provider-prefixed id (`openrouter/…`) for a model stored bare.
    if let Some(stripped) = crate::model_normalization::strip_provider_prefix(model) {
        if let Some(entry) = reg_state.lookup(&stripped.to_lowercase()) {
            return Some(entry);
        }
    }

    // Known real-world aliases.
    for alias in common_aliases(model) {
        if let Some(entry) = reg_state.lookup(&alias.to_lowercase()) {
            return Some(entry);
        }
    }

    None
}

/// Resolve pricing for a model using the layered lookup system.
///
/// Priority:
/// 1. FTM override — owns what the user actually pays for a free/promotional
///    model. When the override names no reference value of its own, the
///    reference is looked up in the dynamic registry, so a free model is not
///    reported as "no reference pricing configured" while its paid equivalent
///    is known.
/// 2. Dynamic LiteLLM registry (exact, prefix-stripped, then aliased model).
/// 3. Unknown.
fn resolve_pricing(provider: &str, model: &str, registry: Option<&RegistryState>) -> Option<ResolvedProfile> {
    // Layer 1: FTM overrides.
    if let Some(override_profile) = resolve_override(OVERRIDES, provider, model) {
        let mut reference = override_profile.reference.map(|r| PricingRates {
            input_per_million: r.input_per_million,
            output_per_million: r.output_per_million,
            cached_per_million: r.cached_per_million,
        });
        let mut reference_model = override_profile.reference_model.map(|m| m.to_string());

        // An override with no reference still has an equivalent paid value when
        // the registry knows the model — directly, or via the model the override
        // points at.
        if reference.is_none() {
            let lookup: &str = override_profile.reference_model.unwrap_or(model);
            if let Some(entry) = lookup_dynamic(registry, lookup) {
                reference = Some(PricingRates::from(&entry.rates));
                reference_model = Some(entry.model_id.clone());
            }
        }

        return Some(ResolvedProfile {
            display_name: override_profile.display_name.to_string(),
            actual: override_profile.actual,
            reference,
            reference_model,
            source: PricingSource::FtmOverride,
            version: override_profile.version.to_string(),
        });
    }

    // Layer 2: Dynamic registry.
    lookup_dynamic(registry, model).map(|entry| resolved_from_dynamic(&entry, false))
}

/// Build a ResolvedProfile from a dynamic registry entry.
fn resolved_from_dynamic(entry: &DynamicPricingEntry, _from_alias: bool) -> ResolvedProfile {
    ResolvedProfile {
        display_name: entry.display_name.clone(),
        // The dynamic registry supplies reference (list) pricing. It never
        // claims to know what the user actually pays.
        actual: PricingRates::FREE,
        reference: Some(PricingRates {
            input_per_million: entry.rates.input_per_million,
            output_per_million: entry.rates.output_per_million,
            cached_per_million: entry.rates.cached_per_million,
        }),
        reference_model: Some(entry.model_id.clone()),
        source: PricingSource::LitellmRegistry,
        version: "litellm".to_string(),
    }
}

/// Common model aliases that aren't covered by prefix stripping.
///
/// These are real-world aliases seen across providers.
fn common_aliases(model: &str) -> Vec<String> {
    let lower = model.to_lowercase();
    let mut aliases = Vec::new();

    // "gpt-4o" ↔ "gpt-4o-2024-08-06" and similar dated variants
    if lower.starts_with("gpt-4o-") && lower.len() > 7 {
        aliases.push("gpt-4o".to_string());
    }
    if lower.starts_with("gpt-4-turbo-") && lower.len() > 12 {
        aliases.push("gpt-4-turbo".to_string());
    }

    // "claude-3-5-sonnet" ↔ "claude-sonnet-3.5" etc.
    if lower.contains("claude-3-5-sonnet") || lower.contains("claude-sonnet-3-5") {
        aliases.push("claude-3-5-sonnet".to_string());
        aliases.push("claude-sonnet-3.5".to_string());
    }
    if lower.contains("claude-3-5-haiku") || lower.contains("claude-haiku-3-5") {
        aliases.push("claude-3-5-haiku".to_string());
        aliases.push("claude-haiku-3.5".to_string());
    }

    // Gemini dated variants.
    if lower.starts_with("gemini-2.0-flash-") && lower.len() > 17 {
        aliases.push("gemini-2.0-flash".to_string());
    }
    if lower.starts_with("gemini-2.5-pro-") && lower.len() > 15 {
        aliases.push("gemini-2.5-pro".to_string());
    }

    // DeepSeek variants.
    if lower.starts_with("deepseek-r1-") && lower.len() > 12 {
        aliases.push("deepseek-r1".to_string());
    }

    // Vendor-name spelling: OpenCode/Freebuff report Z.ai models as `z-ai/…`,
    // while the registry keys the same model as `zai/…`.
    if let Some(rest) = lower.strip_prefix("z-ai/") {
        aliases.push(format!("zai/{}", rest));
    }

    aliases
}

// ──────────────────────────────────────────────────────────────────────
// Public API
// ──────────────────────────────────────────────────────────────────────

/// Return FTM override profiles for display in the pricing registry view.
pub fn ftm_overrides() -> &'static [ModelPricingProfile] {
    &OVERRIDES
}

/// Result of a pricing calculation for a usage record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageValue {
    /// What the user is actually billed, when it can be established. `None`
    /// means "not established" — subscription usage, or a model whose cost the
    /// provider does not report. It is never replaced with 0.
    pub actual_cost_usd: Option<f64>,

    /// Numeric mirror of `actual_cost_usd` for callers that only need a number
    /// (`None` collapses to 0.0). Never displayed as a price on its own.
    pub actual_cost: f64,

    /// Cost the provider's own tooling computed. For subscription products this
    /// is typically a list-price estimate rather than the user's bill.
    pub provider_reported_cost_usd: Option<f64>,

    /// Billed units in `billing_unit` (e.g. GitHub AI credits).
    pub billing_units: Option<f64>,

    /// The unit this usage is metered in.
    pub billing_unit: BillingUnit,

    /// Payment mode established for this usage.
    pub payment_mode: PaymentMode,

    /// Equivalent cost at reference rates. `None` if no reference exists.
    pub reference_value: Option<f64>,

    /// free_value = reference_value - actual_cost. `None` if no reference exists.
    pub free_value: Option<f64>,

    /// Status of this pricing calculation.
    pub pricing_status: PricingStatus,

    /// Where the pricing came from.
    pub pricing_source: PricingSource,

    /// The pricing profile version used (for historical reproducibility).
    pub pricing_version: String,

    /// The model whose pricing was used as reference.
    pub reference_model: Option<String>,

    /// Reference input rate per million tokens at time of valuation.
    pub reference_input_rate: Option<f64>,

    /// Reference output rate per million tokens at time of valuation.
    pub reference_output_rate: Option<f64>,

    /// Reference cached rate per million tokens at time of valuation.
    pub reference_cached_rate: Option<f64>,
}

/// Cost at `rates` for the token classes that have a rate.
///
/// Only input, output and cached-read tokens are rated. `cache_write_tokens` is
/// deliberately not a parameter: no pricing source reachable from here — neither
/// the FTM overrides nor the dynamic LiteLLM registry — carries a cache-write
/// rate (upstream's `cache_creation_input_token_cost` is not parsed), so there is
/// no rate to multiply it by. Deciding what an unrateable cache-write count means
/// belongs to `value_usage`; this function never guesses a rate.
fn calculate_cost(
    rates: &PricingRates,
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
) -> f64 {
    let input_cost = (input_tokens as f64 / 1_000_000.0) * rates.input_per_million;
    let output_cost = (output_tokens as f64 / 1_000_000.0) * rates.output_per_million;
    let cached_cost = (cached_tokens as f64 / 1_000_000.0) * rates.cached_per_million;
    input_cost + output_cost + cached_cost
}

// ──────────────────────────────────────────────────────────────────────
// Provider-facing valuation (billing units + payment modes)
// ──────────────────────────────────────────────────────────────────────

/// Everything needed to value one usage event.
///
/// A struct rather than positional arguments so providers can express metrics
/// that only some sources expose (billing units, provider-reported cost)
/// without every call site changing.
pub struct PricingInput<'a> {
    /// Canonical provider id.
    pub provider: &'a str,
    /// Model id exactly as the provider reported it.
    pub model: &'a str,
    /// Cost the provider itself reported, if it reports one at all.
    pub provider_cost_usd: Option<f64>,
    /// Billed units reported by the provider (AI credits, plan units, ...).
    pub billing_units: Option<f64>,
    /// The unit those `billing_units` are denominated in.
    pub billing_unit: BillingUnit,
    /// How this usage is paid for.
    pub payment_mode: PaymentMode,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
}

fn priced_tokens(value: Option<i64>) -> i64 {
    value.unwrap_or(0).max(0)
}

/// Value one usage event — the single valuation entry point for providers.
///
/// # Rules
///
/// * A positive provider-reported cost is the actual cost.
/// * A FreeTokenMeter override that confirms the model is free yields `$0.00`
///   (known free) and keeps the reference value separate.
/// * Subscription usage has **no** per-token actual cost, so `actual_cost_usd`
///   stays `None` and only the reference value is shown.
/// * An unpriced model with no reported cost stays `Unknown` — never `$0`.
/// * Cache-write tokens are counted but never rated, because no pricing source
///   exposes a rate for them: a cache-write-only record has **no** reference
///   value rather than a confident `$0.00`.
/// * `free_value` needs both a reference value and a known actual cost, so the
///   UI never claims the user "saved" money under a flat-fee plan.
pub fn value_usage(input: &PricingInput<'_>, registry: Option<&RegistryState>) -> UsageValue {
    let input_tokens = priced_tokens(input.input_tokens);
    let output_tokens = priced_tokens(input.output_tokens);
    let cached_tokens = priced_tokens(input.cached_tokens);
    // Cache-write tokens are real and belong to the record, but no pricing source
    // reachable from here rates them (see `calculate_cost`), so they are carried
    // separately and never mixed in with the rateable counts.
    let cache_write_tokens = priced_tokens(input.cache_write_tokens);

    // The token classes a price exists for: input, output and cached read.
    let rateable_tokens = input_tokens + output_tokens + cached_tokens;
    // A record that carries cache-write tokens and nothing priceable cannot be
    // valued, so it must not be reported as a confident `$0.00`.
    let cache_write_only = cache_write_tokens > 0 && rateable_tokens == 0;

    let provider_cost = input.provider_cost_usd.filter(|cost| *cost > 0.0);

    let any_metric = input.input_tokens.is_some()
        || input.output_tokens.is_some()
        || input.cached_tokens.is_some()
        || input.cache_write_tokens.is_some();

    let base = UsageValue {
        actual_cost_usd: None,
        actual_cost: 0.0,
        provider_reported_cost_usd: input.provider_cost_usd,
        reference_value: None,
        free_value: None,
        billing_units: input.billing_units,
        billing_unit: input.billing_unit,
        payment_mode: input.payment_mode,
        pricing_status: PricingStatus::Unknown,
        pricing_source: PricingSource::Unknown,
        pricing_version: PRICING_VERSION.to_string(),
        reference_model: None,
        reference_input_rate: None,
        reference_output_rate: None,
        reference_cached_rate: None,
    };

    if !any_metric {
        return UsageValue {
            pricing_status: PricingStatus::NoTokens,
            ..base
        };
    }

    let profile = match resolve_pricing(input.provider, input.model, registry) {
        Some(profile) => profile,
        None => {
            // Unpriced model. A provider-reported cost is still worth keeping,
            // but for a subscription it is a list-price estimate rather than a
            // bill, so it never becomes `actual_cost_usd`.
            let status = if provider_cost.is_some() {
                PricingStatus::ActualCostKnown
            } else {
                PricingStatus::Unknown
            };
            // The payment mode is a statement about billing, not about model
            // pricing, so it still applies when the model is unpriced:
            // subscription usage has no per-token spend, promotional usage is
            // genuinely $0.00, and anything else stays unknown.
            let actual = match input.payment_mode {
                PaymentMode::Subscription => None,
                PaymentMode::Promotional => Some(0.0),
                _ => provider_cost,
            };
            return UsageValue {
                actual_cost_usd: actual,
                actual_cost: actual.unwrap_or(0.0),
                pricing_status: status,
                ..base
            };
        }
    };

    // Does this mapping explicitly state that the provider charges nothing?
    let explicitly_free =
        profile.source == PricingSource::FtmOverride && profile.actual.is_free();

    // A cache-write-only record has nothing a rate can be applied to, so its
    // reference value stays `None` instead of collapsing to `$0.00`.
    let reference_value = profile.reference.as_ref().and_then(|rates| {
        if cache_write_only {
            None
        } else {
            Some(calculate_cost(rates, input_tokens, output_tokens, cached_tokens))
        }
    });

    // * subscription → the user pays a flat fee, so there is no per-token cost
    // * promotional → the user's marginal cash cost really is $0.00
    // * anything else → unknown, which is not zero
    let actual_cost_usd = match (provider_cost, input.payment_mode, explicitly_free) {
        (Some(cost), _, _) => Some(cost),
        (None, PaymentMode::Subscription, _) => None,
        (None, PaymentMode::Promotional, _) => Some(0.0),
        (None, _, true) => Some(0.0),
        _ => None,
    };

    // `ReferenceApplied` means the record's value was established from the
    // pricing registry. For subscription usage there is a reference value but
    // no per-token actual cost, which is exactly this case.
    let pricing_status = if provider_cost.is_some() {
        PricingStatus::ActualCostKnown
    } else if reference_value.is_some() {
        PricingStatus::ReferenceApplied
    } else {
        PricingStatus::ReferenceUnavailable
    };

    let free_value = match (reference_value, actual_cost_usd) {
        (Some(reference), Some(actual)) => Some(reference - actual),
        _ => None,
    };

    let (ref_model, ref_in_rate, ref_out_rate, ref_cache_rate) = match &profile.reference {
        Some(rates) => (
            Some(
                profile
                    .reference_model
                    .clone()
                    .unwrap_or_else(|| profile.display_name.clone()),
            ),
            Some(rates.input_per_million),
            Some(rates.output_per_million),
            Some(rates.cached_per_million),
        ),
        None => (None, None, None, None),
    };

    UsageValue {
        actual_cost_usd,
        actual_cost: actual_cost_usd.unwrap_or(0.0),
        reference_value,
        free_value,
        pricing_status,
        pricing_source: profile.source,
        pricing_version: profile.version,
        reference_model: ref_model,
        reference_input_rate: ref_in_rate,
        reference_output_rate: ref_out_rate,
        reference_cached_rate: ref_cache_rate,
        ..base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::PricingRegistry;
    use std::collections::HashMap;

    const OC: &str = "opencode";
    const FB: &str = "freebuff";

    /// Value one event the way a provider adapter does, so every case below
    /// exercises the shipping entry point (`value_usage`) rather than a private
    /// test copy of the resolver.
    fn value(
        provider: &str,
        model: &str,
        provider_cost_usd: Option<f64>,
        payment_mode: PaymentMode,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        cached_tokens: Option<i64>,
    ) -> UsageValue {
        value_usage(
            &PricingInput {
                provider,
                model,
                provider_cost_usd,
                billing_units: None,
                billing_unit: BillingUnit::Usd,
                payment_mode,
                input_tokens,
                output_tokens,
                cached_tokens,
                cache_write_tokens: None,
            },
            None,
        )
    }

    /// An event with every metric unset — each test states only what it is about
    /// through field assignment.
    fn event<'a>(provider: &'a str, model: &'a str) -> PricingInput<'a> {
        PricingInput {
            provider,
            model,
            provider_cost_usd: None,
            billing_units: None,
            billing_unit: BillingUnit::Usd,
            payment_mode: PaymentMode::Unknown,
            input_tokens: None,
            output_tokens: None,
            cached_tokens: None,
            cache_write_tokens: None,
        }
    }

    /// A `RegistryState` holding `(model_id, input, output, cached)` per-million
    /// rates — the dynamic layer paid models actually resolve through.
    fn registry(entries: &[(&str, f64, f64, f64)]) -> RegistryState {
        let state = RegistryState::new(std::path::Path::new("."));
        let mut map = HashMap::new();
        for (model_id, input, output, cached) in entries {
            map.insert(
                model_id.to_lowercase(),
                DynamicPricingEntry {
                    model_id: model_id.to_string(),
                    litellm_provider: "test".to_string(),
                    display_name: format!("Test {}", model_id),
                    rates: RegistryRates {
                        input_per_million: *input,
                        output_per_million: *output,
                        cached_per_million: *cached,
                    },
                    source: "test".to_string(),
                    max_input_tokens: None,
                    max_output_tokens: None,
                },
            );
        }
        *state.registry.lock().unwrap() = Some(PricingRegistry {
            version: "test".to_string(),
            downloaded_at: "test".to_string(),
            source_url: "test".to_string(),
            entries: map,
            valid_count: entries.len(),
            skipped_count: 0,
        });
        state
    }

    // ──────────────────────────────────────────────────────────────────
    // Override registry sanity
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn overrides_have_no_duplicate_provider_model_pairs() {
        for (i, a) in OVERRIDES.iter().enumerate() {
            for b in OVERRIDES.iter().skip(i + 1) {
                let duplicate = a.provider == b.provider
                    && (a.model_id.eq_ignore_ascii_case(b.model_id)
                        || a.aliases.iter().any(|x| x.eq_ignore_ascii_case(b.model_id)));
                assert!(
                    !duplicate,
                    "duplicate override: {}/{}",
                    a.provider, b.model_id
                );
            }
        }
    }

    #[test]
    fn overrides_are_fully_documented() {
        for p in OVERRIDES {
            assert!(!p.provider.is_empty(), "{} has no provider", p.model_id);
            assert!(!p.model_id.is_empty(), "entry has no model id");
            assert!(!p.display_name.is_empty(), "{} has no display name", p.model_id);
            assert!(!p.version.is_empty(), "{} has no version", p.model_id);
            assert!(!p.verified_at.is_empty(), "{} has no verified date", p.model_id);
            if p.reference.is_some() {
                assert!(
                    p.reference_model.is_some(),
                    "{} has reference rates but no reference model",
                    p.model_id
                );
            }
        }
    }

    #[test]
    fn free_models_have_zero_actual_rates() {
        for p in OVERRIDES {
            assert!(
                p.actual.is_free(),
                "{} is listed but has non-zero actual rates",
                p.model_id
            );
        }
    }

    // ──────────────────────────────────────────────────────────────────
    // Known free model WITH a reference mapping
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn known_free_model_actual_cost_is_zero() {
        let v = value(OC, "mimo-v2.5-free", None, PaymentMode::Promotional, Some(1_000_000), Some(500_000), Some(100_000));
        // The provider confirms the model is free, so 0 is a price, not a guess.
        assert_eq!(v.actual_cost_usd, Some(0.0));
        assert_eq!(v.actual_cost, 0.0);
        assert_eq!(v.pricing_status, PricingStatus::ReferenceApplied);
    }

    #[test]
    fn known_free_model_reference_value_calculated() {
        let v = value(OC, "mimo-v2.5-free", None, PaymentMode::Promotional, Some(1_000_000), Some(500_000), Some(100_000));
        let ref_val = v.reference_value.unwrap();
        let expected = 0.14 + 0.14 + 0.00028;
        assert!((ref_val - expected).abs() < 0.0001, "Expected ~{}, got {}", expected, ref_val);
    }

    #[test]
    fn known_free_model_free_value_equals_reference() {
        let v = value(OC, "mimo-v2.5-free", None, PaymentMode::Promotional, Some(1_000_000), Some(500_000), Some(0));
        let free_val = v.free_value.unwrap();
        let ref_val = v.reference_value.unwrap();
        assert!((free_val - ref_val).abs() < 0.0001);
    }

    #[test]
    fn known_reference_mapping_is_reported() {
        let profile = resolve_override(OVERRIDES, FB, "mimo/mimo-v2.5").expect("registry entry");
        assert_eq!(profile.reference_model, Some("mimo-v2.5"));
        assert_eq!(profile.provider, "freebuff");
        assert!(profile.source.contains("MiMo-V2.5"));

        let rates = profile.reference.expect("reference rates");
        assert_eq!(rates.input_per_million, 0.14);
        assert_eq!(rates.output_per_million, 0.28);
        assert_eq!(rates.cached_per_million, 0.0028);
    }

    // ──────────────────────────────────────────────────────────────────
    // Known free model WITHOUT a reference mapping
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn known_free_model_without_reference_is_not_unpriced() {
        let v = value(OC, "nemotron-3-ultra-free", None, PaymentMode::Promotional, Some(1_000_000), Some(500_000), Some(0));
        assert_eq!(v.actual_cost, 0.0);
        assert_eq!(v.reference_value, None);
        assert_eq!(v.free_value, None);
        assert_eq!(v.pricing_status, PricingStatus::ReferenceUnavailable);
    }

    #[test]
    fn missing_reference_pricing_never_fabricates_a_value() {
        for model in ["big-pickle", "fable/fable-5", "stealth/ox-alpha"] {
            let v = value(FB, model, None, PaymentMode::Promotional, Some(1_000_000), Some(1_000_000), Some(0));
            assert_eq!(v.reference_value, None, "{} fabricated a reference value", model);
            assert_eq!(v.free_value, None, "{} fabricated a free value", model);
        }
    }

    // ──────────────────────────────────────────────────────────────────
    // Unknown models
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn unknown_model_is_reported_as_unknown_not_free() {
        let v = value(OC, "new-model-x", None, PaymentMode::Unknown, Some(1_000_000), Some(1_000_000), Some(0));
        assert_eq!(v.actual_cost, 0.0);
        assert_eq!(v.reference_value, None);
        assert_eq!(v.pricing_status, PricingStatus::Unknown);
        assert_eq!(v.pricing_status.as_str(), "unknown");
    }

    #[test]
    fn unknown_model_with_provider_cost_keeps_actual_cost() {
        let v = value(OC, "new-model-x", Some(5.0), PaymentMode::Unknown, Some(1_000_000), Some(1_000_000), Some(0));
        // The persisted field, not only its numeric mirror.
        assert_eq!(v.actual_cost_usd, Some(5.0));
        assert_eq!(v.actual_cost, 5.0);
        assert_eq!(v.reference_value, None);
        assert_eq!(v.pricing_status, PricingStatus::ActualCostKnown);
    }

    #[test]
    fn pricing_status_round_trips_through_strings() {
        for status in [
            PricingStatus::ActualCostKnown,
            PricingStatus::ReferenceApplied,
            PricingStatus::ReferenceUnavailable,
            PricingStatus::Unknown,
            PricingStatus::NoTokens,
        ] {
            assert_eq!(PricingStatus::from_str(status.as_str()), status);
        }
    }

    // ──────────────────────────────────────────────────────────────────
    // Provider-specific pricing
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn provider_specific_entry_wins_over_shared_entry() {
        static TEST_REGISTRY: &[ModelPricingProfile] = &[
            ModelPricingProfile {
                provider: "api",
                model_id: "shared-model",
                aliases: &[],
                display_name: "Shared",
                actual: PricingRates {
                    input_per_million: 1.0,
                    output_per_million: 1.0,
                    cached_per_million: 1.0,
                },
                reference: None,
                reference_model: None,
                source: "test",
                version: "test",
                verified_at: "test",
            },
            ModelPricingProfile {
                provider: "opencode",
                model_id: "shared-model",
                aliases: &[],
                display_name: "Shared (OpenCode)",
                actual: PricingRates::FREE,
                reference: None,
                reference_model: None,
                source: "test",
                version: "test",
                verified_at: "test",
            },
        ];

        let opencode = resolve_override(TEST_REGISTRY, "opencode", "shared-model").unwrap();
        assert_eq!(opencode.provider, "opencode");
        assert!(opencode.actual.is_free());

        let freebuff = resolve_override(TEST_REGISTRY, "freebuff", "shared-model").unwrap();
        assert_eq!(freebuff.provider, "api");
        assert!(!freebuff.actual.is_free());
    }

    #[test]
    fn provider_specific_pricing_applies_to_real_override_models() {
        let oc_profile = resolve_override(OVERRIDES, OC, "mimo-v2.5-free").expect("opencode entry");
        assert_eq!(oc_profile.provider, "opencode");

        let fb_profile = resolve_override(OVERRIDES, FB, "mimo/mimo-v2.5").expect("freebuff entry");
        assert_eq!(fb_profile.provider, "freebuff");
    }

    // ──────────────────────────────────────────────────────────────────
    // Edge cases
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn zero_tokens_returns_no_tokens() {
        let v = value("api", "gpt-4o", None, PaymentMode::Unknown, None, None, None);
        assert_eq!(v.pricing_status, PricingStatus::NoTokens);
        assert_eq!(v.actual_cost, 0.0);
        // A source that *reports* zero tokens still produced a record, so only
        // "no metric reported at all" is NoTokens (unknown != zero).
        let reported_zero =
            value("api", "gpt-4o", None, PaymentMode::Unknown, Some(0), Some(0), Some(0));
        assert_ne!(reported_zero.pricing_status, PricingStatus::NoTokens);
    }

    #[test]
    fn cached_tokens_use_separate_rate() {
        let v = value(OC, "mimo-v2.5-free", None, PaymentMode::Promotional, Some(0), Some(0), Some(1_000_000));
        let ref_val = v.reference_value.unwrap();
        assert!((ref_val - 0.0028).abs() < 0.0001, "Expected ~0.0028, got {}", ref_val);
    }

    // ──────────────────────────────────────────────────────────────────
    // Versioning
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn pricing_version_is_set() {
        let v = value("api", "gpt-4o", Some(1.0), PaymentMode::Unknown, Some(100), Some(100), Some(0));
        assert!(!v.pricing_version.is_empty());
    }

    #[test]
    fn historical_records_keep_the_version_they_were_priced_with() {
        assert_ne!(PricingStatus::from_str("unknown"), PricingStatus::NoTokens);
    }

    #[test]
    fn display_names_come_from_the_overrides() {
        let profile = resolve_pricing(OC, "mimo-v2.5-free", None).unwrap();
        assert_eq!(profile.display_name, "MiMo-V2.5 Free");
        assert!(resolve_pricing(OC, "unknown-model", None).is_none());
    }

    // ──────────────────────────────────────────────────────────────────
    // Pricing source tracking
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn ftm_override_source_is_tracked() {
        let v = value(OC, "mimo-v2.5-free", None, PaymentMode::Promotional, Some(1_000_000), Some(500_000), Some(0));
        assert_eq!(v.pricing_source, PricingSource::FtmOverride);
        // The profile version travels with the value, for historical repro.
        assert_eq!(v.pricing_version, "2025-07.1");
    }

    #[test]
    fn unknown_model_source_is_unknown() {
        let v = value(OC, "nonexistent", None, PaymentMode::Unknown, Some(1_000_000), Some(500_000), Some(0));
        assert_eq!(v.pricing_source, PricingSource::Unknown);
    }

    // ──────────────────────────────────────────────────────────────────
    // Payment modes and billing units
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn subscription_usage_has_no_per_token_actual_cost() {
        // The reference value is known, but a flat fee buys no per-token spend,
        // so there is nothing to compare against and no knowable saving.
        let v = value(OC, "mimo-v2.5-free", None, PaymentMode::Subscription, Some(1_000_000), Some(500_000), Some(0));
        assert_eq!(v.payment_mode, PaymentMode::Subscription);
        assert_eq!(v.actual_cost_usd, None);
        assert_eq!(v.actual_cost, 0.0);
        assert!(v.reference_value.is_some());
        assert_eq!(v.free_value, None, "free_value needs a known actual cost too");
        assert_eq!(v.pricing_status, PricingStatus::ReferenceApplied);
    }

    #[test]
    fn promotional_usage_is_a_known_zero_and_keeps_its_reference_value() {
        let v = value(OC, "mimo-v2.5-free", None, PaymentMode::Promotional, Some(1_000_000), Some(500_000), Some(0));
        assert_eq!(v.payment_mode, PaymentMode::Promotional);
        assert_eq!(v.actual_cost_usd, Some(0.0));
        // Known values, not two `None`s that happen to compare equal.
        let reference = v.reference_value.expect("reference value is known");
        assert!((reference - 0.28).abs() < 1e-9, "got {}", reference);
        assert_eq!(v.free_value, Some(reference));
    }

    #[test]
    fn billing_units_pass_through_without_becoming_dollars() {
        // GitHub Copilot-style credit metering: the unit is recorded, but it is
        // never translated into money the user did not spend.
        let v = value_usage(
            &PricingInput {
                provider: "github-copilot",
                model: "copilot-model-x",
                provider_cost_usd: None,
                billing_units: Some(14.0),
                billing_unit: BillingUnit::AiCredits,
                payment_mode: PaymentMode::Subscription,
                input_tokens: Some(900_000),
                output_tokens: Some(10_000),
                cached_tokens: None,
                cache_write_tokens: None,
            },
            None,
        );
        assert_eq!(v.billing_units, Some(14.0));
        assert_eq!(v.billing_unit, BillingUnit::AiCredits);
        assert_eq!(v.actual_cost_usd, None, "credits are not dollars");
        assert_eq!(v.pricing_status, PricingStatus::Unknown);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Cache-write tokens
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn cache_write_tokens_are_counted_but_never_rated() {
        // Nothing reachable from the shipping path rates a cache write, so the
        // tokens are left unvalued rather than multiplied by a guess.
        let mut mixed = event(OC, "mimo-v2.5-free");
        mixed.payment_mode = PaymentMode::Promotional;
        mixed.input_tokens = Some(1_000_000);
        mixed.output_tokens = Some(500_000);
        mixed.cache_write_tokens = Some(2_000_000);
        let v = value_usage(&mixed, None);

        // The reference value covers only the classes that have a rate: 1M input
        // + 0.5M output at the MiMo reference rates, cache writes adding nothing.
        let reference = v.reference_value.expect("reference value is known");
        assert!((reference - 0.28).abs() < 1e-9, "got {}", reference);

        // A record whose tokens are all cache writes has nothing priceable, so it
        // is not `NoTokens` and it is not a confident `$0.00` either.
        let mut only_writes = event(OC, "mimo-v2.5-free");
        only_writes.payment_mode = PaymentMode::Promotional;
        only_writes.cache_write_tokens = Some(2_000_000);
        let v = value_usage(&only_writes, None);

        assert_ne!(
            v.pricing_status,
            PricingStatus::NoTokens,
            "the record does carry tokens"
        );
        assert_eq!(
            v.reference_value, None,
            "unrateable tokens must not become $0.00"
        );
        assert_eq!(v.free_value, None);
        assert_eq!(v.pricing_status, PricingStatus::ReferenceUnavailable);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Dynamic registry — the layer paid models resolve through
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn registry_exact_match_values_a_paid_model_from_reference_rates() {
        let reg = registry(&[("paid-model-x", 3.0, 15.0, 0.3)]);
        let mut e = event("api", "paid-model-x");
        e.payment_mode = PaymentMode::Api;
        e.input_tokens = Some(1_000_000);
        e.output_tokens = Some(100_000);
        e.cached_tokens = Some(1_000_000);
        let v = value_usage(&e, Some(&reg));

        assert_eq!(v.pricing_source, PricingSource::LitellmRegistry);
        // The dynamic registry carries reference (list) pricing only, so it never
        // claims to know what the user actually paid.
        assert_eq!(v.actual_cost_usd, None);
        assert_eq!(v.reference_model.as_deref(), Some("paid-model-x"));
        assert_eq!(v.pricing_status, PricingStatus::ReferenceApplied);
        let expected = 3.0 + 1.5 + 0.3;
        assert!((v.reference_value.unwrap() - expected).abs() < 1e-9);
    }

    #[test]
    fn registry_resolves_a_provider_prefixed_model() {
        let reg = registry(&[("deepseek-r1", 1.0, 2.0, 0.1)]);
        let mut e = event("api", "deepseek/deepseek-r1");
        e.payment_mode = PaymentMode::Api;
        e.input_tokens = Some(1_000_000);
        e.output_tokens = Some(100_000);
        let v = value_usage(&e, Some(&reg));

        assert_eq!(v.pricing_source, PricingSource::LitellmRegistry);
        assert_eq!(v.reference_model.as_deref(), Some("deepseek-r1"));
        let expected = 1.0 + 0.2;
        assert!((v.reference_value.unwrap() - expected).abs() < 1e-9);
    }

    #[test]
    fn registry_resolves_a_dated_alias() {
        let reg = registry(&[("gpt-4o", 2.5, 10.0, 1.25)]);
        let mut e = event("api", "gpt-4o-2024-08-06");
        e.payment_mode = PaymentMode::Api;
        e.input_tokens = Some(1_000_000);
        let v = value_usage(&e, Some(&reg));

        assert_eq!(v.pricing_source, PricingSource::LitellmRegistry);
        assert_eq!(v.reference_model.as_deref(), Some("gpt-4o"));
        assert!((v.reference_value.unwrap() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn registry_without_the_model_falls_back_to_unknown() {
        let reg = registry(&[("some-other-model", 1.0, 1.0, 0.1)]);
        let mut e = event("api", "not-in-the-registry");
        e.payment_mode = PaymentMode::Api;
        e.input_tokens = Some(1_000_000);
        let v = value_usage(&e, Some(&reg));

        // A registry that does not know the model is not a free pass: no price is
        // invented and the record stays unpriced.
        assert_eq!(v.pricing_source, PricingSource::Unknown);
        assert_eq!(v.reference_value, None);
        assert_eq!(v.actual_cost_usd, None);
        assert_eq!(v.pricing_status, PricingStatus::Unknown);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Free override + dynamic registry reference
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn override_without_a_reference_borrows_the_registry_reference() {
        // Freebuff serves deepseek/deepseek-v4-flash for free; the registry knows
        // the paid rate, so the reference value must not read "unavailable".
        let reg = registry(&[("deepseek/deepseek-v4-flash", 0.2, 0.8, 0.02)]);
        let mut e = event(FB, "deepseek/deepseek-v4-flash");
        e.payment_mode = PaymentMode::Promotional;
        e.input_tokens = Some(1_000_000);
        e.output_tokens = Some(500_000);
        let v = value_usage(&e, Some(&reg));

        assert_eq!(v.pricing_source, PricingSource::FtmOverride);
        assert_eq!(v.actual_cost_usd, Some(0.0), "still free through the provider");
        assert_eq!(v.reference_model.as_deref(), Some("deepseek/deepseek-v4-flash"));
        assert_eq!(v.pricing_status, PricingStatus::ReferenceApplied);
        // 1M input @0.2 + 0.5M output @0.8
        assert!((v.reference_value.unwrap() - 0.6).abs() < 1e-9);
    }

    #[test]
    fn override_without_a_reference_stays_unavailable_when_the_registry_is_silent() {
        let reg = registry(&[("some-other-model", 1.0, 1.0, 0.1)]);
        let mut e = event(FB, "big-pickle");
        e.payment_mode = PaymentMode::Promotional;
        e.input_tokens = Some(1_000_000);
        let v = value_usage(&e, Some(&reg));

        // Nothing to borrow: N/A stays N/A rather than becoming a guess.
        assert_eq!(v.reference_value, None);
        assert_eq!(v.free_value, None);
        assert_eq!(v.actual_cost_usd, Some(0.0));
        assert_eq!(v.pricing_status, PricingStatus::ReferenceUnavailable);
    }

    #[test]
    fn an_explicit_override_reference_beats_the_registry() {
        // The override names its own reference rates; a registry entry for the
        // same model must not replace them.
        let reg = registry(&[("mimo/mimo-v2.5", 9.0, 9.0, 9.0)]);
        let mut e = event(FB, "mimo/mimo-v2.5");
        e.payment_mode = PaymentMode::Promotional;
        e.input_tokens = Some(1_000_000);
        let v = value_usage(&e, Some(&reg));

        assert_eq!(v.reference_model.as_deref(), Some("mimo-v2.5"));
        assert!((v.reference_value.unwrap() - 0.14).abs() < 1e-9);
    }

    #[test]
    fn z_ai_vendor_spelling_resolves_through_the_alias() {
        // Freebuff reports `z-ai/glm-5.3-flash`; the registry keys it `zai/…`.
        let reg = registry(&[("zai/glm-5.3-flash", 0.5, 1.5, 0.05)]);
        let mut e = event(FB, "z-ai/glm-5.3-flash");
        e.payment_mode = PaymentMode::Promotional;
        e.input_tokens = Some(1_000_000);
        let v = value_usage(&e, Some(&reg));

        assert_eq!(v.reference_model.as_deref(), Some("zai/glm-5.3-flash"));
        assert_eq!(v.pricing_status, PricingStatus::ReferenceApplied);
        assert!((v.reference_value.unwrap() - 0.5).abs() < 1e-9);
    }
}
