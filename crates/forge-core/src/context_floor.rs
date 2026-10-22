//! The elision floor: one transcript index, below which old tool output may be shortened, that
//! only ever moves forward.
//!
//! Provider prompt caches key on a prefix, so every rewrite of an already-sent message discards
//! the cache from that message on. Forge used to shorten old tool output from four independent
//! places — a prune at every user turn, an elision boundary that jumped to each new turn, a dedupe
//! window counted from the tail, and an every-step aging pass — and each of them rewrote the
//! middle of the prompt on its own schedule. Now all of them act only below this floor, so while
//! it stands still every request is a pure append to the previous one. It advances in blocks
//! (see [`plan_advance`]) and under context pressure, which is the one deliberate cache break.

use forge_types::{Message, MessageKind, Role};

use crate::context_pipeline::{message_tokens, prune_one, PRUNE_KEEP_ROUNDS};
use crate::Session;

/// The floor and the transcript epoch it was set in. A different epoch means the transcript was
/// rewritten from outside the loop (compaction, rewind, reload), so the index no longer points at
/// the same message and is recomputed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FloorState {
    pub(crate) epoch: u64,
    pub(crate) index: usize,
}

impl Default for FloorState {
    fn default() -> Self {
        Self {
            epoch: u64::MAX,
            index: 0,
        }
    }
}

/// A window trim that later requests keep reusing while it still fits, so a session at its
/// window ceiling drops a block of old messages once instead of re-deriving the cut every call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrimState {
    pub(crate) epoch: u64,
    pub(crate) floor: usize,
    pub(crate) dropped: usize,
}

/// What the window trim did to the latest request, and what this turn has already done about it.
#[derive(Debug, Default)]
pub(crate) struct TrimWatch {
    /// What the most recent request build dropped; a `Cell` so the `&self` builders can set it.
    pub(crate) last: std::cell::Cell<TrimDrop>,
    /// The "window full, older messages left out" notice went out this turn.
    pub(crate) noticed: bool,
    /// Compactions this turn started because the trim would have dropped real history.
    pub(crate) compactions: u8,
    /// The last such compaction folded nothing, so another would only repeat the model call.
    pub(crate) stalled: bool,
}

impl TrimWatch {
    pub(crate) fn begin_turn(&mut self) {
        self.noticed = false;
        self.compactions = 0;
        self.stalled = false;
    }
}

/// Most compactions one turn starts in place of a trim. Each folds a whole stretch, so needing a
/// fourth means the working set itself no longer fits and a summary cannot help.
const MAX_TRIM_COMPACTIONS: u8 = 3;

/// Why the floor moved, for the cache instrument's miss label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FloorMove {
    /// Released only earlier turns.
    Advance,
    /// Released rounds of the current turn (what the old every-step aging pass did).
    AgeRounds,
}

/// Step `index` back to a position no transform can reorder across: a user or assistant message
/// (or either end). A tool result or a system message sitting between results can be moved by
/// call/result pairing, which would make "the messages below the floor" depend on what follows.
pub(crate) fn snap_to_boundary(messages: &[Message], index: usize) -> usize {
    let mut index = index.min(messages.len());
    while index > 0
        && index < messages.len()
        && !matches!(messages[index].role, Role::User | Role::Assistant)
    {
        index -= 1;
    }
    index
}

/// Index of the oldest of the last `keep_rounds` tool rounds, or the end when there are fewer.
fn tool_round_boundary(messages: &[Message], keep_rounds: usize) -> usize {
    if keep_rounds == 0 {
        return messages.len();
    }
    messages
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, m)| m.role == Role::Assistant && !m.tool_calls.is_empty())
        .nth(keep_rounds - 1)
        .map_or(0, |(i, _)| i)
}

/// Index of the `keep_recent`-th newest tool result (0 when there are fewer, the end when none
/// are protected): the newest results a tool loop just produced always stay verbatim.
fn recent_results_boundary(messages: &[Message], keep_recent: usize) -> usize {
    if keep_recent == 0 {
        return messages.len();
    }
    messages
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, m)| m.role == Role::Tool)
        .nth(keep_recent - 1)
        .map_or(0, |(i, _)| i)
}

/// Where the floor would go if it advanced now: past everything except the working set — the last
/// [`PRUNE_KEEP_ROUNDS`] tool rounds, the newest `keep_recent` results and the last
/// `keep_recent_msgs` messages.
pub(crate) fn advance_target(
    messages: &[Message],
    keep_recent: usize,
    keep_recent_msgs: usize,
) -> usize {
    let target = tool_round_boundary(messages, PRUNE_KEEP_ROUNDS)
        .min(recent_results_boundary(messages, keep_recent))
        .min(messages.len().saturating_sub(keep_recent_msgs));
    snap_to_boundary(messages, target)
}

