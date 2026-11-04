use axum::http::StatusCode;
use forge_types::Role;

use super::super::test_support::*;
use super::*;

fn live_session(store: &forge_store::Store, cwd: &Path, worktree: Option<&Path>) -> String {
    let id = store
        .create_session(&cwd.display().to_string(), "default")
        .unwrap();
    if let Some(worktree) = worktree {
        store
            .set_session_worktree(&id, &worktree.display().to_string())
            .unwrap();
    }
    store.set_session_local_live(&id, true).unwrap();
    store.touch_session_local_presence(&id, false).unwrap();
    id
}

fn edited(store: &forge_store::Store, session: &str, tool: &str, path: &str) {
    let mid = store
        .add_message(session, 0, Role::Assistant, "edit", Some("m"))
        .unwrap();
    let args = serde_json::json!({ "path": path, "content": "x", "new": "x" }).to_string();
    store
        .record_tool_call(&mid, tool, &args, "ok", "allowed", "ok")
        .unwrap();
}

fn session<'a>(body: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session_id"] == id)
        .unwrap_or_else(|| panic!("no session {id} in {body}"))
}

fn paths(row: &serde_json::Value) -> Vec<String> {
    row["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn shared_tree_changes_are_attributed_to_the_sessions_that_edited_them() {
    let dir = repo_with(&[("a.txt", "a\n"), ("b.txt", "b\n"), ("c.txt", "c\n")]);
    let canonical = dir.path().canonicalize().unwrap();
    let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
    let one = live_session(&store, dir.path(), None);
    let two = live_session(&store, dir.path(), None);

    std::fs::write(dir.path().join("a.txt"), "a\nedited by both\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "b\nedited by two\n").unwrap();
    std::fs::write(dir.path().join("c.txt"), "c\nedited by hand\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "n1\nn2\n").unwrap();
    // `one` used an absolute path, `two` a relative one: both must land on the same file.
    edited(
        &store,
        &one,
        "edit_file",
        &canonical.join("a.txt").to_string_lossy(),
    );
    edited(&store, &two, "edit_file", "a.txt");
    edited(&store, &two, "edit_file", "b.txt");
    edited(&store, &two, "write_file", "new.txt");

    let app = router(store, dir.path());
    let url = format!("/api/projects/changes?cwd={}", dir.path().display());
    let (status, body) = http_get(&app, &url).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(paths(session(&body, &one)), ["a.txt"]);
    assert_eq!(
        paths(session(&body, &two)),
        ["a.txt", "b.txt", "new.txt"],
        "a file several sessions edited is listed under each"
    );
    let hand = session(&body, "unattributed");
    assert_eq!(paths(hand), ["c.txt"]);
    assert_eq!(hand["busy"], false);
    assert_eq!(hand["worktree"], serde_json::Value::Null);

    let two_files = session(&body, &two)["files"].as_array().unwrap().clone();
    let new = two_files.iter().find(|f| f["path"] == "new.txt").unwrap();
    assert_eq!(
        (new["kind"].as_str(), new["adds"].as_u64()),
        (Some("created"), Some(2))
    );
    let b = two_files.iter().find(|f| f["path"] == "b.txt").unwrap();
    assert_eq!(
        (b["kind"].as_str(), b["adds"].as_u64(), b["dels"].as_u64()),
        (Some("modified"), Some(1), Some(0))
    );
    assert_eq!(session(&body, &one)["branch"], "main");

    // The hash is the same for the same bytes in either session's list, and moves with them.
    let hash_of = |body: &serde_json::Value, sid: &str, path: &str| {
        session(body, sid)["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == path)
            .unwrap()["content_hash"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let before = hash_of(&body, &one, "a.txt");
    assert_eq!(before, hash_of(&body, &two, "a.txt"));
    let (_, again) = http_get(&app, &url).await;
    assert_eq!(
        before,
        hash_of(&again, &one, "a.txt"),
        "stable while unchanged"
    );
    std::fs::write(dir.path().join("a.txt"), "a\nedited again\n").unwrap();
    let (_, after) = http_get(&app, &url).await;
    assert_ne!(before, hash_of(&after, &one, "a.txt"));
}

#[tokio::test]
async fn edits_that_predate_the_last_commit_do_not_claim_a_file() {
    let dir = repo_with(&[("a.txt", "a\n")]);
    let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
    let sid = live_session(&store, dir.path(), None);
    edited(&store, &sid, "edit_file", "a.txt");
    // Back-date the commit's successor so the recorded edit is older than HEAD.
    std::fs::write(dir.path().join("a.txt"), "a\nhuman, later\n").unwrap();
    git_in(dir.path(), &["add", "a.txt"]);
    let later = (chrono::Utc::now().timestamp() + 3_600).to_string();
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "later",
        ])
        .env("GIT_COMMITTER_DATE", &later)
        .output()
        .unwrap();
    assert!(out.status.success());
    std::fs::write(dir.path().join("a.txt"), "a\nhuman, later\nuncommitted\n").unwrap();

    let app = router(store, dir.path());
    let (_, body) = http_get(
        &app,
        &format!("/api/projects/changes?cwd={}", dir.path().display()),
    )
    .await;
    assert!(paths(session(&body, &sid)).is_empty());
    assert_eq!(paths(session(&body, "unattributed")), ["a.txt"]);
}

#[tokio::test]
async fn a_worktree_session_reports_its_diff_against_the_fork_point() {
    let dir = repo_with(&[("a.txt", "a\n"), ("keep.txt", "k\n")]);
    let worktree = dir.path().join(".forge/worktrees/child");
    git_in(
        dir.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "forge/subagent/child",
            worktree.to_str().unwrap(),
        ],
    );
    std::fs::write(worktree.join("a.txt"), "a\nmore\nlines\n").unwrap();
    std::fs::write(worktree.join("fresh.txt"), "f\n").unwrap();

    let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
    let wt_session = live_session(&store, dir.path(), Some(&worktree));
    let shared = live_session(&store, dir.path(), None);
    std::fs::write(dir.path().join("keep.txt"), "k\nshared edit\n").unwrap();
    edited(&store, &shared, "edit_file", "keep.txt");

    let app = router(store, dir.path());
    let url = format!("/api/projects/changes?cwd={}", dir.path().display());
    let (status, body) = http_get(&app, &url).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let row = session(&body, &wt_session);
    assert_eq!(row["branch"], "forge/subagent/child");
    assert_eq!(row["worktree"], worktree.display().to_string());
    assert_eq!(paths(row), ["a.txt", "fresh.txt"]);
    let a = &row["files"][0];
    assert_eq!(
        (a["adds"].as_u64(), a["kind"].as_str()),
        (Some(2), Some("modified"))
    );
    assert_eq!(
        paths(session(&body, &shared)),
        ["keep.txt"],
        "the worktree's edits never leak into the shared tree's list"
    );
    assert!(body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["session_id"] != "unattributed"));

    let before = row["files"][0]["content_hash"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(worktree.join("a.txt"), "a\nchanged\n").unwrap();
    let (_, after) = http_get(&app, &url).await;
    assert_ne!(
        before,
        session(&after, &wt_session)["files"][0]["content_hash"]
            .as_str()
            .unwrap()
    );
}

#[tokio::test]
async fn other_projects_and_bad_input_report_nothing() {
    let dir = repo_with(&[("a.txt", "a\n")]);
    let elsewhere = repo_with(&[("z.txt", "z\n")]);
    let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
    live_session(&store, dir.path(), None);
    std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
    let app = router(store, dir.path());

    let (status, body) = http_get(
        &app,
        &format!("/api/projects/changes?cwd={}", elsewhere.path().display()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["sessions"].as_array().unwrap().is_empty());

    let (status, _) = http_get(&app, "/api/projects/changes?cwd=").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = http_get(&app, "/api/projects/changes?cwd=/no/such/dir").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A sub-directory of the project still finds the project's sessions.
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    let (_, body) = http_get(
        &app,
        &format!("/api/projects/changes?cwd={}/sub", dir.path().display()),
    )
    .await;
    assert_eq!(
        body["sessions"].as_array().unwrap().len(),
        2,
        "session + unattributed"
    );
}

#[test]
fn the_content_hash_names_bytes_not_mtimes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f");
    assert_eq!(content_hash(&file), "deleted");
    std::fs::write(&file, "one").unwrap();
    let one = content_hash(&file);
    std::fs::write(&file, "two").unwrap();
    assert_ne!(one, content_hash(&file));
    std::fs::write(&file, "one").unwrap();
    assert_eq!(one, content_hash(&file));
}
