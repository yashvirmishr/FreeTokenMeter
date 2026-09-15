//! Dynamic pricing registry — downloads and caches the LiteLLM model cost map.
//!
//! # Architecture
//!
//! ```text
//! application startup
//!        ↓
//! load local pricing cache
//!        ↓
//! show cached prices immediately
//!        ↓
//! background refresh (if cache is stale)
//!        ↓
//! validate new registry
//!        ↓
//! atomic cache replacement
//! ```
//!
//! # Sources
//!
//! - **Primary**: LiteLLM `model_prices_and_context_window.json`
//!   <https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json>
//! - **Fallback**: local cache
//! - **Last resort**: pricing is reported as unavailable

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use log::info;
use serde::{Deserialize, Serialize};

/// LiteLLM registry URL.
const LITELLM_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

/// How old the cache can be before a refresh is triggered (24 hours).
const CACHE_MAX_AGE_SECS: u64 = 86_400;

/// HTTP request timeout for registry download.
const DOWNLOAD_TIMEOUT_SECS: u64 = 30;

// ──────────────────────────────────────────────────────────────────────
// Data structures
// ──────────────────────────────────────────────────────────────────────

/// A single model entry from the LiteLLM registry (subset of fields we use).
///
/// This mirrors the upstream document for reference; parsing works off
/// `serde_json::Value` so unknown or renamed fields cannot break a refresh.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryModelEntry {
    /// Provider identifier (e.g., "openai", "anthropic", "deepseek").
    pub litellm_provider: Option<String>,

    /// Input cost per token (USD).
    pub input_cost_per_token: Option<f64>,

    /// Output cost per token (USD).
    pub output_cost_per_token: Option<f64>,

    /// Cache read cost per token (USD).
    pub cache_read_input_token_cost: Option<f64>,

    /// Maximum input tokens.
    pub max_input_tokens: Option<i64>,

    /// Maximum output tokens.
    pub max_output_tokens: Option<i64>,

    /// Model mode (chat, embedding, completion, etc.).
    pub mode: Option<String>,

    /// Source URL for the pricing data.
    pub source: Option<String>,
}

/// FreeTokenMeter's internal pricing rates (per million tokens, USD).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PricingRates {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cached_per_million: f64,
}

/// A pricing entry in our internal format, derived from the LiteLLM registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicPricingEntry {
    /// The model ID as it appears in the registry.
    pub model_id: String,

    /// LiteLLM provider string.
    pub litellm_provider: String,

    /// Display name derived from the model ID.
    pub display_name: String,

    /// Reference pricing rates.
    pub rates: PricingRates,

    /// Source URL.
    pub source: String,

    /// Maximum input tokens.
    pub max_input_tokens: Option<i64>,

    /// Maximum output tokens.
    pub max_output_tokens: Option<i64>,
}

/// The complete pricing registry snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingRegistry {
    /// Registry version (derived from download timestamp).
    pub version: String,

    /// When this registry was downloaded.
    pub downloaded_at: String,

    /// Source URL.
    pub source_url: String,

    /// All model entries, keyed by model ID (lowercased).
    pub entries: HashMap<String, DynamicPricingEntry>,

    /// How many valid entries were parsed.
    pub valid_count: usize,

    /// How many entries were skipped (malformed, unsupported mode, etc.).
    pub skipped_count: usize,
}

/// Status of the pricing registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryStatus {
    /// Whether a registry is loaded (cached or fresh).
    pub loaded: bool,

    /// Whether the cache is current or stale.
    pub current: bool,

    /// Number of models in the registry.
    pub model_count: usize,

    /// When the registry was last updated.
    pub last_updated: Option<String>,

    /// How old the cache is (human-readable).
    pub age: Option<String>,

    /// Source URL.
    pub source: String,
}

/// Shared registry state managed by the application.
pub struct RegistryState {
    pub registry: Mutex<Option<PricingRegistry>>,
    pub cache_path: PathBuf,
}