/// The floor for a transcript Forge has no floor for yet (a resumed or rewritten one): everything
/// before the current turn, minus the newest results — the boundary elision used before the floor.
pub(crate) fn initial_floor(messages: &[Message], keep_recent: usize) -> usize {
    let recent = recent_results_boundary(messages, keep_recent);
    let turn_start = messages
        .iter()
        .rposition(Message::is_real_user)
        .unwrap_or(recent);
    snap_to_boundary(messages, recent.min(turn_start))
}

/// Tokens in `messages[from..to]`.
fn tokens_between(messages: &[Message], from: usize, to: usize) -> usize {
    messages[from.min(to)..to.min(messages.len())]
        .iter()
        .filter(|m| m.visibility.is_llm())
        .map(message_tokens)
        .sum()
}

/// Whether the floor should move from `floor` to `target`, and if so to where.
///
/// Unforced, it moves only when more than `block` tokens sit verbatim above it AND the move would
/// release at least half a block. The second condition is the hysteresis: without it a working set
/// that is itself larger than a block would move the floor one round per step, which is exactly the
/// per-step rewrite this replaces. Forced (context pressure) it moves whenever it can.
pub(crate) fn plan_advance(
    messages: &[Message],
    floor: usize,
    target: usize,
    block: usize,
    forced: bool,
) -> Option<usize> {
    if target <= floor {
        return None;
    }
    if forced {
        return Some(target);
    }
    let above = tokens_between(messages, floor, messages.len());
    let released = tokens_between(messages, floor, target);
    (above > block && released * 2 >= block).then_some(target)
}

/// Shorten every bulky tool result below `floor` in place. Idempotent: a cut result carries the
/// prune marker and is skipped, so with an unchanged floor this changes nothing.
pub(crate) fn prune_below(
    messages: &mut [Message],
    floor: usize,
    kept_outputs: &std::collections::HashMap<String, String>,
) -> usize {
    let floor = floor.min(messages.len());
    messages[..floor]
        .iter_mut()
        .map(|m| prune_one(m, kept_outputs))
        .sum()
}

/// The messages a window trim may never drop: every real user message of the current turn,
/// which includes the turn's originating request and any steer queued into it. Harness nudges
/// stored as `user` are not the task and do not count.
///
/// A turn starts at the newest real user message whose predecessor is not mid-tool-loop (a tool
/// result, an assistant message with calls, or a nudge); a message that does follow one is a
/// steer, so the walk continues back to the real message before it. Without this the trim kept
/// only the newest user message, and a steer evicted the request it was steering.
pub(crate) fn turn_protected(messages: &[Message]) -> Vec<usize> {
    let Some(mut start) = messages.iter().rposition(Message::is_real_user) else {
        return Vec::new();
    };
    loop {
        let mid_turn = messages[..start]
            .iter()
            .rev()
            .find(|m| m.role != Role::System)
            .is_some_and(|m| {
                m.role == Role::Tool
                    || (m.role == Role::Assistant && !m.tool_calls.is_empty())
                    || (m.role == Role::User && m.kind == MessageKind::Nudge)
            });
        if !mid_turn {
            break;
        }
        match messages[..start].iter().rposition(Message::is_real_user) {
            Some(earlier) => start = earlier,
            None => break,
        }
    }
    (start..messages.len())
        .filter(|&i| messages[i].is_real_user())
        .collect()
}

/// Fit `messages` into `budget` tokens, reusing a sticky front trim while it still fits.
///
/// Dropping the oldest messages shifts the whole prompt, so recomputing the cut on every call (as a
/// plain newest-first fill does) breaks the cache on every request once a session is at its window.
/// Here a cut, once needed, drops enough to land at 85% of the budget and is then kept for as long
/// as the result still fits — a smaller window after failover simply fails that check and re-fits.
/// System messages and the current turn's real user messages ([`turn_protected`]) are always kept.
#[cfg(test)]
pub(crate) fn fit_sticky(
    messages: Vec<Message>,
    budget: usize,
    trim: &mut Option<usize>,
) -> Vec<Message> {
    fit_sticky_counted(messages, budget, trim).0
}

/// What a trim dropped, for the notice and the compact-instead-of-trim decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TrimDrop {
    /// Messages cut from the view.
    pub(crate) messages: usize,
    /// Of those, user and assistant messages — real history, not just tool output.
    pub(crate) history: usize,
}

