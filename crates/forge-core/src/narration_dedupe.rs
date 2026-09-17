//! Collapsing a model's own repeated statements out of the request it is about to receive.
//!
//! This is the structural half of loop prevention, and the half that does not rely on catching a
//! loop after it starts. A model's next token is conditioned on its context: a request holding
//! several verbatim copies of one sentence tells it that emitting that sentence is what happens
//! here, and the harness itself used to assemble that pattern — every re-drive appended another
//! copy of its instruction, and every repeated step another copy of the model's narration.
//! Collapsing repeats means the pattern cannot form, so there is nothing for a detector to catch.
//!
//! Observed live (2026-09-17): 40 assistant rows carrying 15 distinct texts, one of them 7 times
//! across 12 minutes, while every tool call differed — so the identical-call doom-loop guard never
//! fired, and a 40-minute spiral burned 6.8M input tokens.

use forge_types::{Message, Role};

/// Shortest repeated assistant/user text worth collapsing. A model that says "Done." twice is
/// not looping; one that restates a whole sentence of intent is.
const NARRATION_MIN_CHARS: usize = 40;

/// Left in place of a message whose text already appears, byte-identical, earlier in the request.
///
/// This is the STRUCTURAL half of loop prevention. A model's next token is conditioned on its
/// context, so a context holding several verbatim copies of one sentence makes that sentence the
/// most probable continuation — the harness was handing the model a pattern that said "this is
/// what you do here", and every re-drive added another copy. Collapsing repeats means the model
/// can never see its own repetition (nor a re-driven nudge's) more than once, so the pattern that
/// sustained the loop cannot form in the first place. The tool calls the message carried stay
/// untouched: the round-trip a provider validates is the call/result pairing, not the prose.
pub(crate) const NARRATION_DEDUPE_MARKER: &str =
    "…[said verbatim above — repeated text dropped; do not restate it, either act or answer]…";

/// Collapse assistant/user messages whose text repeats verbatim earlier in the same request.
///
/// Keeps the FIRST copy and marks every later one, which matters twice over: the model still sees
/// the statement once (nothing is hidden from it), and the prefix the provider caches is left
/// untouched, so this never invalidates a prompt cache the way rewriting older messages would.
/// Tool results are handled by [`dedupe_repeated_tool_results`]; system messages by
/// `normalize_system_context`.
pub(crate) fn collapse_repeated_narration(messages: &mut [Message]) -> usize {
    let mut first_seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut collapse: Vec<usize> = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if !matches!(message.role, Role::Assistant | Role::User)
            || message.content.len() < NARRATION_MIN_CHARS
        {
            continue;
        }
        let fingerprint = narration_fingerprint(&message.content);
        if fingerprint.is_empty() {
            continue;
        }
        match first_seen.entry(fingerprint) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(index);
            }
            std::collections::hash_map::Entry::Occupied(_) => collapse.push(index),
        }
    }

    let mut collapsed = 0;
    for index in collapse {
        messages[index].content = NARRATION_DEDUPE_MARKER.to_string();
        collapsed += 1;
    }
    collapsed
}

/// Case- and whitespace-insensitive identity for a message's prose. Deliberately NOT fuzzy:
/// collapsing two messages that merely open alike would hide real content from the model, and the
/// loops observed live repeated their sentences verbatim.
fn narration_fingerprint(content: &str) -> String {
    content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
