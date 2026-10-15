//! What happens to prompts typed while a turn is running, once that turn ends or is interrupted.
//!
//! A mid-turn prompt sits in two places: the surface's visible queue, and the session's
//! [`SteerInbox`] so the running turn can absorb it at its next step boundary. When the turn
//! absorbs it, the turn persists it as a user message and emits `Steered`, and the surface drops it
//! from the queue. Two things went wrong at the edges:
//!
//! * An interrupt only re-sent prompts still in the queue. A prompt the turn had already absorbed
//!   was gone from the queue but never answered (the turn died), so Esc "did nothing" — whether
//!   it worked depended on whether a tool step had finished since the prompt was typed.
//! * The `Steered` echo travels over the presenter channel, so it can still be in flight when the
//!   surface handles Esc or the turn's done signal. The surface then still saw the prompt as
//!   queued and ran it a second time.
//!
//! [`SteerTracker`] closes both: it remembers absorbed prompts the model has not started answering
//! yet, and asks the inbox ([`SteerInbox::settle`]) which prompts the turn really absorbed instead
//! of trusting the echoes it has seen so far. Shared by the local TUI loop and the `forge serve`
//! driver so `forge attach`, the web page and the keyboard behave the same.

use forge_core::steer::SteerInbox;
use forge_tui::{App, PresenterEvent};

use super::busy_commands::head_is_command;
use super::dequeue_prompt;

/// What an interrupt starts next.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InterruptNext {
    /// A prompt that never reached the model: an ordinary turn (echo, persist).
    Fresh(String),
    /// Prompts the aborted turn already absorbed (persisted, echoed as steers) but never answered:
    /// the next turn only re-drives the model, it must not echo or persist them again.
    Resume(String),
}

#[derive(Debug, Default)]
pub(crate) struct SteerTracker {
    /// Absorbed prompts the user has seen echoed but the model has not started answering.
    unanswered: Vec<String>,
    /// Every `Steered` echo observed during the current turn.
    observed: Vec<String>,
    /// Prompts taken out of the queue on the inbox's word before their echo arrived. The late
    /// echo is matched here so it cannot remove an unrelated queued prompt with the same text.
    stale: Vec<String>,
}

impl SteerTracker {
    /// Feed every presenter event from the turn through here before applying it.
    pub(crate) fn observe(
        &mut self,
        event: &PresenterEvent,
        queue: &mut Vec<String>,
        app: &mut App,
    ) {
        match event {
            PresenterEvent::Steered(text) => {
                if let Some(i) = self.stale.iter().position(|s| s == text) {
                    self.stale.remove(i);
                    return;
                }
                if let Some(i) = queue.iter().position(|q| q == text) {
                    queue.remove(i);
                    app.set_queued(queue);
                }
                self.observed.push(text.clone());
                self.unanswered.push(text.clone());
            }
            PresenterEvent::AssistantText(t) | PresenterEvent::AssistantDelta(t)
                if !t.trim().is_empty() =>
            {
                self.unanswered.clear();
            }
            PresenterEvent::ToolStart { .. } => self.unanswered.clear(),
            _ => {}
        }
    }

    /// The turn ended on its own, so everything it absorbed was answered. Drop absorbed prompts
    /// whose echo is still in flight from the queue so they do not run again as their own turn.
    pub(crate) fn turn_finished(
        &mut self,
        queue: &mut Vec<String>,
        steer: &SteerInbox,
        app: &mut App,
    ) {
        self.unanswered.clear();
        self.settle(queue, steer, app);
    }

    /// The turn was interrupted. Returns the next turn: every absorbed prompt the model never
    /// started answering (resumed, joined, because the aborted turn died before it could reply),
    /// otherwise the head of the queue.
    pub(crate) fn interrupted(
        &mut self,
        queue: &mut Vec<String>,
        steer: &SteerInbox,
        app: &mut App,
    ) -> Option<InterruptNext> {
        let mut resend = std::mem::take(&mut self.unanswered);
        resend.extend(self.settle(queue, steer, app));
        if resend.is_empty() {
            // A queued command at the head runs through dispatch (the caller's idle pump), not as
            // a model prompt, and keeps its place ahead of later prompts.
            if head_is_command(queue) {
                return None;
            }
            dequeue_prompt(queue, app).map(InterruptNext::Fresh)
        } else {
            Some(InterruptNext::Resume(resend.join("\n\n")))
        }
    }

