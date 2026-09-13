//! Plan & dispatch's host half against a real HTTP server on an ephemeral port: every new
//! [`BoardAction`] goes through `actions::perform` exactly as the render loop sends it, and the
//! fixture router records what arrived. Like `http_tests.rs`, the router is a stand-in serving the
//! contract's routes and shapes (docs: the dispatch contract), not `forge serve` itself.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post};
use axum::Router;
use forge_tui::board::{BoardAction, BoardEvent, ToastLevel};
use tokio::sync::mpsc;

use super::actions::{perform, Host};
use super::dispatch_client::fetch_dispatches;
use super::sockets::SocketPool;
use super::Ev;

const TOKEN: &str = "tok";

#[derive(Default)]
struct Fixture {
    /// `(path, body)` of every POST, in arrival order.
    posts: Mutex<Vec<(String, serde_json::Value)>>,
}

impl Fixture {
    fn record(&self, path: String, body: &Bytes) {
        let json = serde_json::from_slice(body).unwrap_or_default();
        self.posts.lock().unwrap().push((path, json));
    }

    fn paths(&self) -> Vec<String> {
        self.posts
            .lock()
            .unwrap()
            .iter()
            .map(|(p, _)| p.clone())
            .collect()
    }
}

fn json(status: StatusCode, value: serde_json::Value) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

fn dispatch_json(id: &str) -> serde_json::Value {
    let item = |index: usize, status: &str, session: &str| {
        serde_json::json!({
            "index": index, "title": format!("part {index}"), "prompt": "do it",
            "depends_on": [], "status": status, "session_id": session,
            "outcome": null, "started_at": 1, "finished_at": null
        })
    };
    serde_json::json!({
        "id": id, "coordinator_session_id": "c-1", "coordinator_title": "Dispatch: parser",
        "cwd": "/repo", "prompt": "split the parser", "summary": "four parts",
        "status": "running", "worktree": true, "permission_mode": "accept-edits",
        "max_running": 4, "max_items": 8, "created_at": 1, "updated_at": 2,
        "items": [
            item(3, "succeeded", "s-3"),
            item(1, "succeeded", "s-1"),
            item(2, "succeeded", "s-2"),
            item(4, "failed", "s-4"),
        ],
        "a_field_from_a_newer_daemon": true
    })
}

/// `POST /api/dispatches/{id}/merge` per fixture id; `old-1` plays a daemon without the route.
fn merge_reply(id: &str) -> Response {
    let merged = |index: usize, commit: Option<&str>| serde_json::json!({ "index": index, "title": format!("part {index}"), "commit": commit });
    match id {
        "d-1" => json(
            StatusCode::OK,
            serde_json::json!({
                "merged": [merged(1, Some("aaa")), merged(2, Some("bbb")), merged(3, None)],
                "stopped_at": null, "remaining": [], "base_branch": "main",
                "dispatch": dispatch_json(id),
            }),
        ),
        "d-conf" => json(
            StatusCode::OK,
            serde_json::json!({
                "merged": [merged(1, Some("aaa"))],
                "stopped_at": {
                    "index": 2, "title": "part 2", "reason": "merge conflicts",
                    "conflicts": ["src/a.rs", "src/b.rs"]
                },
                "remaining": [3], "base_branch": "main",
                "dispatch": dispatch_json(id),
            }),
        ),
        "d-shared" => json(
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "error": "this dispatch ran in a shared directory — there is nothing to merge" }),
        ),
        "d-dirty" => json(
            StatusCode::CONFLICT,
            serde_json::json!({
                "error": "the base repo has uncommitted changes — commit or stash them, then merge again",
                "dirty_files": ["README.md"]
            }),
        ),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `with_list`: whether the daemon knows `GET /api/dispatches` (an older one answers 404).
