/// Pricing engine — separates actual provider cost from reference API value.
///
/// FreeTokenMeter distinguishes between:
///   - What the user actually paid (actual cost from OpenCode)
///   - What equivalent usage would cost at a paid/reference rate (reference value)
///   - The difference (free value = reference - actual)
///
/// All prices are per million tokens in USD.
/// Reference prices are sourced independently from OpenCode's promotional pricing.

/// Status of pricing calculation for a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PricingStatus {
    /// Actual cost was provided by OpenCode (non-zero).
    ActualCostKnown,
    /// OpenCode reported $0, reference pricing applied.
    ReferenceApplied,
    /// OpenCode reported $0, no reference pricing available.
    ReferenceUnavailable,
    /// No tokens to price.
    NoTokens,
}

impl PricingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PricingStatus::ActualCostKnown => "actual_cost_known",
            PricingStatus::ReferenceApplied => "reference_applied",
            PricingStatus::ReferenceUnavailable => "reference_unavailable",
            PricingStatus::NoTokens => "no_tokens",
        }
    }

    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "actual_cost_known" => PricingStatus::ActualCostKnown,
            "reference_applied" => PricingStatus::ReferenceApplied,
            "reference_unavailable" => PricingStatus::ReferenceUnavailable,
            "no_tokens" => PricingStatus::NoTokens,
            _ => PricingStatus::ReferenceUnavailable,
        }
    }
}

/// Pricing rates for a single model (per million tokens, USD).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct PricingRates {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cached_per_million: f64,
}

/// Complete pricing profile for a model.
///
/// Contains both actual pricing (what OpenCode charges) and optional reference
/// pricing (what the equivalent usage would cost at a paid API rate).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelPricingProfile {
    /// Display name for the model.
    pub display_name: &'static str,

    /// What the user actually pays (via OpenCode).
    /// For free models: all zeros.
    /// For paid models: the real API rate.
    pub actual: PricingRates,

    /// Reference/equivalent pricing — what this usage would cost at a paid rate.
    /// `None` means no reference is configured (unknown != free).
    pub reference: Option<PricingRates>,

    /// The model ID whose pricing is used as reference (e.g., "mimo-v2.5").
    /// `None` when no reference mapping exists.
    pub reference_model: Option<&'static str>,

    /// Where the reference pricing was sourced from.
    pub reference_source: &'static str,

    /// Date the reference pricing was last verified.
    pub reference_verified: &'static str,
}

/// Result of pricing calculation for a usage record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UsageValue {
    /// What OpenCode actually charged for this usage.
    pub actual_cost: f64,

    /// Equivalent cost at reference rates. `None` if no reference exists.
    pub reference_value: Option<f64>,

    /// free_value = reference_value - actual_cost. `None` if no reference exists.
    pub free_value: Option<f64>,

    /// Status of this pricing calculation.
    pub pricing_status: PricingStatus,

    /// The pricing profile version used (for historical reproducibility).
    pub pricing_version: &'static str,
}

/// Current pricing engine version.
/// Bump this when reference prices change so historical records remain reproducible.
pub const PRICING_VERSION: &str = "ftm-pricing-v1";

