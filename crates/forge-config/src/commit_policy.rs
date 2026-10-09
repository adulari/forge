//! `[git] commit_policy`: how eagerly the model is told to commit its own work.

use serde::{Deserialize, Serialize};

/// When the model commits (docs/features/commit-discipline.md). `unit` is the long-standing
/// behaviour; the others suit users who want one commit per task, or none at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommitPolicy {
    /// Commit each verified unit of work as it is finished; the harness reminds the model about
    /// uncommitted files at turn start and every `commit_nudge_edits` edits.
    #[default]
    Unit,
    /// Make all edits, verify, and commit once when the whole task is done. No harness reminders.
    End,
    /// Never commit or stage; leave the changes in the working tree. No harness reminders.
    Never,
}

impl CommitPolicy {
    /// Whether the git-hygiene reminders (uncommitted-files, push) apply. They push the model to
    /// commit early and often, which only the `unit` policy wants.
    pub fn nudges(self) -> bool {
        self == CommitPolicy::Unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, GitConfig};

    #[test]
    fn defaults_to_unit_and_parses_each_value() {
        assert_eq!(Config::default().git.commit_policy, CommitPolicy::Unit);
        for (raw, want) in [
            ("unit", CommitPolicy::Unit),
            ("end", CommitPolicy::End),
            ("never", CommitPolicy::Never),
        ] {
            let cfg: GitConfig = toml::from_str(&format!("commit_policy = \"{raw}\"")).unwrap();
            assert_eq!(cfg.commit_policy, want);
        }
        assert!(toml::from_str::<GitConfig>("commit_policy = \"sometimes\"").is_err());
    }
}
