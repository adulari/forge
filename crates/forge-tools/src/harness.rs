//! Process-wide `[harness]` flags for the tools. Tools are unit structs with no config handle, so
//! the CLI installs the resolved flags once at startup. Tests exercise the pure helpers that take
//! the flag as a parameter and never flip these.

use std::sync::atomic::{AtomicBool, Ordering};

use forge_config::HarnessConfig;

static EDIT_ERRORS: AtomicBool = AtomicBool::new(true);
static ARG_COERCION: AtomicBool = AtomicBool::new(true);
static READ_SHAPING: AtomicBool = AtomicBool::new(false);
static SCRIPT_PARALLEL: AtomicBool = AtomicBool::new(false);
static SHELL_AUTOBACKGROUND: AtomicBool = AtomicBool::new(false);

pub fn set_harness(config: &HarnessConfig) {
    EDIT_ERRORS.store(config.edit_errors, Ordering::Relaxed);
    ARG_COERCION.store(config.arg_coercion, Ordering::Relaxed);
    READ_SHAPING.store(config.read_shaping, Ordering::Relaxed);
    SCRIPT_PARALLEL.store(config.script_parallel, Ordering::Relaxed);
    SHELL_AUTOBACKGROUND.store(config.shell_autobackground, Ordering::Relaxed);
}

pub fn edit_errors() -> bool {
    EDIT_ERRORS.load(Ordering::Relaxed)
}
pub fn arg_coercion() -> bool {
    ARG_COERCION.load(Ordering::Relaxed)
}
pub fn read_shaping() -> bool {
    READ_SHAPING.load(Ordering::Relaxed)
}
pub fn script_parallel() -> bool {
    SCRIPT_PARALLEL.load(Ordering::Relaxed)
}
pub fn shell_autobackground() -> bool {
    SHELL_AUTOBACKGROUND.load(Ordering::Relaxed)
}
