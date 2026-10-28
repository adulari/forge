//! Feeds the TUI's subagent session view. The child's conversation lives in the store (its own
//! session), so opening reads it from there and, while the child runs, re-reads it as it grows.
//! A separate store handle is used on purpose: the parent session's lock is held for a whole turn.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{DoneGuard, Session};

const POLL_EVERY: Duration = Duration::from_millis(400);

pub(crate) struct ChildFeed {
    store: Arc<forge_store::Store>,
    polled_at: Instant,
    seen: (usize, usize),
    settled: bool,
}

impl ChildFeed {
    pub(crate) fn new(store: Arc<forge_store::Store>) -> Self {
        Self {
            store,
            polled_at: Instant::now(),
            seen: (0, 0),
            settled: false,
        }
    }

    pub(crate) fn open(&mut self, app: &mut forge_tui::App, id: &str) -> bool {
        let Ok(items) = forge_core::stored_replay_items(&self.store, id) else {
            return false;
        };
        self.seen = signature(&items);
        self.polled_at = Instant::now();
        let opened = app.open_child_view(id, &items);
        self.settled = !app.child_view_running();
        opened
    }

    pub(crate) fn poll(&mut self, app: &mut forge_tui::App) -> bool {
        let Some(id) = app.child_view_id().map(str::to_owned) else {
            return false;
        };
        app.sync_child_view_tick();
        let running = app.child_view_running();
        if (!running && self.settled) || self.polled_at.elapsed() < POLL_EVERY {
            return false;
        }
        self.settled = !running;
        self.polled_at = Instant::now();
        let Ok(items) = forge_core::stored_replay_items(&self.store, &id) else {
            return false;
        };
        let sig = signature(&items);
        if sig == self.seen {
            return false;
        }
        self.seen = sig;
        app.refresh_child_view(&items);
        true
    }
}

/// Run `message` as a follow-up turn of subagent `child_id`, shaped like any other turn (spinner,
/// done signal) so the chat treats it as the work in flight. The reply lands in the child's own
/// session, which the open view is already following.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_child_message(
    child_id: String,
    message: String,
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut Instant,
) -> tokio::task::JoinHandle<()> {
    app.reopen_subagent(&child_id);
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    tokio::spawn(async move {
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        if let Err(e) = sess.message_child(&child_id, &message).await {
            sess.notify_error(&format!("message to subagent failed: {e}"));
        }
    })
}

fn signature(items: &[forge_types::ReplayItem]) -> (usize, usize) {
    let tail = match items.last() {
        Some(forge_types::ReplayItem::Assistant(text) | forge_types::ReplayItem::User(text)) => {
            text.len()
        }
        _ => 0,
    };
    (items.len(), tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_tui::PresenterEvent;
    use forge_types::Role;

    #[test]
    fn opens_a_stored_child_then_follows_it_while_it_runs() {
        let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
        let parent = store.create_session(".", "default").unwrap();
        let child = store.create_child_session(".", "default", &parent).unwrap();
        store
            .add_message(&child, 0, Role::User, "scan", None)
            .unwrap();

        let mut app = forge_tui::App::default();
        app.apply(PresenterEvent::SubagentStart {
            id: child.clone(),
            agent: "explore".into(),
            task: "scan".into(),
            model: None,
            phase: None,
        });
        let mut feed = ChildFeed::new(store.clone());
        assert!(!feed.open(&mut app, "missing-row"));
        assert!(feed.open(&mut app, &child));
        assert_eq!(app.child_view_id(), Some(child.as_str()));

        feed.polled_at = Instant::now() - POLL_EVERY;
        assert!(!feed.poll(&mut app), "nothing new yet");

        store
            .add_message(&child, 1, Role::Assistant, "done scanning", None)
            .unwrap();
        feed.polled_at = Instant::now() - POLL_EVERY;
        assert!(feed.poll(&mut app), "a new message refreshes the view");

        app.apply(PresenterEvent::SubagentResult {
            id: child.clone(),
            agent: "explore".into(),
            ok: true,
            summary: "ok".into(),
            cost_usd: 0.0,
        });
        store
            .add_message(&child, 2, Role::Assistant, "final word", None)
            .unwrap();
        feed.polled_at = Instant::now() - POLL_EVERY;
        assert!(
            feed.poll(&mut app),
            "one last read after the child finishes"
        );
        feed.polled_at = Instant::now() - POLL_EVERY;
        assert!(!feed.poll(&mut app), "then it stops reading");
    }
}