/// Look up the full pricing profile for a model.
///
/// Returns `None` for completely unknown models (not $0 — "unknown" != "free").
/// Free models with reference pricing return a profile with actual=$0 and reference=Some(...).
/// Free models without a known equivalent return actual=$0 and reference=None.
fn lookup_profile(model: &str) -> Option<ModelPricingProfile> {
    match model {
        // ──────────────────────────────────────────────────────────────
        // Free models WITH a known reference equivalent
        // ──────────────────────────────────────────────────────────────

    // MiMo-V2.5 Free — promotional free model via OpenCode/Zen.
    // Reference: Xiaomi MiMo-V2.5 paid API pricing.
    // Source: OpenRouter / Xiaomi API pricing (as of 2025-07).
    // Input:  $0.14 / 1M tokens
    // Output: $0.28 / 1M tokens
    // Cached: $0.0028 / 1M tokens
    "mimo-v2.5-free" => Some(ModelPricingProfile {
            display_name: "MiMo-V2.5 Free",
            actual: PricingRates {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cached_per_million: 0.0,
            },
            reference: Some(PricingRates {
                input_per_million: 0.14,
                output_per_million: 0.28,
                cached_per_million: 0.0028,
            }),
            reference_model: Some("mimo-v2.5"),
            reference_source: "OpenRouter / Xiaomi MiMo-V2.5 paid API pricing",
            reference_verified: "2025-07",
        }),

        // ──────────────────────────────────────────────────────────────
        // Free models WITHOUT a known reference equivalent
        // ──────────────────────────────────────────────────────────────

        "nemotron-3-ultra-free" => Some(ModelPricingProfile {
            display_name: "Nemotron 3 Ultra Free",
            actual: PricingRates {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cached_per_million: 0.0,
            },
            reference: None,
            reference_model: None,
            reference_source: "no configured reference",
            reference_verified: "n/a",
        }),
        "nemotron-3.5-lightning-free" => Some(ModelPricingProfile {
            display_name: "Nemotron 3.5 Lightning Free",
            actual: PricingRates {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cached_per_million: 0.0,
            },
            reference: None,
            reference_model: None,
            reference_source: "no configured reference",
            reference_verified: "n/a",
        }),
        "big-pickle" => Some(ModelPricingProfile {
            display_name: "Big Pickle",
            actual: PricingRates {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cached_per_million: 0.0,
            },
            reference: None,
            reference_model: None,
            reference_source: "no configured reference",
            reference_verified: "n/a",
        }),

        // ──────────────────────────────────────────────────────────────
        // Paid models — actual and reference are the same
        // ──────────────────────────────────────────────────────────────

        "gpt-4o" => Some(ModelPricingProfile {
            display_name: "GPT-4o",
            actual: PricingRates {
                input_per_million: 2.50,
                output_per_million: 10.00,
                cached_per_million: 1.25,
            },
            reference: Some(PricingRates {
                input_per_million: 2.50,
                output_per_million: 10.00,
                cached_per_million: 1.25,
            }),
            reference_model: Some("gpt-4o"),
            reference_source: "OpenAI official pricing",
            reference_verified: "2025-07",
        }),
        "gpt-4o-mini" => Some(ModelPricingProfile {
            display_name: "GPT-4o Mini",
            actual: PricingRates {
                input_per_million: 0.15,
                output_per_million: 0.60,
                cached_per_million: 0.075,
            },
            reference: Some(PricingRates {
                input_per_million: 0.15,
                output_per_million: 0.60,
                cached_per_million: 0.075,
            }),
            reference_model: Some("gpt-4o-mini"),
            reference_source: "OpenAI official pricing",
            reference_verified: "2025-07",
        }),
        "gpt-4-turbo" => Some(ModelPricingProfile {
            display_name: "GPT-4 Turbo",
            actual: PricingRates {
                input_per_million: 10.00,
                output_per_million: 30.00,
                cached_per_million: 5.00,
            },
            reference: Some(PricingRates {
                input_per_million: 10.00,
                output_per_million: 30.00,
                cached_per_million: 5.00,
            }),
            reference_model: Some("gpt-4-turbo"),
            reference_source: "OpenAI official pricing",
            reference_verified: "2025-07",
        }),
        "gpt-4.1" => Some(ModelPricingProfile {
            display_name: "GPT-4.1",
            actual: PricingRates {
                input_per_million: 2.00,
                output_per_million: 8.00,
                cached_per_million: 0.50,
            },
            reference: Some(PricingRates {
                input_per_million: 2.00,
                output_per_million: 8.00,
                cached_per_million: 0.50,
            }),
            reference_model: Some("gpt-4.1"),
            reference_source: "OpenAI official pricing",
            reference_verified: "2025-07",
        }),
        "gpt-4.1-mini" => Some(ModelPricingProfile {
            display_name: "GPT-4.1 Mini",
            actual: PricingRates {
                input_per_million: 0.40,
                output_per_million: 1.60,
                cached_per_million: 0.10,
            },
            reference: Some(PricingRates {
                input_per_million: 0.40,
                output_per_million: 1.60,
                cached_per_million: 0.10,
            }),
            reference_model: Some("gpt-4.1-mini"),
            reference_source: "OpenAI official pricing",
            reference_verified: "2025-07",
        }),

        // Anthropic
        "claude-sonnet-4-20250514" | "claude-sonnet-4" => Some(ModelPricingProfile {
            display_name: "Claude Sonnet 4",
            actual: PricingRates {
                input_per_million: 3.00,
                output_per_million: 15.00,
                cached_per_million: 0.30,
            },
            reference: Some(PricingRates {
                input_per_million: 3.00,
                output_per_million: 15.00,
                cached_per_million: 0.30,
            }),
            reference_model: Some("claude-sonnet-4"),
            reference_source: "Anthropic official pricing",
            reference_verified: "2025-07",
        }),
        "claude-haiku-3-5-20241022" | "claude-3-5-haiku-20241022" => Some(ModelPricingProfile {
            display_name: "Claude 3.5 Haiku",
            actual: PricingRates {
                input_per_million: 0.80,
                output_per_million: 4.00,
                cached_per_million: 0.08,
            },
            reference: Some(PricingRates {
                input_per_million: 0.80,
                output_per_million: 4.00,
                cached_per_million: 0.08,
            }),
            reference_model: Some("claude-3-5-haiku"),
            reference_source: "Anthropic official pricing",
            reference_verified: "2025-07",
        }),
        "claude-3-opus-20240229" => Some(ModelPricingProfile {
            display_name: "Claude 3 Opus",
            actual: PricingRates {
                input_per_million: 15.00,
                output_per_million: 75.00,
                cached_per_million: 1.50,
            },
            reference: Some(PricingRates {
                input_per_million: 15.00,
                output_per_million: 75.00,
                cached_per_million: 1.50,
            }),
            reference_model: Some("claude-3-opus"),
            reference_source: "Anthropic official pricing",
            reference_verified: "2025-07",
        }),

        // Google
        "gemini-2.0-flash" | "gemini-2.0-flash-001" => Some(ModelPricingProfile {
            display_name: "Gemini 2.0 Flash",
            actual: PricingRates {
                input_per_million: 0.10,
                output_per_million: 0.40,
                cached_per_million: 0.025,
            },
            reference: Some(PricingRates {
                input_per_million: 0.10,
                output_per_million: 0.40,
                cached_per_million: 0.025,
            }),
            reference_model: Some("gemini-2.0-flash"),
            reference_source: "Google AI official pricing",
            reference_verified: "2025-07",
        }),
        "gemini-2.5-pro" => Some(ModelPricingProfile {
            display_name: "Gemini 2.5 Pro",
            actual: PricingRates {
                input_per_million: 1.25,
                output_per_million: 10.00,
                cached_per_million: 0.315,
            },
            reference: Some(PricingRates {
                input_per_million: 1.25,
                output_per_million: 10.00,
                cached_per_million: 0.315,
            }),
            reference_model: Some("gemini-2.5-pro"),
            reference_source: "Google AI official pricing",
            reference_verified: "2025-07",
        }),

        // DeepSeek
        "deepseek-r1" => Some(ModelPricingProfile {
            display_name: "DeepSeek R1",
            actual: PricingRates {
                input_per_million: 0.55,
                output_per_million: 2.19,
                cached_per_million: 0.14,
            },
            reference: Some(PricingRates {
                input_per_million: 0.55,
                output_per_million: 2.19,
                cached_per_million: 0.14,
            }),
            reference_model: Some("deepseek-r1"),
            reference_source: "DeepSeek official pricing",
            reference_verified: "2025-07",
        }),
        "deepseek-v3" => Some(ModelPricingProfile {
            display_name: "DeepSeek V3",
            actual: PricingRates {
                input_per_million: 0.27,
                output_per_million: 1.10,
                cached_per_million: 0.07,
            },
            reference: Some(PricingRates {
                input_per_million: 0.27,
                output_per_million: 1.10,
                cached_per_million: 0.07,
            }),
            reference_model: Some("deepseek-v3"),
            reference_source: "DeepSeek official pricing",
            reference_verified: "2025-07",
        }),

        // Unknown — return None, not $0
        _ => None,
    }
}

