//! Per-turn context that is appended, never rewritten: the turn contract, the clock and the git
//! branch. Each says only what changed since the transcript last said it, so a turn adds a few
//! tokens at the end of the prompt instead of editing something the provider already cached.

use forge_types::Role;

use crate::{clock, context_pack, turn_contract, CoreError, Session};

/// Opens the message that tells the model which branch it is on.
const GIT_BRANCH_PREFIX: &str = "[env] git_branch: ";

impl Session {
    /// The newest system message the transcript holds that starts with `prefix`.
    fn newest_system_with(&self, prefix: &str) -> Option<&str> {
        self.transcript
            .iter()
            .rev()
            .find(|m| m.role == Role::System && m.content.trim_start().starts_with(prefix))
            .map(|m| m.content.as_str())
    }

    /// Append this turn's contract when it differs from the one standing in the transcript.
    pub(crate) fn inject_turn_contract(
        &mut self,
        pack: &mut context_pack::ContextPack,
        contract: &turn_contract::TurnContract,
    ) -> Result<(), CoreError> {
        let standing = self.newest_system_with(crate::system_context::TURN_CONTRACT_PREFIX);
        let Some(message) = turn_contract::contract_message(contract.guidance(), standing) else {
            return Ok(());
        };
        self.inject_context(
            pack,
            context_pack::ContextSource::TurnContract,
            "explicit turn completion contract",
            &message,
        )
    }

    /// The full clock instruction on the first turn (or after compaction folded it away), just the
    /// timestamp afterwards.
    pub(crate) fn inject_turn_clock(
        &mut self,
        pack: &mut context_pack::ContextPack,
    ) -> Result<(), CoreError> {
        let told = self
            .transcript
            .iter()
            .any(|m| m.role == Role::System && clock::is_full_line(&m.content));
        let (line, kind) = if told {
            (clock::short_line(), forge_types::MessageKind::Clock)
        } else {
            (clock::now_line(), forge_types::MessageKind::Normal)
        };
        self.inject_context_kind(
            pack,
            context_pack::ContextSource::Time,
            "current date and time",
            &line,
            kind,
        )
    }

    /// `[harness] append_only_prefix`: the task list the pinned block used to carry, appended to the
    /// transcript instead, only when it differs from the newest snapshot the transcript holds. A
    /// list that changed between turns then adds a few tokens at the end of the prompt instead of
    /// rewriting the block in front of the whole conversation. Returns the note's text when one was
    /// appended.
    pub(crate) fn push_task_snapshot_if_changed(&mut self) -> Result<Option<String>, CoreError> {
        use crate::droid_loop::TASK_SNAPSHOT_PREFIX;
        if !self.config.harness.append_only_prefix || self.tasks.is_empty() {
            return Ok(None);
        }
        let note = format!(
            "{TASK_SNAPSHOT_PREFIX}{}",
            crate::pinned_context::render_task_list(&self.tasks)
        );
        if self.newest_system_with(TASK_SNAPSHOT_PREFIX) == Some(note.as_str()) {
            return Ok(None);
        }
        self.push_note(forge_types::MessageKind::Normal, note.clone())?;
        Ok(Some(note))
    }

    /// Tell the model its git branch when the transcript does not already say it. The branch used
    /// to sit in the `<env>` preamble, where a switch rewrote the very first message of every later
    /// request — a full cache miss for one line of information.
    pub(crate) fn inject_git_branch_if_changed(
        &mut self,
        pack: &mut context_pack::ContextPack,
    ) -> Result<(), CoreError> {
        let Some(branch) = self.cached_git_branch.as_deref() else {
            return Ok(());
        };
        let line = format!("{GIT_BRANCH_PREFIX}{branch}");
        if self.newest_system_with(GIT_BRANCH_PREFIX) == Some(line.as_str()) {
            return Ok(());
        }
        self.inject_context(pack, context_pack::ContextSource::Git, "git branch", &line)
    }
}
