//! Pure rules for what a compaction keeps and what counts as a usable summary.

use forge_types::{Message, Role};

use crate::context_pipeline::message_tokens;

/// Rendered transcript size above which a summary must say something substantial.
const THIN_SUMMARY_INPUT_CHARS: usize = 20_000;
/// Least a summary of a transcript that size may be. 189 chars once stood in for 240K tokens.
const THIN_SUMMARY_MIN_CHARS: usize = 600;

/// Whether `summary` is too little to replace `entries`: empty always, and under
/// [`THIN_SUMMARY_MIN_CHARS`] when the input was large.
pub(crate) fn summary_too_thin(summary: &str, entries: &[String]) -> bool {
    let summary = summary.trim();
    let input: usize = entries.iter().map(String::len).sum();
    summary.is_empty()
        || (input > THIN_SUMMARY_INPUT_CHARS && summary.chars().count() < THIN_SUMMARY_MIN_CHARS)
}

/// Most tokens of recent tool rounds a summary keeps verbatim next to it.
pub(crate) const COMPACT_KEEP_ROUNDS_TOKENS: usize = 40_000;

/// Where the verbatim tail after a summary starts: the last `keep_recent` messages, extended back
/// over up to [`crate::context_pipeline::PRUNE_KEEP_ROUNDS`] whole tool rounds while they cost at
/// most `token_budget`.
///
/// Six messages is one round of a model that reads six files at once, so a summary taken mid-task
/// used to drop the files the model was working from and it read them again. The budget keeps a
/// huge round from surviving every summary — the caller passes a quarter of the transcript, so a
/// compaction always shrinks it.
pub(crate) fn kept_tail_start(
    messages: &[Message],
    keep_recent: usize,
    token_budget: usize,
) -> usize {
    let mut start = messages.len().saturating_sub(keep_recent);
    let round_starts = messages
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, m)| m.role == Role::Assistant && !m.tool_calls.is_empty())
        .map(|(i, _)| i)
        .take(crate::context_pipeline::PRUNE_KEEP_ROUNDS);
    for round_start in round_starts {
        let cost: usize = messages[round_start..].iter().map(message_tokens).sum();
        if cost > token_budget {
            break;
        }
        start = start.min(round_start);
    }
    start
}