/// Calculate the cost for a set of tokens at given rates.
fn calculate_cost(rates: &PricingRates, input_tokens: i64, output_tokens: i64, cached_tokens: i64) -> f64 {
    let input_cost = (input_tokens as f64 / 1_000_000.0) * rates.input_per_million;
    let output_cost = (output_tokens as f64 / 1_000_000.0) * rates.output_per_million;
    let cached_cost = (cached_tokens as f64 / 1_000_000.0) * rates.cached_per_million;
    input_cost + output_cost + cached_cost
}

/// Calculate the full usage value for a session.
///
/// This is the primary pricing entry point. It determines:
/// - actual_cost: what OpenCode charged
/// - reference_value: what equivalent paid usage would cost
/// - free_value: reference_value - actual_cost
/// - pricing_status: how the pricing was determined
pub fn calculate_usage_value(
    model: &str,
    open_code_cost: f64,
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
) -> UsageValue {
    let total_tokens = input_tokens + output_tokens + cached_tokens;
    if total_tokens == 0 {
        return UsageValue {
            actual_cost: 0.0,
            reference_value: None,
            free_value: None,
            pricing_status: PricingStatus::NoTokens,
            pricing_version: PRICING_VERSION,
        };
    }

    let profile = match lookup_profile(model) {
        Some(p) => p,
        None => {
            // Completely unknown model — we don't fabricate prices.
            let status = if open_code_cost > 0.0 {
                PricingStatus::ActualCostKnown
            } else {
                PricingStatus::ReferenceUnavailable
            };
            return UsageValue {
                actual_cost: open_code_cost,
                reference_value: None,
                free_value: None,
                pricing_status: status,
                pricing_version: PRICING_VERSION,
            };
        }
    };

    // Determine actual cost
    let actual_cost = if open_code_cost > 0.0 {
        // OpenCode reported a real cost — use it as authoritative.
        open_code_cost
    } else {
        // OpenCode reported $0 — use our profile's actual pricing.
        calculate_cost(&profile.actual, input_tokens, output_tokens, cached_tokens)
    };

    // Determine reference value
    let (reference_value, free_value, pricing_status) = match &profile.reference {
        Some(ref_rates) => {
            let ref_val = calculate_cost(ref_rates, input_tokens, output_tokens, cached_tokens);
            let free_val = ref_val - actual_cost;
            let status = if open_code_cost > 0.0 {
                PricingStatus::ActualCostKnown
            } else {
                PricingStatus::ReferenceApplied
            };
            (Some(ref_val), Some(free_val), status)
        }
        None => {
            // No reference configured — unknown != free
            let status = if open_code_cost > 0.0 {
                PricingStatus::ActualCostKnown
            } else {
                PricingStatus::ReferenceUnavailable
            };
            (None, None, status)
        }
    };

    UsageValue {
        actual_cost,
        reference_value,
        free_value,
        pricing_status,
        pricing_version: PRICING_VERSION,
    }
}

