//! Route tests over a real store and a temporary Claude Code config directory.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::http::StatusCode;
use forge_store::Store;
use forge_types::Role;
use serde_json::{json, Value};

use crate::serve_git::test_support::{http_get, http_post, router};

/// `CLAUDE_CONFIG_DIR` and `CODEX_HOME` are process-wide, so these tests take turns.
static ENV: Mutex<()> = Mutex::new(());

struct Fixture {
    _guard: MutexGuard<'static, ()>,
    claude: tempfile::TempDir,
    work: tempfile::TempDir,
    store: Arc<Store>,
}

impl Fixture {
    fn new() -> Self {
        let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let claude = tempfile::tempdir().unwrap();
        let codex = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", claude.path());
        std::env::set_var("CODEX_HOME", codex.path());
        std::mem::forget(codex);
        Self {
            _guard: guard,
            claude,
            work: tempfile::tempdir().unwrap(),
            store: Arc::new(Store::open_in_memory().unwrap()),
        }
    }

    fn router(&self) -> axum::Router {
        router(self.store.clone(), self.work.path())
    }

    fn session(&self, turns: &[(&str, &str)]) -> String {
        let id = self
            .store
            .create_session(&self.work.path().display().to_string(), "Default")
            .unwrap();
        self.add_turns(&id, 0, turns);
        id
    }

    fn add_turns(&self, id: &str, from: i64, turns: &[(&str, &str)]) {
        for (i, (who, text)) in turns.iter().enumerate() {
            let role = if *who == "user" { Role::User } else { Role::Assistant };
            self.store
                .add_message(id, from + i as i64, role, text, None)
                .unwrap();
        }
    }

