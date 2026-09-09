//! Whether to spend another continue-nudge on a model that stopped with work still open.
//!
//! A nudge costs a full provider call at the turn's current context size — on a long session that
//! is six figures of input tokens for one short reply. The model loop is therefore not allowed to
//! re-drive blindly: it asks here first, and this module owns the reasoning so it can be tested
//! without a session, a store, or a provider.

/// Evidence that a nudge produced something: tools executed, and tasks resolved.
///
/// These two are exactly what the nudge text demands ("call a tool or mark a task Done"), so they
/// are the honest measure of whether it worked.
pub(crate) type Progress = (u64, usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContinueNudge {
    /// Re-drive the model.
    Send,
    /// The last nudge changed nothing, so another cannot help — stop and say the model is blocked.
    BlockedStop,
    /// Nudges kept producing progress but the work is still open and the budget is spent.
    BudgetSpent,
}

/// Decide what to do with a model that ended its reply with tasks still unfinished.
///
/// The rule that matters: a nudge is only worth sending if the PREVIOUS one achieved something.
/// The bridge re-drive path has always gated on progress this way; the direct path did not, and a
/// model that had decided it was blocked was re-driven the entire budget, answering in prose every
/// time. Observed live on a real session: four extra provider calls at ~119k input each, none of
/// which could have changed anything.
///
/// Note what is deliberately NOT used: similarity between the model's replies. On the session that
/// motivated this, consecutive stalled replies were only 0.18-0.32 word-set similar to each other
/// against a 0.08-0.12 baseline for unrelated messages — a threshold able to catch them would also
/// fire on ordinary on-topic work. The signal has to be structural.
pub(crate) fn decide(
    nudges_sent: usize,
    max_nudges: usize,
    progress_at_last_nudge: Option<Progress>,
    progress_now: Progress,
) -> ContinueNudge {
    if nudges_sent > 0 && progress_at_last_nudge == Some(progress_now) {
        return ContinueNudge::BlockedStop;
    }
    if nudges_sent < max_nudges {
        return ContinueNudge::Send;
    }
    ContinueNudge::BudgetSpent
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
/// this, what it answered with was another round of reading to decide what one task meant — which
/// [`decide`] scores as progress, so the budget kept refilling. Naming the task and listing the
/// ways out (finish it, mark it Done, drop it, ask the user) is what ends that.
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

pub(crate) fn continuing_warning(unfinished: usize, sent: usize, max: usize) -> String {
    format!("model stopped with {unfinished} task(s) unfinished — continuing it ({sent}/{max})")
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

pub(crate) fn budget_spent_warning(unfinished: usize, max: usize) -> String {
    format!(
        "model stopped with {unfinished} task(s) unfinished after {max} continue nudge(s) — \
         giving up. Send `continue` to resume."
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
        assert_eq!(decide(0, 4, None, (0, 0)), ContinueNudge::Send);
    }

    #[test]
    fn a_nudge_that_changed_nothing_ends_the_turn_instead_of_spending_the_budget() {
        // The live failure: same tool count, same resolved-task count, so the model answered the
        // nudge without doing either thing it asked for.
        assert_eq!(
            decide(1, 4, Some((7, 2)), (7, 2)),
            ContinueNudge::BlockedStop
        );
        // And it stays stopped rather than resuming later in the budget.
        assert_eq!(
            decide(3, 4, Some((7, 2)), (7, 2)),
            ContinueNudge::BlockedStop
        );
    }

    #[test]
    fn a_tool_call_since_the_last_nudge_earns_another_one() {
        assert_eq!(decide(1, 4, Some((7, 2)), (8, 2)), ContinueNudge::Send);
    }

    #[test]
    fn resolving_a_task_counts_as_progress_even_with_no_tool_call() {
        // `update_tasks` is itself a tool, but a task can also be resolved by other means; either
        // way the nudge achieved what it asked for.
        assert_eq!(decide(1, 4, Some((7, 2)), (7, 3)), ContinueNudge::Send);
    }

    #[test]
    fn steady_progress_still_stops_at_the_cap() {
        // The budget is not removed, only spent more carefully: a model that keeps acting but never
        // closes the work is still bounded.
        assert_eq!(
            decide(4, 4, Some((7, 2)), (9, 2)),
            ContinueNudge::BudgetSpent
        );
    }
}