/// Get the display name for a model.
#[allow(dead_code)]
pub fn get_model_display_name(model: &str) -> &str {
    match lookup_profile(model) {
        Some(profile) => profile.display_name,
        None => model,
    }
}

/// Get the pricing profile info for display (pricing basis indicator).
#[allow(dead_code)]
pub fn get_pricing_info(model: &str) -> Option<(&str, &str, &str)> {
    lookup_profile(model).map(|p| {
        let basis = match p.reference_model {
            Some(ref_model) => ref_model,
            None => "unavailable",
        };
        (basis, p.reference_source, p.reference_verified)
    })
}

/// Legacy compatibility: estimate_model_cost for backward compatibility.
/// Returns the actual cost for a session (what the user paid).
#[allow(dead_code)]
pub fn estimate_model_cost(model: &str, input_tokens: i64, output_tokens: i64, cached_tokens: i64) -> f64 {
    let result = calculate_usage_value(model, 0.0, input_tokens, output_tokens, cached_tokens);
    result.actual_cost
}

#[cfg(test)]
mod tests {
    use super::*;

    // ──────────────────────────────────────────────────────────────────
    // MiMo-V2.5 Free
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn mimo_free_actual_cost_is_zero() {
        let v = calculate_usage_value("mimo-v2.5-free", 0.0, 1_000_000, 500_000, 100_000);
        assert_eq!(v.actual_cost, 0.0);
        assert_eq!(v.pricing_status, PricingStatus::ReferenceApplied);
    }

