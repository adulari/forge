//! Debug trace of what happens between Enter and the first streamed token
//! (`RUST_LOG=forge::turn=debug`). One line per phase; the log's own timestamps give the deltas, so
//! a slow first token can be attributed to a phase instead of guessed at.

use std::sync::atomic::{AtomicBool, Ordering};

static TEXT_SEEN: AtomicBool = AtomicBool::new(false);

pub(crate) fn mark(phase: &str) {
    tracing::debug!(target: "forge::turn", "{phase}");
}

/// The request for `model` is sized and about to leave; the next text delta is its first.
pub(crate) fn dispatching(model: &str) {
    TEXT_SEEN.store(false, Ordering::Relaxed);
    tracing::debug!(target: "forge::turn", "dispatch {model}");
}

/// One boundary inside a model-loop step. With the `dispatch` and `first stream event` lines the
/// deltas split a step into setup before the request, the model's own time, what Forge does with
/// the response, and the tools it then runs.
pub(crate) fn step_phase(step: usize, phase: &str) {
    tracing::debug!(target: "forge::turn", "step {step}: {phase}");
}

pub(crate) fn first_event(kind: &str) {
    tracing::debug!(target: "forge::turn", "first stream event: {kind}");
}

pub(crate) fn text_delta() {
    if !TEXT_SEEN.swap(true, Ordering::Relaxed) {
        tracing::debug!(target: "forge::turn", "first text delta");
    }
}
