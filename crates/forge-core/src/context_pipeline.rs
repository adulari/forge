//! Two-phase context pipeline: the ONE disciplined seam between the session transcript and a
//! provider request (competitor-gap-analysis #9).
//!
//! Phase 1 — [`prune_and_inject`]: mutates the transcript in place (reclaim old tool output;
//! the injection point where future context transforms belong). Runs at turn boundaries.
//!
//! Phase 2 — [`to_llm`]: pure view of the transcript for one provider call — strips
//! [`Visibility::UiOnly`] messages (user-facing notes never spend prompt tokens), then fits the
//! rest to the model's context window. Every main-loop request goes through this; a message the
//! model shouldn't see needs only the `UiOnly` tag, no per-call-site filtering.

use forge_types::{Message, Role};

use crate::tokens;

/// Char length above which an OLD tool result is pruned from the model-facing transcript. Tool
/// output (file dumps, command logs, search hits) dominates context but its bulk has little value
/// once the turn has moved on — the model rarely needs the 30th file read verbatim. Pruning trims
/// the in-memory transcript only; the full text stays in the store for replay.
pub(crate) const PRUNE_TOOL_RESULT_MAX: usize = 3000;
/// How much of a pruned tool result's head to keep (enough to see what the tool produced).
const PRUNE_HEAD_KEEP: usize = 1500;
/// Marker left in place of the dropped tail; also makes pruning idempotent (a result already ending
/// with it is skipped).
pub(crate) const PRUNE_MARKER: &str =
    "\n…[older tool output pruned to save context; full text in replay]…";

/// Marker used for older tool results in a provider request. Unlike [`PRUNE_MARKER`], this is a
/// pure request-view transform: persistence and the newest tool batch remain intact.
const TOOL_RESULT_ELISION_MARKER: &str =
    "\n…[{} chars (~{} tokens) elided — re-run the tool to see full output]…\n";

/// Conservative chars-per-token bound used ONLY when slicing a single oversized message's content
/// down to a token budget (real token offsets aren't worth the cost there). Counting elsewhere uses
/// the real BPE tokenizer ([`tokens`]); this 3 under-estimates so the sliced text stays within
/// budget rather than overflowing.
const CHARS_PER_TOKEN: usize = 3;
const MESSAGE_TRUNCATION_MARKER: &str =
    "[… earlier of this message truncated to fit the model's context …]\n";

/// Phase 1: mutate the transcript at a turn boundary. Today that is zero-LLM context reclaim
/// (truncating large old tool results); future injections/transforms that must SURVIVE in the
/// transcript (rather than apply per-request) belong here, not scattered across call sites.
/// Returns the number of chars reclaimed.
pub(crate) fn prune_and_inject(messages: &mut [Message], keep_recent: usize) -> usize {
    prune_tool_results(messages, keep_recent)
}

/// messages, then elides bulky older tool results while retaining a balanced head and tail. The
/// newest `keep_recent_tool_results` tool messages remain verbatim so an in-progress tool loop
/// keeps all of the data it just produced. Pure — persistence and the in-memory transcript are
/// untouched.
pub(crate) fn to_llm(
    messages: &[Message],
    budget_tokens: usize,
    tool_result_token_budget: usize,
    keep_recent_tool_results: usize,
) -> Vec<Message> {
    let llm_only: Vec<Message> = messages
        .iter()
        .filter(|m| m.visibility.is_llm())
        .cloned()
        .collect();
    let normalized = normalize_tool_pairs(llm_only);
    let elided = elide_old_tool_results(
        &normalized.messages,
        &normalized.synthetic_tool_results,
        tool_result_token_budget,
        keep_recent_tool_results,
    );
    fit_messages_owned(elided, budget_tokens)
}

const INTERRUPTED_TOOL_RESULT: &str = "error: tool call interrupted before a result was recorded";
const MAX_TOOL_CALL_ID_LEN: usize = 256;

struct NormalizedMessages {
    messages: Vec<Message>,
    synthetic_tool_results: std::collections::HashSet<usize>,
}

fn valid_tool_call_id(id: &str) -> bool {
    !id.trim().is_empty()
        && id.len() <= MAX_TOOL_CALL_ID_LEN
        && !id.chars().any(char::is_whitespace)
        && !id.chars().any(char::is_control)
}

