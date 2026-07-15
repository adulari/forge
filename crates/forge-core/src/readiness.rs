//! Shared provider-readiness snapshot for every routing surface.
//!
//! Routing needs two mutable facts: model health and subscription pressure. Keeping their
//! construction here prevents the session, model browser, duel, and subagent paths from applying
//! subtly different plan aliases or conservation policy.

use forge_config::Config;
use forge_store::Store;
use forge_types::{ModelHealth, SubscriptionQuota};

/// The currently routable state of providers, captured atomically enough for one decision.
#[derive(Debug, Clone, Default)]
pub struct ProviderReadiness {
    pub health: ModelHealth,
    pub quota: SubscriptionQuota,
}

impl ProviderReadiness {
    /// Build the one readiness view used by mesh consumers.
    ///
    /// Store reads are best-effort by design: an unavailable local database must not make a
    /// provider request impossible. Empty health/quota is the conservative compatibility default.
    pub fn snapshot(config: &Config, store: &Store) -> Self {
        let health = store.current_benched().unwrap_or_default();
        let quota = store
            .current_quota()
            .unwrap_or_default()
            .with_plans(crate::resolved_subscription_plans_with_store(config, store))
            .with_conserve(config.mesh.subscription_conserve);
        Self { health, quota }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_applies_configured_subscription_conservation() {
        let store = Store::open_in_memory().unwrap();
        let mut config = Config::default();
        config.mesh.subscription_conserve = false;
        let readiness = ProviderReadiness::snapshot(&config, &store);
        assert!(!readiness.quota.conserve_enabled());
        assert!(readiness.health.is_empty());
    }
}
