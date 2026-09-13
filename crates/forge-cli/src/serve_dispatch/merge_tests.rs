//! The batch merge over a real temporary git repo, real worktrees and mock drivers.

use std::path::Path;
use std::sync::Arc;

use axum::http::StatusCode;
use forge_core::dispatch::item_status;
use forge_store::{NewDispatchItem, Store};
use tower::ServiceExt;

use crate::cli::commands::run::{spawn_session_driver, DriverSpec};
use crate::serve::tests::FORGE_DB_LOCK;
use crate::serve::{daemon_router, DaemonState, SessionRegistry};

const DISPATCH: &str = "dispatch-0001abcd";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repo with a configured identity and one commit of `files`.
fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    for (name, content) in files {
        std::fs::write(dir.join(name), content).unwrap();
    }
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "init"]);
    tmp
}

fn state(cwd: &Path) -> Arc<DaemonState> {
    Arc::new(DaemonState {
        registry: Arc::new(SessionRegistry::new()),
        terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
        store: Arc::new(Store::open_in_memory().unwrap()),
        base: "/tok".into(),
        mock: true,
        default_cwd: cwd.display().to_string(),
        project_roots: Vec::new(),
        push: None,
        apns: None,
        voice: crate::voice::VoiceState::new(),
        anywhere_enable: tokio::sync::watch::channel(false).0,
    })
}

/// A live mock worker in a fresh worktree of `repo` whose edits are `files`.
async fn worker(state: &DaemonState, repo: &Path, files: &[(&str, &str)]) -> (String, String) {
    let wt_id = forge_types::new_id().chars().take(12).collect::<String>();
    let guard = forge_core::worktree::WorktreeGuard::create(repo, &wt_id).unwrap();
    let wt = guard.path().display().to_string();
    std::mem::forget(guard);
    for (name, content) in files {
        std::fs::write(Path::new(&wt).join(name), content).unwrap();
    }
    let handle = spawn_session_driver(DriverSpec {
        cwd: wt.clone(),
        worktree: Some(wt.clone()),
        title: "worker".into(),
        mock: true,
        model: None,
        resume: None,
        temper: None,
        push: None,
        apns: None,
        registry: None,
        dispatch_coordinator: false,
        dispatch_worker: false,
    })
    .await
    .unwrap();
    let handle = state.registry.insert(handle, &state.store).await;
    (handle.session_id.clone(), wt)
}

/// A running dispatch whose items are `succeeded` with the given sessions.
fn seed(store: &Store, cwd: &Path, worktree: bool, items: &[(&str, Option<&str>)]) {
    let coordinator = store
        .create_session(&cwd.display().to_string(), "default")
        .unwrap();
    store
        .create_dispatch(
            DISPATCH,
            &coordinator,
            &cwd.display().to_string(),
            "Split the parser work\nwith more detail below",
            worktree,
            None,
            4,
            8,
        )
        .unwrap();
    let new_items: Vec<NewDispatchItem> = items
        .iter()
        .map(|(title, _)| NewDispatchItem {
            title: (*title).into(),
            prompt: "do it".into(),
            depends_on: vec![],
        })
        .collect();
    store
        .replace_dispatch_proposal(DISPATCH, "s", &new_items)
        .unwrap();
    for (i, (_, session)) in items.iter().enumerate() {
        let status = if session.is_some() {
            item_status::SUCCEEDED
        } else {
            item_status::RUNNING
        };
        store
            .set_dispatch_item(DISPATCH, i as i64 + 1, status, *session, None)
            .unwrap();
    }
}