fn normalize_tool_pairs(messages: Vec<Message>) -> NormalizedMessages {
    let mut input: std::collections::VecDeque<Message> = messages.into();
    let mut out = Vec::with_capacity(input.len());
    let mut synthetic_tool_results = std::collections::HashSet::new();

    while let Some(mut message) = input.pop_front() {
        if message.role == Role::Tool {
            continue;
        }
        if message.role != Role::Assistant || message.tool_calls.is_empty() {
            out.push(message);
            continue;
        }

        let mut call_id_counts = std::collections::HashMap::new();
        for call in &message.tool_calls {
            *call_id_counts.entry(call.id.clone()).or_insert(0usize) += 1;
        }
        message.tool_calls.retain(|call| {
            valid_tool_call_id(&call.id) && call_id_counts.get(call.id.as_str()) == Some(&1)
        });
        if message.tool_calls.is_empty() {
            continue;
        }

        let call_ids: std::collections::HashSet<String> = message
            .tool_calls
            .iter()
            .map(|call| call.id.clone())
            .collect();
        let call_order: Vec<String> = message
            .tool_calls
            .iter()
            .map(|call| call.id.clone())
            .collect();
        let mut results = std::collections::HashMap::new();
        let mut deferred = Vec::new();

        while let Some(next) = input.front() {
            if matches!(next.role, Role::User | Role::Assistant) {
                break;
            }
            let next = input.pop_front().expect("front checked above");
            if next.role == Role::Tool {
                if let Some(id) = next.tool_call_id.as_deref() {
                    if call_ids.contains(id) {
                        results.insert(id.to_string(), next);
                    }
                }
            } else {
                deferred.push(next);
            }
        }

        out.push(message);
        for call_id in call_order {
            if let Some(result) = results.remove(&call_id) {
                out.push(result);
            } else {
                synthetic_tool_results.insert(out.len());
                out.push(Message::tool_result(call_id, INTERRUPTED_TOOL_RESULT));
            }
        }
        out.extend(deferred);
    }

    NormalizedMessages {
        messages: out,
        synthetic_tool_results,
    }
}

fn elide_old_tool_results(
    messages: &[Message],
    synthetic_tool_results: &std::collections::HashSet<usize>,
    token_budget: usize,
    keep_recent_tool_results: usize,
) -> Vec<Message> {
    if token_budget == 0 {
        return messages.to_vec();
    }
    let protected_from = if keep_recent_tool_results == 0 {
        messages.len()
    } else {
        messages
            .iter()
            .enumerate()
            .filter(|(index, message)| {
                message.role == Role::Tool && !synthetic_tool_results.contains(index)
            })
            .rev()
            .nth(keep_recent_tool_results - 1)
            .map(|(index, _)| index)
            .unwrap_or(0)
    };
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            if message.role != Role::Tool
                || synthetic_tool_results.contains(&index)
                || index >= protected_from
            {
                return message.clone();
            }
            elide_tool_result(message, token_budget)
        })
        .collect()
}

fn elide_tool_result(message: &Message, token_budget: usize) -> Message {
    if tokens::count_text(&message.content) <= token_budget {
        return message.clone();
    }
    let chars: Vec<char> = message.content.chars().collect();
    let marker = TOOL_RESULT_ELISION_MARKER.replace("{}", &format!("{}", chars.len()));
    let marker_tokens = tokens::count_text(&marker);
    let keep_chars = token_budget
        .saturating_sub(marker_tokens)
        .saturating_mul(CHARS_PER_TOKEN);
    let head_chars = keep_chars / 2;
    let tail_chars = keep_chars.saturating_sub(head_chars);
    let omitted = chars.len().saturating_sub(head_chars + tail_chars);
    let marker = TOOL_RESULT_ELISION_MARKER
        .replacen("{}", &omitted.to_string(), 1)
        .replacen("{}", &(omitted / CHARS_PER_TOKEN).to_string(), 1);
    let mut elided = message.clone();
    elided.content = format!(
        "{}{}{}",
        chars[..head_chars.min(chars.len())]
            .iter()
            .collect::<String>(),
        marker,
        chars[chars.len().saturating_sub(tail_chars)..]
            .iter()
            .collect::<String>()
    );
    elided
}

