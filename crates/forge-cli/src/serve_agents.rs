//! Subagent child sessions as nested, read-only sessions for the apps.
//!
//! A child lives in the store as an ordinary session row (`parent_session_id` set) with its own
//! transcript, but it has no driver in the registry, so `/ws?session=<child>` has nothing to
//! attach to. [`child_view_response`] answers that handshake with a store-backed view: the same
//! snapshot wire shape, refreshed while the child runs, with every input dropped. The transcript
//! itself rides the existing `/api/history` route, which already serves any stored session.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::WebSocketUpgrade;
use axum::http::StatusCode;
use axum::response::Response;
use forge_store::{ChildSessionInfo, DetachedChildStatus, Store};
use tokio::sync::{mpsc, watch};

use crate::remote::{self, EventLog, SnapParent, Snapshot, SnapshotFrame, EVENT_LOG_CAP};
use crate::serve::{err_response, DaemonState};

const BUSY_REFRESH: Duration = Duration::from_millis(800);
const IDLE_REFRESH: Duration = Duration::from_secs(3);
const TAIL_ROWS: usize = 2;
const TAIL_ROW_CHARS: usize = 400;

/// Remembers which snapshot subagent rows are persisted child sessions of the parent, so the
/// store is asked once per row id instead of once per snapshot frame.
#[derive(Default)]
pub(crate) struct ChildLinks {
    verdicts: HashMap<String, bool>,
}

impl ChildLinks {
    pub(crate) fn mark(&mut self, store: &Store, parent_id: &str, snap: &mut Snapshot) {
        for row in &mut snap.subagents {
            if row.id.is_empty() {
                continue;
            }
            let linked = *self
                .verdicts
                .entry(row.id.clone())
                .or_insert_with(|| store.is_child_of(&row.id, parent_id).unwrap_or(false));
            row.session_id = linked.then(|| row.id.clone());
        }
    }
}

/// Answer a `/ws` handshake for a session with no driver: a stored subagent child session gets
/// the read-only child view, anything else a 404.
pub(crate) async fn child_or_404(
    state: &Arc<DaemonState>,
    session: &str,
    since: u64,
    ws: WebSocketUpgrade,
) -> Response {
    let store = state.store.clone();
    let id = session.to_string();
    let known = tokio::task::spawn_blocking(move || store.child_session_info(&id))
        .await
        .ok()
        .and_then(Result::ok)
        .flatten();
    if known.is_none() {
        return err_response(StatusCode::NOT_FOUND, "no such session");
    }
    let state = state.clone();
    let child_id = session.to_string();
    ws.on_upgrade(move |socket| async move {
        let mut first = build_snapshot(&state, &child_id).await.unwrap_or_default();
        first.revision = 1;
        let (snapshot_tx, snapshot_rx) =
            watch::channel(Arc::new(SnapshotFrame::new(first.clone())));
        let events = Arc::new(std::sync::Mutex::new(EventLog::new(EVENT_LOG_CAP)));
        publish(&snapshot_tx, &events, first.clone());
        let (input_tx, mut input_rx) = mpsc::channel(8);
        let drain = tokio::spawn(async move { while input_rx.recv().await.is_some() {} });
        let refresher = tokio::spawn(refresh_loop(
            state,
            child_id,
            snapshot_tx,
            events.clone(),
            first,
        ));
        remote::pump_ws(socket, snapshot_rx, events, input_tx, since).await;
        refresher.abort();
        drain.abort();
    })
}

async fn build_snapshot(state: &DaemonState, child_id: &str) -> Option<Snapshot> {
    let store = state.store.clone();
    let id = child_id.to_string();
    let view = tokio::task::spawn_blocking(move || load_view(&store, &id))
        .await
        .ok()
        .flatten()?;
    let live = live_state(state, &view.info.parent_id, child_id).await;
    Some(view.into_snapshot(child_id, live))
}

async fn refresh_loop(
    state: Arc<DaemonState>,
    child_id: String,
    tx: watch::Sender<Arc<SnapshotFrame>>,
    events: Arc<std::sync::Mutex<EventLog>>,
    first: Snapshot,
) {
    let mut revision = first.revision;
    let mut last = first;
    loop {
        tokio::time::sleep(if last.busy {
            BUSY_REFRESH
        } else {
            IDLE_REFRESH
        })
        .await;
        let Some(mut snap) = build_snapshot(&state, &child_id).await else {
            let mut gone = last.clone();
            gone.closed = true;
            gone.revision = revision + 1;
            publish(&tx, &events, gone);
            return;
        };
        snap.revision = last.revision;
        if snap != last {
            revision += 1;
            snap.revision = revision;
            last = snap.clone();
            publish(&tx, &events, snap);
        }
    }
}

fn publish(
    tx: &watch::Sender<Arc<SnapshotFrame>>,
    events: &Arc<std::sync::Mutex<EventLog>>,
    snap: Snapshot,
) {
    let frame = Arc::new(SnapshotFrame::new(snap));
    if let Ok(mut log) = events.lock() {
        log.push(frame.snapshot.revision, frame.clone());
    }
    let _ = tx.send(frame);
}

struct LoadedView {
    info: ChildSessionInfo,
    tail: Vec<remote::HistoryRow>,
    detached_running: bool,
}

