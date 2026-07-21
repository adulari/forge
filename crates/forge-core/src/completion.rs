//! Completion policy shared by every Forge execution surface.
//!
//! A model saying that work is done is not evidence by itself. This contract keeps the
//! verification rule independent from the direct-provider and CLI-bridge loops so they cannot
//! silently accept different definitions of completion.

use crate::TaskIntent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum VerificationFamily {
    Typecheck,
    Lint,
    Test,
    Build,
}

impl VerificationFamily {
    const fn label(self) -> &'static str {
        match self {
            Self::Typecheck => "typecheck",
            Self::Lint => "lint",
            Self::Test => "test",
            Self::Build => "build",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerificationObservation {
    Ignore,
    Generic,
    Check(VerificationFamily),
}

/// Outcome-aware evidence for the completion gate. Failed verification families remain unresolved
/// until that same family succeeds; unrelated reads can add evidence but cannot erase a failure.
#[derive(Debug, Default)]
pub(crate) struct VerificationLedger {
    unresolved: std::collections::BTreeSet<VerificationFamily>,
    successful_observations: u64,
}

impl VerificationLedger {
    pub(crate) const fn checkpoint(&self) -> u64 {
        self.successful_observations
    }

    pub(crate) fn observe(&mut self, observation: VerificationObservation, ok: bool) {
        match observation {
            VerificationObservation::Ignore => {}
            VerificationObservation::Generic => {
                if ok {
                    self.successful_observations = self.successful_observations.saturating_add(1);
                }
            }
            VerificationObservation::Check(family) => {
                if ok {
                    self.unresolved.remove(&family);
                    self.successful_observations = self.successful_observations.saturating_add(1);
                } else {
                    self.unresolved.insert(family);
                }
            }
        }
    }

    pub(crate) fn verified_since(&self, checkpoint: u64) -> bool {
        self.unresolved.is_empty() && self.successful_observations > checkpoint
    }

    pub(crate) fn unresolved_summary(&self) -> Option<String> {
        (!self.unresolved.is_empty()).then(|| {
            self.unresolved
                .iter()
                .map(|family| family.label())
                .collect::<Vec<_>>()
                .join(", ")
        })
    }
}

pub(crate) fn classify_tool(name: &str, args: &str) -> VerificationObservation {
    if name.ends_with("update_tasks") || name.ends_with("present_plan") {
        return VerificationObservation::Ignore;
    }
    if !name.ends_with("shell") && !name.ends_with("exec_command") {
        return VerificationObservation::Generic;
    }

    let command = args.to_ascii_lowercase();
    let family = if command.contains("tsc")
        || command.contains("typecheck")
        || command.contains("type-check")
        || command.contains("cargo check")
    {
        Some(VerificationFamily::Typecheck)
    } else if command.contains("eslint")
        || command.contains("clippy")
        || command.contains(" lint")
        || command.contains("lint ")
    {
        Some(VerificationFamily::Lint)
    } else if command.contains("test")
        || command.contains("pytest")
        || command.contains("vitest")
        || command.contains("jest")
        || command.contains("nextest")
    {
        Some(VerificationFamily::Test)
    } else if command.contains("build")
        || command.contains("compile")
        || command.contains("xcodebuild")
    {
        Some(VerificationFamily::Build)
    } else {
        None
    };
    family.map_or(
        VerificationObservation::Generic,
        VerificationObservation::Check,
    )
}

/// Evidence observed while a model claims that every tracked task is complete.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CompletionEvidence {
    /// The turn performed work that left an external artifact which can be inspected.
    pub(crate) did_real_work: bool,
    /// The model explicitly established that a change was not required.
    pub(crate) no_change_required: bool,
    /// The current turn inspected real state rather than merely repeating its claim.
    pub(crate) inspected_this_turn: bool,
}

/// The action the agent loop takes after an all-done claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionDecision {
    /// Ask for a tool-grounded observation before accepting the claim.
    RequestObservation,
    /// The claim is backed by an inspection.
    AcceptClean,
    /// There was no external artifact to inspect.
    AcceptNoArtifacts,
    /// Verification was requested but never provided before the bounded retry budget expired.
    AcceptUnverified,
}

/// Bounded completion-verification policy for an execution surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompletionContract {
    max_observation_requests: usize,
}

impl CompletionContract {
    /// Construct a contract with an explicit bounded observation budget (primarily for tests).
    pub(crate) const fn with_observation_budget(max_observation_requests: usize) -> Self {
        Self {
            max_observation_requests,
        }
    }

