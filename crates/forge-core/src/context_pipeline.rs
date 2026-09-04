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

use forge_store::HarnessEntry;
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

/// Smallest tool result worth deduplicating. Collapsing a 20-char "ok" saves nothing and costs a
/// line of marker text; the waste that matters is a re-read file or a repeated command dump.
const DEDUPE_MIN_CHARS: usize = 1000;

/// Left in place of a REPEAT of a tool result whose text already appears, byte-identical, earlier
/// in the same conversation. Deliberately states the RELATIONSHIP rather than just eliding:
/// "identical, unchanged" is itself the answer to "did this change since I last looked", which a
/// bare elision would throw away and the model would have to spend another tool call to recover.
const DEDUPE_MARKER: &str =
    "…[identical to an earlier result above — unchanged since, kept once there to save context]…";

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
    let bounded_system_context = normalize_system_context(llm_only);
    let mut normalized = normalize_tool_pairs(bounded_system_context);
    // Before elision, not after: a duplicate collapsed here frees budget that elision would
    // otherwise have spent shortening a result the model has a verbatim copy of anyway. Rewrites
    // content in place so `synthetic_tool_results` — a set of INDICES into these messages — stays
    // valid; removing the messages instead would silently shift every index in it.
    dedupe_repeated_tool_results(&mut normalized.messages, keep_recent_tool_results);
    let elided = elide_old_tool_results(
        &normalized.messages,
        &normalized.synthetic_tool_results,
        tool_result_token_budget,
        keep_recent_tool_results,
    );
    fit_messages_owned(elided, budget_tokens)
}

const TURN_CONTRACT_PREFIX: &str = "Turn contract:";

/// Derive the provider's system-context view without rewriting the persisted transcript. A turn
/// contract is scoped to one turn, so only the newest contract may remain authoritative.
///
/// Exact repeated system guidance is standing context, not cumulative evidence, so only one full
/// copy is sent — but WHICH copy matters more than it looks. Prompt caches key on a PREFIX, so
/// keeping the newest copy deletes the previously-kept one from the middle of the prompt and
/// discards every cached token after it. One live session showed 344 exact-duplicate system
/// messages, mostly re-emitted `[lsp diagnostics]` blocks: 344 invalidations of a ~50k-token
/// prompt, each to save ~380 chars. Keeping the FIRST copy is just as bounded and costs nothing,
/// because whether a message is dropped then depends only on the messages before it — so a
/// transcript that grows never rewrites what the provider has already cached.
fn normalize_system_context(messages: Vec<Message>) -> Vec<Message> {
    let newest_contract = messages.iter().rposition(|message| {
        message.role == Role::System
            && message
                .content
                .trim_start()
                .starts_with(TURN_CONTRACT_PREFIX)
    });

    let mut seen = std::collections::HashSet::<String>::new();
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(index, message)| {
            if message.role != Role::System {
                return Some(message);
            }
            if message
                .content
                .trim_start()
                .starts_with(TURN_CONTRACT_PREFIX)
            {
                return (Some(index) == newest_contract).then_some(message);
            }
            seen.insert(message.content.clone()).then_some(message)
        })
        .collect()
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

