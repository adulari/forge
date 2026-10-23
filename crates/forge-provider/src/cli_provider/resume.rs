//! CLI-bridge session continuity: what the bridge remembers between calls so the next turn can
//! `--resume` the CLI's own session and send only the delta, and the rule that decides when that
//! is still safe.

use forge_types::{Message, MessageKind, Role};

use super::{bare_model, CheckpointContext};

/// Cross-call session-continuity state for the bridge (claude `--resume`). After the first turn we
/// hold the CLI's own `session_id` and how many transcript messages it has already seen; the next
/// turn RESUMES that session and sends ONLY the new messages, so claude reloads its context from its
/// own store instead of Forge re-rendering + re-sending the whole transcript every re-drive. That is
/// the headline bridge-efficiency win (fewer tokens in *and* a prompt-cache hit on claude's side).
#[derive(Default)]
pub(super) struct ResumeState {
    /// The CLI session id captured from a prior turn's stream (`None` → next turn is fresh).
    pub(super) session_id: Option<String>,
    /// Count of transcript messages already handed to that session (the resume "high-water mark").
    pub(super) sent: usize,
    /// The bare model the live session was started under. Resuming it under a DIFFERENT model (after
    /// a mesh re-route / failover) would be wrong, so a model change forces a fresh session.
    pub(super) model: String,
    /// The identifying id ([`CheckpointContext::session`]) of the conversation that recorded this
    /// slot. This single `ResumeState` slot is shared by every caller of this `CliProvider`
    /// instance — the main session AND any subagents spawned within it — so without this key a
    /// resume decided for one conversation could `--resume` a DIFFERENT conversation's live
    /// claude/codex session, cross-wiring their turns. `None` when the call carried no checkpoint
    /// context (legacy inherited-env fallback).
    pub(super) owner: Option<String>,
    /// [`CheckpointContext::epoch`] of the conversation when this slot was recorded. A rewind
    /// bumps the epoch without necessarily shrinking the transcript below `sent` (the rewound
    /// prompt is re-sent and the context pack re-injected), so the length check alone cannot
    /// tell a rewritten history from a grown one — but the CLI's own session still holds the
    /// removed turns, and resuming it would put them straight back in front of the model.
    pub(super) epoch: u64,
    /// Identity of the newest messages handed to that session. `sent` is an index into the view of
    /// the transcript at the time, and that view is trimmed from the FRONT as it grows, so the
    /// same index later points at a different message. The mark finds the real position again.
    pub(super) mark: Option<SentMark>,
    /// Fingerprint of the pinned session context the CLI session was last given
    /// ([`pinned_fingerprint`]); `None` when the request carried none. The block sits in the
    /// leading system run, which a resume delta never re-sends, so a change to it is carried in
    /// the delta explicitly.
    pub(super) pinned: Option<u64>,
    /// Codex only: the usage total the session's thread had reported after its last turn. A
    /// resumed `codex exec` reports the thread's running total, so this is what the next turn's
    /// report is measured from (see [`super::codex_usage`]).
    pub(super) codex_total: Option<forge_types::Usage>,
    /// Set when a new Forge process read this state back from disk (see `resume_store`): the
    /// [`prefix_digest`] of everything the CLI session was given. `epoch` counts rewrites inside
    /// one process and says nothing about another, so a restored state is checked against the
    /// transcript itself instead.
    pub(super) restored: Option<u64>,
}

/// Fingerprints of the last two messages a CLI session was sent, which is how a later call finds
/// where the new ones start even after the view was trimmed at the front. Two messages rather than
/// one, because a single message is often a repeat of an earlier one (the same nudge twice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct SentMark {
    last: u64,
    prev: Option<u64>,
}

fn fingerprint(m: &Message) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    m.role.as_str().hash(&mut h);
    m.content.hash(&mut h);
    m.tool_call_id.hash(&mut h);
    for call in &m.tool_calls {
        call.id.hash(&mut h);
    }
    h.finish()
}

/// One fingerprint of a whole request after its leading system run (the system prompt and the
/// pinned context, which a resumed session handles on its own): it matches only when every message
/// the CLI session was given is still there, unchanged and in order.
pub(super) fn prefix_digest(messages: &[Message]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for m in messages.iter().skip_while(|m| m.role == Role::System) {
        fingerprint(m).hash(&mut h);
    }
    h.finish()
}

