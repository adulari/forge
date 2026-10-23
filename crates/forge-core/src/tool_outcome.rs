//! Per-call turn bookkeeping shared by every serially executed direct-path tool call: a plain
//! model call and each inner call a `run_code` script makes. Keeping it in one place is what
//! makes the phantom-edit gate, the verification ledger, the progress ledger's observations and the
//! env-fight cap see a script's writes and tests exactly as they see the same calls made one per
//! step.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// The turn-scoped counters a tool outcome feeds. Borrowed from the model loop for one step.
pub(crate) struct TurnLedgers<'a> {
    pub(crate) tools_ran: &'a std::sync::Arc<AtomicU64>,
    pub(crate) inspect_ran: &'a std::sync::Arc<AtomicU64>,
    pub(crate) mutations_ran: &'a std::sync::Arc<AtomicU64>,
    pub(crate) verification_ledger: &'a std::sync::Arc<std::sync::Mutex<VerificationLedger>>,
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
        // The `run_code` wrapper is not a tool outcome of its own: its inner calls were observed
        // one by one. It fails only when the script did (its result then starts `error:`), not
        // when it returns a failing test's output.
        let wrapper = call.name == crate::codemode::RUN_CODE_TOOL;
        let failure = if wrapper {
            result
                .trim_start()
                .starts_with("error:")
                .then(|| classify_tool_failure(result))
                .flatten()
        } else {
            classify_tool_failure(result)
        };
        // The footer's elapsed time differs on every run, so it would make every script look
        // like new information to the progress ledger.
        let observed = if wrapper {
            crate::codemode::without_footer(result)
        } else {
            result
        };
        let verification = completion::classify_tool(&call.name, &call.args.to_string());
        let flipped = ledgers.verification_ledger.lock().unwrap().observe_result(
            verification,
            failure.is_none(),
            result,
        );
        let side_effect = self.tools.get(&call.name).map(|tool| tool.side_effect());
        let writes = side_effect == Some(forge_types::SideEffect::Write);
        ledgers.observations.push(
            crate::loop_progress::CallObservation::for_call(
                call,
                failure.is_none(),
                writes,
                observed,
            )
            .with_check(verification.check_family(), flipped),
        );
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
        // Env-fight spend cap: shell commands that look like environment provisioning after a
        // failure are venv archaeology. Allow one alternate recovery attempt, then tell the model
        // once per turn to stop. Delivered via pending_hints so it lands right after this result.
        if self.config.mesh.env_fight_nudge && call.name == "shell" {
            if let Some(cmd) = forge_types::shell_command_text(&call.args) {
                if is_env_setup_command(&cmd)
                    && self.env_fight.observe(shell_command_did_fail(result))
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

impl Session {
    /// Draw a finished tool call's card from its result text.
    pub(crate) fn emit_tool_card(&mut self, name: &str, ok: bool, result: &str) {
        self.presenter.emit(PresenterEvent::ToolResult {
            name: name.to_string(),
            ok,
            summary: summarize(result),
            detail: crate::tool_detail(result),
        });
    }
}
