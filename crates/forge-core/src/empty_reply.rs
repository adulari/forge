//! Recovery from a reply with neither text nor a tool call.
//!
//! One nudge per empty streak: of 59 measured "your last response was empty" nudges only 11 led
//! to a productive next call, and each one re-sends the whole context. A model that answers the
//! nudge with another empty reply is benched and the turn fails over (or stops honestly on a
//! pinned turn). The nudge re-arms only once the model makes tool progress again, so one transient
//! empty reply late in a long turn does not bench a model that has been working for a hundred
//! steps — before the cap that case got a nudge too, and a pinned turn would otherwise stop.

use super::*;

pub(crate) const EMPTY_REPLY_NUDGE: &str = "Your last response was empty. Continue with the task: \
call a tool to make progress, or state your final answer. Do not reply with an empty message.";

impl Session {
    /// Spend the empty-reply nudge. Returns `false` when it is already spent and the model has
    /// made no tool progress since.
    pub(crate) fn nudge_empty_reply(&mut self) -> bool {
        if self.empty_reply_nudged {
            return false;
        }
        self.empty_reply_nudged = true;
        self.require_tool_call = !crate::nudge_policy::open_titles(&self.tasks).is_empty();
        self.presenter.emit(PresenterEvent::Warning(
            "model returned an empty response — nudging it to continue (once)".to_string(),
        ));
        // Provider chat APIs require a continuation request to end in a user message. Appending
        // this as System after the empty Assistant response made the recovery request invalid
        // (`last message role must be 'user'`).
        let _ = self.push_nudge(EMPTY_REPLY_NUDGE);
        true
    }

    pub(crate) fn rearm_empty_reply_nudge(&mut self) {
        self.empty_reply_nudged = false;
    }
}