async fn merge(state: &Arc<DaemonState>, id: &str) -> (StatusCode, serde_json::Value) {
    let request = axum::http::Request::post(format!("/tok/api/dispatches/{id}/merge"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = daemon_router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn item_status_of(state: &DaemonState, idx: i64) -> String {
    let row = state.store.dispatch(DISPATCH).unwrap().unwrap();
    row.items.into_iter().find(|i| i.idx == idx).unwrap().status
}

fn use_temp_db(dir: &Path) {
    std::env::set_var("FORGE_DB", dir.join("merge.db"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn merging_a_dispatch_commits_each_item_in_order_and_removes_the_worktrees() {
    let _env = FORGE_DB_LOCK.lock().await;
    let db = tempfile::tempdir().unwrap();
    use_temp_db(db.path());
    let repo = repo(&[("README.md", "hi\n")]);
    let state = state(repo.path());
    let (s1, wt1) = worker(&state, repo.path(), &[("a.txt", "from one\n")]).await;
    let (s2, wt2) = worker(&state, repo.path(), &[("b.txt", "from two\n")]).await;
    seed(
        &state.store,
        repo.path(),
        true,
        &[("Notes file", Some(&s1)), ("Tasks", Some(&s2))],
    );

    let (status, body) = merge(&state, DISPATCH).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["stopped_at"].is_null(), "{body}");
    assert_eq!(body["remaining"], serde_json::json!([]));
    let merged = body["merged"].as_array().unwrap();
    assert_eq!(merged.len(), 2, "{body}");
    assert_eq!(merged[0]["index"], 1);
    assert_eq!(merged[1]["title"], "Tasks");
    assert_eq!(body["dispatch"]["id"], DISPATCH);

    let subjects = git(repo.path(), &["log", "--format=%s"]);
    assert_eq!(
        subjects.lines().collect::<Vec<_>>(),
        vec![
            "Merge dispatch item 2: Tasks",
            "Merge dispatch item 1: Notes file",
            "init"
        ]
    );
    assert_eq!(
        merged[1]["commit"].as_str().unwrap(),
        git(repo.path(), &["rev-parse", "HEAD"])
    );
    let message = git(repo.path(), &["log", "-1", "--format=%b", "HEAD~1"]);
    assert_eq!(
        message,
        format!(
            "From Forge dispatch dispatch — Split the parser work. Session {}, branch forge/subagent/{}.",
            &s1[..8],
            Path::new(&wt1).file_name().unwrap().to_str().unwrap()
        )
    );
    assert_eq!(
        git(
            repo.path(),
            &["status", "--porcelain", "--untracked-files=no"]
        ),
        ""
    );
    assert!(repo.path().join("a.txt").exists() && repo.path().join("b.txt").exists());
    assert!(!Path::new(&wt1).exists() && !Path::new(&wt2).exists());
    assert_eq!(item_status_of(&state, 1), item_status::MERGED);
    assert_eq!(item_status_of(&state, 2), item_status::MERGED);
    std::env::remove_var("FORGE_DB");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_conflict_stops_the_run_after_committing_the_items_before_it() {
    let _env = FORGE_DB_LOCK.lock().await;
    let db = tempfile::tempdir().unwrap();
    use_temp_db(db.path());
    let repo = repo(&[("f.txt", "l1\nl2\nl3\n")]);
    let state = state(repo.path());
    let (s1, _) = worker(&state, repo.path(), &[("f.txt", "l1\nONE\nl3\n")]).await;
    let (s2, wt2) = worker(&state, repo.path(), &[("f.txt", "l1\nTWO\nl3\n")]).await;
    seed(
        &state.store,
        repo.path(),
        true,
        &[("First", Some(&s1)), ("Second", Some(&s2))],
    );

    let (status, body) = merge(&state, DISPATCH).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["merged"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["merged"][0]["index"], 1);
    assert_eq!(body["stopped_at"]["index"], 2);
    assert_eq!(body["stopped_at"]["title"], "Second");
    assert!(
        body["stopped_at"]["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "f.txt"),
        "{body}"
    );
    assert_eq!(body["remaining"], serde_json::json!([]));

    assert_eq!(
        git(repo.path(), &["log", "-1", "--format=%s"]),
        "Merge dispatch item 1: First"
    );
    assert_eq!(
        git(
            repo.path(),
            &["status", "--porcelain", "--untracked-files=no"]
        ),
        ""
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join("f.txt")).unwrap(),
        "l1\nONE\nl3\n"
    );
    assert_eq!(item_status_of(&state, 1), item_status::MERGED);
    assert_eq!(item_status_of(&state, 2), item_status::SUCCEEDED);
    assert!(Path::new(&wt2).exists(), "the conflicted worktree is kept");
    let respawned = state.registry.get(&s2).await;
    assert!(respawned.is_some(), "the conflicted session keeps running");
    respawned.unwrap().shutdown();
    std::env::remove_var("FORGE_DB");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dirty_base_refuses_the_whole_run_and_touches_nothing() {
    let _env = FORGE_DB_LOCK.lock().await;
    let db = tempfile::tempdir().unwrap();
    use_temp_db(db.path());
    let repo = repo(&[("README.md", "hi\n")]);
    let state = state(repo.path());
    let (s1, wt1) = worker(&state, repo.path(), &[("a.txt", "one\n")]).await;
    seed(&state.store, repo.path(), true, &[("Only", Some(&s1))]);
    std::fs::write(repo.path().join("README.md"), "uncommitted\n").unwrap();

    let (status, body) = merge(&state, DISPATCH).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("commit or stash them"));
    assert_eq!(body["dirty_files"], serde_json::json!(["README.md"]));
    assert_eq!(git(repo.path(), &["rev-list", "--count", "HEAD"]), "1");
    assert!(Path::new(&wt1).exists());
    assert_eq!(item_status_of(&state, 1), item_status::SUCCEEDED);
    let live = state.registry.get(&s1).await;
    assert!(live.is_some(), "the session was never stopped");
    live.unwrap().shutdown();
    std::env::remove_var("FORGE_DB");
}

/// A commit the base repo refuses (here a pre-commit hook that rejects commits in the main
/// checkout but lets the worktree snapshot through) stops the run with that item staged.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_commit_stops_with_the_merge_staged_and_gits_reason() {
    use std::os::unix::fs::PermissionsExt;
    let _env = FORGE_DB_LOCK.lock().await;
    let db = tempfile::tempdir().unwrap();
    use_temp_db(db.path());
    let repo = repo(&[("README.md", "hi\n")]);
    let hook = repo.path().join(".git/hooks/pre-commit");
    std::fs::write(
        &hook,
        "#!/bin/sh\ncase \"$(git rev-parse --git-dir)\" in */worktrees/*) exit 0;; esac\n\
         echo 'refused by the test hook' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let state = state(repo.path());
    let (s1, _) = worker(&state, repo.path(), &[("a.txt", "one\n")]).await;
    let (s2, wt2) = worker(&state, repo.path(), &[("b.txt", "two\n")]).await;
    seed(
        &state.store,
        repo.path(),
        true,
        &[("First", Some(&s1)), ("Second", Some(&s2))],
    );

    let (status, body) = merge(&state, DISPATCH).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["merged"], serde_json::json!([]), "{body}");
    assert_eq!(body["stopped_at"]["index"], 1);
    let reason = body["stopped_at"]["reason"].as_str().unwrap();
    assert!(reason.starts_with("merged but not committed"), "{reason}");
    assert!(reason.contains("refused by the test hook"), "{reason}");
    assert_eq!(body["remaining"], serde_json::json!([2]));
    assert_eq!(
        git(repo.path(), &["diff", "--cached", "--name-only"]),
        "a.txt"
    );
    assert_eq!(git(repo.path(), &["rev-list", "--count", "HEAD"]), "1");
    assert_eq!(item_status_of(&state, 1), item_status::MERGED);
    assert_eq!(item_status_of(&state, 2), item_status::SUCCEEDED);
    assert!(Path::new(&wt2).exists());
    let live = state.registry.get(&s2).await;
    assert!(live.is_some(), "the item after the stop was never touched");
    live.unwrap().shutdown();
    std::env::remove_var("FORGE_DB");
}

#[tokio::test]
async fn merge_refuses_an_unknown_a_shared_and_an_unfinished_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let (status, _) = merge(&state, "nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    seed(&state.store, dir.path(), false, &[("Only", Some("s-1"))]);
    let (status, body) = merge(&state, DISPATCH).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"],
        "this dispatch ran in a shared directory — there is nothing to merge"
    );

    let state = self::state(dir.path());
    seed(&state.store, dir.path(), true, &[("Only", None)]);
    let (status, body) = merge(&state, DISPATCH).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("no item has succeeded"));
}