/// Replace every REPEAT of a tool result with a short marker, keeping the FIRST copy verbatim.
///
/// Agents re-read the same file across a long session — the model asks again rather than trusting
/// that nothing changed, which is reasonable behaviour that the harness should make cheap instead
/// of punishing. Measured on one real day of a real session: 280 redundant copies across 84 groups,
/// 1.10M characters of byte-identical tool output re-sent to the model, including one 84,521-char
/// file carried five times in a single session.
///
/// **The first copy is kept, not the newest, and prompt caching is the whole reason.** Providers
/// cache on a prefix: rewriting a message at position N invalidates every cached token from N
/// onward. Keeping the newest would mean editing an OLD message each time a repeat appears —
/// throwing away the cached prefix on exactly the requests that had the most of it to lose. Keeping
/// the first leaves everything before the repeat byte-identical forever, so the collapse is free.
/// (75-80% of the input tokens on the measured traffic were cache reads, so this is not a marginal
/// consideration.)
///
/// The newest tool results are exempt regardless, using the same `keep_recent_tool_results` window
/// [`elide_old_tool_results`] protects: a tool loop that just produced data must get that data
/// back, not a pointer to something further up.
///
/// Identical text means the tool genuinely returned the same bytes, so nothing factual is lost —
/// and the marker says "identical", which answers "has this changed since I last looked" outright.
/// Request-view only: the persisted transcript and replay keep every copy.
///
/// Returns the number of characters reclaimed.
fn dedupe_repeated_tool_results(messages: &mut [Message], keep_recent: usize) -> usize {
    let is_candidate = |m: &Message| m.role == Role::Tool && m.content.len() >= DEDUPE_MIN_CHARS;
    // The newest `keep_recent` REAL tool results are off limits, matching what elision protects.
    let protected_from = if keep_recent == 0 {
        messages.len()
    } else {
        messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == Role::Tool)
            .rev()
            .nth(keep_recent - 1)
            .map(|(index, _)| index)
            .unwrap_or(0)
    };

    let mut first_seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut collapse: Vec<usize> = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if !is_candidate(message) || index >= protected_from {
            continue;
        }
        match first_seen.entry(message.content.as_str()) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(index);
            }
            std::collections::hash_map::Entry::Occupied(_) => collapse.push(index),
        }
    }

    let mut reclaimed = 0;
    for index in collapse {
        let before = messages[index].content.len();
        messages[index].content = DEDUPE_MARKER.to_string();
        reclaimed += before.saturating_sub(DEDUPE_MARKER.len());
    }
    reclaimed
}

/// The tail-relative boundary: everything from here on stays verbatim. Moves forward with every
/// new tool result, which is precisely what used to rewrite already-sent messages.
fn sliding_boundary(
    messages: &[Message],
    synthetic_tool_results: &std::collections::HashSet<usize>,
    keep_recent: usize,
) -> usize {
    if keep_recent == 0 {
        return messages.len();
    }
    messages
        .iter()
        .enumerate()
        .filter(|(index, message)| {
            message.role == Role::Tool && !synthetic_tool_results.contains(index)
        })
        .rev()
        .nth(keep_recent - 1)
        .map(|(index, _)| index)
        .unwrap_or(0)
}