/// [`fit_sticky`] plus how much it dropped.
pub(crate) fn fit_sticky_counted(
    messages: Vec<Message>,
    budget: usize,
    trim: &mut Option<usize>,
) -> (Vec<Message>, TrimDrop) {
    let before: Vec<Role> = messages.iter().map(|m| m.role).collect();
    let out = fit_sticky_inner(messages, budget, trim);
    let count = |roles: &mut dyn Iterator<Item = Role>, only_history: bool| {
        roles
            .filter(|r| {
                *r != Role::System && (!only_history || matches!(r, Role::User | Role::Assistant))
            })
            .count()
    };
    let kept_all = count(&mut out.iter().map(|m| m.role), false);
    let kept_history = count(&mut out.iter().map(|m| m.role), true);
    let drop = TrimDrop {
        messages: count(&mut before.iter().copied(), false).saturating_sub(kept_all),
        history: count(&mut before.iter().copied(), true).saturating_sub(kept_history),
    };
    (out, drop)
}

fn fit_sticky_inner(
    messages: Vec<Message>,
    budget: usize,
    trim: &mut Option<usize>,
) -> Vec<Message> {
    let protected: std::collections::HashSet<usize> =
        turn_protected(&messages).into_iter().collect();
    let droppable: Vec<usize> = (0..messages.len())
        .filter(|&i| messages[i].role != Role::System)
        .collect();
    // Always sent, whatever is dropped: system messages and the turn's real user messages.
    let fixed: usize = messages
        .iter()
        .enumerate()
        .filter(|(i, m)| m.role == Role::System || protected.contains(i))
        .map(|(_, m)| message_tokens(m))
        .sum();
    // The fewest leading droppable messages to drop so the rest fits `limit`. Walks newest-first
    // and stops at the first message that does not fit, so a long history is never tokenized
    // past the part that is actually sent.
    let first_kept = |limit: usize| -> Option<usize> {
        let mut total = fixed;
        if total > limit {
            return None;
        }
        for slot in (0..droppable.len()).rev() {
            let index = droppable[slot];
            if protected.contains(&index) {
                continue;
            }
            total += message_tokens(&messages[index]);
            if total > limit {
                return Some(slot + 1);
            }
        }
        Some(0)
    };
    let fits_after = |n: usize| {
        let mut total = fixed;
        for &index in droppable[n.min(droppable.len())..].iter().rev() {
            if !protected.contains(&index) {
                total += message_tokens(&messages[index]);
                if total > budget {
                    return false;
                }
            }
        }
        total <= budget
    };
    if let Some(n) = (*trim).filter(|&n| fits_after(n)) {
        return drop_front(messages, &droppable[..n.min(droppable.len())], &protected);
    }
    if first_kept(budget) == Some(0) {
        *trim = None;
        return messages;
    }
    match first_kept(budget * 85 / 100) {
        Some(n) if n < droppable.len() => {
            *trim = Some(n);
            drop_front(messages, &droppable[..n], &protected)
        }
        _ => {
            *trim = None;
            crate::context_pipeline::fit_messages_owned(messages, budget)
        }
    }
}

fn drop_front(
    messages: Vec<Message>,
    dropped: &[usize],
    protected: &std::collections::HashSet<usize>,
) -> Vec<Message> {
    let dropped: std::collections::HashSet<usize> = dropped
        .iter()
        .copied()
        .filter(|i| !protected.contains(i))
        .collect();
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(i, m)| (!dropped.contains(&i)).then_some(m))
        .collect()
}

impl Session {
    /// Drop everything that describes the previous transcript's shape, for `/new` and `/resume`,
    /// which swap the transcript without a new epoch.
    pub(crate) fn forget_context_shape(&mut self) {
        self.context_floor.take();
        self.context_trim.take();
        self.cache = Default::default();
        self.tools_advertised_once = false;
    }

    /// The current elision floor, recomputed when the transcript was rewritten since it was set.
    pub(crate) fn elision_floor(&self) -> usize {
        let state = self.context_floor.get();
        if state.epoch == self.history_epoch && state.index <= self.transcript.len() {
            return state.index;
        }
        let index = initial_floor(
            &self.transcript,
            self.config.mesh.tool_result_context_keep_recent,
        );
        self.context_floor.set(FloorState {
            epoch: self.history_epoch,
            index,
        });
        index
    }

    /// The sticky trim for the current floor, if one is still valid.
    pub(crate) fn sticky_trim(&self, floor: usize) -> Option<usize> {
        self.context_trim
            .get()
            .filter(|t| t.epoch == self.history_epoch && t.floor == floor)
            .map(|t| t.dropped)
    }

    pub(crate) fn store_trim(&self, floor: usize, dropped: Option<usize>) {
        self.context_trim.set(dropped.map(|dropped| TrimState {
            epoch: self.history_epoch,
            floor,
            dropped,
        }));
    }

    /// Whether the request just built had to drop user or assistant messages and a compaction may
    /// still run in its place: the trim is the last resort, a summary keeps what it would drop.
    pub(crate) fn trim_calls_for_compaction(&self) -> bool {
        let watch = &self.trim_watch;
        watch.last.get().history > 0 && !watch.stalled && watch.compactions < MAX_TRIM_COMPACTIONS
    }

