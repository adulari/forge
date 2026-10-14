//! A session whose recorded host and workspace differ from where it is resumed: the resume appends
//! one persisted moved-session notice with a remote-based path map, a file tool aimed at the old
//! host's path is refused with the local equivalent, and a second resume does not repeat the notice.

use std::path::Path;
use std::sync::{Arc, Mutex};

use forge_config::{Config, OneOrMany, PriceOverride};
use forge_core::host_move::{HostFingerprint, HostState, RepoRef};
use forge_core::Session;
use forge_mesh::HeuristicRouter;
use forge_provider::{EventSink, ModelResponse, Provider, ProviderError, ToolSpec};
use forge_store::Store;
use forge_tui::HeadlessPresenter;
use forge_types::{new_id, Message, PermissionMode, Role, ToolCall, Usage};

struct ReadOldPath {
    path: String,
    tool_results: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Provider for ReadOldPath {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        let reply = |content: &str, tool_calls| ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: content.into(),
            tool_calls,
            usage: Usage::default(),
            quotas: Vec::new(),
        };
        if let Some(result) = messages.iter().rev().find(|m| m.role == Role::Tool) {
            self.tool_results
                .lock()
                .unwrap()
                .push(result.content.clone());
            return Ok(reply("done", vec![]));
        }
        Ok(reply(
            "reading",
            vec![ToolCall {
                id: new_id(),
                name: "read_file".into(),
                args: serde_json::json!({ "path": self.path }),
            }],
        ))
    }
}

fn fake_repo(dir: &Path, remote: &str) {
    std::fs::create_dir_all(dir.join(".git/refs/heads")).unwrap();
    std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(dir.join(".git/refs/heads/main"), "abc123\n").unwrap();
    std::fs::write(
        dir.join(".git/config"),
        format!("[remote \"origin\"]\n\turl = {remote}\n"),
    )
    .unwrap();
}

fn config() -> Config {
    let mut config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    config
        .mesh
        .models
        .insert("trivial".into(), OneOrMany::One("ollama::w".into()));
    config.mesh.pricing.insert(
        "ollama::w".into(),
        PriceOverride {
            input_per_1k: 0.0,
            output_per_1k: 0.0,
            cache_read_per_1k: None,
        },
    );
    config
}

fn resume(store: &Arc<Store>, provider: Arc<ReadOldPath>, id: &str) -> Session {
    let config = config();
    Session::resume(
        Arc::clone(store),
        provider,
        Arc::new(HeuristicRouter::new(config.clone())),
        forge_tools::ToolRegistry::with_core_tools(),
        Box::new(HeadlessPresenter::new(false)),
        config,
        id,
    )
    .unwrap()
}

#[tokio::test]
async fn a_moved_session_gets_one_notice_and_stale_paths_name_the_local_checkout() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let local = root.join("server/API/app");
    fake_repo(&local, "https://github.com/acme/app.git");
    std::fs::create_dir_all(local.join("src")).unwrap();
    std::fs::write(local.join("src/main.rs"), "fn main() {}\n").unwrap();
    let old = root.join("laptop/API/app-wt-feature");

    let store = Arc::new(Store::open_in_memory().unwrap());
    let id = store
        .create_session(local.to_str().unwrap(), "Bypass")
        .unwrap();
    store
        .add_message(&id, 0, Role::User, "fix the parser", None)
        .unwrap();
    let edited = format!("I edited {}/src/main.rs", old.display());
    store
        .add_message(&id, 1, Role::Assistant, &edited, None)
        .unwrap();
    let laptop = HostState {
        fingerprint: HostFingerprint {
            host: "laptop".into(),
            machine_id: Some("not-this-machine".into()),
            cwd: old.display().to_string(),
            repo_root: Some(old.display().to_string()),
            repos: vec![RepoRef {
                path: old.display().to_string(),
                remotes: vec!["github.com/acme/app".into()],
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    store
        .swap_session_host_state(&id, None, &serde_json::to_string(&laptop).unwrap())
        .unwrap();

    let provider = Arc::new(ReadOldPath {
        path: old.join("src/main.rs").display().to_string(),
        tool_results: Mutex::new(Vec::new()),
    });
    let mut session = resume(&store, Arc::clone(&provider), &id);
    let notices = || {
        store
            .load_messages(&id)
            .unwrap()
            .into_iter()
            .filter(|m| {
                m.role == Role::System && m.content.starts_with("[host] This session moved")
            })
            .map(|m| m.content)
            .collect::<Vec<_>>()
    };
    let found = notices();
    assert_eq!(found.len(), 1, "one persisted notice");
    let notice = &found[0];
    assert!(notice.contains("host `laptop`"), "{notice}");
    assert!(notice.contains(&format!("{} → {}", old.display(), local.display())));
    assert!(notice.contains("Tool outputs in this session are real and trustworthy"));

    session.run_turn("continue").await.unwrap();
    let results = provider.tool_results.lock().unwrap().clone();
    let guard = results.first().expect("the read was answered");
    assert!(guard.contains("does not exist on this host"), "{guard}");
    assert!(
        guard.contains(&format!("`{}`", local.join("src/main.rs").display())),
        "{guard}"
    );
    drop(session);

    let _again = resume(&store, provider, &id);
    assert_eq!(notices().len(), 1, "the notice is sent once per move");
}