/// Real token cost of one message: its content (BPE-counted, cached) + the chat framing overhead +
/// any tool-call name/arguments it carries (which the model also pays for).
pub(crate) fn message_tokens(m: &Message) -> usize {
    let mut n = tokens::count_message(&m.content);
    for tc in &m.tool_calls {
        n += tokens::count_text(&tc.name) + tokens::count_tool_args(&tc.id, &tc.args);
    }
    n
}

fn truncate_message_to_budget(mut message: Message, budget_tokens: usize) -> Option<Message> {
    let chars: Vec<char> = message.content.chars().collect();
    message.content = MESSAGE_TRUNCATION_MARKER.to_string();
    if message_tokens(&message) > budget_tokens {
        return None;
    }

    let mut low = 0;
    let mut high = chars.len();
    while low < high {
        let keep = low + (high - low).div_ceil(2);
        let start = chars.len() - keep;
        message.content = format!(
            "{}{}",
            MESSAGE_TRUNCATION_MARKER,
            chars[start..].iter().collect::<String>()
        );
        if message_tokens(&message) <= budget_tokens {
            low = keep;
        } else {
            high = keep - 1;
        }
    }

    let start = chars.len() - low;
    message.content = format!(
        "{}{}",
        MESSAGE_TRUNCATION_MARKER,
        chars[start..].iter().collect::<String>()
    );
    Some(message)
}

/// Trim a transcript to fit within `budget_tokens` (the model's context window minus the reserved
/// reply), counted with the real BPE tokenizer. System messages are ALWAYS kept (the standing
/// instructions); the rest are included newest-first until the budget is hit, then re-ordered to
/// the original sequence. If even the single most-recent message overflows alone, its content is
/// truncated from the FRONT (keeping the latest text — usually the actual request). Returns the
/// input unchanged when it already fits. This is what stops a long conversation from overflowing a
/// model's window and failing the turn as "unavailable" across every model.
#[cfg(test)]
pub(crate) fn fit_messages(messages: &[Message], budget_tokens: usize) -> Vec<Message> {
    fit_messages_owned(messages.to_vec(), budget_tokens)
}

fn fit_messages_owned(messages: Vec<Message>, budget_tokens: usize) -> Vec<Message> {
    let total: usize = messages.iter().map(message_tokens).sum();
    if total <= budget_tokens {
        return messages;
    }
    let system_cost: usize = messages
        .iter()
        .filter(|m| m.role == Role::System)
        .map(message_tokens)
        .sum();
    let mut remaining = budget_tokens.saturating_sub(system_cost);
    let mut keep_idx = std::collections::HashSet::new();

    for i in (0..messages.len()).rev() {
        if messages[i].role == Role::System {
            continue;
        }
        let cost = message_tokens(&messages[i]);
        if cost <= remaining {
            remaining -= cost;
            keep_idx.insert(i);
        } else if keep_idx.is_empty() {
            if messages[i].role == Role::Tool || !messages[i].tool_calls.is_empty() {
                let mut pair_start = i;
                while pair_start > 0 && messages[pair_start - 1].role == Role::Tool {
                    pair_start -= 1;
                }
                if pair_start > 0 && !messages[pair_start - 1].tool_calls.is_empty() {
                    pair_start -= 1;
                }
                let reduced = messages
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, message)| {
                        (index < pair_start || index > i).then_some(message)
                    })
                    .collect();
                return fit_messages_owned(reduced, budget_tokens);
            }

            let truncated = truncate_message_to_budget(messages[i].clone(), remaining);
            let mut out: Vec<Message> = messages
                .into_iter()
                .filter(|message| message.role == Role::System)
                .collect();
            if let Some(message) = truncated {
                out.push(message);
            }
            return out;
        } else {
            break;
        }
    }

    let mut ordered: Vec<usize> = keep_idx.iter().copied().collect();
    ordered.sort_unstable();
    for i in ordered {
        if messages[i].role == Role::Tool {
            keep_idx.remove(&i);
        } else {
            break;
        }
    }
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(index, message)| {
            (message.role == Role::System || keep_idx.contains(&index)).then_some(message)
        })
        .collect()
}

