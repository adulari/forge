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
        || (input > THIN_SUMMARY_INPUT_CHARS
            && (summary.chars().count() < THIN_SUMMARY_MIN_CHARS
                || !summary.to_lowercase().contains("goal")))
}

/// Where an auto-compaction should leave the transcript, as a share of the trigger that fired it.
///
/// Compacting down to just under the trigger means the very next tool result trips it again.
/// Measured over 60 days of the live store: after a compaction that folded only 3.5k-14k tokens,
/// the session was back over its ceiling within 2-8 minutes because the verbatim tail — the last
/// messages, which a summary never touches — was itself most of the ceiling. Half leaves the
/// session room for a real stretch of work before it pays for another summary.
pub(crate) const COMPACT_TARGET_PERCENT: u64 = 50;

/// Tokens an auto-compaction should leave the transcript at, for a given trigger.
pub(crate) fn compact_target_tokens(trigger: u64) -> u64 {
    trigger.saturating_mul(COMPACT_TARGET_PERCENT) / 100
}

/// Smallest a clipped tool result is cut down to; below this the saving is not worth the loss.
const CLIP_FLOOR_TOKENS: usize = 800;

/// Shrink the largest tool results in place, head and tail kept, until the LLM-visible messages
/// total at most `budget` tokens or nothing clippable is left. Returns the tokens reclaimed.
///
/// A summary only replaces the OLDER part of a transcript. The tail it keeps verbatim can by
/// itself exceed the ceiling (a few whole-file reads), and then compaction frees almost nothing
/// and the next turn compacts again. Clipping the biggest results first reclaims the most for the
/// least lost; the full text stays in the store for replay.
pub(crate) fn clip_tool_results_to_budget(messages: &mut [Message], budget: usize) -> usize {
    let mut sizes: Vec<usize> = messages
        .iter()
        .map(|m| {
            if m.visibility.is_llm() {
                message_tokens(m)
            } else {
                0
            }
        })
        .collect();
    let mut total: usize = sizes.iter().sum();
    let start = total;
    let mut stuck = vec![false; messages.len()];
    while total > budget {
        let Some((index, size)) = sizes
            .iter()
            .copied()
            .enumerate()
            .filter(|(i, size)| {
                !stuck[*i] && messages[*i].role == Role::Tool && *size > CLIP_FLOOR_TOKENS
            })
            .max_by_key(|(_, size)| *size)
        else {
            break;
        };
        let target = size.saturating_sub(total - budget).max(CLIP_FLOOR_TOKENS);
        let clipped = crate::context_pipeline::elide_tool_result(&messages[index], target);
        let clipped_size = message_tokens(&clipped);
        if clipped_size >= size {
            stuck[index] = true;
            continue;
        }
        messages[index] = clipped;
        total -= size - clipped_size;
        sizes[index] = clipped_size;
        stuck[index] = true; // one pass per result: re-clipping a floored one gains nothing
    }
    start.saturating_sub(total)
}

/// Most characters of the user's own messages a deterministic digest carries.
const DIGEST_MAX_CHARS: usize = 16_000;
/// Longest single user message kept whole in a digest.
const DIGEST_MESSAGE_MAX_CHARS: usize = 1_500;