fn load_view(store: &Store, child_id: &str) -> Option<LoadedView> {
    let info = store.child_session_info(child_id).ok().flatten()?;
    let epoch = store.history_epoch_with(child_id, true).unwrap_or_default();
    let mut tail: Vec<remote::HistoryRow> = store
        .load_history_page_with(child_id, None, TAIL_ROWS, true)
        .unwrap_or_default()
        .into_iter()
        .map(|row| crate::cli::commands::run::map_history_row(row, epoch))
        .collect();
    tail.reverse();
    let detached_running = store
        .list_detached_children(&info.parent_id)
        .unwrap_or_default()
        .iter()
        .any(|d| d.child_id == child_id && d.status == DetachedChildStatus::Running);
    Some(LoadedView {
        info,
        tail,
        detached_running,
    })
}

/// What the parent's live snapshot says about this child, when the parent is running.
struct LiveRow {
    done: bool,
    ok: bool,
    last: String,
    parent_title: String,
}

async fn live_state(state: &DaemonState, parent_id: &str, child_id: &str) -> Option<LiveRow> {
    let handle = state.registry.get(parent_id).await?;
    let frame = handle.snapshot_rx.borrow().clone();
    let row = frame.snapshot.subagents.iter().find(|s| s.id == child_id)?;
    Some(LiveRow {
        done: row.done,
        ok: row.ok,
        last: row.last.clone(),
        parent_title: frame.snapshot.title.clone(),
    })
}

impl LoadedView {
    fn into_snapshot(self, child_id: &str, live: Option<LiveRow>) -> Snapshot {
        let busy = self.detached_running || live.as_ref().is_some_and(|l| !l.done);
        let tail_text: Vec<String> = self
            .tail
            .iter()
            .map(|r| forge_types::truncate_ellipsis(&r.content, TAIL_ROW_CHARS))
            .collect();
        let tail_rows = self
            .tail
            .iter()
            .zip(&tail_text)
            .map(|(r, text)| remote::SnapTranscriptRow {
                kind: r.kind.clone(),
                text: text.clone(),
                tool: r.tool.clone(),
                meta: None,
            })
            .collect();
        let parent_title = live
            .as_ref()
            .map(|l| l.parent_title.clone())
            .filter(|t| !t.is_empty())
            .or(self.info.parent_title.clone())
            .unwrap_or_default();
        let outcome = live
            .as_ref()
            .filter(|l| l.done)
            .map(|l| if l.ok { "success" } else { "failed" }.to_string());
        Snapshot {
            session_id: child_id.to_string(),
            title: self.info.title.clone().unwrap_or_else(|| "subagent".into()),
            cwd: self.info.cwd,
            exposure: "daemon".into(),
            busy,
            done: !busy,
            last_turn_outcome: outcome,
            temper: "Read-only".into(),
            model: self.info.model.unwrap_or_else(|| "—".into()),
            cost_usd: self.info.cost_usd,
            activity: live.map(|l| l.last).filter(|_| busy).unwrap_or_default(),
            transcript: tail_text,
            transcript_rows: tail_rows,
            transcript_total: self.info.message_count.max(0) as u64,
            parent: Some(SnapParent {
                id: self.info.parent_id,
                title: parent_title,
            }),
            ..Snapshot::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::Role;

    fn fixture() -> (Store, String, String) {
        let store = Store::open_in_memory().unwrap();
        let parent = store.create_session("/w", "Default").unwrap();
        store.set_session_title(&parent, "fix the bug").unwrap();
        let child = store
            .create_child_session("/w", "Default", &parent)
            .unwrap();
        store.set_session_title(&child, "explore").unwrap();
        store
            .add_message(&child, 0, Role::User, "look around", None)
            .unwrap();
        store
            .add_message(&child, 1, Role::Assistant, "found it", Some("m-1"))
            .unwrap();
        (store, parent, child)
    }

    #[test]
    fn a_child_snapshot_names_its_parent_and_is_idle_without_a_live_row() {
        let (store, parent, child) = fixture();
        let snap = load_view(&store, &child)
            .unwrap()
            .into_snapshot(&child, None);
        assert_eq!(snap.session_id, child);
        assert_eq!(snap.title, "explore");
        assert!(!snap.busy);
        let p = snap.parent.unwrap();
        assert_eq!(
            (p.id.as_str(), p.title.as_str()),
            (parent.as_str(), "fix the bug")
        );
        assert_eq!(snap.transcript, vec!["look around", "found it"]);
        assert_eq!(snap.transcript_rows[1].kind, "assistant");
        assert_eq!(snap.transcript_total, 2);
    }

    #[test]
    fn a_running_live_row_makes_the_child_busy_and_carries_its_activity() {
        let (store, _, child) = fixture();
        let live = LiveRow {
            done: false,
            ok: true,
            last: "reading src".into(),
            parent_title: "live title".into(),
        };
        let snap = load_view(&store, &child)
            .unwrap()
            .into_snapshot(&child, Some(live));
        assert!(snap.busy);
        assert_eq!(snap.activity, "reading src");
        assert_eq!(snap.parent.unwrap().title, "live title");
    }

    #[test]
    fn links_only_rows_that_are_children_of_this_parent() {
        let (store, parent, child) = fixture();
        let mut snap = Snapshot::default();
        for id in [child.as_str(), "toolu_bridge", ""] {
            snap.subagents.push(remote::SnapSubagent {
                id: id.into(),
                agent: "a".into(),
                task: String::new(),
                model: None,
                phase: None,
                last: String::new(),
                done: false,
                ok: true,
                cost: 0.0,
                permission_prompt: None,
                session_id: None,
            });
        }
        let mut links = ChildLinks::default();
        links.mark(&store, &parent, &mut snap);
        assert_eq!(
            snap.subagents[0].session_id.as_deref(),
            Some(child.as_str())
        );
        assert_eq!(snap.subagents[1].session_id, None);
        assert_eq!(snap.subagents[2].session_id, None);
    }
}