/// Zero-LLM context reclaim: truncate large OLD tool results in place so a long conversation fits
/// without paying for an LLM summarize round-trip. Protects the most recent `keep_recent` messages
/// and only touches `Tool` results longer than [`PRUNE_TOOL_RESULT_MAX`], keeping a
/// [`PRUNE_HEAD_KEEP`]-char head + a marker. Returns the number of chars reclaimed; idempotent (a
/// result already ending with [`PRUNE_MARKER`] is skipped). The full text remains in the store for
/// replay — only the model-facing transcript is trimmed.
pub(crate) fn prune_tool_results(messages: &mut [Message], keep_recent: usize) -> usize {
    let len = messages.len();
    if len <= keep_recent {
        return 0;
    }
    let protect_from = len - keep_recent;
    let mut reclaimed = 0usize;
    for m in &mut messages[..protect_from] {
        if m.role != Role::Tool
            || m.content.len() <= PRUNE_TOOL_RESULT_MAX
            || m.content.ends_with(PRUNE_MARKER)
        {
            continue;
        }
        let before = m.content.len();
        let mut head = PRUNE_HEAD_KEEP.min(m.content.len());
        while !m.content.is_char_boundary(head) {
            head -= 1;
        }
        let mut kept = m.content[..head].to_string();
        kept.push_str(PRUNE_MARKER);
        reclaimed += before - kept.len();
        m.content = kept;
    }
    reclaimed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_call(id: &str) -> forge_types::ToolCall {
        forge_types::ToolCall {
            id: id.into(),
            name: "shell".into(),
            args: serde_json::json!({"command": "true"}),
        }
    }

    #[test]
    fn to_llm_repairs_unmatched_tool_calls_without_mutating_transcript() {
        let messages = vec![
            Message::assistant_tool_calls("", vec![tool_call("call-1")]),
            Message::user("continue"),
        ];

        let output = to_llm(&messages, 10_000, 4_096, 2);

        assert_eq!(output.len(), 3);
        assert_eq!(output[0].role, Role::Assistant);
        assert_eq!(output[1].role, Role::Tool);
        assert_eq!(output[1].tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(
            output[1].content,
            "error: tool call interrupted before a result was recorded"
        );
        assert_eq!(output[2].content, "continue");
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn to_llm_keeps_real_sibling_results_before_synthetic_results() {
        let messages = vec![
            Message::assistant_tool_calls("", vec![tool_call("call-1"), tool_call("call-2")]),
            Message::tool_result("call-1", "ok"),
            Message::user("continue"),
        ];

        let output = to_llm(&messages, 10_000, 4_096, 2);

        assert_eq!(output.len(), 4);
        assert_eq!(output[1].tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(output[1].content, "ok");
        assert_eq!(output[2].tool_call_id.as_deref(), Some("call-2"));
        assert_eq!(
            output[2].content,
            "error: tool call interrupted before a result was recorded"
        );
        assert_eq!(output[3].content, "continue");
    }

    #[test]
    fn to_llm_does_not_synthesize_results_for_ui_only_calls() {
        let messages = vec![
            Message::assistant_tool_calls("", vec![tool_call("hidden")]).ui_only(),
            Message::user("visible"),
        ];

        let output = to_llm(&messages, 10_000, 4_096, 2);

        assert_eq!(output.len(), 1);
        assert_eq!(output[0].role, Role::User);
        assert_eq!(output[0].content, "visible");
    }

    #[test]
    fn to_llm_matches_results_within_each_call_batch() {
        let messages = vec![
            Message::assistant_tool_calls("", vec![tool_call("reused")]),
            Message::tool_result("reused", "first"),
            Message::user("next"),
            Message::assistant_tool_calls("", vec![tool_call("reused")]),
        ];

        let output = to_llm(&messages, 10_000, 4_096, 2);

        assert_eq!(output.len(), 5);
        assert_eq!(output[1].content, "first");
        assert_eq!(output[4].tool_call_id.as_deref(), Some("reused"));
        assert_eq!(
            output[4].content,
            "error: tool call interrupted before a result was recorded"
        );
    }

    #[test]
    fn to_llm_drops_ambiguous_calls_and_keeps_the_latest_valid_result() {
        let mut duplicate = tool_call("duplicate");
        duplicate.args = serde_json::json!({"command": "second"});
        let messages = vec![
            Message::tool_result("orphan", "orphaned"),
            Message::assistant_tool_calls(
                "content",
                vec![
                    tool_call(""),
                    tool_call("duplicate"),
                    duplicate,
                    tool_call("valid"),
                ],
            ),
            Message::tool_result("unrelated", "wrong batch"),
            Message::tool_result("duplicate", "ambiguous"),
            Message::tool_result("valid", "failed first"),
            Message::tool_result("valid", "succeeded last"),
        ];

        let output = to_llm(&messages, 10_000, 4_096, 2);

        assert_eq!(output.len(), 2);
        assert_eq!(output[0].tool_calls.len(), 1);
        assert_eq!(output[0].tool_calls[0].id, "valid");
        assert_eq!(output[1].tool_call_id.as_deref(), Some("valid"));
        assert_eq!(output[1].content, "succeeded last");
    }

    #[test]
    fn to_llm_collects_late_results_across_system_messages() {
        let messages = vec![
            Message::assistant_tool_calls("", vec![tool_call("a"), tool_call("b")]),
            Message::tool_result("a", "result a"),
            Message::system("queued tool hint"),
            Message::tool_result("b", "result b"),
            Message::user("continue"),
        ];

        let output = to_llm(&messages, 10_000, 4_096, 2);

        assert_eq!(output.len(), 5);
        assert_eq!(output[1].content, "result a");
        assert_eq!(output[2].content, "result b");
        assert_eq!(output[3].content, "queued tool hint");
        assert_eq!(output[4].content, "continue");
    }

    #[test]
    fn to_llm_drops_batches_with_no_valid_call_ids() {
        let overlong = "x".repeat(MAX_TOOL_CALL_ID_LEN + 1);
        let messages = vec![
            Message::assistant_tool_calls(
                "unsafe prefill",
                vec![tool_call(" "), tool_call("bad\nid"), tool_call(&overlong)],
            ),
            Message::tool_result(" ", "invalid"),
            Message::user("continue"),
        ];

        let output = to_llm(&messages, 10_000, 4_096, 2);

        assert_eq!(output.len(), 1);
        assert_eq!(output[0].role, Role::User);
        assert_eq!(output[0].content, "continue");
    }

    #[test]
    fn normalize_tool_pairs_is_idempotent() {
        let messages = vec![
            Message::assistant_tool_calls("", vec![tool_call("call-1")]),
            Message::user("continue"),
        ];

        let once = normalize_tool_pairs(messages).messages;
        let twice = normalize_tool_pairs(once.clone()).messages;

        assert_eq!(
            serde_json::to_value(&once).unwrap(),
            serde_json::to_value(&twice).unwrap()
        );
    }

    #[test]
    fn to_llm_budget_trim_does_not_leave_half_a_tool_pair() {
        let latest = Message::user("latest");
        let synthetic = Message::tool_result("call-1", INTERRUPTED_TOOL_RESULT);
        let budget = message_tokens(&latest) + message_tokens(&synthetic);
        let messages = vec![
            Message::assistant_tool_calls("", vec![tool_call("call-1")]),
            latest,
        ];

        let output = to_llm(&messages, budget, 4_096, 2);

        assert_eq!(output.len(), 1);
        assert_eq!(output[0].role, Role::User);
        assert_eq!(output[0].content, "latest");
    }

    #[test]
    fn to_llm_tiny_budget_drops_tool_pair_instead_of_faking_a_user_turn() {
        let synthetic = Message::tool_result("call-1", INTERRUPTED_TOOL_RESULT);
        let messages = vec![Message::assistant_tool_calls("", vec![tool_call("call-1")])];

        let output = to_llm(&messages, message_tokens(&synthetic) - 1, 4_096, 2);

        assert!(output.is_empty());
    }

    #[test]
    fn dropping_an_oversized_tool_pair_keeps_system_messages() {
        let before = Message::system("before");
        let after = Message::system("after");
        let synthetic = Message::tool_result("call-1", INTERRUPTED_TOOL_RESULT);
        let budget =
            message_tokens(&before) + message_tokens(&after) + message_tokens(&synthetic) - 1;
        let messages = vec![
            before,
            Message::assistant_tool_calls("", vec![tool_call("call-1")]),
            after,
        ];

        let output = to_llm(&messages, budget, 4_096, 2);

        assert_eq!(output.len(), 2);
        assert_eq!(output[0].content, "before");
        assert_eq!(output[1].content, "after");
    }

    #[test]
    fn synthetic_results_do_not_consume_recent_real_result_quota() {
        let real_result = "x".repeat(30_000);
        let messages = vec![
            Message::assistant_tool_calls("", vec![tool_call("real")]),
            Message::tool_result("real", real_result.clone()),
            Message::assistant_tool_calls("", vec![tool_call("interrupted")]),
        ];

        let output = to_llm(&messages, 100_000, 200, 1);

        assert_eq!(output[1].content, real_result);
        assert_eq!(output[3].content, INTERRUPTED_TOOL_RESULT);
    }

    #[test]
    fn fit_messages_does_not_return_oversized_content_for_tiny_budget() {
        let output = fit_messages(&[Message::user("x".repeat(30_000))], 1);

        assert!(output.is_empty());
    }

    #[test]
    fn fit_messages_uses_exact_tokens_for_multibyte_content() {
        let output = fit_messages(&[Message::user("😀".repeat(30_000))], 100);

        assert_eq!(output.len(), 1);
        assert!(output.iter().map(message_tokens).sum::<usize>() <= 100);
        assert!(output[0].content.starts_with(MESSAGE_TRUNCATION_MARKER));
    }

    #[test]
    fn to_llm_strips_ui_only_messages() {
        let msgs = vec![
            Message::system("standing instructions"),
            Message::user("do the thing"),
            Message::system("⚠ budget cap reached — routing stopped").ui_only(),
            Message::assistant("done"),
        ];
        let out = to_llm(&msgs, 10_000, 4_096, 2);
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|m| m.visibility.is_llm()));
        assert!(!out.iter().any(|m| m.content.contains("budget cap")));
    }

    #[test]
    fn to_llm_budget_applies_after_the_ui_strip() {
        // A huge UI-only note must not eat the token budget of real context.
        let msgs = vec![
            Message::user("keep me"),
            Message::system("x".repeat(100_000)).ui_only(),
            Message::user("and me"),
        ];
        let out = to_llm(&msgs, 200, 4_096, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].content, "keep me");
        assert_eq!(out[1].content, "and me");
    }

    #[test]
    fn to_llm_elides_old_tool_output_and_keeps_recent_results_verbatim() {
        let old = format!("head-{}-tail", "x".repeat(30_000));
        let recent = "recent tool evidence".to_string();
        let msgs = vec![
            Message::assistant_tool_calls("calling tools", vec![tool_call("old")]),
            Message::tool_result("old", old.clone()),
            Message::assistant_tool_calls("next call", vec![tool_call("recent")]),
            Message::tool_result("recent", recent.clone()),
        ];

        let out = to_llm(&msgs, 100_000, 200, 1);
        assert!(out[1].content.starts_with("head-"));
        assert!(out[1].content.ends_with("-tail"));
        assert!(out[1]
            .content
            .contains("elided — re-run the tool to see full output"));
        assert!(out[1].content.len() < old.len() / 10);
        assert_eq!(out[3].content, recent);
        assert_eq!(
            msgs[1].content, old,
            "request transform must not mutate persistence"
        );
    }

    #[test]
    fn to_llm_zero_tool_budget_disables_elision() {
        let result = "x".repeat(30_000);
        let msgs = vec![
            Message::assistant_tool_calls("", vec![tool_call("c1")]),
            Message::tool_result("c1", result.clone()),
        ];
        let out = to_llm(&msgs, 100_000, 0, 0);
        assert_eq!(out[1].content, result);
    }

    #[test]
    fn prune_and_inject_delegates_to_tool_result_reclaim() {
        let mut msgs = vec![
            Message::tool_result("c1", "y".repeat(10_000)),
            Message::user("recent 1"),
            Message::user("recent 2"),
        ];
        let reclaimed = prune_and_inject(&mut msgs, 2);
        assert!(reclaimed > 0);
        assert!(msgs[0].content.ends_with(PRUNE_MARKER));
    }
}
