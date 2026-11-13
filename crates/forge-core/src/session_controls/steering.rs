//! Prompts that join a running turn: the steer inbox handle and folding queued steers and
//! background-job wakes into the transcript.

use super::*;

impl Session {
    /// A handle a surface keeps to queue prompts INTO a running turn (docs/features/queue-autopilot.md).
    pub fn steer_handle(&self) -> crate::steer::SteerInbox {
        self.steer.clone()
    }

    /// Hand every queued steer prompt to the model as a persisted user message and tell the
    /// surface. Returns whether anything was injected — the loop then continues instead of
    /// treating the last response as the end of the turn.
    ///
    /// `answered` is the text the model just finished streaming when the injection replaces the
    /// end of the turn. That reply is stored provider-only until a turn accepts it as terminal, and
    /// this turn will not end on it, so it is published first: otherwise history, resume and the
    /// phone show the steer straight after the user's prompt with the reply the user watched
    /// stream missing, and the steer's own answer reading as the reply to the wrong message.
    pub(crate) fn inject_steers(&mut self, answered: &str) -> bool {
        let steers = self.steer.drain();
        let wakes = self.job_wake.drain();
        if steers.is_empty() && wakes.is_empty() {
            return false;
        }
        self.ledger.note_steer();
        if let Err(error) = self.publish_terminal_answer(answered) {
            tracing::warn!(%error, "could not publish the reply a steer interrupted");
        }
        self.adaptive_effort.restore();
        for (text, note) in steers
            .into_iter()
            .map(|t| (t, STEER_NOTE))
            .chain(wakes.into_iter().map(|t| (t, JOB_WAKE_NOTE)))
        {
            let seq = self.next_seq();
            let _ = self
                .store
                .add_message(&self.id, seq, Role::User, &text, None);
            self.transcript.push(Message::user(&text));
            // A steer is stored exactly like a typed prompt, so on its own it reads as a new turn
            // in the store. The UI-only note right after it says it is not one.
            let seq = self.next_seq();
            let _ = self.store.add_ui_note(&self.id, seq, Role::System, note);
            self.presenter.emit(PresenterEvent::Steered(text));
        }
        true
    }
}