    /// The production policy allows two observation requests before accepting an explicitly
    /// unverified completion. This preserves Forge's existing anti-spiral behavior.
    pub(crate) const fn production() -> Self {
        Self::with_observation_budget(2)
    }

    pub(crate) const fn max_observation_requests(self) -> usize {
        self.max_observation_requests
    }

    /// Decide whether completion is credible from the observed evidence.
    pub(crate) fn decide(
        self,
        intent: TaskIntent,
        observation_requests: usize,
        evidence: CompletionEvidence,
    ) -> CompletionDecision {
        if intent.is_observational() || evidence.no_change_required {
            return if evidence.inspected_this_turn {
                CompletionDecision::AcceptClean
            } else {
                CompletionDecision::AcceptNoArtifacts
            };
        }

        if observation_requests > 0 && (evidence.inspected_this_turn || !evidence.did_real_work) {
            return if evidence.inspected_this_turn {
                CompletionDecision::AcceptClean
            } else {
                CompletionDecision::AcceptNoArtifacts
            };
        }

        if observation_requests < self.max_observation_requests {
            CompletionDecision::RequestObservation
        } else {
            CompletionDecision::AcceptUnverified
        }
    }
}

/// Whether completion text explicitly states that no external change was needed.
pub(crate) fn claims_no_change(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "no change needed",
        "no changes needed",
        "no change is needed",
        "no changes are needed",
        "no change required",
        "no changes required",
        "make no changes",
        "no file changes",
        "already satisfied",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

/// The bridge sent no assistant text after a verification request, which is terminal only when
/// the prior assistant answer already completed every tracked task.
pub(crate) fn empty_verification_is_terminal(
    observation_requests: usize,
    tasks: &[forge_types::TodoItem],
    has_prior_final: bool,
) -> bool {
    observation_requests > 0
        && !tasks.is_empty()
        && tasks
            .iter()
            .all(|task| matches!(task.status, forge_types::TodoStatus::Done))
        && has_prior_final
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutating_claims_are_challenged_then_evaluated_from_evidence() {
        let contract = CompletionContract::production();
        let work = CompletionEvidence {
            did_real_work: true,
            ..CompletionEvidence::default()
        };
        assert_eq!(
            contract.decide(TaskIntent::Mutating, 0, work),
            CompletionDecision::RequestObservation
        );
        assert_eq!(
            contract.decide(TaskIntent::Mutating, 2, work),
            CompletionDecision::AcceptUnverified
        );
        assert_eq!(
            contract.decide(
                TaskIntent::Mutating,
                1,
                CompletionEvidence {
                    inspected_this_turn: true,
                    ..work
                }
            ),
            CompletionDecision::AcceptClean
        );
    }

    #[test]
    fn observational_work_never_requires_a_mutating_redrive() {
        assert_eq!(
            CompletionContract::production().decide(
                TaskIntent::ReadOnlyReview,
                0,
                CompletionEvidence::default(),
            ),
            CompletionDecision::AcceptNoArtifacts
        );
    }

    #[test]
    fn failed_typecheck_is_not_cleared_by_a_successful_file_read() {
        let mut ledger = VerificationLedger::default();
        ledger.observe(
            classify_tool("shell", r#"{"command":"npx tsc --noEmit"}"#),
            false,
        );
        let checkpoint = ledger.checkpoint();
        ledger.observe(
            classify_tool("read_file", r#"{"path":"package.json"}"#),
            true,
        );

        assert!(!ledger.verified_since(checkpoint));
        assert_eq!(ledger.unresolved_summary().as_deref(), Some("typecheck"));
    }

    #[test]
    fn failed_lint_test_and_build_each_require_a_matching_success() {
        for (failed, unrelated, matching, label) in [
            ("npm run lint", "npm test", "npm run lint", "lint"),
            ("cargo test", "git diff", "cargo test", "test"),
            ("cargo build", "cat Cargo.toml", "cargo build", "build"),
        ] {
            let mut ledger = VerificationLedger::default();
            ledger.observe(
                classify_tool("shell", &format!(r#"{{"command":"{failed}"}}"#)),
                false,
            );
            let checkpoint = ledger.checkpoint();
            ledger.observe(
                classify_tool("shell", &format!(r#"{{"command":"{unrelated}"}}"#)),
                true,
            );
            assert!(
                !ledger.verified_since(checkpoint),
                "{label} cleared by {unrelated}"
            );
            ledger.observe(
                classify_tool("shell", &format!(r#"{{"command":"{matching}"}}"#)),
                true,
            );
            assert!(
                ledger.verified_since(checkpoint),
                "successful {label} did not clear failure"
            );
        }
    }
}
