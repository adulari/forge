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