impl RegistryState {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            registry: Mutex::new(None),
            cache_path: data_dir.join("pricing_cache.json"),
        }
    }

    /// Load the registry from cache on startup.
    /// Does NOT block — returns immediately with whatever is cached.
    pub fn load_cache(&self) {
        match load_cache(&self.cache_path) {
            Ok(registry) => {
                info!(
                    "Pricing registry loaded from cache: {} models (downloaded {})",
                    registry.valid_count, registry.downloaded_at
                );
                if let Ok(mut slot) = self.registry.lock() {
                    *slot = Some(registry);
                }
            }
            Err(e) => {
                info!("No pricing cache found: {}. Will download on first refresh.", e);
            }
        }
    }

    /// Check if the current cache needs a refresh.
    pub fn needs_refresh(&self) -> bool {
        let registry = match self.registry.lock() {
            Ok(r) => r,
            Err(_) => return true,
        };

        match registry.as_ref() {
            None => true,
            Some(r) => is_cache_stale(&r.downloaded_at),
        }
    }

    /// Get the current registry status.
    pub fn status(&self) -> RegistryStatus {
        let registry = self.registry.lock().ok().and_then(|r| r.clone());
        match registry {
            Some(r) => {
                let current = !is_cache_stale(&r.downloaded_at);
                let age = cache_age_display(&r.downloaded_at);
                RegistryStatus {
                    loaded: true,
                    current,
                    model_count: r.valid_count,
                    last_updated: Some(r.downloaded_at),
                    age: Some(age),
                    source: r.source_url,
                }
            }
            None => RegistryStatus {
                loaded: false,
                current: false,
                model_count: 0,
                last_updated: None,
                age: None,
                source: LITELLM_REGISTRY_URL.to_string(),
            },
        }
    }

    /// Refresh the registry from the network.
    /// Returns Ok(true) if a new registry was loaded, Ok(false) if unchanged.
    pub fn refresh(&self) -> Result<bool, String> {
        let new_registry = download_and_parse()?;

        // Atomic replacement: validate, then write cache, then replace in-memory.
        save_cache(&self.cache_path, &new_registry)?;

        let new_version = new_registry.version.clone();
        let new_count = new_registry.valid_count;

        if let Ok(mut slot) = self.registry.lock() {
            *slot = Some(new_registry);
        }

        info!(
            "Pricing registry refreshed: {} models (version {})",
            new_count, new_version
        );
        Ok(true)
    }

    /// Look up pricing for a model (lowercased model ID).
    pub fn lookup(&self, model_lower: &str) -> Option<DynamicPricingEntry> {
        let registry = self.registry.lock().ok()?;
        let r = registry.as_ref()?;
        r.entries.get(model_lower).cloned()
    }

    /// Get all entries (for the pricing registry view).
    pub fn all_entries(&self) -> Vec<DynamicPricingEntry> {
        let registry = self.registry.lock().ok();
        match registry {
            Some(r) => match r.as_ref() {
                Some(reg) => {
                    let mut entries: Vec<DynamicPricingEntry> = reg.entries.values().cloned().collect();
                    entries.sort_by(|a, b| a.model_id.cmp(&b.model_id));
                    entries
                }
                None => Vec::new(),
            },
            None => Vec::new(),
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// Download and parse
// ──────────────────────────────────────────────────────────────────────

/// Download the LiteLLM registry, parse it, and return a PricingRegistry.
fn download_and_parse() -> Result<PricingRegistry, String> {
    let body = download_registry()?;
    parse_registry(&body)
}

/// Download the raw JSON from the LiteLLM GitHub URL.
fn download_registry() -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .build();

    let body = agent
        .get(LITELLM_REGISTRY_URL)
        .call()
        .map_err(|e| format!("Failed to download pricing registry: {}", e))?
        .into_string()
        .map_err(|e| format!("Failed to read registry response: {}", e))?;

    Ok(body)
}

/// Parse the raw JSON into our internal PricingRegistry format.
///
/// Tolerates malformed entries: they are counted as skipped.
fn parse_registry(body: &str) -> Result<PricingRegistry, String> {
    let raw: HashMap<String, serde_json::Value> =
        serde_json::from_str(body).map_err(|e| format!("Failed to parse registry JSON: {}", e))?;

    let now = Utc::now();
    let version = format!("litellm-{}", now.format("%Y%m%d-%H%M"));
    let downloaded_at = now.to_rfc3339();

    let mut entries = HashMap::new();
    let mut valid_count = 0;
    let mut skipped_count = 0;

    for (model_id, value) in &raw {
        // Skip the sample_spec documentation entry.
        if model_id == "sample_spec" {
            continue;
        }

        match parse_entry(model_id, value) {
            Some(entry) => {
                entries.insert(model_id.to_lowercase(), entry);
                valid_count += 1;
            }
            None => {
                skipped_count += 1;
            }
        }
    }

    info!(
        "Pricing registry loaded: {} models, {} valid, {} skipped",
        valid_count + skipped_count,
        valid_count,
        skipped_count
    );

    Ok(PricingRegistry {
        version,
        downloaded_at,
        source_url: LITELLM_REGISTRY_URL.to_string(),
        entries,
        valid_count,
        skipped_count,
    })
}

/// Parse a single model entry from the JSON.
///
/// Returns `None` if the entry is malformed or unsupported (e.g., embedding models,
/// image generation models — we only care about chat/completion models with token pricing).
fn parse_entry(model_id: &str, value: &serde_json::Value) -> Option<DynamicPricingEntry> {
    let obj = value.as_object()?;

    // Only include models with actual token-based pricing.
    let input_cost = obj.get("input_cost_per_token")?.as_f64()?;
    let output_cost = obj.get("output_cost_per_token")?.as_f64()?;

    // Skip embedding models, image generation, etc. — only chat/completion.
    let mode = obj.get("mode").and_then(|v| v.as_str()).unwrap_or("chat");
    if mode != "chat" && mode != "completion" && mode != "responses" {
        return None;
    }

    let litellm_provider = obj
        .get("litellm_provider")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let cache_cost = obj
        .get("cache_read_input_token_cost")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let max_input = obj
        .get("max_input_tokens")
        .and_then(|v| v.as_i64());

    let max_output = obj
        .get("max_output_tokens")
        .and_then(|v| v.as_i64());

    let source = obj
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("LiteLLM registry")
        .to_string();

    // Convert per-token to per-million rates.
    let rates = PricingRates {
        input_per_million: input_cost * 1_000_000.0,
        output_per_million: output_cost * 1_000_000.0,
        cached_per_million: cache_cost * 1_000_000.0,
    };

    // Skip models that are effectively free (all rates = 0) in the registry.
    // Free models are handled by FTM overrides, not the generic registry.
    if rates.input_per_million == 0.0 && rates.output_per_million == 0.0 {
        return None;
    }

    let last_part = model_id.split('/').last().unwrap_or(model_id);
    let display_name = if let Some(first) = last_part.chars().next() {
        let capitalized: String = first.to_uppercase().collect();
        format!("{}{}", capitalized, &last_part[first.len_utf8()..])
    } else {
        last_part.to_string()
    };

    Some(DynamicPricingEntry {
        model_id: model_id.to_string(),
        litellm_provider,
        display_name,
        rates,
        source,
        max_input_tokens: max_input,
        max_output_tokens: max_output,
    })
}

// ──────────────────────────────────────────────────────────────────────
// Cache management
// ──────────────────────────────────────────────────────────────────────

/// Load the cached pricing registry from disk.
fn load_cache(path: &Path) -> Result<PricingRegistry, String> {
    let data = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read cache file: {}", e))?;

    let registry: PricingRegistry = serde_json::from_str(&data)
        .map_err(|e| format!("Failed to parse cache file: {}", e))?;

    Ok(registry)
}

/// Save the pricing registry to disk atomically.
///
/// Writes to a temporary file first, then renames for atomic replacement.
fn save_cache(path: &Path, registry: &PricingRegistry) -> Result<(), String> {
    let tmp_path = path.with_extension("json.tmp");

    let data = serde_json::to_string_pretty(registry)
        .map_err(|e| format!("Failed to serialize registry: {}", e))?;

    std::fs::write(&tmp_path, data)
        .map_err(|e| format!("Failed to write cache file: {}", e))?;

    // Atomic rename.
    std::fs::rename(&tmp_path, path)
        .map_err(|e| format!("Failed to rename cache file: {}", e))?;

    Ok(())
}

/// Check if the cache is older than the maximum age.
fn is_cache_stale(downloaded_at: &str) -> bool {
    let Ok(dt) = DateTime::parse_from_rfc3339(downloaded_at) else {
        return true;
    };
    let age = Utc::now().signed_duration_since(dt);
    age.num_seconds() as u64 > CACHE_MAX_AGE_SECS
}

/// Human-readable age of the cache.
fn cache_age_display(downloaded_at: &str) -> String {
    let Ok(dt) = DateTime::parse_from_rfc3339(downloaded_at) else {
        return "unknown".to_string();
    };
    let age = Utc::now().signed_duration_since(dt);
    if age.num_minutes() < 60 {
        format!("{}m ago", age.num_minutes())
    } else if age.num_hours() < 24 {
        format!("{}h ago", age.num_hours())
    } else {
        format!("{}d ago", age.num_days())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_entry_with_valid_chat_model() {
        let json = serde_json::json!({
            "input_cost_per_token": 2.5e-06,
            "output_cost_per_token": 1e-05,
            "cache_read_input_token_cost": 1.25e-06,
            "litellm_provider": "openai",
            "mode": "chat",
            "max_input_tokens": 128000,
            "max_output_tokens": 16384,
            "source": "https://openai.com/pricing"
        });

        let entry = parse_entry("gpt-4o", &json).expect("should parse");
        assert_eq!(entry.model_id, "gpt-4o");
        assert_eq!(entry.litellm_provider, "openai");
        assert!((entry.rates.input_per_million - 2.50).abs() < 0.01);
        assert!((entry.rates.output_per_million - 10.00).abs() < 0.01);
        assert!((entry.rates.cached_per_million - 1.25).abs() < 0.01);
    }

    #[test]
    fn parse_entry_skips_embedding_models() {
        let json = serde_json::json!({
            "input_cost_per_token": 0.0001,
            "mode": "embedding",
            "litellm_provider": "openai"
        });

        assert!(parse_entry("text-embedding-3-small", &json).is_none());
    }

    #[test]
    fn parse_entry_skips_free_models() {
        let json = serde_json::json!({
            "input_cost_per_token": 0.0,
            "output_cost_per_token": 0.0,
            "mode": "chat",
            "litellm_provider": "some-free-provider"
        });

        assert!(parse_entry("free-model", &json).is_none());
    }

    #[test]
    fn parse_entry_skips_missing_pricing() {
        let json = serde_json::json!({
            "litellm_provider": "openai",
            "mode": "chat"
        });

        assert!(parse_entry("no-pricing", &json).is_none());
    }

    #[test]
    fn parse_registry_skips_sample_spec() {
        let json = r#"{"sample_spec": {"input_cost_per_token": 0}, "gpt-4o": {"input_cost_per_token": 2.5e-06, "output_cost_per_token": 1e-05, "litellm_provider": "openai", "mode": "chat"}}"#;
        let registry = parse_registry(json).expect("should parse");
        assert_eq!(registry.valid_count, 1);
        assert!(registry.entries.contains_key("gpt-4o"));
    }

    #[test]
    fn cache_atomic_write() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-registry-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let cache_path = dir.join("pricing_cache.json");
        let registry = PricingRegistry {
            version: "test-1".to_string(),
            downloaded_at: Utc::now().to_rfc3339(),
            source_url: "https://example.com".to_string(),
            entries: HashMap::new(),
            valid_count: 0,
            skipped_count: 0,
        };

        save_cache(&cache_path, &registry).expect("save");
        let loaded = load_cache(&cache_path).expect("load");
        assert_eq!(loaded.version, "test-1");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
