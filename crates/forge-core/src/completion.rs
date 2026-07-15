//! Completion policy shared by every Forge execution surface.
//!
//! A model saying that work is done is not evidence by itself. This contract keeps the
//! verification rule independent from the direct-provider and CLI-bridge loops so they cannot
//! silently accept different definitions of completion.

use crate::TaskIntent;

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
}
