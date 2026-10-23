//! What the chat loop does with the events a turn has queued for the screen, and whether an Esc
//! pressed now still has a turn to stop.

use std::sync::mpsc::{Receiver, Sender};

use forge_tui::{App, ConfirmReply, UiMsg};
use forge_types::PermissionRule;

use super::{trust_hint, SteerTracker};

/// The notice under a turn the person stopped. Plain about what happened and what comes next:
/// the old "stopped responding" read like a hang.
pub(crate) const INTERRUPTED_NOTE: &str = "⏹ Interrupted. Type what to do instead.";

/// A permission prompt the turn task is blocked on, answered through the sender.
pub(super) type PendingPrompt = Option<(String, Sender<ConfirmReply>)>;

/// The loop state a queued message can change: an open permission prompt or `ask_user` form, and
/// the id that tells a remote client which prompt its answer is for.
pub(super) struct PromptState<'a> {
    pub pending: &'a mut PendingPrompt,
    pub pending_question: &'a mut Option<Sender<Vec<forge_types::Answer>>>,
    pub prompt_seq: &'a mut u64,
    pub trust_hint: TrustHintInputs<'a>,
}

/// What a permission prompt needs to say whether `forge trust` would spare it.
#[derive(Clone, Copy)]
pub(super) struct TrustHintInputs<'a> {
    pub rules: &'a [PermissionRule],
    pub workspace: &'a std::path::Path,
}

/// Apply everything queued so far, in order. `true` when there was anything to apply.
pub(super) fn apply_ui_msgs(
    rx: &Receiver<UiMsg>,
    app: &mut App,
    steer_tracker: &mut SteerTracker,
    queued_prompts: &mut Vec<String>,
    prompts: PromptState<'_>,
) -> bool {
    let mut applied = false;
    while let Ok(msg) = rx.try_recv() {
        applied = true;
        match msg {
            UiMsg::Event(e) => {
                // The running turn consumed a queued prompt: it must not run again as its own
                // turn once this one ends (and an interrupt must still send it, busy_queue.rs).
                steer_tracker.observe(&e, queued_prompts, app);
                app.apply(e)
            }
            UiMsg::Permission {
                tool,
                side_effect,
                reply,
            } => {
                app.open_permission(&tool, &format!("{side_effect:?}"));
                let hint = trust_hint::trust_hint_for(
                    app,
                    &tool,
                    prompts.trust_hint.rules,
                    prompts.trust_hint.workspace,
                );
                app.set_permission_trust_hint(hint);
                *prompts.pending = Some((tool, reply));
                // New prompt, new identity: remote answers rendered against an older prompt now
                // carry a stale seq and are ignored instead of approving this one.
                *prompts.prompt_seq += 1;
            }
            UiMsg::Question { questions, reply } => {
                app.open_form(questions);
                *prompts.pending_question = Some(reply);
                *prompts.prompt_seq += 1;
            }
        }
    }
    applied
}

/// How long Esc waits for an aborted turn task to actually stop. An abort takes effect at the
/// task's next await; one that is already past its last emits its result event and returns at once.
const STOP_WAIT: std::time::Duration = std::time::Duration::from_millis(30);

/// Abort `task` and, when `wait`, give it a moment to stop, so every event it will ever send is
/// already queued when this returns. A task parked on a prompt answer cannot stop until the answer
/// channel is dropped, so the caller passes `wait = false` while one is open.
pub(super) async fn stop_turn_task(task: &mut tokio::task::JoinHandle<()>, wait: bool) {
    task.abort();
    if wait {
        let _ = tokio::time::timeout(STOP_WAIT, task).await;
    }
}

