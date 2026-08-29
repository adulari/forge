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
    preserves_public_api: bool,
}

impl TurnContract {
    /// Derive the contract without a model call or I/O.
    pub fn derive(prompt: &str, mode: PermissionMode, expect_code_change: bool) -> Self {
        let preserves_public_api = explicitly_preserves_public_api(prompt);
        if mode == PermissionMode::Plan {
            return Self {
                intent: TaskIntent::PlanOnly,
                source: ContractSource::PermissionMode,
                requires_changed_artifact: false,
                preserves_public_api,
            };
        }
        // A harness expectation is an explicit caller contract. It must win over incidental
        // wording in a generated prompt that asks for inspection before implementation.
        if expect_code_change {
            return Self {
                intent: TaskIntent::Mutating,
                source: ContractSource::HarnessExpectation,
                requires_changed_artifact: true,
                preserves_public_api,
            };
        }
        if explicitly_read_only(prompt) {
            return Self {
                intent: TaskIntent::ReadOnlyReview,
                source: ContractSource::ExplicitReadOnly,
                requires_changed_artifact: false,
                preserves_public_api,
            };
        }
        let explicit_change = explicitly_requests_change(prompt);
        Self {
            intent: TaskIntent::Mutating,
            source: if explicit_change {
                ContractSource::ExplicitChange
            } else {
                ContractSource::Unspecified
            },
            requires_changed_artifact: explicit_change,
            preserves_public_api,
        }
    }

    /// Construct a fixed-intent contract for internal policy tests.
    #[cfg(test)]
    pub(crate) fn for_test(intent: TaskIntent) -> Self {
        Self {
            intent,
            source: ContractSource::Unspecified,
            requires_changed_artifact: false,
            preserves_public_api: false,
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

    /// Whether the user explicitly forbade changes to the public API/signature surface.
    pub fn preserves_public_api(&self) -> bool {
        self.preserves_public_api
    }

    /// Provider-visible clarification for an easy-to-misread preservation constraint.
    pub(crate) fn public_api_guidance(&self) -> Option<&'static str> {
        self.preserves_public_api.then_some(
            "Public-API preservation contract: additions are changes too. Do not add, remove, \
             rename, or alter any public function, method, type, or export. Before claiming \
             completion, compare the complete public surface against the task's base state; do \
             not audit only the methods you edited.",
        )
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

/// Directive phrases that are imperative by construction, so they are safe to match anywhere in
/// the prompt: no ordinary description of a system contains them.
const READ_ONLY_DIRECTIVES: [&str; 7] = [
    "do not make changes",
    "without changing files",
    "this is read-only",
    "this is read only",
    "this task is read-only",
    "this request is read-only",
    "this turn is read-only",
];

/// Bare read-only tokens. These are ordinary technical vocabulary — a bug report says the sandbox
/// stays read-only, or that a turn silently ran READ-ONLY — so they only count as a contract when
/// they OPEN a segment, i.e. when they address this turn rather than describe something.
const READ_ONLY_ANCHORS: [&str; 2] = ["read-only", "read only"];

/// A read-only contract removes write capability for the whole turn: [`crate::TaskScope`] denies
/// every mutating tool and the model is told to change nothing. Inferring that from ANY occurrence
/// of the token silently downgraded turns whose SUBJECT was read-only behavior — a `--mode bypass`
/// run asked to fix a read-only bug produced a full analysis, zero edits, and a report that it
/// lacked permission to write. So the bare tokens must open a segment to count.
fn explicitly_read_only(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    if READ_ONLY_DIRECTIVES
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return true;
    }
    lower.split(['\n', '.', '!', '?', ';']).any(|segment| {
        let segment = trim_directive_decoration(segment);
        READ_ONLY_ANCHORS
            .iter()
            .any(|needle| segment.starts_with(needle))
    })
}

/// Strip list bullets, markdown emphasis and one leading framing label so an anchored directive is
/// still recognized as `- **Read-only**: ...` or `Note: read only`.
fn trim_directive_decoration(segment: &str) -> &str {
    let segment = segment
        .trim()
        .trim_start_matches(|c: char| {
            matches!(c, '-' | '*' | '#' | '>' | '`' | '_' | '[' | '(')
                || c.is_ascii_digit()
                || c.is_whitespace()
        })
        .trim_start();
    for label in [
        "note:",
        "important:",
        "constraint:",
        "scope:",
        "mode:",
        "please",
    ] {
        if let Some(rest) = segment.strip_prefix(label) {
            return rest.trim_start();
        }
    }
    segment
}

fn explicitly_preserves_public_api(prompt: &str) -> bool {
    let prompt = prompt.to_ascii_lowercase();
    [
        "without changing public method signatures",
        "without changing public signatures",
        "no public signatures changed",
        "public signatures unchanged",
        "unchanged public signatures",
        "preserve the public api",
        "preserve public api",
        "do not change the public api",
        "do not change public api",
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

    #[test]
    fn harness_change_expectation_overrides_incidental_read_only_wording() {
        let contract = TurnContract::derive(
            "Inspect with read-only tools, then implement the fix",
            PermissionMode::AcceptEdits,
            true,
        );

        assert_eq!(contract.intent(), TaskIntent::Mutating);
        assert_eq!(contract.source(), ContractSource::HarnessExpectation);
        assert!(contract.requires_changed_artifact());
    }

    #[test]
    fn describing_read_only_behavior_does_not_strip_write_capability() {
        // The reported failure: a `--mode bypass` turn whose SUBJECT was read-only behavior was
        // given a read-only contract, so every mutating tool was denied and the model reported it
        // lacked permission to write. Descriptions must not be read as directives.
        for described in [
            "BUG: a --mode bypass turn silently runs READ-ONLY and produces no edits. Fix it.",
            "The codex sandbox stays read-only, so writes go through the MCP gate. Add a test.",
            "Its reasoning cites a read-only contract from the harness; work out why.",
            "Explain why the bridge is read only",
        ] {
            let contract = TurnContract::derive(described, PermissionMode::Bypass, false);
            assert_eq!(
                contract.intent(),
                TaskIntent::Mutating,
                "described read-only behavior became a read-only contract: {described}"
            );
        }
    }

    #[test]
    fn an_addressed_read_only_directive_still_holds_in_every_mode() {
        for directive in [
            "Read-only: inspect the parser",
            "read only — tell me what the router does",
            "- **Read-only**: audit the store",
            "Note: read-only investigation, no edits",
            "Audit the store. This is read-only.",
            "Answer without changing files",
            "Do not make changes; just explain the failover chain",
        ] {
            for mode in [
                PermissionMode::Default,
                PermissionMode::AcceptEdits,
                PermissionMode::Bypass,
            ] {
                let contract = TurnContract::derive(directive, mode, false);
                assert_eq!(
                    contract.intent(),
                    TaskIntent::ReadOnlyReview,
                    "explicit read-only directive was dropped in {mode:?}: {directive}"
                );
            }
        }
    }

    #[test]
    fn public_api_preservation_treats_additions_as_changes() {
        let contract = TurnContract::derive(
            "Finish and confirm no public signatures changed.",
            PermissionMode::Default,
            false,
        );
        assert!(contract.preserves_public_api());
        let guidance = contract.public_api_guidance().unwrap();
        assert!(guidance.contains("additions are changes too"));
        assert!(guidance.contains("complete public surface"));
    }
}