async fn serve(fixture: Arc<Fixture>, with_list: bool) -> String {
    let mut app = Router::new()
        .route(
            &format!("/{TOKEN}/api/dispatch"),
            post(|State(f): State<Arc<Fixture>>, body: Bytes| async move {
                f.record("dispatch".into(), &body);
                json(
                    StatusCode::OK,
                    serde_json::json!({
                        "dispatch_id": "d-1", "coordinator_session_id": "c-1",
                        "title": "Dispatch: parser"
                    }),
                )
            }),
        )
        .route(
            &format!("/{TOKEN}/api/dispatches/{{id}}"),
            get(|Path(id): Path<String>| async move { json(StatusCode::OK, dispatch_json(&id)) }),
        )
        .route(
            &format!("/{TOKEN}/api/dispatches/{{id}}/{{verb}}"),
            post(
                |State(f): State<Arc<Fixture>>,
                 Path((id, verb)): Path<(String, String)>,
                 body: Bytes| async move {
                    f.record(format!("{verb} {id}"), &body);
                    if verb == "merge" {
                        return merge_reply(&id);
                    }
                    if id == "done-1" {
                        return json(
                            StatusCode::CONFLICT,
                            serde_json::json!({ "error": "dispatch done-1 has already finished" }),
                        );
                    }
                    json(StatusCode::OK, dispatch_json(&id))
                },
            ),
        )
        .route(
            &format!("/{TOKEN}/api/sessions/{{id}}/{{verb}}"),
            post(
                |State(f): State<Arc<Fixture>>,
                 Path((id, verb)): Path<(String, String)>,
                 body: Bytes| async move {
                    f.record(format!("{verb} {id}"), &body);
                    if verb == "merge" && id == "s-2" {
                        return json(
                            StatusCode::CONFLICT,
                            serde_json::json!({
                                "error": "merge conflicts — resolve them by hand in the worktree",
                                "conflicts": ["src/a.rs", "src/b.rs"],
                                "branch": "forge/s-2",
                                "worktree": "/home/me/.forge/worktrees/wt-s2",
                            }),
                        );
                    }
                    json(StatusCode::OK, serde_json::json!({ "ok": true }))
                },
            ),
        );
    if with_list {
        app = app.route(
            &format!("/{TOKEN}/api/dispatches"),
            get(|| async { json(StatusCode::OK, serde_json::json!([dispatch_json("d-1")])) }),
        );
    }
    let app = app.with_state(fixture);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

struct Run {
    events: Vec<Ev>,
}

impl Run {
    fn toasts(&self) -> Vec<(ToastLevel, String)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                Ev::Board(BoardEvent::Toast(level, text)) => Some((*level, text.clone())),
                _ => None,
            })
            .collect()
    }
}

/// Perform one action through the real dispatcher and collect what it reported, waiting until it
/// toasts (every action ends in exactly one final toast).
async fn perform_one(base: &str, action: BoardAction) -> Run {
    let http = reqwest::Client::new();
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
    let (refresh_tx, _refresh_rx) = mpsc::unbounded_channel();
    let mut pool = SocketPool::new(base.to_string(), TOKEN.into(), ev_tx.clone());
    let mut clipboard = None;
    let mut host = Host {
        http: &http,
        base,
        token: TOKEN,
        ev: &ev_tx,
        refresh: &refresh_tx,
        pool: &mut pool,
        clipboard: &mut clipboard,
    };
    perform(action, &mut host);
    let mut events = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ev_rx.recv()).await {
            Ok(Some(ev)) => {
                let toast = matches!(ev, Ev::Board(BoardEvent::Toast(..)));
                events.push(ev);
                if toast {
                    // Give a trailing event (DispatchStarted follows its toast) a moment.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    while let Ok(more) = ev_rx.try_recv() {
                        events.push(more);
                    }
                    return Run { events };
                }
            }
            _ => panic!("the action never reported back"),
        }
    }
}

#[tokio::test]
async fn starting_a_dispatch_posts_the_form_and_names_the_coordinator() {
    let fixture = Arc::new(Fixture::default());
    let base = serve(fixture.clone(), true).await;
    let run = perform_one(
        &base,
        BoardAction::StartDispatch {
            cwd: "/repo".into(),
            prompt: "split the parser".into(),
            worktree: true,
            mode: "bypass".into(),
            max_running: 2,
            max_items: 5,
        },
    )
    .await;
    let posts = fixture.posts.lock().unwrap().clone();
    assert_eq!(posts[0].0, "dispatch");
    assert_eq!(
        posts[0].1,
        serde_json::json!({
            "cwd": "/repo", "prompt": "split the parser", "worktree": true,
            "mode": "bypass", "max_running": 2, "max_items": 5
        })
    );
    assert_eq!(run.toasts()[0].0, ToastLevel::Ok);
    assert!(run.events.iter().any(|e| matches!(
        e,
        Ev::Board(BoardEvent::DispatchStarted { dispatch_id, coordinator_session_id })
            if dispatch_id == "d-1" && coordinator_session_id == "c-1"
    )));
}

#[tokio::test]
async fn approve_sends_all_as_null_and_a_subset_as_a_list() {
    let fixture = Arc::new(Fixture::default());
    let base = serve(fixture.clone(), true).await;
    perform_one(
        &base,
        BoardAction::ApproveDispatch {
            id: "d-1".into(),
            selected: None,
        },
    )
    .await;
    let run = perform_one(
        &base,
        BoardAction::ApproveDispatch {
            id: "d-1".into(),
            selected: Some(vec![1, 3]),
        },
    )
    .await;
    let posts = fixture.posts.lock().unwrap().clone();
    assert_eq!(
        posts[0],
        (
            "approve d-1".into(),
            serde_json::json!({ "selected": null })
        )
    );
    assert_eq!(
        posts[1],
        (
            "approve d-1".into(),
            serde_json::json!({ "selected": [1, 3] })
        )
    );
    assert_eq!(run.toasts()[0].0, ToastLevel::Ok);
}

