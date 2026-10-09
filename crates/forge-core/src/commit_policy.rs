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

const NEVER_COMMIT: &str = "\
- Do not create commits. Never run `git commit` or `git add`; leave every change in the working \
tree for the user to review and commit themselves. Reading git state (status, diff, log) is fine.
";

/// `base` with the version-control paragraph rewritten for `policy`. `unit` (and a base that has
/// no such paragraph, e.g. a user-supplied override) is returned untouched.
pub(crate) fn apply(base: &str, policy: CommitPolicy) -> Cow<'_, str> {
    let replacement = match policy {
        CommitPolicy::Unit => return Cow::Borrowed(base),
        CommitPolicy::End => COMMIT_AT_END,
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
