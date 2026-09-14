/// Pricing engine — completely separated from data collection.
///
/// Contains model-specific pricing data and cost estimation logic.
/// All prices are per million tokens in USD.

pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cached_per_million: f64,
}

/// Look up known pricing for a model.
/// Returns None for unknown models (not $0 — "unknown" != "free").
fn lookup_pricing(model: &str) -> Option<ModelPricing> {
    match model {
        // Known free models — genuinely free, no equivalent paid pricing
        "mimo-v2.5-free"
        | "nemotron-3-ultra-free"
        | "nemotron-3.5-lightning-free"
        | "big-pickle" => Some(ModelPricing {
            input_per_million: 0.0,
            output_per_million: 0.0,
            cached_per_million: 0.0,
        }),

        // OpenAI
        "gpt-4o" => Some(ModelPricing {
            input_per_million: 2.50,
            output_per_million: 10.00,
            cached_per_million: 1.25,
        }),
        "gpt-4o-mini" => Some(ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.60,
            cached_per_million: 0.075,
        }),
        "gpt-4-turbo" => Some(ModelPricing {
            input_per_million: 10.00,
            output_per_million: 30.00,
            cached_per_million: 5.00,
        }),
        "gpt-4.1" => Some(ModelPricing {
            input_per_million: 2.00,
            output_per_million: 8.00,
            cached_per_million: 0.50,
        }),
        "gpt-4.1-mini" => Some(ModelPricing {
            input_per_million: 0.40,
            output_per_million: 1.60,
            cached_per_million: 0.10,
        }),

        // Anthropic
        "claude-sonnet-4-20250514" | "claude-sonnet-4" => Some(ModelPricing {
            input_per_million: 3.00,
            output_per_million: 15.00,
            cached_per_million: 0.30,
        }),
        "claude-haiku-3-5-20241022" | "claude-3-5-haiku-20241022" => Some(ModelPricing {
            input_per_million: 0.80,
            output_per_million: 4.00,
            cached_per_million: 0.08,
        }),
        "claude-3-opus-20240229" => Some(ModelPricing {
            input_per_million: 15.00,
            output_per_million: 75.00,
            cached_per_million: 1.50,
        }),

        // Google
        "gemini-2.0-flash" | "gemini-2.0-flash-001" => Some(ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.40,
            cached_per_million: 0.025,
        }),
        "gemini-2.5-pro" => Some(ModelPricing {
            input_per_million: 1.25,
            output_per_million: 10.00,
            cached_per_million: 0.315,
        }),

        // DeepSeek
        "deepseek-r1" => Some(ModelPricing {
            input_per_million: 0.55,
            output_per_million: 2.19,
            cached_per_million: 0.14,
        }),
        "deepseek-v3" => Some(ModelPricing {
            input_per_million: 0.27,
            output_per_million: 1.10,
            cached_per_million: 0.07,
        }),

        // Unknown — return None, not $0
        _ => None,
    }
}

/// Estimate what a session would have cost at standard API rates.
///
/// Returns:
/// - `$0.00` for known free models (actual cost is $0)
/// - A calculated estimate for known paid models
/// - `$0.00` for unknown models (we don't fabricate prices)
///
/// This function is ONLY called when OpenCode reports cost = $0.
/// When OpenCode has an actual cost, that value is used directly.
pub fn estimate_model_cost(model: &str, input_tokens: i64, output_tokens: i64, cached_tokens: i64) -> f64 {
    let pricing = match lookup_pricing(model) {
        Some(p) => p,
        None => return 0.0, // Unknown model — don't guess
    };

    let input_cost = (input_tokens as f64 / 1_000_000.0) * pricing.input_per_million;
    let output_cost = (output_tokens as f64 / 1_000_000.0) * pricing.output_per_million;
    let cached_cost = (cached_tokens as f64 / 1_000_000.0) * pricing.cached_per_million;

    input_cost + output_cost + cached_cost
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_models_return_zero() {
        assert_eq!(estimate_model_cost("mimo-v2.5-free", 1_000_000, 500_000, 100_000), 0.0);
        assert_eq!(estimate_model_cost("nemotron-3-ultra-free", 1_000_000, 500_000, 0), 0.0);
    }

    #[test]
    fn known_paid_model_calculates_correctly() {
        // gpt-4o: $2.50 input, $10.00 output, $1.25 cached per million
        let cost = estimate_model_cost("gpt-4o", 1_000_000, 1_000_000, 1_000_000);
        assert!((cost - 13.75).abs() < 0.01, "Expected $13.75, got {}", cost);
    }

    #[test]
    fn unknown_model_returns_zero() {
        assert_eq!(estimate_model_cost("some-random-model", 1_000_000, 1_000_000, 0), 0.0);
    }

    #[test]
    fn zero_tokens_returns_zero() {
        assert_eq!(estimate_model_cost("gpt-4o", 0, 0, 0), 0.0);
    }

    #[test]
    fn partial_tokens_calculate_correctly() {
        // gpt-4o-mini: $0.15 input, $0.60 output per million
        let cost = estimate_model_cost("gpt-4o-mini", 500_000, 250_000, 0);
        // 0.5 * 0.15 + 0.25 * 0.60 = 0.075 + 0.15 = 0.225
        assert!((cost - 0.225).abs() < 0.001, "Expected $0.225, got {}", cost);
    }
}
