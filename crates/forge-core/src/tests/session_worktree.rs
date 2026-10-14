//! A session started in a repository's main checkout moves itself into a worktree with the
//! `worktree` tool, works there with plain relative paths and a cwd-less shell, then leaves and
//! removes it — and nothing lands in the main checkout on the way.

use super::*;

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn repo(dir: &std::path::Path) -> std::path::PathBuf {
    let root = dir.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["config", "user.name", "t"]);
    std::fs::write(root.join("a.txt"), "main\n").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-qm", "init"]);
    root.canonicalize().unwrap()
}

struct Script {
    calls: std::sync::atomic::AtomicUsize,
    steps: Vec<Vec<(&'static str, serde_json::Value)>>,
}

#[async_trait::async_trait]
impl Provider for Script {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        use forge_types::{new_id, ToolCall, Usage};
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tool_calls: Vec<ToolCall> = self
            .steps
            .get(n)
            .map(|step| {
                step.iter()
                    .map(|(name, args)| ToolCall {
                        id: new_id(),
                        name: (*name).into(),
                        args: args.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if tool_calls.is_empty() {
                "done".into()
            } else {
                String::new()
            },
            tool_calls,
            usage: Usage::default(),
            quotas: Vec::new(),
        })
    }
}

fn session_in(root: &std::path::Path, script: Script, store: Arc<Store>) -> Session {
    let config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    Session::start(
        store,
        Arc::new(script),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(root),
        Box::new(CapturePresenter::default()),
        config,
        root.to_str().unwrap(),
    )
    .unwrap()
}

fn tool_results(session: &Session) -> Vec<String> {
    session
        .transcript
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| m.content.clone())
        .collect()
}

#[tokio::test]
async fn a_main_checkout_session_works_in_its_own_worktree_and_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let wt = main.join(".forge/worktrees/feat");
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut session = session_in(
        &main,
        Script {
            calls: Default::default(),
            steps: vec![
                vec![(
                    "worktree",
                    serde_json::json!({ "action": "create", "name": "feat", "branch": "feat/x" }),
                )],
                vec![(
                    "write_file",
                    serde_json::json!({ "path": "new.txt", "content": "in the worktree\n" }),
                )],
                vec![(
                    "shell",
                    serde_json::json!({ "command": "git add new.txt && git commit -qm 'add new'" }),
                )],
            ],
        },
        Arc::clone(&store),
    );
    let checkpoints = session.checkpoint_root().to_path_buf();
    session.run_turn("build it in a worktree").await.unwrap();

    let results = tool_results(&session);
    assert!(results[0].contains("created worktree"), "{results:?}");
    assert_eq!(session.workspace_root(), wt.canonicalize().unwrap());
    assert_eq!(session.cached_git_branch.as_deref(), Some("feat/x"));
    assert_eq!(
        store.session_cwd(session.id()).unwrap().as_deref(),
        Some(session.workspace_root().to_str().unwrap()),
        "resume must reopen the session in the worktree"
    );
    assert_eq!(session.checkpoint_root(), checkpoints);
    assert!(wt.join("new.txt").exists());
    assert!(!main.join("new.txt").exists(), "nothing lands in main");
    assert_eq!(git(&wt, &["log", "-1", "--format=%s"]), "add new");
    assert_eq!(git(&main, &["log", "-1", "--format=%s"]), "init");
    assert_eq!(
        git(&main, &["status", "--porcelain"]),
        "",
        "the nested worktree stays out of the main checkout's status"
    );

    git(&main, &["merge", "-q", "feat/x"]);
    let call = forge_types::ToolCall {
        id: "x".into(),
        name: "worktree".into(),
        args: serde_json::json!({ "action": "exit", "remove": true }),
    };
    let msg = session
        .store
        .add_message(session.id(), 999, Role::Assistant, "", None);
    let out = session.worktree_tool(&msg.unwrap(), &call).unwrap();
    assert!(out.contains("deleted branch feat/x"), "{out}");
    assert_eq!(session.workspace_root(), main);
    assert_eq!(session.worktree_home, None);
    assert!(!wt.exists());
    assert_eq!(
        store.session_cwd(session.id()).unwrap().as_deref(),
        Some(main.to_str().unwrap())
    );
    assert_eq!(
        std::fs::read_to_string(main.join("new.txt")).unwrap(),
        "in the worktree\n"
    );
}

#[tokio::test]
async fn a_session_enters_an_existing_sibling_worktree_and_exits_back_home() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let sibling = dir.path().join("work/wt/topic");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "topic",
            sibling.to_str().unwrap(),
        ],
    );
    let sibling = sibling.canonicalize().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut session = session_in(
        &main,
        Script {
            calls: Default::default(),
            steps: vec![
                vec![(
                    "worktree",
                    serde_json::json!({ "action": "enter", "branch": "topic" }),
                )],
                vec![(
                    "edit_file",
                    serde_json::json!({ "path": "a.txt", "old": "main", "new": "topic" }),
                )],
                vec![("worktree", serde_json::json!({ "action": "exit" }))],
            ],
        },
        store,
    );
    session.run_turn("switch over").await.unwrap();
    let results = tool_results(&session);
    assert!(results[0].contains("entered worktree"), "{results:?}");
    assert!(results[2].contains("returned to"), "{results:?}");
    assert_eq!(
        std::fs::read_to_string(sibling.join("a.txt")).unwrap(),
        "topic\n"
    );
    assert_eq!(
        std::fs::read_to_string(main.join("a.txt")).unwrap(),
        "main\n"
    );
    assert_eq!(session.workspace_root(), main);
    assert!(sibling.exists(), "plain exit keeps the worktree");
}

#[test]
fn a_linked_worktree_head_is_read_through_its_gitdir_pointer() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let wt = dir.path().join("wt");
    git(
        &main,
        &["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()],
    );
    assert_eq!(current_git_branch(&wt).as_deref(), Some("side"));
    assert_eq!(current_git_branch(&main).as_deref(), Some("main"));
}

#[test]
fn the_session_follows_a_root_the_bridge_recorded_in_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(dir.path());
    let wt = dir.path().join("bridge-wt");
    git(
        &main,
        &["worktree", "add", "-q", "-b", "b", wt.to_str().unwrap()],
    );
    let wt = wt.canonicalize().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let script = Script {
        calls: Default::default(),
        steps: Vec::new(),
    };
    let mut session = session_in(&main, script, Arc::clone(&store));
    let before = session.stored_cwd();
    assert!(
        !session.adopt_bridge_workspace(before.clone()),
        "nothing moved yet"
    );
    store
        .set_session_cwd(session.id(), wt.to_str().unwrap())
        .unwrap();
    assert!(session.adopt_bridge_workspace(before));
    assert_eq!(session.workspace_root(), wt);
    assert_eq!(session.worktree_home.as_deref(), Some(main.as_path()));
    assert_eq!(session.rules_root(), main);
    assert_eq!(
        session.checkpoint_context().workspace,
        wt.to_string_lossy(),
        "the next bridge spawn starts in the worktree"
    );
}