impl SentMark {
    /// The mark for a call that handed the CLI all of `messages`; `None` for an empty slice.
    pub(super) fn of(messages: &[Message]) -> Option<Self> {
        let last = fingerprint(messages.last()?);
        // The leading system run (system prompt, pinned context) is rendered again for every
        // request and may differ next time, so the first request's mark is its message alone.
        let lead = messages
            .iter()
            .take_while(|m| m.role == Role::System)
            .count();
        let prev = messages
            .len()
            .checked_sub(2)
            .filter(|&i| i >= lead)
            .map(|i| fingerprint(&messages[i]));
        Some(Self { last, prev })
    }

    fn matches_at(&self, messages: &[Message], index: usize) -> bool {
        fingerprint(&messages[index]) == self.last
            && match (self.prev, index.checked_sub(1)) {
                (Some(prev), Some(before)) => fingerprint(&messages[before]) == prev,
                (None, _) => true,
                (Some(_), None) => false,
            }
    }

    /// Index of the first message in `messages` the CLI has not seen, when `sent` messages had
    /// been handed over. Front-trimming only moves the marked message toward the start, so the
    /// unshifted spot is tried first, then earlier ones; more than one candidate there means the
    /// position is ambiguous, and `None` makes the caller start a fresh session instead of
    /// risking a skipped message.
    pub(super) fn locate(&self, messages: &[Message], sent: usize) -> Option<usize> {
        let top = sent.checked_sub(1)?.min(messages.len().checked_sub(1)?);
        if sent <= messages.len() && self.matches_at(messages, sent - 1) {
            return Some(sent);
        }
        let mut found = (0..=top).rev().filter(|&i| self.matches_at(messages, i));
        let first = found.next()?;
        found.next().is_none().then_some(first + 1)
    }
}

/// The pinned session context (project instructions, memories, goal, task list) in a request.
fn pinned_block(messages: &[Message]) -> Option<&Message> {
    messages
        .iter()
        .take_while(|m| m.role == Role::System)
        .find(|m| m.kind == MessageKind::Pinned)
}

/// A fingerprint of the pinned block, `None` when the request has none.
pub(super) fn pinned_fingerprint(messages: &[Message]) -> Option<u64> {
    pinned_block(messages).map(fingerprint)
}

/// The pinned block as a resumed CLI session has to be told, when it is not what that session was
/// last given. A fresh render carries it in the system preamble; a delta carries only what came
/// after, so without this a changed task list or edited AGENTS.md never reaches a resumed CLI.
pub(super) fn pinned_update(messages: &[Message], sent: Option<u64>) -> Option<String> {
    let block = pinned_block(messages)?;
    (Some(fingerprint(block)) != sent).then(|| {
        format!(
            "Updated session context (replaces the earlier version):\n{}",
            block.content
        )
    })
}

/// Where the not-yet-sent messages start in `messages`, for a slot recorded with `sent` messages
/// and `mark`. `Some(0)` when nothing was sent. Without a mark (a slot recorded without one) the
/// raw count is trusted only while it still fits.
pub(super) fn locate_sent(
    sent: usize,
    mark: Option<SentMark>,
    messages: &[Message],
) -> Option<usize> {
    if sent == 0 {
        return Some(0);
    }
    match mark {
        Some(mark) => mark.locate(messages, sent),
        None => (sent <= messages.len()).then_some(sent),
    }
}

/// Whether the recorded CLI session may be resumed for a call over `messages_len` messages of
/// `model` from the conversation `checkpoint` describes. Every clause guards a way the CLI's
/// server-side history could diverge from the transcript Forge is about to send a delta of: the
/// transcript shrank (compaction/reset), the model changed, another conversation shares this
/// provider instance, or the history was rewritten in place (`/rewind`, `/uncompact` — the
/// epoch), which can leave the length equal while the content differs.
pub(super) fn allowed(
    resumes: bool,
    st: &ResumeState,
    messages: &[Message],
    model: &str,
    checkpoint: Option<&CheckpointContext>,
) -> bool {
    resumes
        && st.session_id.is_some()
        && locate_sent(st.sent, st.mark, messages).is_some()
        && st.model == bare_model(model)
        // A call with no checkpoint context has no conversation identity (subagents, legacy
        // callers): two of them would match each other's `None` owner and cross-wire transcripts.
        && checkpoint.is_some_and(|c| st.owner.as_deref() == Some(c.session.as_str()))
        && match st.restored {
            Some(prefix) => locate_sent(st.sent, st.mark, messages)
                .is_some_and(|at| prefix_digest(&messages[..at]) == prefix),
            None => st.epoch == checkpoint.map_or(0, |c| c.epoch),
        }
}

