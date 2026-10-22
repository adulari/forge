//! `[git] commit_policy`: how eagerly the model is told to commit its own work.

use serde::{Deserialize, Serialize};

/// When the model commits (docs/features/commit-discipline.md). `unit` is the long-standing
/// behaviour; the others suit users who want one commit per task, or none at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommitPolicy {
    /// Not chosen by the user: `unit` in an interactive session, `end` for a headless one-shot
    /// run (`forge run`, `forge run -p`, `forge bench`), which has nobody to review per-unit
    /// commits and pays a model round trip for each one.
    #[default]
    Auto,
    /// Commit each verified unit of work as it is finished; the harness reminds the model about
    /// uncommitted files at turn start and every `commit_nudge_edits` edits.
    Unit,
    /// Make all edits, verify, and commit once when the whole task is done. No harness reminders.
    End,
    /// Never commit or stage; leave the changes in the working tree. No harness reminders.
    Never,
}

impl CommitPolicy {
    /// The policy a session acts on: an explicit choice stands, `auto` becomes `fallback`.
    pub fn or(self, fallback: CommitPolicy) -> CommitPolicy {
        match self {
            CommitPolicy::Auto => fallback,
            chosen => chosen,
        }
    }

    /// Whether the git-hygiene reminders (uncommitted-files, push) apply. They push the model to
    /// commit early and often, which only a resolved `unit` policy wants.
    pub fn nudges(self) -> bool {
        self == CommitPolicy::Unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, GitConfig};

    #[test]
    fn defaults_to_auto_and_parses_each_value() {
        assert_eq!(Config::default().git.commit_policy, CommitPolicy::Auto);
        for (raw, want) in [
            ("auto", CommitPolicy::Auto),
            ("unit", CommitPolicy::Unit),
            ("end", CommitPolicy::End),
            ("never", CommitPolicy::Never),
        ] {
            let cfg: GitConfig = toml::from_str(&format!("commit_policy = \"{raw}\"")).unwrap();
            assert_eq!(cfg.commit_policy, want);
        }
        assert!(toml::from_str::<GitConfig>("commit_policy = \"sometimes\"").is_err());
    }

    #[test]
    fn auto_takes_the_fallback_and_an_explicit_choice_stands() {
        assert_eq!(CommitPolicy::Auto.or(CommitPolicy::End), CommitPolicy::End);
        assert_eq!(
            CommitPolicy::Auto.or(CommitPolicy::Unit),
            CommitPolicy::Unit
        );
        for chosen in [CommitPolicy::Unit, CommitPolicy::End, CommitPolicy::Never] {
            assert_eq!(chosen.or(CommitPolicy::End), chosen);
        }
    }
}
