//! Model name normalization and aliasing.
//!
//! Models appear under different names across OpenCode, Freebuff, LiteLLM,
//! OpenRouter, and upstream provider APIs. This module provides a single
//! normalization layer that maps any known variant to a canonical form.
//!
//! # Lookup order
//!
//! 1. Exact match in the pricing registry (provider + model_id)
//! 2. Alias match (explicit FTM-defined aliases)
//! 3. Normalized match (provider prefix stripping, case folding)
//! 4. Unknown

use std::collections::HashMap;

/// A single alias mapping: raw model ID → canonical model ID.
///
/// The runtime alias map is not wired into the pricing resolver yet —
/// `strip_provider_prefix` below is what the resolver currently uses — so this
/// scaffolding is kept but unused until the alias layer lands.
#[allow(dead_code)]
struct AliasEntry {
    raw: String,
    canonical: String,
}

/// The normalization map: built once at startup from a list of aliases.
#[allow(dead_code)]
pub struct NormalizationMap {
    /// raw (lowercased) → canonical
    aliases: HashMap<String, String>,
    /// normalized (lowercased, prefix-stripped) → canonical
    normalized: HashMap<String, String>,
}

#[allow(dead_code)]
impl NormalizationMap {
    /// Build a normalization map from explicit alias pairs and model IDs.
    ///
    /// `alias_pairs`: list of (raw_id, canonical_id) — explicit FTM aliases.
    /// `model_ids`: all known model IDs from both the static and dynamic registries.
    pub fn build(
        alias_pairs: &[(&str, &str)],
        model_ids: &[&str],
    ) -> Self {
        let mut aliases = HashMap::new();

        // Register explicit aliases.
        for (raw, canonical) in alias_pairs {
            aliases.insert(raw.to_lowercase(), canonical.to_lowercase());
        }

        // Build normalized forms for all known model IDs.
        let mut normalized = HashMap::new();
        for id in model_ids {
            let lower = id.to_lowercase();
            normalized.insert(lower.clone(), lower.clone());

            // Also register the stripped prefix form.
            if let Some(stripped) = strip_provider_prefix(id) {
                let stripped_lower = stripped.to_lowercase();
                if stripped_lower != lower {
                    normalized.insert(stripped_lower, lower.clone());
                }
            }
        }

        Self { aliases, normalized }
    }

    /// Resolve a raw model ID to its canonical form.
    ///
    /// Returns `None` if the model is completely unknown.
    pub fn resolve(&self, raw_model: &str) -> Option<String> {
        let lower = raw_model.to_lowercase();

        // 1. Exact alias match.
        if let Some(canonical) = self.aliases.get(&lower) {
            return Some(canonical.clone());
        }

        // 2. Exact normalized match (the model ID itself is known).
        if let Some(canonical) = self.normalized.get(&lower) {
            return Some(canonical.clone());
        }

        // 3. Stripped prefix match.
        if let Some(stripped) = strip_provider_prefix(raw_model) {
            let stripped_lower = stripped.to_lowercase();
            if let Some(canonical) = self.normalized.get(&stripped_lower) {
                return Some(canonical.clone());
            }

            // 4. Bare model name after any remaining vendor namespace
            //    (`openrouter/xiaomi/mimo-v2.5` → `mimo-v2.5`).
            if let Some((_, tail)) = stripped_lower.rsplit_once('/') {
                if let Some(canonical) = self.normalized.get(tail) {
                    return Some(canonical.clone());
                }
            }
        }

        None
    }

    /// Register a new model ID in the normalization map (called when the
    /// dynamic registry is loaded).
    pub fn register_model(&mut self, model_id: &str) {
        let lower = model_id.to_lowercase();
        self.normalized.insert(lower.clone(), lower.clone());

        if let Some(stripped) = strip_provider_prefix(model_id) {
            let stripped_lower = stripped.to_lowercase();
            if stripped_lower != lower {
                self.normalized.insert(stripped_lower, lower);
            }
        }
    }

