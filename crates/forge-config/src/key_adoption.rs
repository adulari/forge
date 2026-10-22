//! Provider keys a parent Forge process resolved, adopted by a child that inherits its environment.

/// Provider-key variables [`super::inject_provider_keys`] (or [`adopt_parent_provider_keys`])
/// populated in this process. Tracked so a re-run (a long-running session noticing `forge auth` in
/// another terminal) can refresh or clear them, while a variable the user exported is never touched.
pub(crate) static INJECTED: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());
static KEYS_RESOLVED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn mark_resolved() {
    KEYS_RESOLVED.store(true, std::sync::atomic::Ordering::Release);
}

/// The key variables this process put into its own environment, or `None` when it never resolved
/// provider keys. A child that inherits this environment can adopt the list instead of repeating
/// every secret-store lookup (see [`adopt_parent_provider_keys`]).
pub fn injected_provider_key_vars() -> Option<Vec<String>> {
    KEYS_RESOLVED
        .load(std::sync::atomic::Ordering::Acquire)
        .then(|| {
            INJECTED
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned()
                .collect()
        })
}

/// Take over the provider keys a parent Forge process resolved, instead of looking them up again.
///
/// A bridged claude hands its MCP servers its whole environment, so `forge mcp-serve` already holds
/// every key its parent found; re-running [`super::inject_provider_keys`] there only repeated ~150
/// secret-store lookups (a D-Bus round trip each, about half a second) for the providers that have
/// none, before the server could answer claude's handshake. `names` is the parent's
/// comma-separated [`injected_provider_key_vars`]. They are registered exactly as if injected here,
/// so the shell tool still strips them from the commands it runs: inherited, they looked
/// user-exported and leaked into every command.
pub fn adopt_parent_provider_keys(names: &str) {
    let mut set = INJECTED.lock().unwrap_or_else(|e| e.into_inner());
    for var in names.split(',').map(str::trim).filter(|v| !v.is_empty()) {
        if super::env_set(var) {
            set.insert(var.to_string());
            forge_types::child_env::scrub(var);
        }
    }
    mark_resolved();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bridge child adopts the keys its parent resolved: it skips the lookups, and the shell tool
    /// still strips the adopted variables from the commands it runs.
    #[test]
    fn adopted_parent_keys_are_registered_like_injected_ones() {
        std::env::set_var("FORGE_TEST_ADOPTED_KEY", "secret");
        std::env::remove_var("FORGE_TEST_ABSENT_KEY");
        adopt_parent_provider_keys("FORGE_TEST_ADOPTED_KEY, FORGE_TEST_ABSENT_KEY,");
        let vars = injected_provider_key_vars().expect("keys count as resolved");
        assert!(vars.iter().any(|v| v == "FORGE_TEST_ADOPTED_KEY"));
        assert!(!vars.iter().any(|v| v == "FORGE_TEST_ABSENT_KEY"));
        assert!(forge_types::child_env::scrubbed_vars()
            .iter()
            .any(|v| v == "FORGE_TEST_ADOPTED_KEY"));
        std::env::remove_var("FORGE_TEST_ADOPTED_KEY");
    }
}
