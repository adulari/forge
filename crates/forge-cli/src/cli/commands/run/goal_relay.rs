//! The fresh-worker relay inside the goal loop (docs/features/goal-contract.md, "Fresh-worker
//! relay"). [`settle_goal`](super::settle_goal) decides a goal should move; [`perform`] creates the
//! successor session and ends this one's hold on the goal; the driver then brings the successor
//! up (the daemon spawns a hosted session, the TUI switches the live one) and
//! [`successor_outcome`] starts the goal there from the stored brief.

use forge_core::goal_relay::{RelayReason, RelayRequest};
use forge_tui::goal_status::{GoalPhase, GoalRelayLink};

use super::dispatch::DispatchOutcome;
use super::*;

/// The hand-off [`perform`] made.
#[derive(Debug, Clone)]
pub(crate) struct RelayPlan {
    pub(crate) successor_id: String,
    pub(crate) reason: RelayReason,
}

/// The relay chain `session_id` belongs to, oldest first, with the session holding the goal last.
pub(crate) fn relay_links(store: &forge_store::Store, session_id: &str) -> Vec<GoalRelayLink> {
    let chain = store.relay_chain(session_id).unwrap_or_default();
    let Some(last) = chain.last() else {
        return Vec::new();
    };
    let mut out: Vec<GoalRelayLink> = chain
        .iter()
        .map(|link| GoalRelayLink {
            session_id: link.predecessor_id.clone(),
            handed_off: Some(link.reason.clone()),
        })
        .collect();
    out.push(GoalRelayLink {
        session_id: last.successor_id.clone(),
        handed_off: None,
    });
    out
}

/// `S1 handed off (wedged) -> S2 holds the goal`, or `None` for a goal that never changed worker.
pub(crate) fn chain_line(links: &[GoalRelayLink]) -> Option<String> {
    if links.is_empty() {
        return None;
    }
    Some(
        links
            .iter()
            .enumerate()
            .map(|(i, link)| link.line(i + 1, "holds the goal"))
            .collect::<Vec<_>>()
            .join(" -> "),
    )
}

/// The first eight characters of a session id.
fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
}

/// The text after a stop message's `🎯 goal wedged — ` lead.
pub(crate) fn stop_detail(stop: &str) -> String {
    stop.split_once(" — ")
        .map_or(stop, |(_, rest)| rest)
        .trim()
        .to_string()
}

/// Hand the goal to a fresh worker: create the successor on the same worktree with the brief, end
/// this session's hold on the goal, and leave the successor a paused goal record it starts from.
pub(crate) async fn perform(
    gs: &GoalState,
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    reason: RelayReason,
    detail: &str,
) -> Result<RelayPlan, String> {
    let mut s = session.lock().await;
    let request = RelayRequest {
        goal: gs.goal.clone(),
        started_unix: gs.started_unix,
        reason,
        detail: detail.to_string(),
        contract: gs.contract.clone(),
        max_relays: s.relay_policy().max_relays,
        envelope: gs.envelope.as_ref().map(|e| e.meter()),
        tokens_used: gs.budget.tokens_used,
        iteration: gs.iter,
    };
    let hand_off = s.hand_off_goal(&request).await.map_err(|e| e.to_string())?;
    let store = s.store.clone();
    drop(s);
    app.goal_sync(
        gs.iter,
        gs.budget.tokens_used,
        gs.budget.started_at.elapsed().as_secs(),
    );
    let successor_id = hand_off.successor_id;
    app.goal_set_relay(relay_links(&store, &successor_id));
    let note = format!(
        "goal handed to a fresh worker ({}): {detail} -> {}",
        reason.label(),
        short(&successor_id)
    );
    app.goal_handed_off(&note);
    if let Some(goal) = app.goal.as_ref() {
        let record = GoalRecord::for_successor(goal, &note);
        store
            .set_session_goal_state(&successor_id, Some(&record))
            .map_err(|e| e.to_string())?;
    }
    session.lock().await.set_active_goal(None);
    app.note(&format!("🎯 {note}"));
    Ok(RelayPlan {
        successor_id,
        reason,
    })
}

