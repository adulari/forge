//! Applies `[git] commit_policy` to the base system prompt.
//!
//! `FORGE_SYSTEM` carries the `unit` wording (commit each verified unit as it is finished). For
//! `end` and `never` the paragraph between [`START`] and [`END`] is swapped out at request time, so
//! the prompt constant stays the single source of truth for the default.

use std::borrow::Cow;

use forge_config::CommitPolicy;

/// First words of the commit paragraph in `FORGE_SYSTEM`.
const START: &str = "- In a git repository, commit each verified unit";
/// First words of the bullet that follows it (the push rule, which every policy keeps).
const END: &str = "- Never push, force-push";

const COMMIT_AT_END: &str = "\
- In a git repository, do not commit as you go. Make your edits, verify them, and commit once \
when the whole task is complete: stage the specific files you changed (never `git add -A`), write \
one focused conventional-commit message (feat/fix/refactor/docs/test/chore), and leave unrelated \
changes in the tree alone. If the work is not ready to commit, say so.
";

const COMMIT_ON_REQUEST: &str = "\
- In a git repository, do not commit on your own initiative: the user approves each command here \
and has not asked for a commit. Leave your edits in the working tree. When the user asks you to \
commit (or runs /commit), stage the specific files you changed (never `git add -A`), write one \
focused conventional-commit message (feat/fix/refactor/docs/test/chore), and never pass \
`--no-verify`.
";

const NEVER_COMMIT: &str = "\
- Do not create commits. Never run `git commit` or `git add`; leave every change in the working \
tree for the user to review and commit themselves. Reading git state (status, diff, log) is fine.
";

impl crate::Session {
    /// The policy this session acts on: the configured one, with `auto` resolved to the surface's
    /// default (`unit`, or `end` once a headless one-shot run has said so). An interactive `auto`
    /// session whose own commit would stop at a permission prompt resolves to `request` instead:
    /// the model that commits "each verified unit" costs the user an extra approval after every
    /// edit, where Claude Code commits nothing unasked. An allow rule for `git add`/`git commit`,
    /// "allow for this session", or a mode that runs shell without asking brings `unit` back, so
    /// the discipline stays wherever committing is free.
    pub(crate) fn commit_policy(&self) -> CommitPolicy {
        let chosen = self.config.git.commit_policy.or(self.default_commit_policy);
        if self.config.git.commit_policy == CommitPolicy::Auto
            && chosen == CommitPolicy::Unit
            && self.model_commit_would_ask()
        {
            return CommitPolicy::Request;
        }
        chosen
    }

    /// Whether the permission broker would stop to ask before the model's own commit.
    fn model_commit_would_ask(&self) -> bool {
        let workspace = self.workspace_binding.read().ok().map(|g| g.clone());
        let stage_and_commit = serde_json::json!({
            "command": "git add file && git commit -m message"
        });
        crate::permission::decide_in(
            self.mode,
            forge_types::SideEffect::Shell,
            "shell",
            &stage_and_commit,
            &self.rules,
            workspace.as_deref(),
        ) == forge_types::PermissionDecision::Ask
    }

    /// What `[git] commit_policy = "auto"` means for this session. Headless one-shot runs
    /// (`forge run`, `forge bench`) pass `End`: nobody reviews per-unit commits there, and each
    /// one costs a model round trip. An explicit `commit_policy` in the config still wins.
    pub fn set_default_commit_policy(&mut self, policy: CommitPolicy) {
        self.default_commit_policy = policy;
    }
}

/// `base` with the version-control paragraph rewritten for `policy`. `unit` (and a base that has
/// no such paragraph, e.g. a user-supplied override) is returned untouched, as is an unresolved
/// `auto`.
pub(crate) fn apply(base: &str, policy: CommitPolicy) -> Cow<'_, str> {
    let replacement = match policy {
        CommitPolicy::Unit | CommitPolicy::Auto => return Cow::Borrowed(base),
        CommitPolicy::End => COMMIT_AT_END,
        CommitPolicy::Request => COMMIT_ON_REQUEST,
        CommitPolicy::Never => NEVER_COMMIT,
    };
    let (Some(start), Some(end)) = (base.find(START), base.find(END)) else {
        return Cow::Borrowed(base);
    };
    if end <= start {
        return Cow::Borrowed(base);
    }
    Cow::Owned(format!("{}{replacement}{}", &base[..start], &base[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FORGE_SYSTEM;

    #[test]
    fn unit_is_the_unmodified_default() {
        assert!(matches!(
            apply(FORGE_SYSTEM, CommitPolicy::Unit),
            Cow::Borrowed(_)
        ));
        assert!(FORGE_SYSTEM.contains("commit each verified unit of work"));
    }

    #[test]
    fn markers_exist_in_the_real_prompt() {
        assert!(FORGE_SYSTEM.contains(START));
        assert!(FORGE_SYSTEM.contains(END));
    }

    #[test]
    fn end_swaps_the_paragraph_and_keeps_the_push_rule() {
        let text = apply(FORGE_SYSTEM, CommitPolicy::End);
        assert!(text.contains("do not commit as you go"));
        assert!(!text.contains("commit each verified unit of work"));
        assert!(text.contains("Never push, force-push"));
        assert!(text.contains("Version control:"));
    }

    #[test]
    fn never_forbids_commits() {
        let text = apply(FORGE_SYSTEM, CommitPolicy::Never);
        assert!(text.contains("Never run `git commit`"));
        assert!(!text.contains("commit each verified unit"));
        assert!(!text.contains("Do not end a long task with your edits uncommitted"));
        assert!(text.contains("Never push, force-push"));
    }

    #[test]
    fn prompt_without_the_paragraph_is_left_alone() {
        assert!(matches!(
            apply("custom prompt", CommitPolicy::Never),
            Cow::Borrowed("custom prompt")
        ));
    }

    #[test]
    fn only_unit_nudges() {
        assert!(CommitPolicy::Unit.nudges());
        assert!(!CommitPolicy::End.nudges());
        assert!(!CommitPolicy::Never.nudges());
    }
}
