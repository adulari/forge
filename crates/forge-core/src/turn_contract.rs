//! Explicit, zero-call completion expectations for a Forge turn.
//!
//! This is deliberately narrower than task classification: it reacts only to an explicit
//! read-only instruction or an unambiguous change directive. Ambiguous prompts retain Forge's
//! existing behavior, so the contract improves proof of work without surprising conversational
//! users or adding a model call.

use crate::TaskIntent;
use forge_types::PermissionMode;

/// The source that made the turn's completion expectation explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContractSource {
    /// The caller selected Forge's planning-only permission mode.
    PermissionMode,
    /// A headless caller declared that this turn is expected to modify code.
    HarnessExpectation,
    /// The prompt explicitly says it is read-only.
    ExplicitReadOnly,
    /// The prompt starts with a direct code-change directive.
    ExplicitChange,
    /// No strong contract was inferred; preserve Forge's established behavior.
    Unspecified,
}

/// A narrowly-scoped agreement about what a turn must prove before reporting success.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TurnContract {
    intent: TaskIntent,
    source: ContractSource,
    requires_changed_artifact: bool,
}

impl TurnContract {
    /// Derive the contract without a model call or I/O.
    pub fn derive(prompt: &str, mode: PermissionMode, expect_code_change: bool) -> Self {
        if mode == PermissionMode::Plan {
            return Self {
                intent: TaskIntent::PlanOnly,
                source: ContractSource::PermissionMode,
                requires_changed_artifact: false,
            };
        }
        if explicitly_read_only(prompt) {
            return Self {
                intent: TaskIntent::ReadOnlyReview,
                source: ContractSource::ExplicitReadOnly,
                requires_changed_artifact: false,
            };
        }
        let explicit_change = expect_code_change || explicitly_requests_change(prompt);
        Self {
            intent: TaskIntent::Mutating,
            source: if expect_code_change {
                ContractSource::HarnessExpectation
            } else if explicit_change {
                ContractSource::ExplicitChange
            } else {
                ContractSource::Unspecified
            },
            requires_changed_artifact: explicit_change,
        }
    }

    /// Construct a fixed-intent contract for internal policy tests.
    #[cfg(test)]
    pub(crate) fn for_test(intent: TaskIntent) -> Self {
        Self {
            intent,
            source: ContractSource::Unspecified,
            requires_changed_artifact: false,
        }
    }

    /// The authority used by permission and completion policy.
    pub fn intent(&self) -> TaskIntent {
        self.intent
    }

    /// Why this contract carries its current requirement.
    pub fn source(&self) -> ContractSource {
        self.source
    }

    /// Whether a direct implementation request must leave an inspectable changed artifact.
    pub fn requires_changed_artifact(&self) -> bool {
        self.requires_changed_artifact
    }

    /// Short provider-visible guidance, emitted only for explicit non-default contracts.
    pub(crate) fn guidance(&self) -> Option<&'static str> {
        match self.source {
            ContractSource::ExplicitReadOnly => Some(
                "Turn contract: this request is explicitly read-only. Inspect and explain real state; do not change files or run mutating commands.",
            ),
            ContractSource::ExplicitChange => Some(
                "Turn contract: this request explicitly requires an implementation. Do not report success without a changed artifact and an inspection or verification of that artifact.",
            ),
            ContractSource::HarnessExpectation => Some(
                "Turn contract: this request explicitly requires an implementation. Do not report success without a changed artifact and an inspection or verification of that artifact.",
            ),
            ContractSource::PermissionMode => Some(
                "Turn contract: planning only. Do not change files; produce an actionable plan grounded in inspected state.",
            ),
            ContractSource::Unspecified => None,
        }
    }
}

impl Default for TurnContract {
    fn default() -> Self {
        Self::derive("", PermissionMode::Default, false)
    }
}

fn explicitly_read_only(prompt: &str) -> bool {
    let prompt = prompt.to_ascii_lowercase();
    [
        "read-only",
        "read only",
        "do not make changes",
        "without changing files",
    ]
    .iter()
    .any(|needle| prompt.contains(needle))
}

fn explicitly_requests_change(prompt: &str) -> bool {
    let prompt = prompt.trim_start().to_ascii_lowercase();
    let prompt = prompt.strip_prefix("please ").unwrap_or(&prompt);
    let prompt = prompt.strip_prefix("please, ").unwrap_or(prompt);
    [
        "add ",
        "implement ",
        "fix ",
        "refactor ",
        "update ",
        "remove ",
        "rename ",
        "create ",
        "write ",
        "change ",
    ]
    .iter()
    .any(|verb| prompt.starts_with(verb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_change_requests_require_an_artifact() {
        let contract =
            TurnContract::derive("Please refactor the parser", PermissionMode::Default, false);
        assert_eq!(contract.intent(), TaskIntent::Mutating);
        assert_eq!(contract.source(), ContractSource::ExplicitChange);
        assert!(contract.requires_changed_artifact());
        assert!(contract.guidance().is_some());
    }

    #[test]
    fn ambiguous_questions_keep_existing_behavior() {
        let contract = TurnContract::derive(
            "How would you fix the parser?",
            PermissionMode::Default,
            false,
        );
        assert_eq!(contract.source(), ContractSource::Unspecified);
        assert!(!contract.requires_changed_artifact());
        assert!(contract.guidance().is_none());
    }

    #[test]
    fn explicit_read_only_overrides_the_default_mutating_intent() {
        let contract = TurnContract::derive(
            "Read-only: inspect the parser",
            PermissionMode::Default,
            false,
        );
        assert_eq!(contract.intent(), TaskIntent::ReadOnlyReview);
        assert!(!contract.requires_changed_artifact());
    }
}
