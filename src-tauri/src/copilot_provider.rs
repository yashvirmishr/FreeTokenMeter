//! GitHub Copilot CLI provider.
//!
//! # What was investigated
//!
//! GitHub Copilot CLI stores its local state under `~/.copilot`:
//!
//! ```text
//! ~/.copilot/config.json      user settings (first launch time only)
//! ~/.copilot/ide/             IDE integration locks
//! ~/.copilot/logs/            one process log per CLI launch
//! ```
//!
//! The CLI exposes usage through the interactive `/usage` command, and GitHub
//! documents AI-credit billing and per-model token pricing for metered usage.
//! What it does **not** provide in the versions inspected is a structured local
//! file containing historical per-session token counts: `config.json` holds no
//! usage, and the process logs are unstructured diagnostic output.
//!
//! # Decision
//!
//! FreeTokenMeter does not scrape the `/usage` terminal screen, so this
//! provider reports [`ProviderAvailability::UsageUnavailable`] when Copilot is
//! installed, and contributes **no** records. It is never listed as supported
//! until a supported structured source exists and has been verified end to end.
//!
//! # Cost semantics (for when a source does land)
//!
//! GitHub meters Copilot usage in AI credits (1 credit = $0.01 in the documented
//! billing model), and individual subscriptions include entitlements. So even
//! once usage is readable, the three figures must stay distinct:
//!
//! ```text
//! provider reported value   (AI credits)
//! reference API value       (USD, from the shared pricing registry)
//! actual user cost          (N/A under a subscription entitlement)
//! ```
//!
//! Translating every AI credit into "money spent" would be wrong, so the
//! billing unit is carried separately from USD by the normalized model.

use std::path::PathBuf;

use crate::provider::{FetchOutcome, FetchRequest, UsageProvider};
use crate::usage::{BillingUnit, PaymentMode, ProviderAvailability, SourceType};

pub struct GitHubCopilotProvider {
    config_dir: PathBuf,
}

impl GitHubCopilotProvider {
    pub const ID: &'static str = crate::usage::provider_id::GITHUB_COPILOT;

    pub fn new() -> Self {
        Self::with_config_dir(dirs::home_dir().unwrap_or_default().join(".copilot"))
    }

    pub fn with_config_dir(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    fn logs_dir(&self) -> PathBuf {
        self.config_dir.join("logs")
    }

    /// Number of CLI process logs, used only as evidence that the CLI has run.
    fn log_count(&self) -> usize {
        std::fs::read_dir(self.logs_dir())
            .map(|entries| entries.flatten().count())
            .unwrap_or(0)
    }
}

impl Default for GitHubCopilotProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageProvider for GitHubCopilotProvider {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn display_name(&self) -> &'static str {
        "GitHub Copilot"
    }

    fn short_label(&self) -> &'static str {
        "COPILOT"
    }

    fn source_type(&self) -> &'static str {
        SourceType::CopilotUsageSource.as_str()
    }

    fn payment_mode(&self) -> PaymentMode {
        // Copilot is consumed through a subscription entitlement with metered
        // AI credits; it is not a per-token API invoice.
        PaymentMode::Subscription
    }

    fn billing_unit(&self) -> BillingUnit {
        BillingUnit::AiCredits
    }

    fn availability(&self) -> ProviderAvailability {
        if !self.config_dir.exists() {
            return ProviderAvailability::NotInstalled;
        }
        // Installed, but no supported machine-readable usage source.
        ProviderAvailability::UsageUnavailable
    }

    fn detail(&self) -> Option<String> {
        match self.availability() {
            ProviderAvailability::NotInstalled => Some(format!(
                "No GitHub Copilot CLI state at {}",
                self.config_dir.display()
            )),
            ProviderAvailability::UsageUnavailable => Some(format!(
                "{} log(s) found in {}, but no structured usage file. `/usage` output is not scraped.",
                self.log_count(),
                self.logs_dir().display()
            )),
            other => Some(other.label().to_string()),
        }
    }

    fn fetch(&self, _request: &FetchRequest<'_>) -> Result<FetchOutcome, String> {
        // Deliberately fatal rather than empty: reporting "synced 0 records"
        // would imply Copilot is being monitored when it is not.
        Err(
            "GitHub Copilot CLI exposes no supported structured usage source in the installed \
             version; FreeTokenMeter does not parse the interactive `/usage` screen"
                .to_string(),
        )
    }

    fn default_backfill_days(&self) -> i64 {
        30
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_installation_is_reported_as_not_installed() {
        let provider = GitHubCopilotProvider::with_config_dir(PathBuf::from("/definitely/missing/copilot"));
        assert_eq!(provider.availability(), ProviderAvailability::NotInstalled);
        assert!(provider.detail().unwrap().contains("No GitHub Copilot CLI state"));
    }

    #[test]
    fn installed_without_a_structured_source_reports_usage_unavailable() {
        let dir = std::env::temp_dir().join(format!(
            "ftm-copilot-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::write(dir.join("config.json"), "{\"firstLaunchAt\":\"2026-08-13T07:20:10.121Z\"}")
            .unwrap();
        std::fs::write(dir.join("logs/process-1.log"), "started").unwrap();

        let provider = GitHubCopilotProvider::with_config_dir(dir.clone());
        assert_eq!(provider.availability(), ProviderAvailability::UsageUnavailable);
        assert!(provider.detail().unwrap().contains("no structured usage file"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn never_fabricates_usage_records() {
        let provider = GitHubCopilotProvider::with_config_dir(PathBuf::from("/tmp/copilot-nope"));
        let request = FetchRequest {
            cursor: None,
            since_ms: None,
            registry: None,
            limit: 100,
        };
        assert!(provider.fetch(&request).is_err());
    }

    #[test]
    fn bills_in_ai_credits_not_dollars() {
        let provider = GitHubCopilotProvider::new();
        assert_eq!(provider.billing_unit(), BillingUnit::AiCredits);
        assert_eq!(provider.billing_unit().as_str(), "ai_credits");
        assert_eq!(BillingUnit::from_str("ai_credits"), BillingUnit::AiCredits);
        assert_eq!(provider.payment_mode(), PaymentMode::Subscription);
    }

    #[test]
    fn is_not_registered_as_a_supported_provider_yet() {
        // The provider is registered for detection, but reports a state that the
        // UI renders as USAGE UNAVAILABLE — never as CONNECTED.
        let provider = GitHubCopilotProvider::new();
        assert_ne!(provider.availability().as_str(), "connected");
    }
}
