//! The stored end reason of every turn (docs/features/session-replay.md, "Turn endings"). A `turn` row
//! opens right after the prompt is persisted and closes in `Session::run_turn_with`; the drop
//! guard covers cancellation, and the daemon's start-up sweep covers a killed process.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use forge_store::Store;
use forge_types::{LoopOutcome, StopReason};

use crate::hooks_turn::stop_failure_kind;
use crate::mid_intent::ends_mid_intent;
use crate::{CoreError, Session};

/// Closes the turn as `interrupted` when the turn future is dropped before it finished: Esc, a
/// remote interrupt, a queued prompt sent now, a task abort. Disarmed by the normal close.
pub(crate) struct TurnEndGuard {
    store: Arc<Store>,
    row: Arc<AtomicI64>,
}

impl TurnEndGuard {
    pub(crate) fn new(store: Arc<Store>, row: Arc<AtomicI64>) -> Self {
        Self { store, row }
    }
}

impl Drop for TurnEndGuard {
    fn drop(&mut self) {
        let id = self.row.swap(0, Ordering::SeqCst);
        if id != 0 {
            let _ = self
                .store
                .close_turn(id, "interrupted", Some("{\"cancelled\":true}"));
        }
    }
}

/// Reason strings that need no note: the person (or the restart machinery) already got one.
fn is_self_explained(reason: &str) -> bool {
    matches!(reason, "interrupted" | "daemon_shutdown" | "plan_presented")
}

/// `(plain reason, what to do)` for a non-success end. Unknown reasons fall back to the raw
/// string, so a new `StopReason` shows up readably before it gets its own line here.
pub fn describe_end(reason: &str) -> (String, String) {
    let (label, advice) = match reason {
        "no_output" => (
            "no answer",
            "nothing was changed; send continue to retry or rephrase",
        ),
        "stalled" => (
            "no progress",
            "edits are kept; send continue to start a fresh attempt",
        ),
        "tasks_unfinished" => ("tasks left open", "send continue to finish them"),
        "max_steps" => ("step limit reached", "send continue to keep going"),
        "budget_exhausted" => (
            "budget cap reached",
            "raise the cap in settings or send continue",
        ),
        "provider_down" => (
            "model provider unreachable",
            "send continue once the provider is back",
        ),
        "provider_error" => ("model request failed", "send continue to retry"),
        "no_healthy_model" => (
            "no working model",
            "check your keys and /models, then send continue",
        ),
        "pinned_rejected" => ("pinned model unavailable", "pick another with /model"),
        "review_blocked" => (
            "review gate blocked",
            "fix the findings, then send continue",
        ),
        "hook_blocked" => (
            "a hook blocked it",
            "check the hook output, then send continue",
        ),
        "internal" => ("internal error", "send continue to retry"),
        other => {
            return (
                other.replace('_', " "),
                "send continue to retry".to_string(),
            )
        }
    };
    (label.to_string(), advice.to_string())
}

/// The note text for a non-success end.
pub fn stop_note(reason: &str) -> String {
    let (label, advice) = describe_end(reason);
    format!("⚠ Turn stopped ({label}): {advice}")
}

/// `(end_reason, error kind)` for a turn that ended in `Err`.
pub(crate) fn error_reason(error: &CoreError) -> (&'static str, Option<&'static str>) {
    let kind = stop_failure_kind(error);
    let reason = match error {
        CoreError::Provider(_) => "provider_error",
        CoreError::NoHealthyModel { .. } => "no_healthy_model",
        CoreError::ModelUnavailable { .. } => "pinned_rejected",
        CoreError::TurnBlocked(_) => "review_blocked",
        _ => "internal",
    };
    (reason, kind)
}

impl Session {
    /// Open the turn row for the prompt just persisted at `prompt_seq`. Steers and nudges never
    /// call this: they are not prompts. A turn still open here is the plan turn whose approval
    /// re-runs the build turn.
    pub(crate) fn open_turn_row(&mut self, prompt_seq: i64) {
        let previous = self.turn_row.swap(0, Ordering::SeqCst);
        if previous != 0 {
            let _ = self.store.close_turn(previous, "plan_presented", None);
        }
        // Rows another process left open for this session (a crashed terminal) can never close.
        let _ = self
            .store
            .close_open_session_turns_as(&self.id, "abandoned");
        if let Ok(id) = self.store.open_turn(&self.id, prompt_seq) {
            self.turn_row.store(id, Ordering::SeqCst);
        }
    }

    pub(crate) fn turn_end_guard(&self) -> TurnEndGuard {
        TurnEndGuard::new(self.store.clone(), self.turn_row.clone())
    }

    /// Close the open turn row from the turn's result and, for any end that is not a normal
    /// answer, leave the note that says why. Runs for every surface: the TUI, headless `forge
    /// run`, the daemon, subagents.
    pub(crate) fn finish_turn_row(&mut self, result: &Result<LoopOutcome, CoreError>) {
        let id = self.turn_row.swap(0, Ordering::SeqCst);
        if id == 0 {
            return;
        }
        let open_tasks = self.open_task_count();
        let mut detail = serde_json::json!({
            "model": self.harness_model,
            "open_tasks": open_tasks,
            "mutations": self.mutations_this_turn,
        });
        let reason: String = match result {
            Ok(outcome) => match outcome.stop_reason {
                StopReason::FinalAnswer if open_tasks > 0 => "final_answer_open_tasks".into(),
                StopReason::FinalAnswer if ends_mid_intent(&outcome.text) => {
                    detail["mid_intent"] = true.into();
                    "final_answer_mid_intent".into()
                }
                other => other.as_str().to_string(),
            },
            Err(error) => {
                let (reason, kind) = error_reason(error);
                if let Some(kind) = kind {
                    detail["error_kind"] = kind.into();
                }
                reason.to_string()
            }
        };
        let _ = self
            .store
            .close_turn(id, &reason, Some(&detail.to_string()));
        if reason == "final_answer" || is_self_explained(&reason) {
            return;
        }
        let note = match result {
            Err(error) => {
                let text: String = error.to_string().chars().take(500).collect();
                format!("⚠ Turn failed: {text}")
            }
            Ok(_) if reason.starts_with("final_answer") => return,
            // A hard guard or forced stop already published its reason as the turn's last
            // message (model_loop.rs); a second note would repeat it.
            Ok(outcome)
                if (self.turn_hard_guard_abort || self.turn_forced_stop.is_some())
                    && !outcome.text.trim().is_empty() =>
            {
                return
            }
            Ok(_) => stop_note(&reason),
        };
        let _ = self.store.note_turn_stopped(&self.id, &note);
    }

    fn open_task_count(&self) -> usize {
        let stored = self.store.tasks(&self.id).unwrap_or_default();
        let tasks = if stored.is_empty() {
            &self.tasks
        } else {
            &stored
        };
        tasks
            .iter()
            .filter(|t| !matches!(t.status, forge_types::TodoStatus::Done))
            .count()
    }
}
