//! The stored end reason of every turn (docs/features/session-replay.md, "Turn endings"). A `turn` row
//! opens right after the prompt is persisted and closes in `Session::run_turn_with`; the drop
//! guard covers cancellation, and the daemon's start-up sweep covers a killed process.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use forge_store::Store;
use forge_types::{LoopOutcome, PresenterEvent, StopReason};

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
    matches!(
        reason,
        "interrupted" | "daemon_shutdown" | "plan_presented" | "handed_off"
    )
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
        "final_answer" => ("finished", "nothing to do"),
        "final_answer_mid_intent" => (
            "answer left a step undone",
            "send continue to make it do the step",
        ),
        "final_answer_open_tasks" => ("tasks left open", "send continue to finish them"),
        "daemon_restart_or_crash" => (
            "daemon restarted mid-turn",
            "send continue to pick the work back up",
        ),
        "daemon_shutdown" => ("daemon shut down", "send continue once it is back"),
        "interrupted" => ("stopped by you", "send continue to resume"),
        "plan_presented" => ("plan awaiting approval", "approve or revise the plan"),
        "handed_off" => (
            "goal handed to a fresh worker",
            "the goal continues in the successor session",
        ),
        "abandoned" => ("never closed", "send continue to retry"),
        other if other.starts_with("unknown_") => {
            return (
                format!(
                    "earlier history, {}",
                    shape_label(&other["unknown_".len()..])
                ),
                "recorded before end reasons existed".to_string(),
            )
        }
        other => {
            return (
                other.replace('_', " "),
                "send continue to retry".to_string(),
            )
        }
    };
    (label.to_string(), advice.to_string())
}

/// Whether sending `continue` is the right answer to a turn that ended for `reason`: the work
/// is intact and a fresh attempt picks it up. Not the hard stops that need the person to change
/// something first (a budget cap, a pinned model, a blocked review, no working model).
pub fn is_resumable(reason: &str) -> bool {
    matches!(
        reason,
        "stalled"
            | "tasks_unfinished"
            | "provider_down"
            | "provider_error"
            | "interrupted"
            | "daemon_restart_or_crash"
            | "final_answer_mid_intent"
    )
}

/// Whether an unattended run (`forge run`) must report this end as a failed run.
pub fn end_fails_run(reason: &str) -> bool {
    !matches!(
        reason,
        "final_answer" | "plan_presented" | "interrupted" | "handed_off"
    ) && !reason.starts_with("unknown_")
}

/// One turn as the apps and `forge turns` show it: the stored facts plus the plain label and the
/// way out, so no surface prints a raw enum name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TurnEndView {
    pub id: i64,
    pub session_id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub reason: String,
    pub label: String,
    pub advice: String,
    pub shape: Option<String>,
    pub shape_label: Option<String>,
    pub finished: bool,
    pub resumable: bool,
    /// Expert consults this turn spent (`[harness.expert]`); absent when it made none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expert_consults: Option<u32>,
    /// `codex-oauth::gpt-5.6-sol (stall, done)`: who was consulted and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expert: Option<String>,
    /// Scout runs this turn made (`[harness] scouts`); absent when it made none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scouts: Option<u32>,
}

/// The expert consults a stored turn detail records, and a short description of them.
pub fn expert_of_detail(detail_json: Option<&str>) -> (Option<u32>, Option<String>) {
    let Some(detail) = detail_json.and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
    else {
        return (None, None);
    };
    let Some(n) = detail["expert_consults"].as_u64().filter(|n| *n > 0) else {
        return (None, None);
    };
    let model = detail["expert_model"].as_str().unwrap_or("expert");
    let mut triggers: Vec<&str> = Vec::new();
    for t in detail["expert_triggers"].as_array().into_iter().flatten() {
        if let Some(t) = t.as_str().filter(|t| !triggers.contains(t)) {
            triggers.push(t);
        }
    }
    (
        Some(n as u32),
        Some(format!("{model} ({})", triggers.join(", "))),
    )
}

impl TurnEndView {
    pub fn from_row(row: &forge_store::TurnRow) -> Self {
        let reason = row.end_reason.clone().unwrap_or_else(|| "running".into());
        let (label, advice) = if row.end_reason.is_some() {
            describe_end(&reason)
        } else {
            ("still running".into(), "wait for it".into())
        };
        let (expert_consults, expert) = expert_of_detail(row.detail_json.as_deref());
        let scouts =
            serde_json::from_str::<serde_json::Value>(row.detail_json.as_deref().unwrap_or(""))
                .ok()
                .and_then(|d| d["scouts"].as_u64())
                .filter(|n| *n > 0)
                .map(|n| n as u32);
        Self {
            scouts,
            expert_consults,
            expert,
            id: row.id,
            session_id: row.session_id.clone(),
            started_at: row.started_at,
            ended_at: row.ended_at,
            finished: matches!(reason.as_str(), "final_answer" | "plan_presented"),
            resumable: is_resumable(&reason),
            reason,
            label,
            advice,
            shape_label: row.last_role_shape.as_deref().map(shape_label),
            shape: row.last_role_shape.clone(),
        }
    }
}

/// The plain wording of the tail classifier's shape of a closed turn.
pub fn shape_label(shape: &str) -> String {
    match shape {
        "finished" => "answered",
        "unanswered_prompt" => "prompt never answered",
        "pending_tool_calls" => "tool calls left pending",
        "unanswered_tool_results" => "tool results never read",
        other => return other.replace('_', " "),
    }
    .to_string()
}

/// The plain label for an end-reason count row (`final_answer` is "finished", not the enum).
pub fn end_label(reason: &str) -> String {
    if reason == "running" {
        "still running".into()
    } else {
        describe_end(reason).0
    }
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
        CoreError::ProviderDown { .. } => "provider_down",
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
        self.expert.prompt_seq = prompt_seq;
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
        self.fold_bridge_consults();
        if let Some(n) = self.scouts_this_turn() {
            detail["scouts"] = n.into();
        }
        if let Some(expert) = self.expert_turn_detail() {
            if let (Some(map), Some(extra)) = (detail.as_object_mut(), expert.as_object()) {
                map.extend(extra.clone());
            }
        }
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
        self.presenter.turn_end_reason(&reason);
        if reason == "final_answer" || is_self_explained(&reason) {
            return;
        }
        let note = match result {
            Err(error) => {
                let text: String = error.to_string().chars().take(500).collect();
                format!("⚠ Turn failed: {text}")
            }
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
        let wrote = self
            .store
            .note_turn_stopped(&self.id, &note)
            .unwrap_or(false);
        // An error end already reached the surfaces as an Error event, and a stop an earlier note
        // explained must not be said twice.
        if wrote && result.is_ok() {
            self.presenter.emit(PresenterEvent::TurnStopped {
                reason,
                text: note.trim_start_matches("⚠ ").to_string(),
            });
        }
    }

    /// The stored end reason of the session's newest closed turn.
    pub fn last_end_reason(&self) -> Option<String> {
        self.store
            .last_closed_turn(&self.id)
            .ok()
            .flatten()
            .and_then(|row| row.end_reason)
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
