//! Harness-authored transcript messages: user-role nudges and ephemeral system notes.
//!
//! Both are tagged with a [`MessageKind`] that is persisted with the row, so the request view can
//! tell them from the person's words and from durable context after a resume too.

use forge_types::{Message, MessageKind, Role};

use crate::{context_pack, CoreError, Session};

impl Session {
    /// Append an ephemeral system note of `kind`: persisted and added to the transcript. The
    /// request view keeps only the newest run of each kind (`system_context`).
    pub(crate) fn push_note(
        &mut self,
        kind: MessageKind,
        content: impl Into<String>,
    ) -> Result<(), CoreError> {
        let content = content.into();
        let seq = self.next_seq();
        let stored = self
            .store
            .add_message_kind(&self.id, seq, Role::System, &content, kind);
        self.transcript
            .push(Message::system(content).with_kind(kind));
        stored.map(|_| ()).map_err(CoreError::from)
    }

    /// Append a harness-authored user-role message. It is stored `user` because the next request
    /// must end on a legal user turn, but it is not the task: it never anchors a turn.
    pub(crate) fn push_nudge(&mut self, content: impl Into<String>) -> Result<(), CoreError> {
        let content = content.into();
        let seq = self.next_seq();
        let stored = self.store.add_nudge_message(&self.id, seq, &content);
        self.transcript
            .push(Message::user(content).with_kind(MessageKind::Nudge));
        stored.map(|_| ()).map_err(CoreError::from)
    }

    /// [`Self::inject_context`] for a note whose older copies the request view supersedes.
    pub(crate) fn inject_context_kind(
        &mut self,
        pack: &mut context_pack::ContextPack,
        source: context_pack::ContextSource,
        reason: &str,
        content: &str,
        kind: forge_types::MessageKind,
    ) -> Result<(), CoreError> {
        self.push_note(kind, content)?;
        pack.push(source, reason, content);
        Ok(())
    }
}
