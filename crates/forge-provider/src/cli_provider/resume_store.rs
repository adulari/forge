//! The claude bridge's resume state on disk, so the next Forge process continues the same claude
//! session instead of starting a fresh one.
//!
//! `forge run --resume <id>` (and a daemon restart) builds a new provider with no memory of the
//! claude session the previous process drove. The bridge then rendered the transcript as text into
//! a fresh claude session, and Forge's transcript of a bridged turn holds only the user and
//! assistant text: every file the model had read and every command it had run was gone. In the
//! multi-turn benchmark 57 of 76 `read_file` calls of `claude-cli::sonnet` re-read a file an earlier
//! prompt had already read or written. `claude --resume` keeps all of it, as Claude Code's own
//! `claude -p --resume` does.
//!
//! The record is written after each successful bridge call and removed when a call starts, so a
//! call that fails or a process that dies mid-turn leaves nothing to resume into a history that
//! went further than Forge's transcript. A record is only used when the transcript still holds,
//! unchanged, every message the claude session was given ([`super::resume::prefix_digest`]).

use std::path::{Path, PathBuf};

use super::resume::{self, ResumeState, SentMark};
use super::*;

/// Directory under Forge's data directory that holds one record per conversation.
pub(super) const DIR: &str = "bridge-sessions";

#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    session_id: String,
    sent: usize,
    mark: Option<SentMark>,
    pinned: Option<u64>,
    model: String,
    owner: String,
    workspace: String,
    prefix: u64,
}

fn record_path(dir: &Path, owner: &str) -> PathBuf {
    let name: String = owner
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{name}.json"))
}

fn save(dir: &Path, record: &Record) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = record_path(dir, &record.owner);
    let tmp = path.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec(record)?)?;
    std::fs::rename(&tmp, &path)
}

fn load(dir: &Path, owner: &str, workspace: &str) -> Option<ResumeState> {
    let raw = std::fs::read(record_path(dir, owner)).ok()?;
    let r: Record = serde_json::from_slice(&raw).ok()?;
    if r.owner != owner || r.workspace != workspace {
        return None;
    }
    Some(ResumeState {
        session_id: Some(r.session_id),
        sent: r.sent,
        mark: r.mark,
        pinned: r.pinned,
        model: r.model,
        owner: Some(r.owner),
        epoch: 0,
        codex_total: None,
        restored: Some(r.prefix),
    })
}

impl CliProvider {
    /// Where this provider keeps its records: claude only, and only with resume on.
    fn resume_records(&self) -> Option<&Path> {
        (self.kind == CliKind::ClaudeCode && self.resumes())
            .then_some(self.resume_dir.as_deref())
            .flatten()
    }

    /// Fill an empty resume slot from the record a previous process left for this conversation,
    /// when that record can be resumed for `messages`. Removes the record either way: from here on
    /// this process owns the claude session, and writes a new record when the call succeeds.
    pub(super) fn restore_resume(
        &self,
        st: &mut ResumeState,
        messages: &[Message],
        model: &str,
        checkpoint: Option<&CheckpointContext>,
    ) -> bool {
        let (Some(dir), Some(c)) = (self.resume_records(), checkpoint) else {
            return false;
        };
        let restored = if st.session_id.is_none() || st.owner.as_deref() != Some(&c.session) {
            load(dir, &c.session, &c.workspace)
        } else {
            None
        };
        let _ = std::fs::remove_file(record_path(dir, &c.session));
        let Some(candidate) = restored else {
            return false;
        };
        let usable = resume::allowed(true, &candidate, messages, model, checkpoint);
        tracing::debug!(target: "forge::turn", usable, "bridge: claude session record from an earlier process");
        if usable {
            *st = candidate;
        }
        usable
    }