    #[test]
    fn mimo_free_reference_value_calculated() {
        // 1M input @ $0.14 = $0.14
        // 500K output @ $0.28 = $0.14
        // 100K cached @ $0.0028 = $0.00028
        let v = calculate_usage_value("mimo-v2.5-free", 0.0, 1_000_000, 500_000, 100_000);
        let ref_val = v.reference_value.unwrap();
        let expected = 0.14 + 0.14 + 0.00028;
        assert!((ref_val - expected).abs() < 0.0001, "Expected ~{}, got {}", expected, ref_val);
    }

    #[test]
    fn mimo_free_free_value_equals_reference() {
        let v = calculate_usage_value("mimo-v2.5-free", 0.0, 1_000_000, 500_000, 0);
        let free_val = v.free_value.unwrap();
        let ref_val = v.reference_value.unwrap();
        // For a free model, free_value should equal reference_value (since actual_cost = 0)
        assert!((free_val - ref_val).abs() < 0.0001);
    }

    // ──────────────────────────────────────────────────────────────────
    // Free models WITHOUT reference
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn nemotron_free_reference_unavailable() {
        let v = calculate_usage_value("nemotron-3-ultra-free", 0.0, 1_000_000, 500_000, 0);
        assert_eq!(v.actual_cost, 0.0);
        assert_eq!(v.reference_value, None);
        assert_eq!(v.free_value, None);
        assert_eq!(v.pricing_status, PricingStatus::ReferenceUnavailable);
    }

    // ──────────────────────────────────────────────────────────────────
    // Unknown models
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn unknown_model_reference_unavailable() {
        let v = calculate_usage_value("some-random-model", 0.0, 1_000_000, 1_000_000, 0);
        assert_eq!(v.actual_cost, 0.0);
        assert_eq!(v.reference_value, None);
        assert_eq!(v.pricing_status, PricingStatus::ReferenceUnavailable);
    }

    #[test]
    fn unknown_model_with_opencode_cost() {
        let v = calculate_usage_value("some-random-model", 5.0, 1_000_000, 1_000_000, 0);
        assert_eq!(v.actual_cost, 5.0);
        assert_eq!(v.reference_value, None);
        assert_eq!(v.pricing_status, PricingStatus::ActualCostKnown);
    }

    // ──────────────────────────────────────────────────────────────────
    // Paid models
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn gpt4o_reference_equals_actual() {
        // gpt-4o: $2.50 input, $10.00 output, $1.25 cached per million
        let v = calculate_usage_value("gpt-4o", 13.75, 1_000_000, 1_000_000, 1_000_000);
        assert_eq!(v.actual_cost, 13.75);
        let ref_val = v.reference_value.unwrap();
        assert!((ref_val - 13.75).abs() < 0.01);
        let free_val = v.free_value.unwrap();
        assert!((free_val - 0.0).abs() < 0.01);
        assert_eq!(v.pricing_status, PricingStatus::ActualCostKnown);
    }

