//! Swapping the provider registries at runtime, and summarising which providers are reachable.

use super::provider_registry::{
    build_custom_registry, leak_azure, load_azure_provider, load_runtime_custom_providers,
    publish_custom_registry, AZURE_REGISTRY, CUSTOM_PROVIDER_REGISTRY,
};
use super::*;

pub(crate) fn custom_registry_signature(providers: &[CustomProvider]) -> Vec<String> {
    providers
        .iter()
        .map(|p| {
            format!(
                "{}|{}|{}|{}|{}|{}",
                p.namespace,
                p.endpoint,
                p.env_var,
                p.free,
                p.label,
                p.seed_models.join(",")
            )
        })
        .collect()
}

/// Re-read `[[providers.custom]]` and `[providers.azure]` from config and swap them in when they
/// differ from what this process holds. Returns whether anything changed. Existing `&'static`
/// references keep pointing at the previous snapshot, which is fine: they are only ever read
/// transiently per request.
pub fn reload_provider_registries() -> bool {
    let fresh = build_custom_registry(&load_runtime_custom_providers());
    let mut changed = false;
    {
        let mut w = CUSTOM_PROVIDER_REGISTRY
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let same = w
            .is_some_and(|cur| custom_registry_signature(cur) == custom_registry_signature(&fresh));
        if !same {
            *w = Some(publish_custom_registry(fresh));
            changed = true;
        }
    }
    let azure = load_azure_provider();
    let mut w = AZURE_REGISTRY.write().unwrap_or_else(|e| e.into_inner());
    let same = matches!(*w, Some(cur) if cur == azure.as_ref());
    if !same {
        *w = Some(leak_azure(azure));
        changed = true;
    }
    changed
}

/// A stable summary of which providers this process can reach: every custom/Azure endpoint
/// definition plus which keyed providers currently hold a key. Anything that makes the set of
/// listable models change (a provider added or removed, a key stored or deleted) changes it, so
/// caches keyed on "what we last swept" can tell their input has moved on.
pub fn provider_set_fingerprint() -> String {
    use std::hash::{Hash, Hasher};
    let mut parts = custom_registry_signature(custom_provider_registry());
    if let Some(az) = azure_registry() {
        parts.push(format!(
            "azure|{}|{}",
            az.endpoint,
            az.deployments.join(",")
        ));
    }
    parts.extend(
        known_key_providers()
            .filter(|p| has_api_key(p))
            .map(|p| format!("key|{p}")),
    );
    parts.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
