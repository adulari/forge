//! Recovery from a reply with neither text nor a tool call.
//!
//! One nudge per turn: of 59 measured "your last response was empty" nudges only 11 led to a
//! productive next call, and each one re-sends the whole context. After the one nudge, a further
//! empty reply benches the model and fails over (or stops honestly on a pinned turn).

use super::*;

pub(crate) const EMPTY_REPLY_NUDGE: &str = "Your last response was empty. Continue with the task: \
call a tool to make progress, or state your final answer. Do not reply with an empty message.";

impl Session {
    /// Spend this turn's empty-reply nudge. Returns `false` when it is already spent.
    pub(crate) fn nudge_empty_reply(&mut self) -> bool {
        if self.empty_reply_nudged {
            return false;
        }
        self.empty_reply_nudged = true;
        self.require_tool_call = !crate::nudge_policy::open_titles(&self.tasks).is_empty();
        self.presenter.emit(PresenterEvent::Warning(
            "model returned an empty response — nudging it to continue (once per turn)".to_string(),
        ));
        let nseq = self.next_seq();
        // Provider chat APIs require a continuation request to end in a user message. Appending
        // this as System after the empty Assistant response made the recovery request invalid
        // (`last message role must be 'user'`).
        let _ = self
            .store
            .add_nudge_message(&self.id, nseq, EMPTY_REPLY_NUDGE);
        self.transcript.push(Message::user(EMPTY_REPLY_NUDGE));
        true
    }
}
