//! CLI-bridge session continuity: what the bridge remembers between calls so the next turn can
//! `--resume` the CLI's own session and send only the delta, and the rule that decides when that
//! is still safe.

use forge_types::{Message, Role};

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
    messages_len: usize,
    model: &str,
    checkpoint: Option<&CheckpointContext>,
) -> bool {
    resumes
        && st.session_id.is_some()
        && st.sent <= messages_len
        && st.model == bare_model(model)
        && st.owner.as_deref() == checkpoint.map(|c| c.session.as_str())
        && st.epoch == checkpoint.map_or(0, |c| c.epoch)
}

/// Render only the NEW User/System messages in `tail` (the slice of the transcript not yet sent to a
/// resumed CLI session). Assistant + Tool messages are skipped: the resumed session already holds the
/// model's own prior turn and the tool results it produced, so re-sending Forge's record of them
/// would duplicate. The result is the just the new instruction(s) — a `continue` nudge, or a new user
/// turn. Empty if `tail` carries nothing the model still needs to act on.
pub(super) fn render_resume_delta(tail: &[Message]) -> String {
    let mut out = Vec::new();
    for m in tail {
        match m.role {
            Role::System => out.push(m.content.clone()),
            Role::User => out.push(format!("User: {}", m.content)),
            Role::Assistant | Role::Tool => {} // the resumed session already has these
        }
    }
    out.join("\n\n")
}
