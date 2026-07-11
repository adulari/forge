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
    let elided = elide_old_tool_results(
        &llm_only,
        tool_result_token_budget,
        keep_recent_tool_results,
    );
    fit_messages(&elided, budget_tokens)
}

fn elide_old_tool_results(
    messages: &[Message],
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
            .filter(|(_, message)| message.role == Role::Tool)
            .rev()
            .nth(keep_recent_tool_results - 1)
            .map(|(index, _)| index)
            // Fewer tool results than we want to keep verbatim → protect ALL of them (elide none):
            // `messages.len()` here would protect nothing and elide the single newest result,
            // contradicting the "newest N stay verbatim" contract.
            .unwrap_or(0)
    };
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            if message.role != Role::Tool || index >= protected_from {
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

/// Trim a transcript to fit within `budget_tokens` (the model's context window minus the reserved
/// reply), counted with the real BPE tokenizer. System messages are ALWAYS kept (the standing
/// instructions); the rest are included newest-first until the budget is hit, then re-ordered to
/// the original sequence. If even the single most-recent message overflows alone, its content is
/// truncated from the FRONT (keeping the latest text — usually the actual request). Returns the
/// input unchanged when it already fits. This is what stops a long conversation from overflowing a
/// model's window and failing the turn as "unavailable" across every model.
pub(crate) fn fit_messages(messages: &[Message], budget_tokens: usize) -> Vec<Message> {
    let total: usize = messages.iter().map(message_tokens).sum();
    if total <= budget_tokens {
        return messages.to_vec();
    }
    // System messages are non-negotiable context; reserve their cost up front.
    let system_cost: usize = messages
        .iter()
        .filter(|m| m.role == Role::System)
        .map(message_tokens)
        .sum();
    let mut remaining = budget_tokens.saturating_sub(system_cost);

    // Walk non-system messages newest→oldest, keeping each that fits.
    let mut keep_idx = std::collections::HashSet::new();
    for (i, m) in messages.iter().enumerate().rev() {
        if m.role == Role::System {
            continue;
        }
        let cost = message_tokens(m);
        if cost <= remaining {
            remaining -= cost;
            keep_idx.insert(i);
        } else if keep_idx.is_empty() {
            // Nothing kept yet and even this newest message is too big — truncate it from the
            // front so the latest words survive, and stop (the budget is spent). Slice by a
            // conservative char-per-token bound (exact token offsets aren't worth it here).
            let mut m = m.clone();
            let keep_chars = remaining.saturating_sub(48).saturating_mul(CHARS_PER_TOKEN);
            if keep_chars > 0 {
                let chars: Vec<char> = m.content.chars().collect();
                let start = chars.len().saturating_sub(keep_chars);
                m.content = format!(
                    "[… earlier of this message truncated to fit the model's context …]\n{}",
                    chars[start..].iter().collect::<String>()
                );
            }
            // A lone tool RESULT with no preceding assistant call is a dangling tool_call_id the
            // provider rejects — demote it to a plain user message so the request stays valid.
            if m.role == Role::Tool {
                m.role = Role::User;
                m.tool_call_id = None;
            }
            // Rebuild in order: systems first (in place) then this lone truncated tail.
            let mut out: Vec<Message> = messages
                .iter()
                .filter(|m| m.role == Role::System)
                .cloned()
                .collect();
            out.push(m);
            return out;
        } else {
            break;
        }
    }
    // The kept non-system messages are a contiguous newest-first tail. If that tail BEGINS with a
    // tool result, its assistant tool_calls message was trimmed away — a dangling tool_call_id that
    // makes Anthropic/OpenAI hard-reject the whole request. Drop leading tool results until the
    // tail starts on a non-tool message. (System messages aren't tool-paired, so they're exempt.)
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
        .iter()
        .enumerate()
        .filter(|(i, m)| m.role == Role::System || keep_idx.contains(i))
        .map(|(_, m)| m.clone())
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
            Message::assistant("calling tools"),
            Message::tool_result("old", old.clone()),
            Message::assistant("next call"),
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
        let msgs = vec![Message::tool_result("c1", result.clone())];
        let out = to_llm(&msgs, 100_000, 0, 0);
        assert_eq!(out[0].content, result);
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
