use axum::http::StatusCode;
use forge_types::Role;

use super::*;
use crate::serve_git::test_support::*;

/// A session over `dir` whose assistant turn at seq 1 (after the user prompt at seq 0) wrote
/// `content` to `path` through `write_file`.
fn session_that_wrote(
    store: &forge_store::Store,
    dir: &Path,
    path: &str,
    content: &str,
) -> (String, String) {
    let sid = store
        .create_session(&dir.display().to_string(), "default")
        .unwrap();
    store
        .add_message(
            &sid,
            0,
            Role::User,
            "add the greeting\nand make it loud",
            None,
        )
        .unwrap();
    let mid = store
        .add_message(
            &sid,
            1,
            Role::Assistant,
            "done",
            Some("anthropic::claude-test"),
        )
        .unwrap();
    let args = serde_json::json!({ "path": path, "content": content }).to_string();
    store
        .record_tool_call(&mid, "write_file", &args, "ok", "allowed", "ok")
        .unwrap();
    let call = store
        .file_edits("lib.rs")
        .unwrap()
        .pop()
        .unwrap()
        .tool_call_id;
    (sid, call)
}

#[tokio::test]
async fn blame_maps_each_written_line_to_its_turn_and_leaves_human_lines_out() {
    let dir = repo_with(&[("src/lib.rs", "// human header\n")]);
    let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
    let (sid, call) = session_that_wrote(
        &store,
        dir.path(),
        "src/lib.rs",
        "pub fn hello() {}\nlet loud = 1;\n",
    );
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "// human header\npub fn hello() {}\nlet loud = 1;\n// human footer\n",
    )
    .unwrap();
    let app = router(store, dir.path());

    let (status, body) = http_get(&app, &format!("/api/blame?session={sid}&path=src/lib.rs")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["line_count"], 4);
    let lines = body["lines"].as_array().unwrap();
    assert_eq!(
        lines.len(),
        2,
        "only the lines a recorded edit wrote: {body}"
    );
    assert_eq!(lines[0]["line"], 2);
    assert_eq!(lines[1]["line"], 3);
    for line in lines {
        assert_eq!(line["session_id"], sid.as_str());
        assert_eq!(line["turn"], 1);
        assert_eq!(line["model"], "anthropic::claude-test");
        assert_eq!(line["tool_call_id"], call.as_str());
        assert_eq!(line["prompt_summary"], "add the greeting and make it loud");
    }
}

#[tokio::test]
async fn blame_refuses_paths_outside_the_repository_and_unknown_sessions() {
    let dir = repo_with(&[("a.txt", "a\n")]);
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "s\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.path().join("secret.txt"),
        dir.path().join("link.txt"),
    )
    .unwrap();
    let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
    let sid = store
        .create_session(&dir.path().display().to_string(), "default")
        .unwrap();
    let app = router(store, dir.path());

    let (status, _) = http_get(&app, &format!("/api/blame?session={sid}&path=../x")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = http_get(&app, &format!("/api/blame?session={sid}&path=/etc/passwd")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    #[cfg(unix)]
    {
        let (status, _) = http_get(&app, &format!("/api/blame?session={sid}&path=link.txt")).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a symlink out of the tree is not followed"
        );
    }
    let (status, _) = http_get(&app, &format!("/api/blame?session={sid}&path=nope.txt")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = http_get(&app, "/api/blame?session=ghost&path=a.txt").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = http_get(&app, &format!("/api/blame?session={sid}&path=a.txt")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["lines"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn jobs_list_and_tail_read_the_session_workspace() {
    let _env = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let state = tempfile::tempdir().unwrap();
    std::env::set_var(forge_config::STATE_DIR_ENV, state.path());

    let dir = repo_with(&[("a.txt", "a\n")]);
    let jobs_dir = forge_config::state_subdir(dir.path(), "jobs");
    std::fs::create_dir_all(&jobs_dir).unwrap();
    let log = jobs_dir.join("4242.log");
    let output: String = (1..=50).map(|n| format!("out {n}\n")).collect();
    std::fs::write(&log, output).unwrap();
    let record = serde_json::json!({
        "pid": 4242, "command": "npm run dev", "cwd": dir.path().display().to_string(),
        "log": log.display().to_string(), "started_at": 1_700_000_000u64,
        "start_ticks": null, "exit": 0,
    });
    std::fs::write(jobs_dir.join("4242.json"), record.to_string()).unwrap();

    let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
    let sid = store
        .create_session(&dir.path().display().to_string(), "default")
        .unwrap();
    let app = router(store, dir.path());

    let (status, body) = http_get(&app, &format!("/api/sessions/{sid}/jobs")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let jobs = body["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["id"], 4242);
    assert_eq!(jobs[0]["command"], "npm run dev");
    assert_eq!(jobs[0]["running"], false);
    assert_eq!(jobs[0]["status"], "exited 0");
    assert_eq!(jobs[0]["started_at"], 1_700_000_000u64);

    let (status, body) =
        http_get(&app, &format!("/api/sessions/{sid}/jobs/4242/tail?lines=3")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["text"], "out 48\nout 49\nout 50");
    assert_eq!(body["status"], "exited 0");

    let (status, _) = http_get(&app, &format!("/api/sessions/{sid}/jobs/999/tail")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = http_get(&app, "/api/sessions/ghost/jobs").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    std::env::remove_var(forge_config::STATE_DIR_ENV);
}
