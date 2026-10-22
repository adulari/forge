//! The structural safety floor and `PermissionRequest` / `prompt` hooks through a real turn.

use super::*;

/// Calls each scripted tool once, then finishes. A request whose system prompt is the hook
/// evaluator's gets `hook_reply` instead (the `prompt` hook side call).
struct ToolScript {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    steps: Vec<(&'static str, serde_json::Value)>,
    hook_reply: &'static str,
    hook_calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for ToolScript {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let reply = |content: &str, tool_calls| forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: content.into(),
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        };
        if messages
            .first()
            .is_some_and(|m| m.content.starts_with("You are a hook inside"))
        {
            self.hook_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Ok(reply(self.hook_reply, Vec::new()));
        }
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tool_calls = self
            .steps
            .get(n)
            .map(|(name, args)| {
                vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: (*name).into(),
                    args: args.clone(),
                }]
            })
            .unwrap_or_default();
        let content = if tool_calls.is_empty() { "done" } else { "" };
        Ok(reply(content, tool_calls))
    }
}

struct Run {
    results: Vec<String>,
    warnings: Vec<String>,
    hook_calls: usize,
}

async fn run(
    mode: PermissionMode,
    hooks: Vec<forge_config::HookConfig>,
    workspace: &std::path::Path,
    steps: Vec<(&'static str, serde_json::Value)>,
    hook_reply: &'static str,
) -> Run {
    let mut config = Config {
        permission_mode: mode,
        hooks,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    let hook_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let presenter = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = Arc::clone(&presenter.events);
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(ToolScript {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            steps,
            hook_reply,
            hook_calls: Arc::clone(&hook_calls),
        }),
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(workspace),
        Box::new(presenter),
        config,
        workspace.to_str().unwrap(),
    )
    .unwrap();
    session.run_turn("do the task").await.unwrap();
    let results = session
        .transcript
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| m.content.clone())
        .collect();
    let warnings = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Warning(w) => Some(w.clone()),
            _ => None,
        })
        .collect();
    Run {
        results,
        warnings,
        hook_calls: hook_calls.load(std::sync::atomic::Ordering::SeqCst),
    }
}

fn hook(
    event: forge_config::HookEvent,
    kind: forge_config::HookKind,
    command: &str,
    prompt: Option<&str>,
) -> forge_config::HookConfig {
    forge_config::HookConfig {
        event,
        kind,
        if_rule: None,
        url: None,
        headers: Default::default(),
        allowed_env_vars: Vec::new(),
        run_async: false,
        matcher: None,
        command: command.into(),
        prompt: prompt.map(str::to_string),
        model: Some("fake::hook-judge".into()),
        timeout_secs: 10,
        cc_compat: true,
    }
}

#[tokio::test]
async fn privileged_and_catastrophic_shell_is_refused_even_in_bypass() {
    let ws = tempfile::tempdir().unwrap();
    // Both are harmless if the floor ever let them through: `pkexec --version` only prints, and the
    // `find` cannot match the home directory's own name.
    let out = run(
        PermissionMode::Bypass,
        Vec::new(),
        ws.path(),
        vec![
            ("shell", serde_json::json!({"command": "pkexec --version"})),
            (
                "shell",
                serde_json::json!({"command": "find ~ -maxdepth 0 -name forge-no-such-name-x -delete"}),
            ),
        ],
        "",
    )
    .await;
    assert!(
        out.results[0].starts_with("refused: `pkexec`"),
        "{:?}",
        out.results
    );
    assert!(
        out.results[0].contains("pkexec --version"),
        "{:?}",
        out.results
    );
    assert!(out.results[1].contains("safety floor"), "{:?}", out.results);
    assert!(
        out.warnings.iter().any(|w| w.contains("needs root")),
        "{:?}",
        out.warnings
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_permission_request_hook_answers_the_prompt_for_the_user() {
    let ws = tempfile::tempdir().unwrap();
    let allow = hook(
        forge_config::HookEvent::PermissionRequest,
        forge_config::HookKind::Command,
        r#"echo '{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}'"#,
        None,
    );
    let out = run(
        PermissionMode::Default,
        vec![allow],
        ws.path(),
        vec![(
            "write_file",
            serde_json::json!({"path": "allowed.txt", "content": "x"}),
        )],
        "",
    )
    .await;
    assert!(ws.path().join("allowed.txt").exists(), "{:?}", out.results);

    let deny = hook(
        forge_config::HookEvent::PermissionRequest,
        forge_config::HookKind::Command,
        r#"echo '{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"not on fridays"}}}'"#,
        None,
    );
    let out = run(
        PermissionMode::Default,
        vec![deny],
        ws.path(),
        vec![(
            "write_file",
            serde_json::json!({"path": "denied.txt", "content": "x"}),
        )],
        "",
    )
    .await;
    assert!(!ws.path().join("denied.txt").exists());
    assert_eq!(
        out.results[0],
        "denied by PermissionRequest hook: not on fridays"
    );
}

#[tokio::test]
async fn a_prompt_hook_blocks_a_tool_through_the_session_model() {
    let ws = tempfile::tempdir().unwrap();
    let judge = hook(
        forge_config::HookEvent::PreToolUse,
        forge_config::HookKind::Prompt,
        "",
        Some("Block any write to a file named secret: $ARGUMENTS"),
    );
    let out = run(
        PermissionMode::Bypass,
        vec![judge],
        ws.path(),
        vec![(
            "write_file",
            serde_json::json!({"path": "secret.txt", "content": "x"}),
        )],
        r#"{"ok": false, "reason": "secret files are off limits"}"#,
    )
    .await;
    assert_eq!(out.hook_calls, 1);
    assert!(!ws.path().join("secret.txt").exists());
    assert!(
        out.results[0].contains("secret files are off limits"),
        "{:?}",
        out.results
    );

    let judge = hook(
        forge_config::HookEvent::PreToolUse,
        forge_config::HookKind::Prompt,
        "",
        Some("Allow ordinary files: $ARGUMENTS"),
    );
    let out = run(
        PermissionMode::Bypass,
        vec![judge],
        ws.path(),
        vec![(
            "write_file",
            serde_json::json!({"path": "plain.txt", "content": "x"}),
        )],
        r#"{"ok": true}"#,
    )
    .await;
    assert_eq!(out.hook_calls, 1);
    assert!(ws.path().join("plain.txt").exists(), "{:?}", out.results);
}
