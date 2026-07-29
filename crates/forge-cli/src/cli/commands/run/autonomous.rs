//! Autonomous loop/goal turn lifecycle and background turn spawners.
//!
//! This module owns generation-bound completion and interruption behavior.

use super::*;

/// Context-fill fraction above which a turn-end auto-compact fires (context-compaction.md).
pub(crate) const AUTO_COMPACT_THRESHOLD: f64 = 0.80;

/// The token the model is told to emit when the looped task is fully complete.
pub(crate) const LOOP_DONE_SENTINEL: &str = "LOOP_COMPLETE";

/// Guidance injected on every loop turn: make progress, and signal completion explicitly.
pub(crate) const LOOP_GUIDANCE: &str = "You are running in an autonomous loop. Make concrete progress on the \
task each turn. When — and ONLY when — the task is fully complete, end your final message with \
the token LOOP_COMPLETE on its own line. While work remains, keep going and do NOT emit that token.";

/// Decide whether a loop should stop after a turn. Returns `Some(reason)` to stop (shown to the
/// user), or `None` to run another iteration. Pure so it's unit-testable.
pub(crate) fn loop_stop_reason(last_assistant: Option<&str>, iter: usize) -> Option<&'static str> {
    if last_assistant.is_some_and(|t| t.contains(LOOP_DONE_SENTINEL)) {
        Some("◆ loop complete")
    } else if iter >= LOOP_MAX_ITERS {
        Some("◆ loop stopped — hit the iteration cap")
    } else {
        None
    }
}

/// `/goal` runtime state: the generation of the in-flight goal turn, how many iterations have
/// run, and how many tasks were done as of the last turn — so stalls (no task progress) can be
/// detected alongside completion and the iteration cap.
pub(crate) struct GoalState {
    pub(crate) gen: u64,
    pub(crate) iter: usize,
    pub(crate) prev_done: usize,
    pub(crate) no_progress: usize,
    pub(crate) goal: String,
}

/// Absolute iteration ceiling so a goal that never signals completion can't run forever.
pub(crate) const GOAL_MAX_ITERS: usize = 200;

/// Consecutive turns with no task-list progress before a goal is declared wedged and stopped.
pub(crate) const GOAL_NO_PROGRESS_MAX: usize = 6;

/// Guidance injected on every goal turn: work the tracked plan, never stop for approval, and
/// signal completion explicitly.
pub(crate) const GOAL_GUIDANCE: &str = "You are in autonomous goal mode. Keep working through the \
tracked task plan (update_tasks) one item at a time until the entire goal is met. Never stop to \
ask for approval — you have standing authorization. When (and only when) every task is Done and \
the goal is fully satisfied, reply with one concise final response that states the goal is complete \
and briefly summarizes what was done. Do not emit control sentinels or repeated completion messages.";

/// The prompt each re-drive turn is given once the goal is running autonomously.
pub(crate) const GOAL_CONTINUE_PROMPT: &str = "Continue the goal. Commit/push/PR any finished \
work, then take the single highest-value not-done task and complete it end to end. Keep the \
update_tasks plan current.";

/// Legacy completion marker accepted only as an exact standalone reply. New goal guidance asks
/// for a normal final response and completion is otherwise inferred from the tracked task plan.
pub(crate) fn is_goal_complete_marker(text: Option<&str>) -> bool {
    text.is_some_and(|text| text.trim() == "GOAL COMPLETE")
}

pub(crate) const GOAL_COMPLETE_REASON: &str = "🎯 goal complete";

pub(crate) fn is_goal_complete_reason(reason: &str) -> bool {
    reason == GOAL_COMPLETE_REASON || reason == "🎯 goal complete — all tasks done"
}