    #[test]
    fn gpt4o_no_opencode_cost_uses_reference() {
        let v = calculate_usage_value("gpt-4o", 0.0, 1_000_000, 1_000_000, 0);
        // actual_cost should use profile's actual pricing: $2.50 + $10.00 = $12.50
        assert!((v.actual_cost - 12.50).abs() < 0.01);
        // reference_value should be the same for paid models
        let ref_val = v.reference_value.unwrap();
        assert!((ref_val - 12.50).abs() < 0.01);
        // free_value = 0 (same price)
        let free_val = v.free_value.unwrap();
        assert!(free_val.abs() < 0.01);
    }

    // ──────────────────────────────────────────────────────────────────
    // Edge cases
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn zero_tokens_returns_no_tokens() {
        let v = calculate_usage_value("gpt-4o", 0.0, 0, 0, 0);
        assert_eq!(v.pricing_status, PricingStatus::NoTokens);
        assert_eq!(v.actual_cost, 0.0);
    }

    #[test]
    fn partial_tokens_calculate_correctly() {
        // gpt-4o-mini: $0.15 input, $0.60 output per million
        let v = calculate_usage_value("gpt-4o-mini", 0.0, 500_000, 250_000, 0);
        // 0.5 * 0.15 + 0.25 * 0.60 = 0.075 + 0.15 = 0.225
        assert!((v.actual_cost - 0.225).abs() < 0.001, "Expected $0.225, got {}", v.actual_cost);
    }

    // ──────────────────────────────────────────────────────────────────
    // Pricing version
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn pricing_version_is_set() {
        let v = calculate_usage_value("gpt-4o", 1.0, 100, 100, 0);
        assert_eq!(v.pricing_version, PRICING_VERSION);
    }

    // ──────────────────────────────────────────────────────────────────
    // Historical pricing: changing current prices does not affect old records
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn pricing_version_preserves_history() {
        // Simulate: a record priced at v1 should report v1 even if current is v2
        let v1 = calculate_usage_value("gpt-4o", 1.0, 100, 100, 0);
        assert_eq!(v1.pricing_version, "ftm-pricing-v1");

        // If PRICING_VERSION were bumped to "ftm-pricing-v2" in a future release,
        // old records would retain "ftm-pricing-v1" in their stored pricing_version.
        // New records would get "ftm-pricing-v2".
        // This test documents that intent.
    }

    // ──────────────────────────────────────────────────────────────────
    // Cached tokens
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn cached_tokens_use_separate_rate() {
        // mimo-v2.5-free cached: $0.0028 per million
        let v = calculate_usage_value("mimo-v2.5-free", 0.0, 0, 0, 1_000_000);
        let ref_val = v.reference_value.unwrap();
        assert!((ref_val - 0.0028).abs() < 0.0001, "Expected ~0.0028, got {}", ref_val);
    }

    #[test]
    fn cached_tokens_not_double_counted() {
        // gpt-4o with only cached tokens
        let v = calculate_usage_value("gpt-4o", 0.0, 0, 0, 1_000_000);
        // Only cached cost: $1.25
        assert!((v.actual_cost - 1.25).abs() < 0.01);
    }

    // ──────────────────────────────────────────────────────────────────
    // Display name
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn display_name_for_known_model() {
        assert_eq!(get_model_display_name("mimo-v2.5-free"), "MiMo-V2.5 Free");
        assert_eq!(get_model_display_name("gpt-4o"), "GPT-4o");
    }

    #[test]
    fn display_name_for_unknown_model() {
        assert_eq!(get_model_display_name("unknown-model"), "unknown-model");
    }
}
