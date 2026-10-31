//! The chat's permanent session delete: `/delete` for the current session and Ctrl+K in the
//! sessions picker for any other. Both open the same typed-id confirmation; this module fills it
//! from the store and carries out the confirmed delete through the shared checks.

use std::sync::Arc;

use forge_store::Store;
use forge_tui::{App, SessionDeleteDialog};

use crate::session_delete::{self, Refusal};

/// Open the confirmation for `id`: the dialog lists what goes, or states why it cannot.
pub(super) async fn open(app: &mut App, store: &Arc<Store>, id: &str, current: bool) {
    let preview = {
        let store = Arc::clone(store);
        let id = id.to_string();
        tokio::task::spawn_blocking(move || store.session_delete_preview(&id)).await
    };
    let Ok(Ok(Some(preview))) = preview else {
        app.note("⚠ that session no longer exists");
        return;
    };
    let refusal = session_delete::check_with(store, id, false, current)
        .await
        .err();
    app.session_delete = Some(session_delete::dialog_for(
        &preview,
        current,
        refusal.as_ref(),
    ));
}

/// Re-run the checks right before the destructive step. `Some` is a refusal to show inline.
pub(super) async fn precheck(store: &Arc<Store>, dialog: &SessionDeleteDialog) -> Option<Refusal> {
    session_delete::check_with(store, &dialog.id, false, dialog.current)
        .await
        .err()
}

pub(super) fn show_refusal(app: &mut App, refusal: &Refusal) {
    if let Some(dialog) = app.session_delete.as_mut() {
        dialog.refusal = Some((refusal.message(), refusal.next_step().to_string()));
    }
}

fn done_note(id: &str, done: &session_delete::Deleted) -> String {
    format!(
        "✓ deleted session {} permanently ({} session(s){})",
        &id[..id.len().min(8)],
        done.sessions,
        if done.wal_truncated {
            ", database scrubbed"
        } else {
            ", database scrub finishes at the next checkpoint"
        }
    )
}

/// Delete a session other than the one this chat is driving (picker path). The picker is
/// refreshed by the caller.
pub(super) async fn delete_other(
    app: &mut App,
    store: &Arc<Store>,
    dialog: &SessionDeleteDialog,
) -> bool {
    match session_delete::delete_anywhere(store, &dialog.id).await {
        Ok(done) => {
            app.session_delete = None;
            app.note(&done_note(&dialog.id, &done));
            true
        }
        Err(refusal) => {
            show_refusal(app, &refusal);
            false
        }
    }
}

/// Delete the session this chat is driving. The caller has stopped the turn. The chat first moves
/// to a fresh session (which also hands over this terminal's claim on the old one), then the old
/// one goes through the normal checks, so a refusal leaves it intact and resumable.
pub(super) async fn delete_current(
    app: &mut App,
    session: &Arc<tokio::sync::Mutex<forge_core::Session>>,
    tui: &mut forge_tui::Tui,
    dialog: &SessionDeleteDialog,
) {
    let old = dialog.id.clone();
    let store = {
        let mut s = session.lock().await;
        let cwd = s.workspace_scope();
        if let Err(error) = s.reset_fresh_hooked(&cwd).await {
            app.note(&format!(
                "⚠ nothing deleted, could not start a fresh session: {error}"
            ));
            app.session_delete = None;
            return;
        }
        // Only now that the chat has moved on: a failed reset must not kill the subagents of a
        // session the user keeps.
        s.cancel_all_detached(&old);
        s.store.clone()
    };
    let _ = store.set_session_local_live(&old, false);
    tui.clear_screen();
    app.clear_transcript();
    app.reset_session_stats();
    app.session_delete = None;
    match session_delete::delete_anywhere(&store, &old).await {
        Ok(done) => {
            app.note(&done_note(&old, &done));
            app.note("● new session");
        }
        Err(refusal) => {
            app.note(&format!(
                "⚠ not deleted: {}. The session is intact: /resume {} brings it back.",
                refusal.full(),
                &old[..old.len().min(8)]
            ));
        }
    }
}