    /// Reconcile the queue with what the turn actually absorbed; returns the absorbed prompts
    /// whose echo had not arrived yet (now removed from the queue).
    fn settle(
        &mut self,
        queue: &mut Vec<String>,
        steer: &SteerInbox,
        app: &mut App,
    ) -> Vec<String> {
        let mut observed = std::mem::take(&mut self.observed);
        let mut unseen = Vec::new();
        for text in steer.settle() {
            if let Some(i) = observed.iter().position(|o| *o == text) {
                observed.remove(i);
                continue;
            }
            if let Some(i) = queue.iter().position(|q| *q == text) {
                queue.remove(i);
                self.stale.push(text.clone());
                unseen.push(text);
            }
        }
        app.set_queued(queue);
        unseen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue_while_busy(text: &str, queue: &mut Vec<String>, steer: &SteerInbox) {
        queue.push(text.to_string());
        steer.push(text);
    }

    #[test]
    fn interrupt_sends_the_queued_prompt_when_the_turn_never_absorbed_it() {
        let (mut t, steer, mut app) = (
            SteerTracker::default(),
            SteerInbox::default(),
            App::default(),
        );
        let mut queue = Vec::new();
        queue_while_busy("fix the test", &mut queue, &steer);
        assert_eq!(
            t.interrupted(&mut queue, &steer, &mut app),
            Some(InterruptNext::Fresh("fix the test".into()))
        );
        assert!(queue.is_empty());
        assert!(steer.is_empty(), "the aborted turn can no longer absorb it");
    }

    #[test]
    fn interrupt_resends_a_prompt_the_turn_absorbed_but_never_answered() {
        let (mut t, steer, mut app) = (
            SteerTracker::default(),
            SteerInbox::default(),
            App::default(),
        );
        let mut queue = Vec::new();
        queue_while_busy("use the other API", &mut queue, &steer);
        let absorbed = steer.drain();
        t.observe(
            &PresenterEvent::Steered(absorbed[0].clone()),
            &mut queue,
            &mut app,
        );
        assert!(
            queue.is_empty(),
            "the echo dropped it from the visible queue"
        );
        assert_eq!(
            t.interrupted(&mut queue, &steer, &mut app),
            Some(InterruptNext::Resume("use the other API".into())),
            "Esc must still send it: the turn died before answering"
        );
    }

    #[test]
    fn interrupt_resends_an_absorbed_prompt_whose_echo_is_still_in_flight_exactly_once() {
        let (mut t, steer, mut app) = (
            SteerTracker::default(),
            SteerInbox::default(),
            App::default(),
        );
        let mut queue = Vec::new();
        queue_while_busy("same text", &mut queue, &steer);
        steer.drain(); // absorbed; `Steered` not yet received
        queue.push("same text".into()); // a follow-up that never went to the inbox
        assert_eq!(
            t.interrupted(&mut queue, &steer, &mut app),
            Some(InterruptNext::Resume("same text".into()))
        );
        assert_eq!(
            queue,
            vec!["same text"],
            "only the absorbed copy left the queue"
        );
        t.observe(
            &PresenterEvent::Steered("same text".into()),
            &mut queue,
            &mut app,
        );
        assert_eq!(
            queue,
            vec!["same text"],
            "the late echo is matched as stale"
        );
    }

    #[test]
    fn a_prompt_the_model_started_answering_is_not_resent() {
        let (mut t, steer, mut app) = (
            SteerTracker::default(),
            SteerInbox::default(),
            App::default(),
        );
        let mut queue = Vec::new();
        queue_while_busy("first", &mut queue, &steer);
        steer.drain();
        t.observe(
            &PresenterEvent::Steered("first".into()),
            &mut queue,
            &mut app,
        );
        t.observe(
            &PresenterEvent::AssistantDelta("On it".into()),
            &mut queue,
            &mut app,
        );
        queue_while_busy("second", &mut queue, &steer);
        assert_eq!(
            t.interrupted(&mut queue, &steer, &mut app),
            Some(InterruptNext::Fresh("second".into()))
        );
    }

    #[test]
    fn a_finished_turn_drops_absorbed_prompts_whose_echo_is_late() {
        let (mut t, steer, mut app) = (
            SteerTracker::default(),
            SteerInbox::default(),
            App::default(),
        );
        let mut queue = Vec::new();
        queue_while_busy("absorbed", &mut queue, &steer);
        steer.drain();
        queue_while_busy("not yet", &mut queue, &steer);
        t.turn_finished(&mut queue, &steer, &mut app);
        assert_eq!(
            queue,
            vec!["not yet"],
            "the absorbed prompt must not run twice"
        );
        t.observe(
            &PresenterEvent::Steered("absorbed".into()),
            &mut queue,
            &mut app,
        );
        assert_eq!(queue, vec!["not yet"]);
        assert_eq!(
            t.interrupted(&mut queue, &steer, &mut app),
            Some(InterruptNext::Fresh("not yet".into()))
        );
    }
}