/// The objective a brief opens with (`Goal: <objective>`), for a successor whose goal record never
/// got written.
fn objective_of_brief(brief: &str) -> Option<String> {
    let line = brief.lines().next()?.strip_prefix("Goal: ")?;
    Some(line.trim().to_string()).filter(|o| !o.is_empty())
}

/// The goal start of a session that was created by a hand-off and has not worked yet: its brief is
/// the first prompt and its goal comes back as the paused record the predecessor left. `None`
/// for every other session, including a successor that already ran a turn (a restart after that
/// restores the goal paused, like any other).
pub(crate) async fn successor_outcome(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
) -> Option<DispatchOutcome> {
    let (store, id) = {
        let s = session.lock().await;
        (s.store.clone(), s.id().to_string())
    };
    store.relay_predecessor(&id).ok().flatten()?;
    if store.next_message_seq(&id).ok()? > 0 {
        return None;
    }
    let brief = store.relay_brief(&id).ok().flatten()?;
    if app.goal.is_none() {
        let objective = objective_of_brief(&brief)?;
        app.goal_restore(
            &objective,
            &[],
            None,
            None,
            forge_tui::goal_status::GoalCarry {
                iteration: 0,
                tokens_used: 0,
                elapsed_secs: 0,
            },
            None,
            None,
            None,
            GoalPhase::Paused,
        );
    }
    app.goal_set_relay(relay_links(&store, &id));
    let goal = app.goal_prepare_resume()?;
    {
        let mut s = session.lock().await;
        let mut guidance = vec![goal_session_guidance(&goal.objective)];
        if let Some(contract) = goal.contract.as_ref().filter(|c| c.approved) {
            guidance.push(forge_types::goal_contract::worker_guidance(contract));
        }
        s.prime_guidance(&guidance).ok()?;
        s.set_active_goal(Some(&goal.objective));
    }
    app.note(&format!(
        "🎯 fresh worker taking over the goal from {} — {}",
        store
            .relay_predecessor(&id)
            .ok()
            .flatten()
            .map_or_else(String::new, |l| short(&l.predecessor_id).to_string()),
        goal.objective
    ));
    Some(DispatchOutcome::StartGoal {
        prompt: brief,
        goal: goal.objective,
        gates: goal.gates,
        max_tokens: goal.max_tokens,
        max_minutes: goal.max_minutes,
    })
}

/// The interactive terminal has one live session, so the successor takes its place: switch to it,
/// bring back its goal record and start the goal from the brief. `None` when the switch failed
/// (the predecessor stays on screen with the hand-off noted).
pub(crate) async fn switch_in_place(
    session: &Arc<tokio::sync::Mutex<Session>>,
    tui: Option<&mut forge_tui::Tui>,
    app: &mut forge_tui::App,
    plan: &RelayPlan,
) -> Option<DispatchOutcome> {
    let row = forge_tui::PickerRow {
        id: plan.successor_id.clone(),
        title: String::new(),
        subtitle: String::new(),
    };
    if let Err(error) =
        super::pickers::picker_accept(forge_tui::PickerKind::Sessions, &row, session, tui, app)
            .await
    {
        app.note(&format!("⚠ could not switch to the fresh worker: {error}"));
        return None;
    }
    if session.lock().await.id() != plan.successor_id {
        return None;
    }
    app.goal_clear();
    let store = open_store().ok()?;
    goal_envelope::restore(app, &store, &plan.successor_id);
    successor_outcome(session, app).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chain_reads_as_one_line_and_is_absent_without_a_relay() {
        assert_eq!(chain_line(&[]), None);
        let links = vec![
            GoalRelayLink {
                session_id: "a".into(),
                handed_off: Some("wedged".into()),
            },
            GoalRelayLink {
                session_id: "b".into(),
                handed_off: Some("worn".into()),
            },
            GoalRelayLink {
                session_id: "c".into(),
                handed_off: None,
            },
        ];
        assert_eq!(
            chain_line(&links).as_deref(),
            Some("S1 handed off (wedged) -> S2 handed off (worn) -> S3 holds the goal")
        );
    }

    #[test]
    fn a_stop_message_loses_its_lead() {
        assert_eq!(
            stop_detail("🎯 goal wedged — assertions 2 still fail after 3 validations"),
            "assertions 2 still fail after 3 validations"
        );
        assert_eq!(stop_detail("no lead"), "no lead");
    }
}