/// A summary written without a model, for when none produced a usable one: the user's own words
/// from the folded stretch (newest preferred, shown oldest first) plus a count of what was dropped.
/// Tool output and assistant steps are the bulk and the least irreplaceable part; what the user
/// asked for is not.
pub(crate) fn deterministic_digest(older: &[Message]) -> String {
    let llm: Vec<&Message> = older.iter().filter(|m| m.visibility.is_llm()).collect();
    let mut kept: Vec<String> = Vec::new();
    let mut used = 0usize;
    for m in llm.iter().rev().filter(|m| m.role == Role::User) {
        let text: String = m.content.chars().take(DIGEST_MESSAGE_MAX_CHARS).collect();
        if used + text.len() > DIGEST_MAX_CHARS {
            break;
        }
        used += text.len();
        kept.push(text);
    }
    kept.reverse();
    let tool_results = llm.iter().filter(|m| m.role == Role::Tool).count();
    let steps = llm.iter().filter(|m| m.role == Role::Assistant).count();
    let mut out = format!(
        "No model could summarize this stretch, so it was trimmed instead: {steps} assistant \
         steps and {tool_results} tool results were dropped. Goal and requests, in the user's \
         own words (oldest first):\n"
    );
    for text in kept {
        out.push_str("\n- ");
        out.push_str(text.trim());
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compaction_policy::needs_compaction;

    fn read_round(id: &str, body: &str) -> [Message; 2] {
        [
            Message::assistant_tool_calls(
                "",
                vec![forge_types::ToolCall {
                    id: id.into(),
                    name: "read_file".into(),
                    args: serde_json::json!({}),
                }],
            ),
            Message::tool_result(id, body.to_string()),
        ]
    }

    fn total(messages: &[Message]) -> u64 {
        messages.iter().map(|m| message_tokens(m) as u64).sum()
    }

    #[test]
    fn a_summary_without_the_goal_section_is_thin_for_a_long_transcript() {
        let big = vec!["x".repeat(30_000)];
        assert!(summary_too_thin(&"s".repeat(800), &big));
        assert!(!summary_too_thin(
            &format!("## Goal\n{}", "s".repeat(800)),
            &big
        ));
    }

    #[test]
    fn clipping_brings_an_oversized_tail_under_the_target_and_keeps_results_paired() {
        let trigger = 20_000u64;
        let body = "alpha bravo charlie delta echo foxtrot golf hotel\n".repeat(900);
        let mut tail = vec![Message::system(
            "[Earlier conversation summarized]\nGoal: x",
        )];
        for i in 0..6 {
            tail.extend(read_round(&format!("c{i}"), &body));
        }
        assert!(
            total(&tail) > trigger,
            "setup: tail alone is over the ceiling"
        );

        let reclaimed =
            clip_tool_results_to_budget(&mut tail, compact_target_tokens(trigger) as usize);

        assert!(reclaimed > 0);
        assert!(total(&tail) <= compact_target_tokens(trigger));
        assert_eq!(
            tail.len(),
            13,
            "clipping edits content, never drops messages"
        );
        assert!(
            tail[0].content.contains("Goal: x"),
            "the summary is untouched"
        );
    }

    #[test]
    fn a_simulated_refill_does_not_retrigger_straight_after_a_compaction() {
        let trigger = 20_000u64;
        let target = compact_target_tokens(trigger);
        assert!(target < trigger / 2 + 1);
        let body = "lorem ipsum dolor sit amet consectetur\n".repeat(700);
        let mut transcript = vec![Message::system(
            "[Earlier conversation summarized]\nGoal: x",
        )];
        for i in 0..6 {
            transcript.extend(read_round(&format!("a{i}"), &body));
        }
        clip_tool_results_to_budget(&mut transcript, target as usize);
        assert!(!needs_compaction(total(&transcript), trigger, true));

        // Refill with ordinary-sized rounds: several fit before the ceiling is reached again.
        let small = "x ".repeat(800);
        let mut rounds = 0;
        while !needs_compaction(total(&transcript), trigger, true) {
            transcript.extend(read_round(&format!("r{rounds}"), &small));
            rounds += 1;
            assert!(rounds < 100);
        }
        assert!(
            rounds >= 5,
            "only {rounds} rounds of headroom after a compaction"
        );
    }

    #[test]
    fn the_digest_keeps_the_users_words_and_drops_tool_output() {
        let mut older = vec![Message::user("fix the flaky login test")];
        older.extend(read_round("c1", &"noise ".repeat(5_000)));
        older.push(Message::user("and keep the CI green"));
        let digest = deterministic_digest(&older);
        assert!(digest.contains("fix the flaky login test"));
        assert!(digest.contains("keep the CI green"));
        assert!(!digest.contains("noise"));
        assert!(digest.contains("1 tool results"));
    }
}