    fn transcript(&self, cc_id: &str) -> PathBuf {
        let dir = std::fs::read_dir(self.claude.path().join("projects"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        dir.join(format!("{cc_id}.jsonl"))
    }
}

fn lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

fn count(lines: &[String]) -> usize {
    lines.len()
}

/// Two entries Claude Code would have appended after the last one in `path`.
fn append_claude_turn(path: &Path, text: &str) {
    let existing = lines(path);
    let parse = |l: &String| serde_json::from_str::<Value>(l).unwrap();
    let last_uuid = existing
        .iter()
        .rev()
        .map(parse)
        .find_map(|v| v["uuid"].as_str().map(str::to_string))
        .unwrap();
    let template = |kind: &str| {
        existing
            .iter()
            .rev()
            .map(parse)
            .find(|v| v["type"] == kind)
            .unwrap()
    };
    let mut user = template("user");
    let mut assistant = template("assistant");
    user["uuid"] = json!(forge_types::new_id());
    user["parentUuid"] = json!(last_uuid);
    user["message"]["content"] = json!(format!("{text} (question)"));
    assistant["uuid"] = json!(forge_types::new_id());
    assistant["parentUuid"] = user["uuid"].clone();
    assistant["message"]["content"] = json!([{ "type": "text", "text": format!("{text} (answer)") }]);
    let mut body = std::fs::read_to_string(path).unwrap();
    body.push_str(&format!("{user}\n{assistant}\n"));
    std::fs::write(path, body).unwrap();
}

#[tokio::test]
async fn a_second_export_appends_only_the_new_turns() {
    let fx = Fixture::new();
    let id = fx.session(&[("user", "build a parser"), ("assistant", "done")]);
    let app = fx.router();

    let (status, first) = http_post(&app, &format!("/api/sessions/{id}/claude"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let cc_id = first["cc_session_id"].as_str().unwrap().to_string();
    assert_eq!(first["command"], format!("claude --resume {cc_id}"));
    assert_eq!(first["cwd"], fx.work.path().display().to_string());
    let path = fx.transcript(&cc_id);
    let before = lines(&path);

    let (status, again) = http_post(&app, &format!("/api/sessions/{id}/claude"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["cc_session_id"], cc_id, "same transcript");
    assert_eq!(lines(&path), before, "nothing new, nothing written");

    fx.add_turns(&id, 2, &[("user", "now tests"), ("assistant", "added")]);
    let (status, _) = http_post(&app, &format!("/api/sessions/{id}/claude"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let after = lines(&path);
    assert!(count(&after) > count(&before));
    assert_eq!(&after[..count(&before)], &before[..], "earlier turns untouched");
    assert!(after[count(&before)..].iter().any(|l| l.contains("now tests")));
    assert!(!after[count(&before)..].iter().any(|l| l.contains("build a parser")));

    let (status, link) = http_get(&app, &format!("/api/sessions/{id}/claude")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(link["linked"], true);
    assert_eq!(link["cc_session_id"], cc_id);
    assert_eq!(link["synced"], true);
}

#[tokio::test]
async fn an_unlinked_session_reports_so_and_a_missing_one_is_404() {
    let fx = Fixture::new();
    let id = fx.session(&[("user", "hi"), ("assistant", "hello")]);
    let app = fx.router();
    let (status, link) = http_get(&app, &format!("/api/sessions/{id}/claude")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(link["linked"], false);
    assert!(link["cc_session_id"].is_null());
    let (status, _) = http_get(&app, "/api/sessions/nope/claude").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = http_post(&app, "/api/sessions/nope/claude", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_empty_session_has_nothing_to_carry_over() {
    let fx = Fixture::new();
    let id = fx.session(&[]);
    let (status, body) = http_post(&fx.router(), &format!("/api/sessions/{id}/claude"), json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn a_session_open_in_a_terminal_is_409() {
    let fx = Fixture::new();
    let id = fx.session(&[("user", "hi"), ("assistant", "hello")]);
    fx.store.set_session_local_live(&id, true).unwrap();
    fx.store.touch_session_local_presence(&id, false).unwrap();
    let (status, body) = http_post(&fx.router(), &format!("/api/sessions/{id}/claude"), json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        std::fs::read_dir(fx.claude.path().join("projects")).is_err(),
        "a refused export writes nothing"
    );
}

#[tokio::test]
async fn continuing_a_linked_session_syncs_instead_of_importing_again() {
    let fx = Fixture::new();
    let id = fx.session(&[("user", "build a parser"), ("assistant", "done")]);
    let app = fx.router();
    let (_, exported) = http_post(&app, &format!("/api/sessions/{id}/claude"), json!({})).await;
    let cc_id = exported["cc_session_id"].as_str().unwrap().to_string();
    let path = fx.transcript(&cc_id);
    let sessions_before = fx.store.matching_session_ids("").unwrap().len();

    let (_, scan) = http_get(&app, "/api/import/scan").await;
    let row = scan["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == cc_id)
        .unwrap()
        .clone();
    assert_eq!(row["linked_session_id"], id);

    append_claude_turn(&path, "ran the benchmarks");
    let (status, synced) =
        http_post(&app, &format!("/api/claude/sessions/{cc_id}/continue"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert_eq!(synced["mode"], "synced");
    assert_eq!(synced["session_id"], id);
    assert!(synced["appended"].as_u64().unwrap() >= 2, "{synced}");
    assert_eq!(fx.store.matching_session_ids("").unwrap().len(), sessions_before);

    let (status, again) =
        http_post(&app, &format!("/api/claude/sessions/{cc_id}/continue"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["mode"], "synced");
    assert_eq!(again["appended"], 0, "a second sync has nothing to add");
}

#[tokio::test]
async fn continuing_an_unknown_claude_session_imports_it_and_a_missing_one_is_404() {
    let fx = Fixture::new();
    let source = fx.session(&[("user", "build a parser"), ("assistant", "done")]);
    let app = fx.router();
    let (_, exported) = http_post(&app, &format!("/api/sessions/{source}/claude"), json!({})).await;
    let cc_id = exported["cc_session_id"].as_str().unwrap().to_string();

    // A daemon that never saw the link: the same transcript, an empty store.
    let other = router(Arc::new(Store::open_in_memory().unwrap()), fx.work.path());
    let (status, body) =
        http_post(&other, &format!("/api/claude/sessions/{cc_id}/continue"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mode"], "imported");
    assert!(body["session_id"].as_str().is_some_and(|s| !s.is_empty()));

    let (status, _) = http_post(&other, "/api/claude/sessions/ghost/continue", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn continuing_into_a_session_open_in_a_terminal_is_409() {
    let fx = Fixture::new();
    let id = fx.session(&[("user", "hi"), ("assistant", "hello")]);
    let app = fx.router();
    let (_, exported) = http_post(&app, &format!("/api/sessions/{id}/claude"), json!({})).await;
    let cc_id = exported["cc_session_id"].as_str().unwrap().to_string();
    fx.store.set_session_local_live(&id, true).unwrap();
    fx.store.touch_session_local_presence(&id, true).unwrap();
    let (status, _) =
        http_post(&app, &format!("/api/claude/sessions/{cc_id}/continue"), json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT);
}
