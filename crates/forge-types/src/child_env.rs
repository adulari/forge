//! Environment variables Forge put into its OWN process (provider API keys read from the secret
//! store) that must not leak into the commands it launches for the model. The config layer
//! registers a name when it injects it; the shell tool removes every registered name from a
//! child's environment. Variables the user exported themselves are never registered.

use std::collections::BTreeSet;
use std::sync::Mutex;

static SCRUBBED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

pub fn scrub(var: &str) {
    SCRUBBED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(var.to_string());
}

pub fn unscrub(var: &str) {
    SCRUBBED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(var);
}

pub fn scrubbed_vars() -> Vec<String> {
    SCRUBBED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect()
}
