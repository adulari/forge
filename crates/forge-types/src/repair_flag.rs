//! Whether the provider layer may repair slightly malformed tool-call JSON (`[harness]
//! arg_coercion`). Lives here because the CLI that reads the config and the provider that acts on
//! it do not depend on each other.

use std::sync::atomic::{AtomicBool, Ordering};

static JSON_REPAIR: AtomicBool = AtomicBool::new(true);

pub fn set_json_repair(enabled: bool) {
    JSON_REPAIR.store(enabled, Ordering::Relaxed);
}

pub fn json_repair() -> bool {
    JSON_REPAIR.load(Ordering::Relaxed)
}
