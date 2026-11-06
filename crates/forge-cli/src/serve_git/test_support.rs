//! Shared fixtures for the IDE-mode route tests: a real git repository, a daemon router over an
//! in-memory store (sessions come from the store, so no driver is spawned), and JSON helpers.

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use tower::ServiceExt;

use crate::serve::{daemon_router, DaemonState, SessionRegistry};

pub(crate) const TOKEN: &str = "/tok";

pub(crate) fn git_in(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

pub(crate) fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository with `files` committed.
pub(crate) fn repo_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_in(dir.path(), &["init", "-q", "-b", "main"]);
    for (name, body) in files {
        if let Some(parent) = Path::new(name).parent() {
            std::fs::create_dir_all(dir.path().join(parent)).unwrap();
        }
        std::fs::write(dir.path().join(name), body).unwrap();
        git_in(dir.path(), &["add", name]);
    }
    git_in(dir.path(), &["commit", "-qm", "init"]);
    dir
}

pub(crate) fn state(store: Arc<forge_store::Store>, default_cwd: &Path) -> Arc<DaemonState> {
    Arc::new(DaemonState {
        registry: Arc::new(SessionRegistry::new()),
        terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
        store,
        base: TOKEN.into(),
        mock: true,
        default_cwd: default_cwd.display().to_string(),
        project_roots: Vec::new(),
        push: None,
        apns: None,
        voice: crate::voice::VoiceState::new(),
        anywhere_enable: tokio::sync::watch::channel(false).0,
    })
}

pub(crate) fn router(store: Arc<forge_store::Store>, default_cwd: &Path) -> Router {
    daemon_router(state(store, default_cwd))
}

pub(crate) async fn http_get(
    router: &Router,
    path: &str,
) -> (axum::http::StatusCode, serde_json::Value) {
    let request = axum::http::Request::get(format!("{TOKEN}{path}"))
        .body(axum::body::Body::empty())
        .unwrap();
    send(router, request).await
}

pub(crate) async fn http_post(
    router: &Router,
    path: &str,
    body: serde_json::Value,
) -> (axum::http::StatusCode, serde_json::Value) {
    let request = axum::http::Request::post(format!("{TOKEN}{path}"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    send(router, request).await
}

async fn send(
    router: &Router,
    request: axum::http::Request<axum::body::Body>,
) -> (axum::http::StatusCode, serde_json::Value) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 22)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}