/// The boundary actually used: the sliding one, frozen at the current turn's start.
pub(crate) fn elision_boundary(
    messages: &[Message],
    synthetic_tool_results: &std::collections::HashSet<usize>,
    keep_recent: usize,
) -> usize {
    let sliding = sliding_boundary(messages, synthetic_tool_results, keep_recent);
    let turn_start = messages
        .iter()
        .rposition(|message| message.role == Role::User)
        .unwrap_or(sliding);
    sliding.min(turn_start)
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
    // Freeze the boundary for the duration of a turn.
    //
    // `sliding` alone is measured from the TAIL, so every new tool result pushed it forward and a
    // result that was verbatim on the last request got elided on this one. Prompt caches key on a
    // PREFIX, so rewriting a message in the middle invalidates every token after it — on a real
    // 138k-token prompt whose boundary sat ~69k from the end, that discarded ~69k of cache on
    // EVERY round-trip. Measured over 630 requests: 160 of them were under 50% cached and carried
    // 19.9M fresh tokens, 87% of all fresh input, while the 453 well-cached ones carried 1.5M.
    //
    // Anchoring on the last user message makes the boundary constant for a whole turn: within the
    // turn nothing already sent is ever rewritten, so the cache holds across all of its
    // round-trips, and the boundary advances exactly once — at the next user message — for one
    // invalidation instead of one per step.
    //
    // `min` because a lower index protects MORE: this can only ever keep more verbatim than the
    // sliding window did, never less, so no turn loses data it used to have. The extra bulk that
    // buys is bounded by auto-compaction, which fires mid-turn on the same transcript.
    let protected_from =
        elision_boundary(messages, synthetic_tool_results, keep_recent_tool_results);
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
    // A tokenizer cannot emit more tokens than the UTF-8 byte length it consumes. This cheap
    // bound avoids BPE-tokenizing thousands of small persisted tool results just to prove that
    // each one is already below a much larger per-result budget.
    if message.content.len() <= token_budget {
        return message.clone();
    }
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
/// Tokens the TOOL SCHEMAS occupy in a request. Every provider serialises name, description and
/// the JSON Schema of each tool on every call, so they consume the same window the transcript does
/// — and a trimmer that ignores them will confidently overflow a small context.
pub(crate) fn tool_spec_tokens(specs: &[forge_provider::ToolSpec]) -> usize {
    specs
        .iter()
        .map(|s| {
            tokens::count_text(&s.name)
                + tokens::count_text(&s.description)
                + tokens::count_text(&s.schema.to_string())
                // Per-tool framing the provider adds around each definition.
                + 8
        })
        .sum()
}

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
    // Count standing system context once, then walk ordinary history newest-first. Long sessions
    // normally overflow on a relatively small recent suffix; an eager total over the entire
    // transcript needlessly tokenized every old tool result before immediately discarding it.
    let system_cost: usize = messages
        .iter()
        .filter(|m| m.role == Role::System)
        .map(message_tokens)
        .sum();
    // The newest user message is the turn's task statement. Everything after it — the assistant's
    // own reasoning and its tool results — is work done in service of it, and a long tool loop can
    // fill the whole budget on its own. Walking newest-first without an anchor therefore evicts the
    // instruction while keeping the output it produced, and the model, seeing only tool results,
    // reports that no task text arrived and invents one. Reserve it up front like system context.
    let anchor = messages
        .iter()
        .rposition(|m| m.role == Role::User)
        .filter(|_| budget_tokens > system_cost);
    let anchor_cost = anchor.map_or(0, |i| message_tokens(&messages[i]));
    let mut remaining = budget_tokens.saturating_sub(system_cost);
    // An anchor that does not fit is clipped rather than dropped — a shortened objective still
    // steers the turn, an absent one does not — and nothing else can fit beside it.
    if let Some(i) = anchor.filter(|_| anchor_cost > remaining) {
        let clipped = truncate_message_to_budget(messages[i].clone(), remaining);
        let mut out: Vec<Message> = messages
            .into_iter()
            .filter(|message| message.role == Role::System)
            .collect();
        out.extend(clipped);
        return out;
    }
    remaining -= anchor_cost;
    let mut keep_idx = std::collections::HashSet::new();
    let mut saw_overflow = false;

    for i in (0..messages.len()).rev() {
        if messages[i].role == Role::System || Some(i) == anchor {
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
            // Keep the anchored task statement alongside the standing system context, so the
            // last-resort path still tells the model what it was asked to do.
            let mut out: Vec<Message> = messages
                .into_iter()
                .enumerate()
                .filter(|(index, message)| message.role == Role::System || Some(*index) == anchor)
                .map(|(_, message)| message)
                .collect();
            if let Some(message) = truncated {
                out.push(message);
            }
            return out;
        } else {
            saw_overflow = true;
            break;
        }
    }

    if !saw_overflow {
        return messages;
    }

    if let Some(i) = anchor {
        keep_idx.insert(i);
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

/// Continual Harness (`/refine`): render up to `max_entries` learned prompt/skill/subagent
/// entries as one labeled system-context block, so the model can tell them apart from its
/// immutable base system prompt and from user instructions. `entries` is expected pre-ordered by
/// the caller's scope precedence (session, then project, then global — see
/// `Session::harness_overview`); within each scope the store already orders `prompt` before
/// `skill`/`subagent`, so capping the *front* of the list keeps that precedence intact. `None`
/// when there is nothing to inject, mirroring auto-memory recall's "add nothing" contract.
pub(crate) fn harness_context_block(
    entries: &[HarnessEntry],
    max_entries: u32,
    max_chars: u32,
) -> Option<String> {
    if entries.is_empty() || max_entries == 0 {
        return None;
    }
    let mut block = String::from(
        "Learned harness context (Continual Harness) — supplemental notes, skills, and \
         subagent specs this agent previously proposed about itself from past sessions. \
         These are learned guidance, NOT part of the base system prompt and NOT instructions \
         from the user:\n",
    );
    for entry in entries.iter().take(max_entries as usize) {
        block.push_str(&format!(
            "\n[{} · {}] {}\n{}\n",
            entry.kind,
            entry.scope,
            entry.title,
            clamp_chars(&entry.content, max_chars as usize)
        ));
    }
    Some(block)
}

/// Truncate `s` to at most `max_chars` characters, appending an ellipsis when it was cut.
pub(crate) fn clamp_chars(s: &str, max_chars: usize) -> std::borrow::Cow<'_, str> {
    if s.chars().count() <= max_chars {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    std::borrow::Cow::Owned(truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defect this pins: a Muse-driven fleet session on 2026-09-02 lost its brief mid-turn and
    /// answered "No task text came through" for the rest of the run, inventing work from the tool
    /// results still in the window.
    #[test]
    fn a_long_tool_loop_cannot_evict_the_task_statement() {
        let mut messages = vec![
            Message::system("standing system context"),
            Message::user("TASK: add the opt-in publish flag"),
        ];
        for i in 0..40 {
            messages.push(Message::assistant_tool_calls("", vec![tool_call("call-1")]));
            messages.push(Message::tool_result(
                "call-1",
                format!("{i} {}", "x".repeat(4_000)),
            ));
        }

        let output = to_llm(&messages, 2_000, 100_000, 60);

        let task = output
            .iter()
            .find(|m| m.role == Role::User && m.content.contains("TASK:"));
        assert!(
            task.is_some(),
            "the newest user message must survive a tool loop that fills the budget"
        );
        assert!(
            output.len() < messages.len(),
            "the fit still has to drop something, or the test proves nothing"
        );
    }

    #[test]
    fn an_oversized_task_statement_is_clipped_not_dropped() {
        let messages = vec![
            Message::system("standing system context"),
            Message::user(format!("TASK: {}", "y".repeat(40_000))),
            Message::tool_result("call-1", "z".repeat(4_000)),
        ];

        let output = to_llm(&messages, 500, 100_000, 4);

        assert!(
            output.iter().any(|m| m.role == Role::User),
            "a task statement larger than the whole budget is truncated, never removed"
        );
    }

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
    fn to_llm_keeps_only_the_newest_turn_contract_without_mutating_audit_messages() {
        let messages = vec![
            Message::system("Turn contract: planning only."),
            Message::user("plan it"),
            Message::assistant("plan"),
            Message::system("Turn contract: this request explicitly requires an implementation."),
            Message::user("build it"),
        ];
        let audit = serde_json::to_value(&messages).unwrap();

        let output = to_llm(&messages, 10_000, 4_096, 2);

        let contracts: Vec<&Message> = output
            .iter()
            .filter(|message| message.content.starts_with("Turn contract:"))
            .collect();
        assert_eq!(contracts.len(), 1);
        assert!(contracts[0].content.contains("requires an implementation"));
        assert_eq!(serde_json::to_value(&messages).unwrap(), audit);
    }

    #[test]
    fn to_llm_keeps_the_first_exact_system_guidance_copy_not_the_newest() {
        let guidance = format!("standing workflow guidance {}", "detail ".repeat(40));
        let messages = vec![
            Message::system(&guidance),
            Message::user("first"),
            Message::assistant("first answer"),
            Message::system(&guidance),
            Message::user("second"),
        ];

        let output = to_llm(&messages, 10_000, 4_096, 2);

        let full: Vec<usize> = output
            .iter()
            .enumerate()
            .filter(|(_, message)| message.content == guidance)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(full.len(), 1, "exactly one full copy is sent");

        let first_user = output
            .iter()
            .position(|message| message.content == "first")
            .unwrap();
        // The point of the change: the copy that survives whole is the EARLIER one, so the prompt
        // prefix a provider has already cached is never rewritten. Keeping the newest instead —
        // the previous behaviour — puts the full copy after this index and fails here.
        assert!(
            full[0] < first_user,
            "the first copy is the one kept whole, not the newest"
        );
    }

    /// The cache-critical property, stated directly: appending to a transcript must never change
    /// what was already sent. Anything that rewrites an earlier message discards every cached
    /// token after it, which is exactly what keeping the NEWEST duplicate used to do — 344 times
    /// in one observed session.
    #[test]
    fn growing_a_transcript_never_rewrites_what_was_already_sent() {
        let guidance = format!("[lsp diagnostics] {}", "warning: unused import ".repeat(20));
        let mut messages = vec![
            Message::system(&guidance),
            Message::user("first"),
            Message::assistant("first answer"),
        ];
        let before = to_llm(&messages, 100_000, 4_096, 2);

        messages.push(Message::system(&guidance));
        messages.push(Message::user("second"));
        let after = to_llm(&messages, 100_000, 4_096, 2);

        assert!(after.len() > before.len(), "the transcript really did grow");
        for (index, sent) in before.iter().enumerate() {
            assert_eq!(
                sent.content, after[index].content,
                "message {index} was rewritten after it had already been sent"
            );
        }
    }

    /// A marker longer than the message it replaces would spend tokens to save them. Short system
    /// messages are routing markers ("memory", "shell/diagnose") and stay whole.

    #[test]
    fn repeated_contract_and_guidance_context_converges_to_a_bounded_provider_view() {
        let mut messages = Vec::new();
        for turn in 0..100 {
            messages.push(Message::system(format!("Turn contract: turn {turn}")));
            messages.push(Message::system("same standing guidance"));
        }

        let output = to_llm(&messages, 100_000, 4_096, 2);

        assert_eq!(
            output.len(),
            2,
            "one latest contract plus one guidance copy"
        );
        assert!(output
            .iter()
            .any(|message| message.content == "Turn contract: turn 99"));
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

    /// The waste this removes, in the shape it was measured: one real day of one real session
    /// carried 280 redundant copies across 84 groups — 1.10M characters of byte-identical tool
    /// output re-sent to the model, including an 84,521-char file carried five times.
    /// The measurement behind this change, taken on the mechanism itself.
    ///
    /// Prompt caches key on a PREFIX, so the moment a request rewrites a message it has already
    /// sent, every cached token after that point is discarded. The elision boundary WAS measured
    /// from the tail, so each new tool result pushed it forward and elided a result that had been
    /// verbatim a moment earlier — one such rewrite per round-trip, forever.
    ///
    /// On real traffic that cost 19.9M fresh tokens across 160 requests (87% of all fresh input),
    /// while the 453 well-cached requests carried 1.5M between them.
    ///
    /// Frozen at the turn's start, the boundary does not move for the whole turn, so nothing
    /// already sent is ever rewritten.
    #[test]
    fn the_elision_boundary_does_not_move_during_a_turn() {
        let synthetic = std::collections::HashSet::new();
        let mut transcript = vec![
            Message::system("preamble"),
            Message::user("go do the thing"),
        ];

        let mut frozen = Vec::new();
        let mut sliding = Vec::new();
        for i in 0..8 {
            let id = format!("call{i}");
            transcript.push(Message::assistant_tool_calls("", vec![tool_call(&id)]));
            transcript.push(Message::tool_result(
                &id,
                format!("r{i}: {}", "x".repeat(40_000)),
            ));
            frozen.push(elision_boundary(&transcript, &synthetic, 2));
            sliding.push(sliding_boundary(&transcript, &synthetic, 2));
        }

        assert!(
            sliding.windows(2).any(|w| w[0] != w[1]),
            "the tail-relative boundary must move, or there was never a problem: {sliding:?}"
        );
        // From the first step at which there are at least `keep_recent` results, the boundary is
        // fixed for the rest of the turn. Step 0 is degenerate — fewer results than `keep_recent`,
        // so the tail-relative value has nothing to count back from — and nothing is cached yet
        // anyway, so there is nothing to invalidate.
        assert!(
            frozen[1..].windows(2).all(|w| w[0] == w[1]),
            "the frozen boundary must not move once a turn is under way: {frozen:?}"
        );
        assert_eq!(
            frozen[1], 1,
            "it anchors on the user message that started the turn"
        );
        assert!(
            sliding.last() > sliding.first(),
            "the contrast is the point: {sliding:?} moved every step, {frozen:?} did not"
        );
    }

    /// And it DOES move at a turn boundary — one invalidation per turn instead of one per step,
    /// which is the whole trade.
    #[test]
    fn the_elision_boundary_advances_once_at_each_new_turn() {
        let synthetic = std::collections::HashSet::new();
        let mut transcript = vec![Message::user("first")];
        for i in 0..3 {
            let id = format!("a{i}");
            transcript.push(Message::assistant_tool_calls("", vec![tool_call(&id)]));
            transcript.push(Message::tool_result(&id, "y".repeat(40_000)));
        }
        let before = elision_boundary(&transcript, &synthetic, 2);
        transcript.push(Message::user("second"));
        let after = elision_boundary(&transcript, &synthetic, 2);
        assert!(
            after > before,
            "a new turn must let the boundary advance ({before} → {after})"
        );
    }

    /// The flip side: the freeze must not disable elision. An earlier turn's bulk still goes, or
    /// we have traded a cache problem for an unbounded-prompt problem.
    #[test]
    fn results_from_earlier_turns_are_still_elided() {
        let mut transcript = vec![Message::user("first task")];
        for i in 0..3 {
            let id = format!("old{i}");
            transcript.push(Message::assistant_tool_calls("", vec![tool_call(&id)]));
            transcript.push(Message::tool_result(
                &id,
                format!("old result {i}: {}", "y".repeat(40_000)),
            ));
        }
        transcript.push(Message::user("second task"));
        transcript.push(Message::assistant_tool_calls("", vec![tool_call("new")]));
        transcript.push(Message::tool_result(
            "new",
            format!("new: {}", "z".repeat(40_000)),
        ));

        let rendered = to_llm(&transcript, 1_000_000, 4_096, 2);
        // Semantic rather than a byte threshold: of the previous turn's three results, most must
        // be elided. Exactly one stays verbatim, and deliberately so — see below.
        let elided = rendered
            .iter()
            .filter(|m| m.content.starts_with("old result") && m.content.contains("elided"))
            .count();
        let verbatim = rendered
            .iter()
            .filter(|m| m.content.starts_with("old result") && m.content.len() > 39_000)
            .count();
        assert_eq!(elided, 2, "the previous turn's bulk must still be elided");
        assert_eq!(verbatim, 1);
        // One old result stays verbatim: `sliding` still protects the newest few regardless of
        // turn, and the `min` keeps whichever protects MORE. That is deliberate — a result that
        // was verbatim last request staying verbatim is exactly the cache stability being bought.
        assert!(
            rendered
                .iter()
                .any(|m| m.content.contains(&"z".repeat(1_000))),
            "the CURRENT turn's result stays verbatim"
        );
    }

    #[test]
    fn a_file_read_five_times_is_sent_once() {
        let body = "//! Forge Anywhere account, device, and host commands.\n".repeat(200);
        assert!(body.len() > DEDUPE_MIN_CHARS);
        let mut messages = Vec::new();
        for _ in 0..5 {
            messages.push(Message::new(Role::Assistant, "reading it again"));
            messages.push(Message::new(Role::Tool, &body));
        }
        let before: usize = messages.iter().map(|m| m.content.len()).sum();

        let reclaimed = dedupe_repeated_tool_results(&mut messages, 0);

        let verbatim: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == Role::Tool && m.content == body)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(verbatim.len(), 1, "exactly one copy survives verbatim");
        assert_eq!(reclaimed, (body.len() - DEDUPE_MARKER.len()) * 4);
        let after: usize = messages.iter().map(|m| m.content.len()).sum();
        assert!(after * 4 < before, "the bulk of the duplication is gone");
    }

    /// Prompt caches key on a PREFIX: rewriting a message at position N throws away every cached
    /// token from N onward. Keeping the newest copy would edit an OLD message every time a repeat
    /// appeared — discarding the cached prefix on exactly the requests that had the most of it to
    /// lose. Keeping the FIRST leaves everything before the repeat byte-identical forever. On the
    /// measured traffic 75-80% of input tokens were cache reads, so this is the difference between
    /// the fix paying for itself and it costing more than the duplication did.
    #[test]
    fn the_surviving_copy_is_the_first_so_the_cached_prefix_never_moves() {
        let body = "z".repeat(DEDUPE_MIN_CHARS + 1);
        let mut messages = vec![
            Message::new(Role::Tool, &body),
            Message::new(Role::Assistant, "later"),
            Message::new(Role::Tool, &body),
            Message::new(Role::Assistant, "later still"),
            Message::new(Role::Tool, &body),
        ];
        dedupe_repeated_tool_results(&mut messages, 0);
        assert_eq!(messages[0].content, body, "the earliest copy is untouched");
        assert_eq!(messages[2].content, DEDUPE_MARKER);
        assert_eq!(messages[4].content, DEDUPE_MARKER);
    }

    /// A tool loop that just produced data must get that DATA back, not a pointer to something
    /// further up the transcript. The newest results are exempt using the same window elision
    /// protects, so the exemption cannot drift away from it.
    #[test]
    fn the_newest_results_are_never_collapsed() {
        let body = "q".repeat(DEDUPE_MIN_CHARS + 1);
        let mut messages = vec![
            Message::new(Role::Tool, &body),
            Message::new(Role::Tool, &body),
            Message::new(Role::Tool, &body),
        ];
        dedupe_repeated_tool_results(&mut messages, 2);
        assert_eq!(messages[0].content, body, "the first copy always survives");
        assert_eq!(
            messages[1].content, body,
            "inside the protected window, so left verbatim"
        );
        assert_eq!(messages[2].content, body, "the newest is never collapsed");
    }

    /// The marker must state the RELATIONSHIP, not merely that something was dropped: "identical"
    /// answers "has this changed since I last looked", which a bare elision throws away and the
    /// model then spends another tool call to recover.
    #[test]
    fn a_collapsed_duplicate_says_it_was_identical() {
        let body = "x".repeat(DEDUPE_MIN_CHARS + 1);
        let mut messages = vec![
            Message::new(Role::Tool, &body),
            Message::new(Role::Tool, &body),
        ];
        dedupe_repeated_tool_results(&mut messages, 0);
        assert_eq!(messages[0].content, body, "the first copy survives");
        assert!(
            messages[1].content.contains("identical"),
            "{}",
            messages[1].content
        );
        assert!(
            messages[1].content.contains("unchanged"),
            "the marker must answer \"has this changed since I last looked\""
        );
    }

    #[test]
    fn results_that_differ_are_all_kept_and_small_ones_are_never_touched() {
        let big_a = "a".repeat(DEDUPE_MIN_CHARS + 1);
        let big_b = "b".repeat(DEDUPE_MIN_CHARS + 1);
        // Two identical SMALL results: collapsing these saves nothing and costs a marker line.
        let mut messages = vec![
            Message::new(Role::Tool, &big_a),
            Message::new(Role::Tool, &big_b),
            Message::new(Role::Tool, "ok"),
            Message::new(Role::Tool, "ok"),
        ];
        let reclaimed = dedupe_repeated_tool_results(&mut messages, 0);
        assert_eq!(reclaimed, 0);
        assert_eq!(messages[0].content, big_a);
        assert_eq!(messages[1].content, big_b);
        assert_eq!(messages[2].content, "ok");
        assert_eq!(messages[3].content, "ok");
    }

    /// Only tool results. Two user turns that happen to repeat a long quotation are the user
    /// saying something twice, which is evidence about the conversation, not redundant data.
    #[test]
    fn identical_non_tool_messages_are_left_alone() {
        let body = "y".repeat(DEDUPE_MIN_CHARS + 1);
        let mut messages = vec![
            Message::new(Role::User, &body),
            Message::new(Role::Assistant, &body),
            Message::new(Role::User, &body),
        ];
        assert_eq!(dedupe_repeated_tool_results(&mut messages, 0), 0);
        assert!(messages.iter().all(|m| m.content == body));
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

    #[test]
    fn native_history_sized_tool_pressure_stays_bounded_and_responsive() {
        let mut messages = Vec::with_capacity(14_000);
        // More turns and tool results than the largest real Codex/Claude history observed by the
        // aggregate-only history profiler (654 user turns / 6,995 tool calls).
        for turn in 0..1_024 {
            messages.push(Message::user(format!("continue checkpoint {turn}")));
            for tool in 0..10 {
                let id = format!("call-{turn}-{tool}");
                messages.push(Message::assistant_tool_calls("", vec![tool_call(&id)]));
                messages.push(Message::tool_result(
                    id,
                    format!("result {turn}-{tool}: {}", "x".repeat(1_024)),
                ));
            }
        }

        let started = std::time::Instant::now();
        let output = to_llm(&messages, 50_000, 4_096, 2);
        let output_tokens: usize = output.iter().map(message_tokens).sum();

        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "fitting 10,240 persisted tool results became pathologically slow: {:?}",
            started.elapsed()
        );
        assert!(
            output_tokens <= 50_000,
            "provider view exceeded its token budget: {output_tokens}"
        );
        assert!(
            output.len() < messages.len() / 10,
            "old tool pressure should collapse to a small provider view: {} of {} messages",
            output.len(),
            messages.len()
        );
        assert!(
            messages
                .iter()
                .filter(|message| message.role == Role::Tool)
                .all(|message| message.content.len() > 1_024),
            "request fitting must not mutate the persisted full-history messages"
        );
    }

    fn harness_entry(kind: &str, scope: &str, title: &str, content: &str) -> HarnessEntry {
        HarnessEntry {
            id: format!("{kind}-{title}"),
            scope: scope.to_string(),
            kind: kind.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            source: "refine".to_string(),
            version: 1,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn harness_context_block_is_none_for_empty_input_or_zero_cap() {
        assert!(harness_context_block(&[], 12, 2000).is_none());
        let entries = vec![harness_entry("prompt", "global", "t", "c")];
        assert!(harness_context_block(&entries, 0, 2000).is_none());
    }

    #[test]
    fn harness_context_block_caps_entries_and_labels_them() {
        let entries = vec![
            harness_entry("prompt", "session:s1", "note one", "content one"),
            harness_entry("skill", "project:/repo", "skill one", "content two"),
            harness_entry("subagent", "global", "agent one", "content three"),
        ];
        let block = harness_context_block(&entries, 2, 2000).unwrap();
        assert!(block.contains("note one"));
        assert!(block.contains("skill one"));
        assert!(!block.contains("agent one"), "capped at max_entries");
        assert!(block.contains("Learned harness context"));
    }

    #[test]
    fn harness_context_block_clamps_long_content() {
        let entries = vec![harness_entry("prompt", "global", "t", &"x".repeat(100))];
        let block = harness_context_block(&entries, 12, 10).unwrap();
        assert!(block.contains('…'));
        assert!(!block.contains(&"x".repeat(100)));
    }

    #[test]
    fn clamp_chars_leaves_short_text_untouched() {
        assert_eq!(clamp_chars("short", 10), "short");
        assert!(matches!(
            clamp_chars("short", 10),
            std::borrow::Cow::Borrowed(_)
        ));
    }
}