/// Whether the turn the loop still counts as running has in fact ended: its result event is on
/// screen, or its task has exited and only the done signal is still in the channel.
///
/// Esc then has nothing to stop. Treating it as an interrupt threw away the events still queued
/// for the finished answer (its closing lines and usage row) and printed "interrupted" under an
/// answer that was complete. A `/loop` or `/goal` is the exception: Esc also stops the loop that
/// would start the next turn, so it still goes through the interrupt path.
pub(super) fn esc_has_nothing_to_stop(
    result_shown: bool,
    task_exited: bool,
    loop_or_goal_active: bool,
) -> bool {
    (result_shown || task_exited) && !loop_or_goal_active
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The race itself, without a terminal: a task past its last await cannot be aborted before it
    /// sends its result event, so everything it will ever say is queued once `stop_turn_task`
    /// has returned. The blocking sleep stands in for that synchronous tail.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_task_past_its_last_await_has_delivered_its_result_by_the_time_it_is_stopped() {
        let (tx, rx) = std::sync::mpsc::channel::<UiMsg>();
        let mut task = tokio::spawn(async move {
            std::thread::sleep(std::time::Duration::from_millis(10));
            let _ = tx.send(UiMsg::Event(forge_tui::PresenterEvent::Done {
                final_text: String::new(),
                stop_reason: forge_types::StopReason::FinalAnswer,
            }));
            // The first await after the result: where an abort finally takes effect.
            tokio::task::yield_now().await;
        });
        // Let the task start its synchronous tail before Esc arrives.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        stop_turn_task(&mut task, true).await;

        let mut app = App::default();
        let mut pending: PendingPrompt = None;
        let mut question = None;
        let mut seq = 0;
        let applied = apply_ui_msgs(
            &rx,
            &mut app,
            &mut SteerTracker::default(),
            &mut Vec::new(),
            PromptState {
                pending: &mut pending,
                pending_question: &mut question,
                prompt_seq: &mut seq,
                trust_hint: TrustHintInputs {
                    rules: &[],
                    workspace: std::path::Path::new("."),
                },
            },
        );
        assert!(
            applied,
            "the result event was queued before the stop returned"
        );
        assert!(app.done, "applying it marks the turn done");
        assert!(esc_has_nothing_to_stop(app.done, false, false));
    }

    /// A turn parked on its next await is cancelled there and never reports a result.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_task_parked_on_an_await_is_cancelled_without_a_result() {
        let (tx, rx) = std::sync::mpsc::channel::<UiMsg>();
        let mut task = tokio::spawn(async move {
            std::future::pending::<()>().await;
            let _ = tx.send(UiMsg::Event(forge_tui::PresenterEvent::AssistantDone));
        });
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        stop_turn_task(&mut task, true).await;
        assert!(task.is_finished());
        let mut app = App::default();
        let mut pending: PendingPrompt = None;
        let mut question = None;
        let mut seq = 0;
        let applied = apply_ui_msgs(
            &rx,
            &mut app,
            &mut SteerTracker::default(),
            &mut Vec::new(),
            PromptState {
                pending: &mut pending,
                pending_question: &mut question,
                prompt_seq: &mut seq,
                trust_hint: TrustHintInputs {
                    rules: &[],
                    workspace: std::path::Path::new("."),
                },
            },
        );
        assert!(!applied && !app.done);
        assert!(!esc_has_nothing_to_stop(app.done, false, false));
    }

    #[test]
    fn a_turn_that_is_still_running_is_interrupted() {
        assert!(!esc_has_nothing_to_stop(false, false, false));
        assert!(!esc_has_nothing_to_stop(false, false, true));
    }

    #[test]
    fn a_finished_turn_is_left_to_settle_by_either_signal() {
        assert!(esc_has_nothing_to_stop(true, false, false));
        assert!(esc_has_nothing_to_stop(false, true, false));
        assert!(esc_has_nothing_to_stop(true, true, false));
    }

    #[test]
    fn a_loop_or_goal_is_still_stopped_by_esc() {
        assert!(!esc_has_nothing_to_stop(true, false, true));
        assert!(!esc_has_nothing_to_stop(false, true, true));
    }
}