    /// Compact because the trim would otherwise drop real history, then rebuild the request with
    /// the result. Returns whether the transcript shrank; a compaction that folded nothing latches
    /// `stalled` so the turn does not pay for the same summary call on every step.
    pub(crate) async fn compact_instead_of_trim(&mut self, model: &str) -> bool {
        self.trim_watch.compactions += 1;
        let (before, after) = self.compact(true).await.unwrap_or_default();
        let shrank = after < before;
        if shrank {
            self.emit_context_gauge(model);
        } else {
            self.trim_watch.stalled = true;
        }
        shrank
    }

    /// The request body for `model`, with the trim as the last resort: where building it would
    /// drop user or assistant messages, summarize them first and rebuild from the compacted
    /// transcript; whatever is still left out is announced once per turn.
    pub(crate) async fn request_transcript(
        &mut self,
        model: &str,
        specs: &[forge_provider::ToolSpec],
    ) -> Vec<Message> {
        // The first request of a session waits for the startup window lookup right here, before
        // anything is sized from a window the store may not know yet.
        self.settle_window_lookup().await;
        let mut sent = self.transcript_with_preamble_and_tools(model, specs);
        if self.trim_calls_for_compaction() && self.compact_instead_of_trim(model).await {
            sent = self.transcript_with_preamble_and_tools(model, specs);
        }
        self.note_window_trim();
        crate::turn_trace::dispatching(model);
        sent
    }

    /// Say once per turn that the window is full and older messages are being left out of the
    /// request, since nothing else tells the person the model no longer sees them.
    pub(crate) fn note_window_trim(&mut self) {
        let dropped = self.trim_watch.last.get();
        if dropped.messages == 0 || self.trim_watch.noticed {
            return;
        }
        self.trim_watch.noticed = true;
        self.presenter
            .emit(forge_types::PresenterEvent::Warning(format!(
                "context window full: {} older message(s) are left out of this request. /compact \
             summarizes them instead of dropping them",
                dropped.messages
            )));
    }

    /// The token count at which an idle chat loop should auto-compact, given the gauge's
    /// `window`. For the model the session last ran (or its pin) this is exactly
    /// [`Self::compaction_trigger`], so a `[compact_cap]` entry and the free-model cap apply the
    /// same as in the turn loop; before any model has run it falls back to the global cap.
    pub fn idle_compaction_trigger(&self, window: u64) -> u64 {
        let model = self
            .pinned_model
            .as_ref()
            .and_then(|set| set.first())
            .or(self.gauge_model.as_ref());
        match model {
            Some(model) => self.compaction_trigger(model),
            None => crate::auto_compact_trigger_tokens(
                window,
                self.config.mesh.compact_cap_tokens,
                crate::AUTO_COMPACT_THRESHOLD,
            ),
        }
    }

    /// The compaction trigger for `model`, in tokens.
    pub(crate) fn compaction_trigger(&self, model: &str) -> u64 {
        let window = self.base_context_window(model) as u64;
        // A `[compact_cap]` entry for this model or its provider is the ceiling; otherwise the
        // global paid/free split decides.
        let cap = forge_config::compact_cap_for(model).unwrap_or_else(|| {
            crate::compaction_policy::compaction_cap(
                self.router.model_is_free(model),
                &self.config.mesh,
            )
        });
        crate::auto_compact_trigger_tokens(window, cap, crate::AUTO_COMPACT_THRESHOLD)
    }

    /// Advance the floor when a block of verbatim context has built up (or unconditionally when
    /// `forced`, under context pressure), then cut the bulky results it released. Returns the
    /// chars reclaimed; zero when the floor stayed put.
    pub(crate) fn advance_floor_if_due(&mut self, model: &str, forced: bool) -> usize {
        let floor = self.elision_floor();
        let target = advance_target(
            &self.transcript,
            self.config.mesh.tool_result_context_keep_recent,
            crate::COMPACT_KEEP_RECENT,
        );
        let block = self
            .config
            .mesh
            .context_floor_block_tokens
            .max(usize::try_from(self.compaction_trigger(model) / 4).unwrap_or(usize::MAX));
        let Some(next) = plan_advance(&self.transcript, floor, target, block, forced) else {
            return 0;
        };
        let turn_start = self.transcript.iter().rposition(Message::is_real_user);
        let movement = if turn_start.is_some_and(|start| next > start) {
            FloorMove::AgeRounds
        } else {
            FloorMove::Advance
        };
        self.context_floor.set(FloorState {
            epoch: self.history_epoch,
            index: next,
        });
        self.cache.note_floor_move(movement);
        prune_below(&mut self.transcript, next, &self.kept_outputs)
    }
}

#[cfg(test)]
#[path = "tests/context_floor.rs"]
mod tests;
