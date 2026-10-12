//! Evidence-based detection of a turn that reports a file change nothing performed.
//!
//! The phantom-edit gate used to key only on first-person phrases ("I've added"), so a bulleted
//! "Implemented ... / Added ... / Replaced ..." summary with zero tool calls was accepted as
//! success. The decision now starts from the turn contract (an explicit change request must leave
//! a changed artifact) and only falls back to wording when the contract says nothing.

use crate::completion::claims_no_change;
use crate::turn_contract::{ContractSource, TurnContract};
use crate::TaskIntent;

const CHANGE_VERBS: &[&str] = &[
    "implemented",
    "added",
    "replaced",
    "updated",
    "modified",
    "created",
    "removed",
    "renamed",
    "rewrote",
    "refactored",
    "deleted",
    "edited",
    "changed",
    "fixed",
];

const FILE_EXTENSIONS: &[&str] = &[
    ".py", ".rs", ".ts", ".tsx", ".js", ".jsx", ".go", ".java", ".kt", ".swift", ".c", ".h",
    ".cpp", ".rb", ".php", ".md", ".toml", ".json", ".yaml", ".yml", ".sh", ".css", ".html",
    ".sql",
];

/// A past-tense change report that names a file or a code identifier, e.g.
/// "Implemented X in `memory_service/obsidian.py`" or "- Added explicit `group_sections` ...".
pub(crate) fn claims_applied_change(text: &str) -> bool {
    text.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        let mut rest = lower
            .trim_start_matches(['-', '*', '>', ' ', '\t', '\u{2022}'])
            .trim_start();
        for lead in ["i ", "we ", "also ", "successfully "] {
            rest = rest.strip_prefix(lead).unwrap_or(rest);
        }
        let first = rest
            .split(|c: char| !c.is_ascii_alphabetic())
            .next()
            .unwrap_or("");
        CHANGE_VERBS.contains(&first) && references_artifact(line)
    })
}

fn references_artifact(line: &str) -> bool {
    if line.matches('`').count() >= 2 {
        return true;
    }
    line.split_whitespace().any(|token| {
        let token =
            token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '/' && c != '.');
        let lower = token.to_ascii_lowercase();
        (token.contains('/') && token.len() > 2)
            || FILE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
    })
}

/// Whether a final answer with no mutating tool behind it should be treated as an unapplied edit.
///
/// `empty_diff_gate_runs` means the turn-level empty-diff gate will already re-drive an explicit
/// change contract, so the contract alone must not also nudge here (the two would compound).
pub(crate) fn unapplied_edit_suspected(
    contract: &TurnContract,
    answer: &str,
    empty_diff_gate_runs: bool,
) -> bool {
    if matches!(
        contract.intent(),
        TaskIntent::ReadOnlyReview | TaskIntent::PlanOnly
    ) {
        return false;
    }
    let trimmed = answer.trim();
    if trimmed.is_empty() || trimmed.ends_with('?') || claims_no_change(trimmed) {
        return false;
    }
    // Only a directive in THIS prompt makes the missing edit certain; a session-wide harness
    // expectation also covers pure-answer turns, so those still need a change claim.
    if contract.source() == ContractSource::ExplicitChange {
        return !empty_diff_gate_runs;
    }
    claims_applied_change(trimmed)
}

const MAX_PHANTOM_EDIT_NUDGES: usize = 2;

impl crate::Session {
    /// Re-drive a turn that reported file changes no mutating tool made. Returns true when a nudge
    /// was queued (caller continues the loop); false once the budget is spent, after a loud warning.
    pub(crate) fn nudge_unapplied_edit(&mut self, nudges: &mut usize) -> bool {
        use crate::{Message, PresenterEvent, Role};
        if *nudges >= MAX_PHANTOM_EDIT_NUDGES {
            self.presenter.emit(PresenterEvent::Warning(
                "the turn did NOT apply the file changes it described — no mutating tool ran"
                    .to_string(),
            ));
            return false;
        }
        *nudges += 1;
        self.presenter.emit(PresenterEvent::Warning(format!(
            "model described file changes but no mutating tool ran — asking it to apply them ({nudges}/{MAX_PHANTOM_EDIT_NUDGES})"
        )));
        let nudge = "This turn requires a change to files, but no write or shell tool has run, so \
                     nothing was changed. Perform the edits now with the available tools and \
                     verify the result before giving a final answer. If the change is genuinely \
                     unnecessary, say so explicitly with the reason.";
        let nseq = self.next_seq();
        let _ = self
            .store
            .add_message(&self.id, nseq, Role::System, nudge, None);
        self.transcript.push(Message::system(nudge));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::PermissionMode;

    const ISSUE_REPLY: &str = "Implemented deterministic section handling in \
        `memory_service/obsidian.py`:\n- Added explicit `group_sections` and `perspective_sections` \
        mappings.\n- Replaced the previous keyword-based section building logic with \
        deterministic mapping logic.";

    #[test]
    fn issue_reply_is_a_change_claim() {
        assert!(claims_applied_change(ISSUE_REPLY));
    }

    #[test]
    fn analysis_text_is_not_a_change_claim() {
        for text in [
            "`section_keywords` maps substrings to headings; the loop in generate_daily_timeline \
             picks the first match.",
            "The change would add two mappings to memory_service/obsidian.py.",
            "Added in v2 per the changelog, this function is stable.",
            "There are three call sites. None of them updated the cache.",
        ] {
            assert!(!claims_applied_change(text), "{text}");
        }
    }

    #[test]
    fn contract_decides_without_wording() {
        let change = TurnContract::derive(
            "Modify ONLY memory_service/obsidian.py. Replace section_keywords",
            PermissionMode::Default,
            false,
        );
        assert!(unapplied_edit_suspected(&change, "Done.", false));
        assert!(!unapplied_edit_suspected(&change, "Done.", true));
        assert!(!unapplied_edit_suspected(
            &change,
            "Which file do you mean?",
            false
        ));
        assert!(!unapplied_edit_suspected(
            &change,
            "No changes needed here.",
            false
        ));
        let ask = TurnContract::derive(
            "Explain how section_keywords works",
            PermissionMode::Default,
            false,
        );
        assert!(!unapplied_edit_suspected(
            &ask,
            "It maps substrings to headings.",
            false
        ));
        assert!(unapplied_edit_suspected(&ask, ISSUE_REPLY, false));
        let review = TurnContract::derive(
            "Do not edit anything. Review memory_service/obsidian.py",
            PermissionMode::Default,
            true,
        );
        assert!(!unapplied_edit_suspected(&review, ISSUE_REPLY, false));
    }
}
