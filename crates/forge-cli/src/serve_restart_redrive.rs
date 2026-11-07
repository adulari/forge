//! A daemon restart no longer drops the turn a session was running.
//!
//! `resurrect_fleet` restores a session's transcript; this module notices when that transcript
//! ends mid-turn, tells the user, and re-drives the session once with a continue prompt. User
//! interrupts leave their own note (`USER_INTERRUPT_NOTE`), so a turn stopped on purpose is not
//! restarted behind the user's back.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use forge_store::TailState;

use crate::remote::RemoteInput;
use crate::serve::DaemonState;

/// How long a restored session sits idle before it is re-driven; matches the dispatch
/// supervisor's `RESTART_IDLE_GRACE`, which gives queued prompts and a client's own retry the
/// first move.
#[cfg(not(test))]
const REDRIVE_GRACE: Duration = Duration::from_secs(30);
#[cfg(test)]
const REDRIVE_GRACE: Duration = Duration::from_millis(300);

pub(crate) const CONTINUE_PROMPT: &str = "The daemon restarted while you were working, which cut your last turn off. Check the current state of the workspace (git status, files you were editing, running processes) before redoing anything, then continue the task from where you left off and finish it.";

/// Sessions already re-driven since this daemon started.
#[derive(Default)]
pub(crate) struct RedriveLedger(HashSet<String>);

impl RedriveLedger {
    /// True the first time an interrupted session is seen; never again for the same id.
    pub(crate) fn claim(&mut self, id: &str, tail: TailState) -> bool {
        matches!(tail, TailState::Interrupted(_)) && self.0.insert(id.to_string())
    }
}

/// Whether the delayed re-drive should still fire: the session is idle and still ends mid-turn.
/// A client that prompted it meanwhile makes it busy or moves the tail on.
pub(crate) fn still_due(tail: TailState, busy: bool) -> bool {
    !busy && matches!(tail, TailState::Interrupted(_))
}

/// Dispatch workers are re-driven by the dispatch supervisor, which knows their task.
fn owned_by_dispatch(store: &forge_store::Store, id: &str) -> bool {
    matches!(store.dispatch_item_for_session(id), Ok(Some(_)))
}

/// Before the session is restored: decide whether it is an interrupted one this daemon should
/// re-drive, and leave the note in its history so the restored transcript shows it.
pub(crate) fn mark_interrupted(
    store: &forge_store::Store,
    ledger: &mut RedriveLedger,
    id: &str,
) -> bool {
    // A turn still open belongs to the process that just died: record why before anything else.
    if let Err(error) = store.close_open_session_turns_as(id, "daemon_restart_or_crash") {
        tracing::warn!(session = %id, %error, "could not close the turn the restart cut off");
    }
    let tail = store.session_tail_state(id).unwrap_or(TailState::Finished);
    if !matches!(tail, TailState::Interrupted(_)) || owned_by_dispatch(store, id) {
        return false;
    }
    if !ledger.claim(id, tail) {
        return false;
    }
    if let Err(error) = store.note_restart_interrupt(id, true) {
        tracing::warn!(session = %id, %error, "could not record the restart interruption");
    }
    true
}

