//! Per-call turn bookkeeping shared by every serially executed direct-path tool call: a plain
//! model call and each inner call a `run_code` script makes. Keeping it in one place is what
//! makes the phantom-edit gate, the verification ledger, the failure-loop guard and the env-fight
//! cap see a script's writes and tests exactly as they see the same calls made one per step.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// The turn-scoped counters a tool outcome feeds. Borrowed from the model loop for one step.
pub(crate) struct TurnLedgers<'a> {
    pub(crate) tools_ran: &'a std::sync::Arc<AtomicU64>,
    pub(crate) inspect_ran: &'a std::sync::Arc<AtomicU64>,
    pub(crate) mutations_ran: &'a std::sync::Arc<AtomicU64>,
    pub(crate) verification_ledger: &'a std::sync::Arc<std::sync::Mutex<VerificationLedger>>,
    pub(crate) failure_counts: &'a mut std::collections::HashMap<(String, ErrorCategory), usize>,
    pub(crate) failure_nudged: &'a mut bool,
    pub(crate) observations: &'a mut Vec<crate::loop_progress::CallObservation>,
}

impl TurnLedgers<'_> {
    /// Count a call the direct path is about to run, for the completion gate's progress and
    /// inspection signals. `update_tasks`/`present_plan` are bookkeeping, not inspections.
    pub(crate) fn count_started(&self, name: &str) {
        self.tools_ran.fetch_add(1, Ordering::Relaxed);
        if !name.ends_with("update_tasks") && !name.ends_with("present_plan") {
            self.inspect_ran.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Session {
    /// Record one finished call's outcome in every turn ledger. Hints it queues (the env-fight
    /// nudge) land in `pending_hints`; the caller decides where they are delivered.
    pub(crate) fn observe_tool_outcome(
        &mut self,
        call: &forge_types::ToolCall,
        result: &str,
        ledgers: &mut TurnLedgers<'_>,
    ) {
        let failure = classify_tool_failure(result);
        ledgers.verification_ledger.lock().unwrap().observe(
            completion::classify_tool(&call.name, &call.args.to_string()),
            failure.is_none(),
        );
        let side_effect = self.tools.get(&call.name).map(|tool| tool.side_effect());
        let writes = side_effect == Some(forge_types::SideEffect::Write);
        ledgers
            .observations
            .push(crate::loop_progress::CallObservation::new(
                crate::loop_progress::call_signature(call),
                failure.is_none(),
                writes,
                result,
            ));
        // Count the mutation only once it SUCCEEDED. A denied or failing write changed nothing,
        // so treating the attempt as evidence would let both the phantom-edit gate and the
        // no-output turn classification accept a turn that mutated nothing.
        if failure.is_none()
            && matches!(
                side_effect,
                Some(forge_types::SideEffect::Write | forge_types::SideEffect::Shell)
            )
        {
            ledgers.mutations_ran.fetch_add(1, Ordering::Relaxed);
        }
        match failure {
            Some(kind) => {
                *ledgers
                    .failure_counts
                    .entry((call.name.clone(), kind))
                    .or_insert(0) += 1;
            }
            // A success on this tool means progress — clear its failure streaks so an earlier
            // rough patch doesn't later trip the guard after the model recovered.
            None => {
                // A successful write IS the change of approach the nudge asks for: an edit →
                // compile-error → edit cycle must not read as "failing the same way" just
                // because the failing `cargo test` text matched one category.
                if writes {
                    ledgers.failure_counts.clear();
                }
                ledgers
                    .failure_counts
                    .retain(|(name, _), _| name != &call.name);
                // A genuine success means the model recovered, so a *later* distinct failure
                // loop in the same turn gets its own nudge-before-halt, not an immediate halt.
                *ledgers.failure_nudged = false;
            }
        }
        // Env-fight spend cap: shell commands that look like environment provisioning after a
        // failure are venv archaeology. Allow one alternate recovery attempt, then tell the model
        // once per turn to stop. Delivered via pending_hints so it lands right after this result.
        if self.config.mesh.env_fight_nudge && call.name == "shell" {
            if let Some(cmd) = forge_types::shell_command_text(&call.args) {
                if is_env_setup_command(&cmd)
                    && self.env_fight.observe(shell_command_failed(result))
                {
                    self.presenter.emit(PresenterEvent::Warning(format!(
                        "environment setup/build spend cap reached after a failure \
                         ({ENV_FIGHT_ATTEMPT_THRESHOLD} attempts) — blocking further \
                         provisioning this turn"
                    )));
                    self.pending_hints.push(ENV_FIGHT_NUDGE.to_string());
                }
            }
        }
    }
}