    /// Register an alias at runtime.
    pub fn register_alias(&mut self, raw: &str, canonical: &str) {
        self.aliases.insert(raw.to_lowercase(), canonical.to_lowercase());
    }
}

/// Strip a known provider prefix from a model ID.
///
/// Examples:
/// - `"openrouter/xiaomi/mimo-v2.5"` → `"xiaomi/mimo-v2.5"`
/// - `"deepseek/deepseek-r1"` → `"deepseek-r1"`
/// - `"gpt-4o"` → `None`
pub fn strip_provider_prefix(model: &str) -> Option<&str> {
    // Known prefixes that LiteLLM uses.
    const PREFIXES: &[&str] = &[
        "openrouter/",
        "bedrock/",
        "bedrock_converse/",
        "vertex_ai/",
        "azure/",
        "azure_ai/",
        "deepinfra/",
        "together_ai/",
        "fireworks_ai/",
        "groq/",
        "anyscale/",
        "cloudflare/",
        "perplexity/",
        "mistral/",
        "ollama/",
        "deepseek/",
    ];

    for prefix in PREFIXES {
        if let Some(stripped) = model.strip_prefix(prefix) {
            if !stripped.is_empty() {
                return Some(stripped);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_explicit_alias() {
        let map = NormalizationMap::build(
            &[("mimo-v2.5-free", "mimo-v2.5")],
            &["mimo-v2.5", "gpt-4o"],
        );
        assert_eq!(map.resolve("mimo-v2.5-free"), Some("mimo-v2.5".to_string()));
    }

    #[test]
    fn resolve_exact_model_id() {
        let map = NormalizationMap::build(&[], &["gpt-4o", "claude-sonnet-4"]);
        assert_eq!(map.resolve("gpt-4o"), Some("gpt-4o".to_string()));
    }

    #[test]
    fn resolve_case_insensitive() {
        let map = NormalizationMap::build(&[], &["GPT-4o"]);
        assert_eq!(map.resolve("gpt-4o"), Some("gpt-4o".to_string()));
        assert_eq!(map.resolve("GPT-4O"), Some("gpt-4o".to_string()));
    }

    #[test]
    fn resolve_stripped_prefix() {
        let map = NormalizationMap::build(&[], &["deepseek-r1"]);
        assert_eq!(
            map.resolve("deepseek/deepseek-r1"),
            Some("deepseek-r1".to_string())
        );
    }

    #[test]
    fn resolve_openrouter_prefix() {
        let map = NormalizationMap::build(&[], &["mimo-v2.5"]);
        assert_eq!(
            map.resolve("openrouter/xiaomi/mimo-v2.5"),
            Some("mimo-v2.5".to_string())
        );
    }

    #[test]
    fn unknown_model_returns_none() {
        let map = NormalizationMap::build(&[], &["gpt-4o"]);
        assert_eq!(map.resolve("totally-unknown"), None);
    }

    #[test]
    fn register_model_at_runtime() {
        let mut map = NormalizationMap::build(&[], &["gpt-4o"]);
        assert_eq!(map.resolve("deepseek-v3"), None);
        map.register_model("deepseek-v3");
        assert_eq!(map.resolve("deepseek-v3"), Some("deepseek-v3".to_string()));
    }

    #[test]
    fn register_alias_at_runtime() {
        let mut map = NormalizationMap::build(&[], &["mimo-v2.5"]);
        map.register_alias("mimo-v2.5-free", "mimo-v2.5");
        assert_eq!(map.resolve("mimo-v2.5-free"), Some("mimo-v2.5".to_string()));
    }

    #[test]
    fn strip_known_prefixes() {
        assert_eq!(strip_provider_prefix("deepseek/deepseek-r1"), Some("deepseek-r1"));
        assert_eq!(strip_provider_prefix("openrouter/xiaomi/mimo-v2.5"), Some("xiaomi/mimo-v2.5"));
        assert_eq!(strip_provider_prefix("gpt-4o"), None);
        assert_eq!(strip_provider_prefix("bedrock/anthropic.claude-3"), Some("anthropic.claude-3"));
    }
}