/// What a live process is told when it is driven again with nothing new in the transcript. Its own
/// history already holds everything, so the honest instruction is to carry on — not a re-render of
/// the transcript, which it would take for a fresh request and answer with "nothing new".
pub(super) const CONTINUE_PROMPT: &str = "Continue from where you left off. Nothing new has been added to the conversation since your last reply; finish the work, or give your final answer if you are done.";

/// Render only the NEW User/System messages in `tail` (the slice of the transcript not yet sent to a
/// resumed CLI session). Assistant + Tool messages are skipped: the resumed session already holds the
/// model's own prior turn and the tool results it produced, so re-sending Forge's record of them
/// would duplicate. The result is the just the new instruction(s) — a `continue` nudge, or a new user
/// turn. Empty if `tail` carries nothing the model still needs to act on.
pub(super) fn render_resume_delta(tail: &[Message], pinned: Option<String>) -> String {
    let mut out: Vec<String> = Vec::new();
    for m in tail {
        match m.role {
            Role::System => out.push(m.content.clone()),
            Role::User => out.push(format!("User: {}", m.content)),
            Role::Assistant | Role::Tool => {} // the resumed session already has these
        }
    }
    if !out.is_empty() {
        out.splice(0..0, pinned);
    }
    out.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msgs(texts: &[&str]) -> Vec<Message> {
        texts.iter().map(|t| Message::user(*t)).collect()
    }

    #[test]
    fn the_delta_starts_after_the_last_sent_message_even_when_the_front_was_trimmed() {
        let sent_view = msgs(&["m0", "m1", "m2", "m3", "m4", "m5"]);
        let mark = SentMark::of(&sent_view);

        // Three messages were trimmed off the front while two new ones arrived: the old count (6)
        // now points past the end of a 5-message view and would skip both.
        let next = msgs(&["m3", "m4", "m5", "new-a", "new-b"]);
        let at = locate_sent(6, mark, &next).expect("the marked message is still in the view");
        assert_eq!(at, 3);
        assert_eq!(
            render_resume_delta(&next[at..], None),
            "User: new-a\n\nUser: new-b"
        );

        // The same count with a trim shorter than the growth is the dangerous case: the index
        // is in range and lands on a message the CLI already has.
        let next = msgs(&["m1", "m2", "m3", "m4", "m5", "new-a", "new-b", "new-c"]);
        let at = locate_sent(6, mark, &next).unwrap();
        assert_eq!(at, 5, "not index 6, which would have skipped new-a");
    }

    #[test]
    fn an_untrimmed_view_resumes_at_the_old_count() {
        let sent_view = msgs(&["a", "b", "c"]);
        let mark = SentMark::of(&sent_view);
        let next = msgs(&["a", "b", "c", "d"]);
        assert_eq!(locate_sent(3, mark, &next), Some(3));
    }

    #[test]
    fn a_missing_or_ambiguous_mark_falls_back_to_a_fresh_session() {
        let mark = SentMark::of(&msgs(&["x", "y", "z"]));
        // The marked message is gone (elided, compacted away).
        assert_eq!(locate_sent(3, mark, &msgs(&["p", "q", "r", "s"])), None);

        // Two identical consecutive pairs: the position cannot be told, so do not guess.
        let mark = SentMark::of(&msgs(&["n", "n"]));
        assert_eq!(
            locate_sent(2, mark, &msgs(&["n", "n", "n", "n", "tail"])).unwrap(),
            2,
            "the unshifted position is tried first and is correct when nothing moved"
        );
        assert_eq!(
            locate_sent(6, mark, &msgs(&["n", "n", "n", "n", "tail"])),
            None
        );
    }

    #[test]
    fn nothing_sent_means_a_fresh_render() {
        assert_eq!(locate_sent(0, None, &msgs(&["a"])), Some(0));
    }
}
