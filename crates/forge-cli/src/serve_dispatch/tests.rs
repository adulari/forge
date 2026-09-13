use super::*;
use tower::ServiceExt;

use crate::serve::daemon_router;
use crate::serve::tests::FORGE_DB_LOCK;

fn state_with(store: Arc<Store>, cwd: &str) -> Arc<DaemonState> {
    Arc::new(DaemonState {
        registry: Arc::new(SessionRegistry::new()),
        terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
        store,
        base: "/tok".into(),
        mock: true,
        default_cwd: cwd.into(),
        project_roots: Vec::new(),
        push: None,
        apns: None,
        voice: crate::voice::VoiceState::new(),
        anywhere_enable: tokio::sync::watch::channel(false).0,
    })
}

async fn call(
    router: &Router,
    request: axum::http::Request<axum::body::Body>,
) -> (StatusCode, serde_json::Value) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn get(path: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::get(format!("/tok{path}"))
        .body(axum::body::Body::empty())
        .unwrap()
}

fn post(path: &str, body: serde_json::Value) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::post(format!("/tok{path}"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

fn post_empty(path: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::post(format!("/tok{path}"))
        .body(axum::body::Body::empty())
        .unwrap()
}

fn error_of(body: &serde_json::Value) -> &str {
    body["error"].as_str().unwrap_or_default()
}

fn three_items() -> serde_json::Value {
    serde_json::json!({
        "summary": "Three parts.",
        "items": [
            {"title": "One", "prompt": "do one"},
            {"title": "Two", "prompt": "do two"},
            {"title": "Three", "prompt": "do three", "depends_on": [1]},
        ],
    })
}

/// A store with a coordinator session and a dispatch for it in `planning`.
fn seeded() -> (Arc<Store>, String, String) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let coordinator = store.create_session("/repo", "default").unwrap();
    store
        .set_session_title(&coordinator, "Dispatch: split it")
        .unwrap();
    store
        .create_dispatch(
            "d1",
            &coordinator,
            "/repo",
            "split it",
            true,
            Some("accept-edits"),
            4,
            8,
        )
        .unwrap();
    (store, "d1".to_string(), coordinator)
}

fn pending_bodies(store: &Store, coordinator: &str) -> Vec<String> {
    store
        .pending_fleet_messages_for(coordinator)
        .unwrap()
        .into_iter()
        .map(|m| m.body)
        .collect()
}

#[tokio::test]
async fn starting_a_dispatch_rejects_bad_requests_with_the_fix_in_the_message() {
    let plain = tempfile::tempdir().unwrap();
    let cwd = plain.path().display().to_string();
    let router = daemon_router(state_with(Arc::new(Store::open_in_memory().unwrap()), &cwd));
    let cases = [
        (
            serde_json::json!({"prompt": "   "}),
            "prompt must not be empty",
        ),
        (
            serde_json::json!({"prompt": "x", "wat": 1}),
            "unknown field `wat`",
        ),
        (serde_json::json!({"cwd": cwd}), "missing field `prompt`"),
        (
            serde_json::json!({"prompt": "x".repeat(MAX_DISPATCH_PROMPT_BYTES + 1)}),
            "over the 16384-byte limit",
        ),
        (
            serde_json::json!({"prompt": "x", "max_running": 0}),
            "max_running must be between 1 and 8",
        ),
        (
            serde_json::json!({"prompt": "x", "max_items": 13}),
            "max_items must be between 1 and 12",
        ),
        (
            serde_json::json!({"prompt": "x", "mode": "plan"}),
            "valid values: default, accept-edits, bypass",
        ),
        (
            serde_json::json!({"prompt": "x", "cwd": plain.path().join("nope")}),
            "cwd is not a directory",
        ),
        (
            serde_json::json!({"prompt": "x"}),
            "is not a git repository",
        ),
    ];
    for (body, expected) in cases {
        let (status, json) = call(&router, post("/api/dispatch", body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {json}");
        assert!(error_of(&json).contains(expected), "{body}: {json}");
    }
    let (status, json) = call(
        &router,
        post("/api/dispatch", serde_json::json!({"prompt": "x"})),
    )
    .await;
    assert!(
        error_of(&json).starts_with(&format!(
            "worktree: {} is not a git repository",
            plain.path().canonicalize().unwrap().display()
        )),
        "{status}: {json}"
    );
}

#[tokio::test]
async fn dispatch_routes_are_token_scoped_and_unknown_ids_are_404() {
    let router = daemon_router(state_with(
        Arc::new(Store::open_in_memory().unwrap()),
        "/tmp",
    ));
    let unscoped = router
        .clone()
        .oneshot(
            axum::http::Request::get("/api/dispatches")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unscoped.status(), StatusCode::NOT_FOUND);
    let (status, json) = call(&router, get("/api/dispatches")).await;
    assert_eq!((status, json), (StatusCode::OK, serde_json::json!([])));
    for request in [
        get("/api/dispatches/nope"),
        post("/api/dispatches/nope/proposal", three_items()),
        post_empty("/api/dispatches/nope/approve"),
        post(
            "/api/dispatches/nope/revise",
            serde_json::json!({"feedback": "more"}),
        ),
        post_empty("/api/dispatches/nope/cancel"),
    ] {
        let (status, json) = call(&router, request).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{json}");
        assert!(error_of(&json).contains("no dispatch with that id"));
    }
}

#[tokio::test]
async fn a_proposal_is_validated_recorded_and_frozen_once_approved() {
    let (store, id, coordinator) = seeded();
    let router = daemon_router(state_with(store.clone(), "/tmp"));
    let path = format!("/api/dispatches/{id}/proposal");

    let (status, json) = call(
        &router,
        post(
            &path,
            serde_json::json!({"summary": "s", "items": [{"title": "", "prompt": "p"}]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&json), "item 1 has no title");

    let (status, json) = call(&router, post(&path, three_items())).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for key in [
        "id",
        "coordinator_session_id",
        "coordinator_title",
        "cwd",
        "prompt",
        "summary",
        "status",
        "worktree",
        "permission_mode",
        "max_running",
        "max_items",
        "created_at",
        "updated_at",
        "items",
    ] {
        assert!(keys.contains(&key), "missing {key}: {json}");
    }
    assert_eq!(json["status"], "proposed");
    assert_eq!(json["coordinator_session_id"], coordinator);
    assert_eq!(json["coordinator_title"], "Dispatch: split it");
    assert_eq!(json["worktree"], true);
    assert_eq!(json["permission_mode"], "accept-edits");
    assert_eq!(json["summary"], "Three parts.");
    let item = &json["items"][2];
    assert_eq!(item["index"], 3);
    assert_eq!(item["title"], "Three");
    assert_eq!(item["depends_on"], serde_json::json!([1]));
    assert_eq!(item["status"], "proposed");
    for nullable in ["session_id", "outcome", "started_at", "finished_at"] {
        assert!(item[nullable].is_null(), "{nullable}: {item}");
    }

    let (status, _) = call(&router, get(&format!("/api/dispatches/{id}"))).await;
    assert_eq!(status, StatusCode::OK);

    store
        .set_dispatch_status(&id, dispatch_status::RUNNING)
        .unwrap();
    let (status, json) = call(&router, post(&path, three_items())).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(error_of(&json).contains("the dispatch is running"));
}

#[tokio::test]
async fn approval_needs_a_proposed_split_and_a_valid_selection() {
    let (store, id, coordinator) = seeded();
    let router = daemon_router(state_with(store.clone(), "/tmp"));
    let path = format!("/api/dispatches/{id}/approve");

    let (status, json) = call(&router, post_empty(&path)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(error_of(&json).contains("only a proposed split can be approved"));

    call(
        &router,
        post(&format!("/api/dispatches/{id}/proposal"), three_items()),
    )
    .await;
    let (status, json) = call(&router, post(&path, serde_json::json!({"selected": [9]}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&json), "item 9 does not exist (1..=3)");
    let (status, json) = call(&router, post(&path, serde_json::json!({"selected": "all"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&json).contains("invalid approval"));

    // Item 3 depends on item 1, which is left out: nothing can run, so no session starts.
    let (status, json) = call(&router, post(&path, serde_json::json!({"selected": [3]}))).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["status"], "running");
    let statuses: Vec<&str> = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["skipped", "skipped", "cancelled"]);
    let messages = pending_bodies(&store, &coordinator);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].starts_with("[dispatch] The user approved the split."));
    assert!(messages[0].contains("Not started:\n- 1. One\n- 2. Two\n- 3. Three"));

    let (status, _) = call(&router, post_empty(&path)).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an approved split is not approved twice"
    );
}

#[tokio::test]
async fn a_revision_returns_the_split_to_planning_and_tells_the_coordinator() {
    let (store, id, coordinator) = seeded();
    let router = daemon_router(state_with(store.clone(), "/tmp"));
    let path = format!("/api/dispatches/{id}/revise");

    let (status, json) = call(&router, post(&path, serde_json::json!({"feedback": "  "}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error_of(&json).contains("feedback must not be empty"));
    let (status, _) = call(
        &router,
        post(&path, serde_json::json!({"feedback": "merge 2 and 3"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "nothing proposed yet");

    call(
        &router,
        post(&format!("/api/dispatches/{id}/proposal"), three_items()),
    )
    .await;
    let (status, json) = call(
        &router,
        post(&path, serde_json::json!({"feedback": "merge 2 and 3"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["status"], "planning");
    let messages = pending_bodies(&store, &coordinator);
    assert_eq!(messages, [plan::revise_message("merge 2 and 3")]);
}

#[tokio::test]
async fn cancelling_cancels_everything_not_running_and_refuses_twice() {
    let (store, id, coordinator) = seeded();
    let router = daemon_router(state_with(store.clone(), "/tmp"));
    call(
        &router,
        post(&format!("/api/dispatches/{id}/proposal"), three_items()),
    )
    .await;
    store
        .set_dispatch_status(&id, dispatch_status::RUNNING)
        .unwrap();
    store
        .set_dispatch_item(&id, 1, item_status::RUNNING, Some("worker-1"), None)
        .unwrap();
    store
        .set_dispatch_item(&id, 2, item_status::QUEUED, None, None)
        .unwrap();

    let (status, json) = call(&router, post_empty(&format!("/api/dispatches/{id}/cancel"))).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["status"], "cancelled");
    let statuses: Vec<&str> = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["running", "cancelled", "cancelled"]);
    assert_eq!(
        pending_bodies(&store, &coordinator),
        [plan::cancelled_message(1)]
    );

    let (status, json) = call(&router, post_empty(&format!("/api/dispatches/{id}/cancel"))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error_of(&json), "the dispatch is already cancelled");
}

#[tokio::test]
async fn the_list_is_newest_first_and_clamps_its_limit() {
    let (store, _, coordinator) = seeded();
    store
        .create_dispatch("d2", &coordinator, "/repo", "again", false, None, 1, 1)
        .unwrap();
    store
        .set_dispatch_status("d2", dispatch_status::DONE)
        .unwrap();
    let router = daemon_router(state_with(store, "/tmp"));
    let (status, json) = call(&router, get("/api/dispatches")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json.as_array().unwrap().len(), 2);
    let (_, json) = call(&router, get("/api/dispatches?limit=0")).await;
    assert_eq!(json.as_array().unwrap().len(), 1);
    assert_eq!(json[0]["permission_mode"], serde_json::Value::Null);
}

#[tokio::test]
async fn session_rows_carry_their_dispatch_role() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let ids: Vec<String> = (0..3)
        .map(|n| {
            let id = store
                .create_session(&format!("/tmp/s{n}"), "default")
                .unwrap();
            store
                .add_message(&id, 0, forge_types::Role::User, "hi", None)
                .unwrap();
            store.set_session_local_live(&id, true).unwrap();
            store.touch_session_local_presence(&id, true).unwrap();
            id
        })
        .collect();
    store
        .create_dispatch("d1", &ids[0], "/repo", "p", true, None, 4, 8)
        .unwrap();
    store
        .replace_dispatch_proposal(
            "d1",
            "s",
            &[
                NewDispatchItem {
                    title: "a".into(),
                    prompt: "a".into(),
                    depends_on: vec![],
                },
                NewDispatchItem {
                    title: "b".into(),
                    prompt: "b".into(),
                    depends_on: vec![],
                },
            ],
        )
        .unwrap();
    store
        .set_dispatch_item("d1", 2, item_status::RUNNING, Some(&ids[1]), None)
        .unwrap();

    let router = daemon_router(state_with(store, "/tmp"));
    let (status, rows) = call(&router, get("/api/sessions")).await;
    assert_eq!(status, StatusCode::OK);
    let row = |id: &str| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap()
            .clone()
    };
    let coordinator = row(&ids[0]);
    assert_eq!(coordinator["dispatch_id"], "d1");
    assert_eq!(coordinator["dispatch_role"], "coordinator");
    assert!(coordinator["dispatch_index"].is_null());
    let worker = row(&ids[1]);
    assert_eq!(worker["dispatch_role"], "worker");
    assert_eq!(worker["dispatch_index"], 2);
    let plain = row(&ids[2]);
    for key in ["dispatch_id", "dispatch_role", "dispatch_index"] {
        assert!(
            plain.get(key).is_some_and(serde_json::Value::is_null),
            "{key}: {plain}"
        );
    }
}

#[tokio::test]
async fn messages_past_the_pending_cap_are_folded_together_not_dropped() {
    let (store, _, coordinator) = seeded();
    let registry = SessionRegistry::new();
    for n in 1..=FLEET_PENDING_CAP + 2 {
        send_to_coordinator(&store, &registry, &coordinator, &format!("update {n}")).await;
    }
    let bodies = pending_bodies(&store, &coordinator);
    assert_eq!(bodies.len(), FLEET_PENDING_CAP);
    assert_eq!(bodies[0], "update 1");
    assert_eq!(bodies.last().unwrap(), "update 8\n\nupdate 9\n\nupdate 10");
}

#[test]
fn combining_over_the_size_limit_keeps_the_newest_update_whole() {
    let older = "o".repeat(FLEET_MESSAGE_MAX_BYTES);
    let newer = "the latest state";
    let combined = combine_messages(&older, newer);
    assert!(combined.len() <= FLEET_MESSAGE_MAX_BYTES);
    assert!(combined.starts_with("[dispatch] (earlier updates were shortened to fit)"));
    assert!(combined.ends_with("\n\nthe latest state"));
    assert_eq!(combine_messages("a", "b"), "a\n\nb");
}

#[test]
fn the_coordinator_title_is_the_first_line_of_the_request_and_short() {
    assert_eq!(
        coordinator_session_title("Add dark mode\nwith details"),
        "Dispatch: Add dark mode"
    );
    let long = coordinator_session_title(&"word ".repeat(40));
    assert_eq!(long.chars().count(), MAX_COORDINATOR_TITLE_CHARS);
    assert!(long.ends_with('…'));
}

/// The whole flow over a real mock daemon: a coordinator proposes, the user approves, two workers
/// run while the third waits on the first, every item finishes, the coordinator hears about each
/// step, and merging a worker marks its item merged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mock_dispatch_runs_from_proposal_to_merge() {
    let _env = FORGE_DB_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("FORGE_DB", dir.path().join("dispatch-e2e.db"));
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("README.md"), "hi\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "init"]);

    let store = Arc::new(crate::open_store().unwrap());
    let state = state_with(store.clone(), &repo.display().to_string());
    let supervisor = tokio::spawn(run_supervisor(state.clone()));
    let router = daemon_router(state.clone());

    let (status, started) = call(
        &router,
        post(
            "/api/dispatch",
            serde_json::json!({"prompt": "mock:dispatch split the work", "cwd": repo}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let id = started["dispatch_id"].as_str().unwrap().to_string();
    let coordinator = started["coordinator_session_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(started["title"], "Dispatch: mock:dispatch split the work");

    async fn wait_for(
        router: &Router,
        id: &str,
        what: &str,
        done: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
        loop {
            let (_, json) = call(router, get(&format!("/api/dispatches/{id}"))).await;
            if done(&json) {
                return json;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}: {json}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    let proposed = wait_for(&router, &id, "the proposal", |d| {
        d["status"] == "proposed" && d["items"].as_array().is_some_and(|i| i.len() == 3)
    })
    .await;
    assert_eq!(
        proposed["summary"],
        "Mock split of the request into three parts."
    );
    assert_eq!(proposed["items"][2]["depends_on"], serde_json::json!([1]));

    let (status, approved) = call(
        &router,
        post_empty(&format!("/api/dispatches/{id}/approve")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(approved["status"], "running");
    for n in [0, 1] {
        assert_eq!(approved["items"][n]["status"], "running", "{approved}");
        assert!(approved["items"][n]["session_id"].is_string());
    }
    assert_eq!(approved["items"][2]["status"], "queued");
    assert!(approved["items"][2]["session_id"].is_null());

    let finished = wait_for(&router, &id, "every item to finish", |d| {
        d["status"] == "done"
    })
    .await;
    let item_status_of = |n: usize| finished["items"][n]["status"].as_str().unwrap().to_string();
    assert_eq!(item_status_of(0), "succeeded", "{finished}");
    for n in 0..3 {
        assert!(
            item_status::is_terminal(&item_status_of(n)),
            "item {n}: {finished}"
        );
    }
    assert!(
        finished["items"][2]["session_id"].is_string(),
        "item 3 started once item 1 succeeded: {finished}"
    );

    // Every coordinator message went through the fleet queue and reached the coordinator's input.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let transcript = store
            .load_messages(&coordinator)
            .unwrap()
            .into_iter()
            .map(|m| m.content)
            .collect::<Vec<_>>()
            .join("\n");
        let expected = [
            "[message from dispatch] [dispatch] The user approved the split.",
            "[dispatch] Session 1/3 \"Notes file\"",
            "[dispatch] Session 2/3 \"Tasks\"",
            "[dispatch] Session 3/3 \"Summary\"",
            "[dispatch] All sessions have finished:",
        ];
        if expected.iter().all(|e| transcript.contains(e)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "coordinator never received {:?}; pending: {:?}; transcript:\n{transcript}",
            expected
                .iter()
                .filter(|e| !transcript.contains(**e))
                .collect::<Vec<_>>(),
            pending_bodies(&store, &coordinator)
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let worker = finished["items"][0]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, merged) = call(
        &router,
        post_empty(&format!("/api/sessions/{worker}/merge")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{merged}");
    let (_, after) = call(&router, get(&format!("/api/dispatches/{id}"))).await;
    assert_eq!(after["items"][0]["status"], "merged", "{after}");

    supervisor.abort();
    for handle in state.registry.all().await {
        handle.shutdown();
    }
    std::env::remove_var("FORGE_DB");
}
