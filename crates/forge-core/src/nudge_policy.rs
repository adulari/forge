//! Whether to spend another continue-nudge on a model that stopped with work still open.
//!
//! A nudge costs a full provider call at the turn's current context size — on a long session that
//! is six figures of input tokens for one short reply. The model loop is therefore not allowed to
//! re-drive blindly: it asks here first, and this module owns the reasoning so it can be tested
//! without a session, a store, or a provider.

/// Evidence that a nudge produced something: `(tools executed, tasks resolved)`.
///
/// Both are meaningful, but they are NOT equal. A tool running is *activity*; a task moving to Done
/// is *convergence*. A model can run tools forever without ever finishing anything — the "goalless
/// work / random shells" loop — so the two are tracked separately and judged differently below.
pub(crate) type Progress = (u64, usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContinueNudge {
    /// Re-drive the model — it is still converging (or has not been tried yet).
    Send,
    /// The last nudge changed nothing at all, so another cannot help — stop and say it is blocked.
    BlockedStop,
    /// Tools keep running but no task has completed for many nudges — likely goalless. Stop and
    /// hand it back to the user rather than spend the whole step budget spinning.
    GoallessStop,
}

/// Consecutive nudges that ran tools but completed no task before the loop is judged goalless.
///
/// Deliberately high: genuine multi-step work legitimately runs many tools between task
/// completions, and every completed task resets the count. This is not a cap on useful work — it
/// is the ceiling on work that is demonstrably not converging. Replaces the old fixed
/// `MAX_CONTINUE_NUDGES = 4`, which gave up on a model that WAS still finishing tasks.
pub(crate) const GOALLESS_NUDGE_LIMIT: usize = 8;

/// Decide what to do with a model that ended its reply with tasks still unfinished.
///
/// The rules, in order:
/// 1. Nothing moved since the last nudge (same tools, same Done count) → `BlockedStop`. A nudge
///    that changed nothing cannot be improved on by repetition. (The direct path used to re-drive
///    the whole budget here; observed live as four ~119k-input calls answered in prose each time.)
/// 2. A task completed since the last nudge → `Send`, with no fixed ceiling. Real convergence is
///    allowed to run to completion — this is the "keep going until done" the user asked for.
/// 3. Tools ran but no task completed, and this has now happened `GOALLESS_NUDGE_LIMIT` times in a
///    row → `GoallessStop`. Activity without convergence is the goalless-loop signal; hand it back.
/// 4. Otherwise (tool progress, under the goalless ceiling) → `Send`.
///
/// `tool_only_streak` is how many prior consecutive nudges ran tools without completing a task; the
/// caller resets it to zero whenever a task completes.
///
/// Note what is deliberately NOT used: textual similarity between replies. On the session that
/// motivated the original guard, consecutive stalled replies were only 0.18–0.32 word-set similar
/// (against a 0.08–0.12 baseline for unrelated messages) — a threshold able to catch them would
/// also fire on ordinary on-topic work. The signal has to be structural.
pub(crate) fn decide(
    tool_only_streak: usize,
    progress_at_last_nudge: Option<Progress>,
    progress_now: Progress,
) -> ContinueNudge {
    let Some(prev) = progress_at_last_nudge else {
        return ContinueNudge::Send; // nothing tried yet — the first nudge is always worth it
    };
    if progress_now == prev {
        return ContinueNudge::BlockedStop;
    }
    if progress_now.1 > prev.1 {
        return ContinueNudge::Send; // a task completed — real progress, no ceiling
    }
    if tool_only_streak >= GOALLESS_NUDGE_LIMIT {
        return ContinueNudge::GoallessStop;
    }
    ContinueNudge::Send
}

/// The re-drive instruction. Names the two things that count as progress, which is exactly what
/// [`decide`] measures — so the model is told the rule it is actually being judged by.
pub(crate) const CONTINUE_NUDGE: &str =
    "You ended your reply, but tasks on your list are NOT yet Done. The turn is not over — do not \
     stop. Continue now: call the next tool to make progress on the remaining work. Only finish \
     once every task is resolved; if one is genuinely complete or impossible, mark it Done via \
     update_tasks and say why. Do not reply again without either calling a tool or marking a task \
     Done.";

/// Maximum unfinished titles quoted back to the model; a 30-task list would otherwise re-enter the
/// prompt in full on every nudge.
const NAMED_TASKS: usize = 6;

/// The titles of the tasks that are not Done — what the completion gate counts, and what a repeat
/// nudge quotes back.
pub(crate) fn open_titles(tasks: &[forge_types::TodoItem]) -> Vec<String> {
    tasks
        .iter()
        .filter(|t| t.status != forge_types::TodoStatus::Done)
        .map(|t| t.title.clone())
        .collect()
}

/// The nudge to send, given how many have already gone out this turn.
///
/// The first one is the generic instruction above. From the second on it names the tasks that are
/// still open and demands a decision about them, because by then the generic version has demonstrably
/// not worked: the model answered it and the work is still open. On the session that motivated
/// this, what it answered with was another round of reading to decide what one task meant. Naming
/// the task and listing the ways out (finish it, mark it Done, drop it, ask the user) is what ends
/// that.
pub(crate) fn continue_nudge(unfinished: &[String], sent: usize) -> String {
    if sent <= 1 || unfinished.is_empty() {
        return CONTINUE_NUDGE.to_string();
    }
    let named = unfinished
        .iter()
        .take(NAMED_TASKS)
        .map(|t| format!("- {t}"))
        .collect::<Vec<_>>()
        .join("\n");
    let more = unfinished.len().saturating_sub(NAMED_TASKS);
    let more = if more > 0 {
        format!("\n- (and {more} more)")
    } else {
        String::new()
    };
    format!(
        "{CONTINUE_NUDGE}\n\nThis is nudge {sent} of this turn and these tasks are still \
         open:\n{named}{more}\n\nGathering more information about them does not count. Pick the \
         first one and, in this step, either carry it out with a concrete tool call, or resolve it \
         on the list: mark it Done via update_tasks and say in one line what you did, or remove it \
         via update_tasks because it is moot, or call ask_user if only the user can decide what it \
         means."
    )
}

pub(crate) fn continuing_warning(unfinished: usize, sent: usize) -> String {
    format!("model stopped with {unfinished} task(s) unfinished — continuing it (nudge {sent})")
}

/// Says the model is blocked rather than idle, and points at where the reason is — the model's own
/// last message, which a user reading only "giving up" would never think to look at.
pub(crate) fn blocked_warning(nudges: usize, unfinished: usize) -> String {
    format!(
        "model answered {nudges} continue nudge(s) without calling a tool or resolving a task — \
         it is blocked, not stalling. Stopping with {unfinished} task(s) unfinished; its last \
         message says why. Send `continue` once the blocker is resolved."
    )
}

/// Says the loop is spending effort without converging — the "goalless work" the user reported.
pub(crate) fn goalless_warning(unfinished: usize, nudges: usize) -> String {
    format!(
        "model ran tools across {nudges} continue nudge(s) but completed no task — this looks like \
         goalless work, not progress. Pausing with {unfinished} task(s) still open so you can \
         steer; send `continue` to keep going as-is."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_nudge_is_the_plain_instruction() {
        assert_eq!(continue_nudge(&["a".into()], 1), CONTINUE_NUDGE);
        // No tracked titles to name (bridge paths track them separately) — stay generic.
        assert_eq!(continue_nudge(&[], 3), CONTINUE_NUDGE);
    }

    #[test]
    fn a_repeat_nudge_names_the_open_tasks_and_the_ways_out() {
        let text = continue_nudge(&["Strip live-reuse".into(), "Re-run the bench".into()], 2);
        assert!(text.contains("- Strip live-reuse"));
        assert!(text.contains("- Re-run the bench"));
        assert!(text.contains("ask_user"));
        assert!(text.contains("does not count"));
    }

    #[test]
    fn a_long_list_is_capped_rather_than_pasted_back_whole() {
        let titles: Vec<String> = (0..12).map(|i| format!("task {i}")).collect();
        let text = continue_nudge(&titles, 2);
        assert!(text.contains("- task 5"));
        assert!(!text.contains("- task 6"));
        assert!(text.contains("(and 6 more)"));
    }

    #[test]
    fn the_first_nudge_is_always_worth_sending() {
        // Nothing has been tried yet, so there is no evidence it is pointless.
        assert_eq!(decide(0, None, (0, 0)), ContinueNudge::Send);
    }

    #[test]
    fn a_nudge_that_changed_nothing_ends_the_turn_instead_of_spending_the_budget() {
        // Same tool count, same resolved-task count: the model answered the nudge without doing
        // either thing it asked for, so another cannot help.
        assert_eq!(decide(0, Some((7, 2)), (7, 2)), ContinueNudge::BlockedStop);
        assert_eq!(decide(3, Some((7, 2)), (7, 2)), ContinueNudge::BlockedStop);
    }

    #[test]
    fn completing_a_task_keeps_going_with_no_fixed_ceiling() {
        // A task moved to Done — real convergence. Allowed even far past the old cap of 4, and
        // even past the goalless ceiling, because completing tasks is exactly "progress".
        assert_eq!(decide(0, Some((7, 2)), (8, 3)), ContinueNudge::Send);
        assert_eq!(decide(100, Some((7, 2)), (99, 3)), ContinueNudge::Send);
    }

    #[test]
    fn tool_progress_without_finishing_anything_is_allowed_up_to_the_goalless_ceiling() {
        // Tools ran, no task completed: fine while under the ceiling...
        assert_eq!(decide(0, Some((7, 2)), (8, 2)), ContinueNudge::Send);
        assert_eq!(
            decide(GOALLESS_NUDGE_LIMIT - 1, Some((7, 2)), (8, 2)),
            ContinueNudge::Send
        );
        // ...but once it has run tools without finishing anything this many times, it is goalless.
        assert_eq!(
            decide(GOALLESS_NUDGE_LIMIT, Some((7, 2)), (8, 2)),
            ContinueNudge::GoallessStop
        );
    }
}