    /// Write the record for the claude session a successful call just left in `st`.
    pub(super) fn persist_resume(
        &self,
        st: &ResumeState,
        messages: &[Message],
        checkpoint: Option<&CheckpointContext>,
    ) {
        let (Some(dir), Some(c), Some(session_id)) =
            (self.resume_records(), checkpoint, st.session_id.clone())
        else {
            return;
        };
        let record = Record {
            session_id,
            sent: st.sent,
            mark: st.mark,
            pinned: st.pinned,
            model: st.model.clone(),
            owner: c.session.clone(),
            workspace: c.workspace.clone(),
            prefix: resume::prefix_digest(messages),
        };
        if let Err(e) = save(dir, &record) {
            tracing::debug!("bridge: writing the claude session record failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(session: &str, workspace: &str) -> CheckpointContext {
        CheckpointContext {
            session: session.into(),
            seq: 1,
            root: String::new(),
            workspace: workspace.into(),
            mode: String::new(),
            epoch: 0,
        }
    }

    fn provider(dir: &Path) -> CliProvider {
        CliProvider::claude_code().with_resume_dir(Some(dir.to_path_buf()))
    }

    fn sent_state(messages: &[Message], owner: &str) -> ResumeState {
        ResumeState {
            session_id: Some("claude-1".into()),
            sent: messages.len(),
            mark: SentMark::of(messages),
            model: "sonnet".into(),
            owner: Some(owner.into()),
            ..ResumeState::default()
        }
    }

    #[test]
    fn a_new_process_resumes_the_claude_session_the_last_one_left() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx("conv-1", "/ws");
        let first = vec![Message::system("system prompt"), Message::user("read a.py")];
        provider(dir.path()).persist_resume(&sent_state(&first, "conv-1"), &first, Some(&c));

        // The next process renders a different system prompt, then the same history plus the
        // model's answer and a new request.
        let mut next = vec![Message::system("system prompt, rendered again")];
        next.extend(first[1..].iter().cloned());
        next.push(Message::assistant("a.py defines main()"));
        next.push(Message::user("now edit it"));

        let p = provider(dir.path());
        let mut st = ResumeState::default();
        assert!(p.restore_resume(&mut st, &next, "claude-cli::sonnet", Some(&c)));
        assert_eq!(st.session_id.as_deref(), Some("claude-1"));
        let at = resume::locate_sent(st.sent, st.mark, &next).unwrap();
        assert_eq!(
            resume::render_resume_delta(&next[at..], None),
            "User: now edit it"
        );
        assert!(
            !record_path(dir.path(), "conv-1").exists(),
            "this process owns the session now; a failed call must leave nothing to resume"
        );
    }

    #[test]
    fn a_record_is_not_resumed_into_a_different_history_workspace_or_model() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx("conv-1", "/ws");
        let first = vec![Message::user("one"), Message::user("two")];
        let st = sent_state(&first, "conv-1");
        let p = provider(dir.path());

        // An earlier message changed (a rewind, an edit): the claude session would hold the old one.
        p.persist_resume(&st, &first, Some(&c));
        let rewritten = vec![
            Message::user("ONE"),
            Message::user("two"),
            Message::user("three"),
        ];
        assert!(!p.restore_resume(
            &mut ResumeState::default(),
            &rewritten,
            "claude-cli::sonnet",
            Some(&c)
        ));

        // Another workspace, or another model.
        let grown = vec![
            Message::user("one"),
            Message::user("two"),
            Message::user("three"),
        ];
        p.persist_resume(&st, &first, Some(&c));
        let elsewhere = ctx("conv-1", "/other");
        assert!(!p.restore_resume(
            &mut ResumeState::default(),
            &grown,
            "claude-cli::sonnet",
            Some(&elsewhere)
        ));
        p.persist_resume(&st, &first, Some(&c));
        assert!(!p.restore_resume(
            &mut ResumeState::default(),
            &grown,
            "claude-cli::opus",
            Some(&c)
        ));

        // The same history grown by a new request is resumed.
        p.persist_resume(&st, &first, Some(&c));
        assert!(p.restore_resume(
            &mut ResumeState::default(),
            &grown,
            "claude-cli::sonnet",
            Some(&c)
        ));
    }

    #[test]
    fn a_live_slot_of_the_same_conversation_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx("conv-1", "/ws");
        let first = vec![Message::user("one")];
        let p = provider(dir.path());
        p.persist_resume(&sent_state(&first, "conv-1"), &first, Some(&c));
        let mut live = sent_state(&first, "conv-1");
        live.session_id = Some("claude-live".into());
        let grown = vec![Message::user("one"), Message::user("two")];
        assert!(!p.restore_resume(&mut live, &grown, "claude-cli::sonnet", Some(&c)));
        assert_eq!(live.session_id.as_deref(), Some("claude-live"));
    }

    #[test]
    fn nothing_is_written_without_a_record_directory_or_for_codex() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx("conv-1", "/ws");
        let first = vec![Message::user("one")];
        let st = sent_state(&first, "conv-1");
        CliProvider::codex()
            .with_resume_dir(Some(dir.path().to_path_buf()))
            .persist_resume(&st, &first, Some(&c));
        CliProvider::claude_code()
            .with_resume_dir(None)
            .persist_resume(&st, &first, Some(&c));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
