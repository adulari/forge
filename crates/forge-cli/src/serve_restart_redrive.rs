//! A daemon restart that cuts a turn off no longer passes silently.
//!
//! `resurrect_fleet` restores a session's transcript; this module notices when that transcript
//! ends mid-turn, leaves a note in its history and notifies the user's devices. A turn the user
//! stopped on purpose carries `USER_INTERRUPT_NOTE` and is left alone.

use std::sync::Arc;

use forge_store::TailState;

use crate::serve::DaemonState;

/// Dispatch workers are handled by the dispatch supervisor, which knows their task.
fn owned_by_dispatch(store: &forge_store::Store, id: &str) -> bool {
    matches!(store.dispatch_item_for_session(id), Ok(Some(_)))
}

/// Before the session is restored: true when its transcript ends mid-turn. Leaves the note in
/// its history so the restored transcript shows it.
pub(crate) fn mark_interrupted(store: &forge_store::Store, id: &str) -> bool {
    let tail = store.session_tail_state(id).unwrap_or(TailState::Finished);
    if !matches!(tail, TailState::Interrupted(_)) || owned_by_dispatch(store, id) {
        return false;
    }
    if let Err(error) = store.note_restart_interrupt(id, false) {
        tracing::warn!(session = %id, %error, "could not record the restart interruption");
    }
    true
}

/// After the session is back in the registry: push the news to the user's devices.
pub(crate) fn announce(state: Arc<DaemonState>, id: String) {
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
            body: "Interrupted by a daemon restart.".to_string(),
            seq: 0,
            ttl: crate::push::TTL_COMPLETION,
        };
        if let Some(apns) = &state.apns {
            apns.dispatch_alert(msg.clone());
        }
        if let Some(push) = &state.push {
            push.dispatch(msg);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_store::Store;
    use forge_types::Role;

    fn count_notes(store: &Store, id: &str) -> usize {
        store
            .load_all_messages(id)
            .unwrap()
            .iter()
            .filter(|m| m.content.starts_with(forge_store::RESTART_INTERRUPT_NOTE))
            .count()
    }

    #[test]
    fn a_cut_off_session_gets_a_note_in_its_history() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        store.add_message(&sid, 0, Role::User, "go", None).unwrap();
        assert!(mark_interrupted(&store, &sid));
        assert_eq!(count_notes(&store, &sid), 1);
    }

    #[test]
    fn finished_and_user_stopped_sessions_are_left_alone() {
        let store = Store::open_in_memory().unwrap();
        let done = store.create_session("/repo", "default").unwrap();
        store.add_message(&done, 0, Role::User, "go", None).unwrap();
        store
            .add_message(&done, 1, Role::Assistant, "done", None)
            .unwrap();
        assert!(!mark_interrupted(&store, &done));
        let stopped = store.create_session("/repo", "default").unwrap();
        store
            .add_message(&stopped, 0, Role::User, "go", None)
            .unwrap();
        store.note_user_interrupt(&stopped).unwrap();
        assert!(!mark_interrupted(&store, &stopped));
        assert_eq!(
            count_notes(&store, &done) + count_notes(&store, &stopped),
            0
        );
    }
}
