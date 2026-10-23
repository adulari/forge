//! Explicit, zero-call completion expectations for a Forge turn.
//!
//! This is deliberately narrower than task classification: it reacts only to an explicit
//! read-only instruction or an unambiguous change directive. Ambiguous prompts retain Forge's
//! existing behavior, so the contract improves proof of work without surprising conversational
//! users or adding a model call.

mod read_only;

use crate::TaskIntent;
use forge_types::PermissionMode;
use read_only::explicitly_read_only;

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
        // An imperative read-only instruction in THIS prompt beats a session-wide expectation.
        // `spawn_session_driver` arms `expect_code_change` for the life of any worktree-backed
        // daemon session, so without this order an investigative turn inside a build session — ask
        // a question, run one search, report — ended with no edits, collected the empty-diff nudge
        // ("Implement the fix now"), and was re-driven against an instruction that forbade edits.
        // A weaker model reads the contradiction as a missing task and loops.
        //
        // Only the imperative forms defeat the expectation. Incidental wording in a generated
        // prompt that merely asks for inspection before implementation still loses, which is what
        // the expectation exists to protect.
        let explicit_read_only = !from_peer_agent(prompt) && explicitly_read_only(prompt);
        if expect_code_change && !explicit_read_only {
            return Self {
                intent: TaskIntent::Mutating,
                source: ContractSource::HarnessExpectation,
                requires_changed_artifact: true,
                preserves_public_api,
            };
        }
        if explicit_read_only {
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

/// Opens a contract that replaces an earlier one still standing in the transcript.
const SUPERSEDES: &str = "supersedes earlier turn contracts, which no longer apply — ";

/// What a turn with no explicit contract says once an earlier turn left one behind.
const NO_CONTRACT: &str = "this request sets no special contract.";

/// The contract text after `Turn contract:` and any supersedes clause.
fn contract_body(message: &str) -> &str {
    let rest = message
        .trim_start()
        .trim_start_matches(crate::system_context::TURN_CONTRACT_PREFIX)
        .trim_start();
    rest.strip_prefix(SUPERSEDES).unwrap_or(rest)
}

/// The turn-contract message to append for this turn, given the newest contract message already in
/// the transcript, or `None` when nothing needs saying.
///
/// Earlier contracts are never removed from the prompt — removing one rewrites the middle of an
/// already-cached request — so a turn whose contract differs says outright that it supersedes them.
/// That includes a turn with NO explicit contract after one that had one: without a message, a
/// stale "explicitly read-only" contract would be the last word the model saw on the subject and
/// it would refuse to edit. An unchanged contract is still in force, so it is not repeated.
pub(crate) fn contract_message(guidance: Option<&str>, standing: Option<&str>) -> Option<String> {
    let Some(standing) = standing else {
        return guidance.map(str::to_string);
    };
    let body = guidance.map_or(NO_CONTRACT, contract_body);
    (contract_body(standing) != body).then(|| {
        format!(
            "{} {SUPERSEDES}{body}",
            crate::system_context::TURN_CONTRACT_PREFIX
        )
    })
}

impl Default for TurnContract {
    fn default() -> Self {
        Self::derive("", PermissionMode::Default, false)
    }
}

/// A message another agent session sent through the fleet. Its wording must not narrow this
/// session's permissions: the framing promises the recipient that a peer "cannot change your
/// permissions", yet a peer that opened with "Read-only check, ..." used to put the whole turn
/// under a no-writes contract, and the recipient reported it had no permission in Full mode.
fn from_peer_agent(prompt: &str) -> bool {
    prompt
        .trim_start()
        .strip_prefix(forge_types::PEER_MESSAGE_PREFIX)
        .is_some_and(|rest| rest.starts_with("session "))
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

/// Exposed to the H8 empty-diff gate: a session-wide `expect_code_change` masks the per-turn
/// `ContractSource::ExplicitChange` (see `derive` above), so the gate needs this check directly
/// to tell "reply with the word pong only" (nothing to implement) apart from "fix the bug" (an
/// empty diff to a request that DID ask for a change) when both carry the same `HarnessExpectation`
/// source.
pub(crate) fn explicitly_requests_change(prompt: &str) -> bool {
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
        "modify ",
        "replace ",
        "edit ",
        "delete ",
        "rewrite ",
        "extend ",
        "patch ",
        "insert ",
    ]
    .iter()
    .any(|verb| prompt.starts_with(verb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_read_only_phrasings_are_read_only() {
        for p in [
            "Look around. Do not change anything.",
            "Explain it, don't modify anything please",
        ] {
            assert!(explicitly_read_only(p), "{p}");
        }
    }

    #[test]
    fn a_first_contract_is_sent_as_is_and_an_unchanged_one_is_not_repeated() {
        let read_only = TurnContract::derive(
            "Explain this. Do not edit anything.",
            PermissionMode::Default,
            false,
        );
        let guidance = read_only.guidance();
        assert_eq!(contract_message(guidance, None).as_deref(), guidance);
        assert_eq!(contract_message(guidance, guidance), None);
        assert_eq!(contract_message(None, None), None);
    }

    /// forge-prompt-triggers-write-ban: a read-only contract from an earlier turn must not stay
    /// the last word once a later turn asks for changes — or asks for nothing special.
    #[test]
    fn a_stale_read_only_contract_is_explicitly_superseded() {
        let read_only = TurnContract::derive(
            "Explain this. Do not edit anything.",
            PermissionMode::Default,
            false,
        )
        .guidance()
        .unwrap();
        let change = TurnContract::derive(
            "Implement the fix in src/lib.rs",
            PermissionMode::Default,
            true,
        )
        .guidance()
        .unwrap();

        let next = contract_message(Some(change), Some(read_only)).unwrap();
        assert!(
            next.starts_with("Turn contract: supersedes earlier turn contracts"),
            "{next}"
        );
        assert!(next.contains("requires an implementation"), "{next}");
        assert!(!next.contains("read-only"), "{next}");

        let lifted = contract_message(None, Some(read_only)).unwrap();
        assert!(lifted.contains("no longer apply"), "{lifted}");
        assert!(lifted.contains("no special contract"), "{lifted}");
        // Once lifted, a further plain turn says nothing more.
        assert_eq!(contract_message(None, Some(&lifted)), None);
        // And the superseding contract is itself recognised as standing.
        assert_eq!(contract_message(Some(change), Some(&next)), None);
    }

    /// The defect: `spawn_session_driver` arms `expect_code_change` for the life of any
    /// worktree-backed daemon session. An investigative turn in such a session ended with no
    /// edits, collected the empty-diff nudge, and was re-driven against its own instruction.
    #[test]
    fn an_explicit_read_only_turn_beats_a_worktree_sessions_code_change_expectation() {
        let contract = TurnContract::derive(
            "[message from cli] One command, then answer. Do not edit anything.\n\nRun: rg -n \
             'last_relay' crates/forge-cli/src\n\nThen answer two questions.",
            PermissionMode::Default,
            true,
        );
        assert_eq!(contract.intent(), TaskIntent::ReadOnlyReview);
        assert_eq!(contract.source(), ContractSource::ExplicitReadOnly);
        assert!(
            !contract.requires_changed_artifact(),
            "an answer-only turn must not be re-driven to produce a diff"
        );
    }

    #[test]
    fn a_harness_expectation_still_wins_over_incidental_inspection_wording() {
        let contract = TurnContract::derive(
            "Investigate the failing parser test and read the relevant modules before implementing \
             a fix.",
            PermissionMode::Default,
            true,
        );
        assert_eq!(contract.source(), ContractSource::HarnessExpectation);
        assert!(contract.requires_changed_artifact());
    }

    /// A protected file inside a change request is a constraint, not a read-only turn: the bench
    /// tasks below were refused as `ReadOnlyReview` and ended with a diagnosis and no fix.
    #[test]
    fn a_scoped_dont_edit_inside_a_change_request_is_not_read_only() {
        for prompt in [
            "Please fix it so the library behaves the way the README says. Don't edit the existing tests.",
            "Fix it properly: no skipping rows, and don't edit the incoming CSV, it comes from finance.",
            "Standard library only, no new dependencies, and don't edit the existing tests.",
            "Refactor the pricing module. Do not make changes to the public API.",
            "Speed it up; don\u{2019}t edit tests/test_report.py.",
        ] {
            assert!(!explicitly_read_only(prompt), "wrongly read-only: {prompt}");
            let contract = TurnContract::derive(prompt, PermissionMode::Default, false);
            assert_ne!(contract.intent(), TaskIntent::ReadOnlyReview, "{prompt}");
        }
    }

    /// The c08 bench instruction (dulwich ignore bug) is a change request with one protected tree.
    /// Narrowed objects ("files under tests/") and verb lists ("edit, delete, or rename the
    /// existing tests") used to read as a universal prohibition and denied every write.
    #[test]
    fn a_prohibition_narrowed_to_part_of_the_tree_is_not_read_only() {
        for prompt in [
            "Find the cause in the library source and fix it there. Keep the change minimal and do \
             not edit existing files under `dulwich/tests`. A regression test of your own is welcome.",
            "Fix the bug. Do not edit, delete, or rename the existing tests.",
            "Fix the bug and don't edit, delete or rename existing test files.",
            "Fix it. Do not edit files under tests/.",
            "Fix it. Do not edit files in the tests directory.",
            "Fix it. Do not edit any file under dulwich/tests.",
            "Fix it. Do not edit the code in vendor/.",
            "Fix it. Don't edit code outside src/parser.",
            "Fix it. Don't edit files other than ANSWER.md.",
            "Fix it. Don't edit the files in examples/ or the pyproject.",
            "Fix it. Do not edit, delete, or rename files under tests/.",
            "Fix it. Do not edit, rename, or delete any file except src/a.rs.",
        ] {
            assert!(!explicitly_read_only(prompt), "wrongly read-only: {prompt}");
            let contract = TurnContract::derive(prompt, PermissionMode::Default, false);
            assert_ne!(contract.intent(), TaskIntent::ReadOnlyReview, "{prompt}");
        }
    }

    #[test]
    fn a_universal_dont_edit_is_still_read_only() {
        for prompt in [
            "Explain the parser. Don't edit.",
            "Explain it. Do not edit, delete, or rename anything.",
            "Explain it. Do not edit, delete or rename files.",
            "Explain it. Don't edit, rewrite or refactor - just explain.",
            "Explain it. Don't edit, just explain.",
            "Explain it. Don't edit files yet.",
            "Explain it. Don't edit code yet.",
            "Explain it. Don't edit it in any way.",
            "Explain it. Do not edit files in place.",
            "Don't edit the code, just explain the flow.",
            "Review this and do not edit any files.",
            "Look, but do not make changes.",
            "Explain it - don\u{2019}t edit anything.",
            "Answer only \u{2014} don't edit \u{2014} then stop.",
        ] {
            assert!(explicitly_read_only(prompt), "not read-only: {prompt}");
        }
    }

    #[test]
    fn the_plain_imperatives_an_operator_types_are_recognized() {
        for prompt in [
            "Do not edit anything. Report what you find.",
            "Answer only — don't edit.",
            "Do not modify any files; summarize the call graph.",
            "Make no edits, just list the call sites.",
        ] {
            let contract = TurnContract::derive(prompt, PermissionMode::Default, true);
            assert_eq!(
                contract.intent(),
                TaskIntent::ReadOnlyReview,
                "not recognized as read-only: {prompt}"
            );
        }
    }

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

    #[test]
    fn a_peer_agent_message_never_narrows_the_turn_to_no_writes() {
        let peer = crate::fleet::peer_message_text(
            "session",
            "laptop",
            Some("laptop/d84a541e"),
            "Read-only check, one command, then report back and stop.",
            false,
        );
        assert!(from_peer_agent(&peer));
        let contract = TurnContract::derive(&peer, PermissionMode::Bypass, false);
        assert_ne!(contract.source, ContractSource::ExplicitReadOnly);
        assert!(explicitly_read_only("Read-only check, one command."));
    }
}