/// After the session is back in the registry: notify the user's devices and schedule the
/// continue prompt.
pub(crate) fn announce_and_schedule(state: Arc<DaemonState>, id: String) {
    tokio::spawn(async move {
        let Some(handle) = state.registry.get(&id).await else {
            return;
        };
        let title = handle.title();
        let title = if title.is_empty() {
            format!("session {}", &id[..id.len().min(8)])
        } else {
            title
        };
        let msg = crate::push::PushMessage {
            kind: "failed",
            session: id.clone(),
            title,
            body: "Interrupted by a daemon restart. Forge is continuing it.".to_string(),
            seq: 0,
            ttl: crate::push::TTL_COMPLETION,
        };
        if let Some(apns) = &state.apns {
            apns.dispatch_alert(msg.clone());
        }
        if let Some(push) = &state.push {
            push.dispatch(msg);
        }

        tokio::time::sleep(REDRIVE_GRACE).await;
        let Some(handle) = state.registry.get(&id).await else {
            return;
        };
        let busy = handle.snapshot_rx.borrow().snapshot.busy;
        let tail = state
            .store
            .session_tail_state(&id)
            .unwrap_or(TailState::Finished);
        if !still_due(tail, busy) {
            return;
        }
        let _ = handle
            .input_tx
            .send(RemoteInput::Prompt {
                text: CONTINUE_PROMPT.to_string(),
                attachments: None,
            })
            .await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_store::{InterruptedTail, Store};
    use forge_types::Role;

    const CUT_OFF: TailState = TailState::Interrupted(InterruptedTail::UnansweredPrompt);

    #[test]
    fn the_restart_sweep_closes_the_turn_the_dead_process_left_open() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/repo", "default").unwrap();
        store.add_message(&id, 0, Role::User, "go", None).unwrap();
        store.open_turn(&id, 0).unwrap();
        mark_interrupted(&store, &mut RedriveLedger::default(), &id);
        let turn = &store.recent_turns(&id, 1).unwrap()[0];
        assert_eq!(turn.end_reason.as_deref(), Some("daemon_restart_or_crash"));
        assert_eq!(turn.last_role_shape.as_deref(), Some("unanswered_prompt"));
    }

    #[test]
    fn ledger_claims_an_interrupted_session_once() {
        let mut ledger = RedriveLedger::default();
        assert!(ledger.claim("a", CUT_OFF));
        assert!(!ledger.claim("a", CUT_OFF), "once per daemon start");
        assert!(ledger.claim("b", CUT_OFF));
    }

    #[test]
    fn ledger_ignores_finished_and_user_interrupted_sessions() {
        let mut ledger = RedriveLedger::default();
        assert!(!ledger.claim("a", TailState::Finished));
        assert!(!ledger.claim("a", TailState::UserInterrupted));
        assert!(
            ledger.claim("a", CUT_OFF),
            "a skipped session is not consumed"
        );
    }

    #[test]
    fn redrive_only_fires_for_an_idle_session_still_cut_off() {
        assert!(still_due(CUT_OFF, false));
        assert!(!still_due(CUT_OFF, true), "a client already prompted it");
        assert!(!still_due(TailState::Finished, false));
        assert!(!still_due(TailState::UserInterrupted, false));
    }

    #[test]
    fn mark_interrupted_notes_and_claims_a_cut_off_session_once() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        store.add_message(&sid, 0, Role::User, "go", None).unwrap();
        let mut ledger = RedriveLedger::default();
        assert!(mark_interrupted(&store, &mut ledger, &sid));
        let notes: Vec<_> = store
            .load_all_messages(&sid)
            .unwrap()
            .into_iter()
            .filter(|m| m.content.starts_with(forge_store::RESTART_INTERRUPT_NOTE))
            .collect();
        assert_eq!(notes.len(), 1);
        assert!(!mark_interrupted(&store, &mut ledger, &sid));
        assert_eq!(
            store
                .load_all_messages(&sid)
                .unwrap()
                .iter()
                .filter(|m| m.content.starts_with(forge_store::RESTART_INTERRUPT_NOTE))
                .count(),
            1,
            "no second note in the same daemon run"
        );
    }

    #[test]
    fn mark_interrupted_re_drives_a_turn_cut_off_after_a_steer() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        store.add_message(&sid, 0, Role::User, "go", None).unwrap();
        store
            .add_ui_note(&sid, 1, Role::Assistant, "first reply")
            .unwrap();
        store
            .add_message(&sid, 2, Role::User, "also do this", None)
            .unwrap();
        store
            .add_ui_note(&sid, 3, Role::System, forge_store::STEER_NOTE)
            .unwrap();
        store
            .add_ui_note(
                &sid,
                4,
                Role::System,
                "⏸ drill::flash: provider down since 10:18 (503), retrying every <=5 min",
            )
            .unwrap();
        assert!(mark_interrupted(
            &store,
            &mut RedriveLedger::default(),
            &sid
        ));
    }

    #[test]
    fn mark_interrupted_skips_finished_and_user_stopped_sessions() {
        let store = Store::open_in_memory().unwrap();
        let mut ledger = RedriveLedger::default();
        let done = store.create_session("/repo", "default").unwrap();
        store.add_message(&done, 0, Role::User, "go", None).unwrap();
        store
            .add_message(&done, 1, Role::Assistant, "done", None)
            .unwrap();
        assert!(!mark_interrupted(&store, &mut ledger, &done));
        let stopped = store.create_session("/repo", "default").unwrap();
        store
            .add_message(&stopped, 0, Role::User, "go", None)
            .unwrap();
        store.note_user_interrupt(&stopped).unwrap();
        assert!(!mark_interrupted(&store, &mut ledger, &stopped));
    }
}