#[tokio::test]
async fn revise_and_cancel_reach_their_routes_and_a_refusal_reads_as_the_daemons_reason() {
    let fixture = Arc::new(Fixture::default());
    let base = serve(fixture.clone(), true).await;
    perform_one(
        &base,
        BoardAction::ReviseDispatch {
            id: "d-1".into(),
            feedback: "fold 2 into 1".into(),
        },
    )
    .await;
    let refused = perform_one(&base, BoardAction::CancelDispatch("done-1".into())).await;
    let posts = fixture.posts.lock().unwrap().clone();
    assert_eq!(
        posts[0],
        (
            "revise d-1".into(),
            serde_json::json!({ "feedback": "fold 2 into 1" })
        )
    );
    assert_eq!(posts[1].0, "cancel done-1");
    assert_eq!(
        refused.toasts()[0],
        (
            ToastLevel::Error,
            "dispatch done-1 has already finished".to_string()
        )
    );
}

#[tokio::test]
async fn merge_and_discard_report_success_and_a_conflict_names_files_and_worktree() {
    let fixture = Arc::new(Fixture::default());
    let base = serve(fixture.clone(), true).await;
    let ok = perform_one(&base, BoardAction::Merge("s-1".into())).await;
    let conflict = perform_one(&base, BoardAction::Merge("s-2".into())).await;
    let discarded = perform_one(&base, BoardAction::Discard("s-4".into())).await;
    assert_eq!(
        fixture.paths(),
        vec!["merge s-1", "merge s-2", "discard s-4"]
    );
    assert_eq!(ok.toasts()[0].0, ToastLevel::Ok);
    assert_eq!(
        conflict.toasts()[0],
        (
            ToastLevel::Error,
            "merge conflicts: 2 files in wt-s2".to_string()
        )
    );
    assert_eq!(discarded.toasts()[0].0, ToastLevel::Ok);
}

#[tokio::test]
async fn merge_finished_is_one_batch_call_and_reports_every_commit() {
    let fixture = Arc::new(Fixture::default());
    let base = serve(fixture.clone(), true).await;
    let run = perform_one(&base, BoardAction::MergeFinished("d-1".into())).await;
    assert_eq!(fixture.paths(), vec!["merge d-1"]);
    assert_eq!(
        run.toasts()[0],
        (
            ToastLevel::Ok,
            "merged 3 sessions into main (2 commits)".to_string()
        )
    );
}

#[tokio::test]
async fn a_batch_merge_that_stops_names_what_merged_and_why() {
    let fixture = Arc::new(Fixture::default());
    let base = serve(fixture.clone(), true).await;
    let run = perform_one(&base, BoardAction::MergeFinished("d-conf".into())).await;
    assert_eq!(fixture.paths(), vec!["merge d-conf"]);
    assert_eq!(
        run.toasts()[0],
        (
            ToastLevel::Error,
            "merged 1 of 3 · stopped at 2 \"part 2\": conflicts in 2 files".to_string()
        )
    );
}

#[tokio::test]
async fn a_refused_batch_merge_reads_as_the_daemons_reason() {
    let fixture = Arc::new(Fixture::default());
    let base = serve(fixture.clone(), true).await;
    let shared = perform_one(&base, BoardAction::MergeFinished("d-shared".into())).await;
    let dirty = perform_one(&base, BoardAction::MergeFinished("d-dirty".into())).await;
    assert_eq!(fixture.paths(), vec!["merge d-shared", "merge d-dirty"]);
    assert_eq!(
        shared.toasts()[0],
        (
            ToastLevel::Error,
            "this dispatch ran in a shared directory — there is nothing to merge".to_string()
        )
    );
    assert_eq!(dirty.toasts()[0].0, ToastLevel::Error);
    assert!(dirty.toasts()[0].1.contains("commit or stash"));
}

#[tokio::test]
async fn an_older_daemon_falls_back_to_merging_session_by_session() {
    let fixture = Arc::new(Fixture::default());
    let base = serve(fixture.clone(), true).await;
    let run = perform_one(&base, BoardAction::MergeFinished("old-1".into())).await;
    // Items arrive as 3, 1, 2, 4: merged 1 then 2 (conflict) — never 3, never the failed 4.
    assert_eq!(
        fixture.paths(),
        vec!["merge old-1", "merge s-1", "merge s-2"]
    );
    let (level, text) = run.toasts()[0].clone();
    assert_eq!(level, ToastLevel::Error);
    assert_eq!(
        text,
        "stopped at 2 part 2: merge conflicts: 2 files in wt-s2 (1 merged first)"
    );
}

#[tokio::test]
async fn the_dispatch_list_parses_and_an_old_daemons_404_is_simply_empty() {
    let http = reqwest::Client::new();
    let base = serve(Arc::new(Fixture::default()), true).await;
    let list = fetch_dispatches(&http, &base, TOKEN).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].items.len(), 4);
    assert_eq!(list[0].permission_mode.as_deref(), Some("accept-edits"));

    let old = serve(Arc::new(Fixture::default()), false).await;
    assert!(fetch_dispatches(&http, &old, TOKEN)
        .await
        .unwrap()
        .is_empty());
}
