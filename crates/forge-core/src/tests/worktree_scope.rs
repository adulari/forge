//! A session rooted in a linked git worktree outside the main checkout: absolute paths of the
//! main checkout (what `git` and the project instructions print) must land in the worktree, not be
//! refused and never edit the checkout the worktree isolates.

use super::*;

struct MainCheckoutEditor {
    calls: std::sync::atomic::AtomicUsize,
    main_file: String,
    main_only: String,
}

#[async_trait::async_trait]
impl Provider for MainCheckoutEditor {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        use forge_types::{new_id, ToolCall, Usage};
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let call = |name: &str, args: serde_json::Value| ToolCall {
            id: new_id(),
            name: name.into(),
            args,
        };
        let tool_calls = match n {
            0 => vec![
                call(
                    "edit_file",
                    serde_json::json!({ "path": self.main_file, "old": "branch", "new": "edited" }),
                ),
                call("read_file", serde_json::json!({ "path": self.main_only })),
            ],
            _ => Vec::new(),
        };
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

#[tokio::test]
async fn main_checkout_paths_resolve_inside_a_worktree_session() {
    let base = tempfile::tempdir().unwrap();
    let origin = base.path().join("repo");
    let admin = origin.join(".git/worktrees/wt");
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::write(admin.join("commondir"), "../..\n").unwrap();
    std::fs::create_dir_all(origin.join("node_modules")).unwrap();
    std::fs::write(origin.join("a.txt"), "main").unwrap();
    std::fs::write(origin.join("node_modules/dep.js"), "dep-only-in-main").unwrap();
    let origin = origin.canonicalize().unwrap();
    let wt = base.path().join("elsewhere/wt");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    std::fs::write(wt.join("a.txt"), "branch").unwrap();
    let wt = wt.canonicalize().unwrap();

    let config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MainCheckoutEditor {
            calls: std::sync::atomic::AtomicUsize::new(0),
            main_file: origin.join("a.txt").display().to_string(),
            main_only: origin.join("node_modules/dep.js").display().to_string(),
        }),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(&wt),
        Box::new(CapturePresenter::default()),
        config,
        wt.to_str().unwrap(),
    )
    .unwrap();
    session.run_turn("edit a.txt").await.unwrap();

    assert_eq!(std::fs::read_to_string(wt.join("a.txt")).unwrap(), "edited");
    assert_eq!(
        std::fs::read_to_string(origin.join("a.txt")).unwrap(),
        "main"
    );
    let results: Vec<&str> = session
        .transcript
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| m.content.as_str())
        .collect();
    assert!(results[0].starts_with("edited a.txt"), "{results:?}");
    assert_eq!(results[1], "dep-only-in-main", "{results:?}");
}