/// Remove the oldest prompt submitted while a turn was active, refresh the visible queue, and
/// retain it in prompt history exactly like an idle submission. Shared by ordinary FIFO drain and
/// `/loop`/`/goal` steering so a user correction always owns the very next iteration.
pub(crate) fn dequeue_prompt(
    queue: &mut Vec<String>,
    app: &mut forge_tui::App,
    prompt_history: &mut Vec<String>,
) -> Option<String> {
    let next = queue.first().cloned()?;
    queue.remove(0);
    app.set_queued(queue);
    let history_entry = next.trim();
    if prompt_history.last().map(String::as_str) != Some(history_entry) {
        prompt_history.push(history_entry.to_string());
    }
    Some(next)
}

/// Close every presenter surface owned by an aborted turn. Local and remote interrupts must share
/// this path: the task that would emit `WorkflowFinished` no longer exists, so leaving its workflow
/// active would poison the queued replacement turn.
pub(crate) fn finish_interrupted_presenter(app: &mut forge_tui::App) {
    app.workflow.on_interrupt();
    app.apply(forge_tui::PresenterEvent::AssistantDone);
}

/// A model-skip retries the same logical autonomous turn under a fresh generation. Keep `/loop`
/// or `/goal` attached to that replacement so its completion advances the mode normally; queued
/// user corrections remain untouched and drain after the retried turn.
pub(crate) fn rebind_autonomous_generation(
    loop_state: &mut Option<LoopState>,
    goal_state: &mut Option<GoalState>,
    generation: u64,
) {
    if let Some(state) = loop_state {
        state.gen = generation;
    }
    if let Some(state) = goal_state {
        state.gen = generation;
    }
}

/// Decide whether a goal should stop after a turn. Returns `Some(reason)` to stop (shown to the
/// user), or `None` to run another iteration. Pure so it's unit-testable.
pub(crate) fn goal_stop_reason(
    said_complete: bool,
    done: usize,
    total: usize,
    iter: usize,
    no_progress: usize,
) -> Option<&'static str> {
    if said_complete {
        Some(GOAL_COMPLETE_REASON)
    } else if total > 0 && done == total {
        Some("🎯 goal complete — all tasks done")
    } else if iter >= GOAL_MAX_ITERS {
        Some("🎯 goal stopped — iteration ceiling")
    } else if no_progress >= GOAL_NO_PROGRESS_MAX {
        Some("🎯 goal stalled — no task progress, stopping")
    } else {
        None
    }
}

/// Echo a prompt + spawn the turn task (shared by normal submit and the `//` literal escape).
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_turn(
    prompt: &str,
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
) -> tokio::task::JoinHandle<()> {
    app.on_turn_start();
    app.submit_user(prompt);
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    let prompt = prompt.to_string();
    tokio::spawn(async move {
        // DoneGuard fires on the way out — normal return, panic unwind, OR abort (interrupt) —
        // so the UI can never stay stuck "working". It carries this turn's generation.
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        if let Err(e) = sess.run_turn(&prompt).await {
            sess.notify_error(&format!("turn failed: {e}"));
        }
    })
}

/// Like [`spawn_turn`] but runs an expanded command/skill: prepends `guidance` and biases routing
/// with the `tier` hint. The displayed user line is the original `/command` (echoed by the
/// dispatcher), so the model receives the expanded `prompt` while the transcript shows the turn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_turn_with(
    prompt: String,
    guidance: Vec<String>,
    tier: Option<forge_types::TaskTier>,
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
) -> tokio::task::JoinHandle<()> {
    app.on_turn_start();
    app.submit_user(&prompt);
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    tokio::spawn(async move {
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        if let Err(e) = sess.run_turn_with(&prompt, &guidance, tier).await {
            sess.notify_error(&format!("turn failed: {e}"));
        }
    })
}

/// Spawn `/compact` as a background task (it makes a cheap model call): the spinner ticks while the
/// older transcript is summarized, exactly like a turn.
pub(crate) fn spawn_compact(
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
) -> tokio::task::JoinHandle<()> {
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    tokio::spawn(async move {
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        if let Err(e) = sess.compact(false).await {
            sess.notify_error(&format!("compact failed: {e}"));
        }
    })
}
