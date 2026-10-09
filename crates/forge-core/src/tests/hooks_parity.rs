//! Claude-Code hook parity end to end: blocking `Stop` hooks, `updatedInput`, and background-job
//! completion notices reaching the model.

use super::*;

fn cc_hook(event: forge_config::HookEvent, command: &str) -> forge_config::HookConfig {
    forge_config::HookConfig {
        event,
        matcher: None,
        command: command.into(),
        timeout_secs: 10,
        cc_compat: true,
    }
}

fn calls(provider: &CountingFinalProvider) -> usize {
    provider.calls.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(unix)]
#[tokio::test]
async fn cc_stop_hook_json_block_feeds_its_reason_back_as_the_next_instruction() {
    let provider = Arc::new(CountingFinalProvider::default());
    let mut config = stop_hook_config("true");
    config.stop_hook_max_blocks = 2;
    config.hooks = vec![cc_hook(
        forge_config::HookEvent::Stop,
        r#"echo '{"decision":"block","reason":"run the tests first"}'"#,
    )];
    let mut session = counting_session(provider.clone(), config, CapturePresenter::default());
    session.run_turn("do the task").await.unwrap();
    assert_eq!(
        calls(&provider),
        3,
        "primary run + 2 configured continuations"
    );
    let fed_back = session
        .transcript
        .iter()
        .filter(|m| m.content.contains("[stop hook] run the tests first"))
        .count();
    assert_eq!(fed_back, 2, "each continuation carries the hook's reason");
}

#[cfg(unix)]
#[tokio::test]
async fn stop_hook_cap_defaults_to_claude_codes_eight() {
    let provider = Arc::new(CountingFinalProvider::default());
    let mut session = counting_session(
        provider.clone(),
        stop_hook_config("exit 2"),
        CapturePresenter::default(),
    );
    session.run_turn("do the task").await.unwrap();
    assert_eq!(calls(&provider), 9);
}

#[cfg(unix)]
#[tokio::test]
async fn stop_hook_sees_the_last_assistant_message_and_continue_false_overrides_a_block() {
    let provider = Arc::new(CountingFinalProvider::default());
    // Blocks unless it was handed the model's final text (so a missing field would loop to the cap).
    let mut config = stop_hook_config(r#"grep -q '"last_assistant_message":"all done"' || exit 2"#);
    let mut session = counting_session(
        provider.clone(),
        config.clone(),
        CapturePresenter::default(),
    );
    session.run_turn("do the task").await.unwrap();
    assert_eq!(
        calls(&provider),
        1,
        "last_assistant_message reached the hook"
    );

    let provider = Arc::new(CountingFinalProvider::default());
    config.hooks = vec![
        cc_hook(forge_config::HookEvent::Stop, "exit 2"),
        cc_hook(
            forge_config::HookEvent::Stop,
            r#"echo '{"continue":false}'"#,
        ),
    ];
    let mut session = counting_session(provider.clone(), config, CapturePresenter::default());
    session.run_turn("do the task").await.unwrap();
    assert_eq!(
        calls(&provider),
        1,
        "continue:false ends the turn despite the other hook"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn stop_hook_additional_context_continues_the_conversation() {
    let provider = Arc::new(CountingFinalProvider::default());
    let mut config = stop_hook_config("true");
    config.stop_hook_max_blocks = 1;
    config.hooks = vec![cc_hook(
        forge_config::HookEvent::Stop,
        r#"echo '{"hookSpecificOutput":{"hookEventName":"Stop","additionalContext":"also update the changelog"}}'"#,
    )];
    let mut session = counting_session(provider.clone(), config, CapturePresenter::default());
    session.run_turn("do the task").await.unwrap();
    assert_eq!(calls(&provider), 2);
    assert!(session
        .transcript
        .iter()
        .any(|m| m.content.contains("also update the changelog")));
}

#[cfg(unix)]
async fn run_shell_with_pre_hook(hook_out: &str, args: serde_json::Value) -> String {
    let base = std::env::temp_dir().join(format!("forge-updated-input-{}", forge_types::new_id()));
    std::fs::create_dir_all(&base).unwrap();
    let config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        hooks: vec![cc_hook(
            forge_config::HookEvent::PreToolUse,
            &format!("echo '{hook_out}'"),
        )],
        ..Config::default()
    };
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(&base),
        Box::new(CapturePresenter::default()),
        config,
        base.to_str().unwrap(),
    )
    .unwrap();
    let call = forge_types::ToolCall {
        id: "c".into(),
        name: "shell".into(),
        args,
    };
    let sid = session.session_id().to_string();
    let msg = session
        .store
        .add_message(&sid, 0, Role::User, "x", None)
        .unwrap();
    let out = session.invoke_tool(&msg, &call).await.unwrap();
    let _ = std::fs::remove_dir_all(base);
    out
}

#[cfg(unix)]
#[tokio::test]
async fn pretooluse_updated_input_rewrites_the_bash_command_before_it_runs() {
    // The exact shape `rtk hook claude` prints.
    let out = run_shell_with_pre_hook(
        r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{"command":"echo rewritten-by-hook"}}}"#,
        serde_json::json!({"command": "echo original"}),
    )
    .await;
    assert!(out.contains("rewritten-by-hook"), "{out}");
    assert!(!out.contains("original"), "{out}");
}

#[cfg(unix)]
#[tokio::test]
async fn updated_input_is_revalidated_against_the_tool_schema() {
    let out = run_shell_with_pre_hook(
        r#"{"hookSpecificOutput":{"updatedInput":{"description":"no command"}}}"#,
        serde_json::json!({"command": "echo original"}),
    )
    .await;
    assert!(out.contains("invalid arguments"), "{out}");
}

#[cfg(unix)]
async fn shell_session(base: &std::path::Path) -> Session {
    let config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(base),
        Box::new(CapturePresenter::default()),
        config,
        base.to_str().unwrap(),
    )
    .unwrap()
}

#[cfg(unix)]
async fn call(session: &mut Session, name: &str, args: serde_json::Value) -> String {
    let sid = session.session_id().to_string();
    let msg = session
        .store
        .add_message(&sid, 0, Role::User, "x", None)
        .unwrap();
    let call = forge_types::ToolCall {
        id: forge_types::new_id(),
        name: name.into(),
        args,
    };
    session.invoke_tool(&msg, &call).await.unwrap()
}

#[cfg(unix)]
async fn wait_pending(wake: &crate::job_wake::JobWake) -> bool {
    for _ in 0..100 {
        if wake.has_pending() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

#[cfg(unix)]
#[tokio::test]
async fn a_finished_background_job_reaches_the_model_at_the_next_step() {
    let base = std::env::temp_dir().join(format!("forge-job-wake-{}", forge_types::new_id()));
    std::fs::create_dir_all(&base).unwrap();
    let mut session = shell_session(&base).await;
    let wake = session.job_wake();
    let started = call(
        &mut session,
        "shell",
        serde_json::json!({"command": "echo built-ok; exit 3", "background": true}),
    )
    .await;
    assert!(started.contains("started background job"), "{started}");
    assert!(wait_pending(&wake).await, "exit was never reported");

    assert!(session.inject_steers(), "the notice joins the running turn");
    let injected = session
        .transcript
        .iter()
        .find(|m| m.content.contains("[background job"))
        .expect("notice in transcript")
        .content
        .clone();
    assert!(injected.contains("exited 3"), "{injected}");
    assert!(injected.contains("built-ok"), "{injected}");
    assert!(!session.inject_steers(), "delivered exactly once");
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[tokio::test]
async fn stopping_a_job_yourself_is_not_news_and_foreign_jobs_are_not_reported() {
    let base = std::env::temp_dir().join(format!("forge-job-stop-{}", forge_types::new_id()));
    std::fs::create_dir_all(&base).unwrap();
    let mut session = shell_session(&base).await;
    let wake = session.job_wake();
    let started = call(
        &mut session,
        "shell",
        serde_json::json!({"command": "sleep 30", "background": true}),
    )
    .await;
    let pid: u64 = started
        .split("job ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|id| id.parse().ok())
        .unwrap();
    call(
        &mut session,
        "shell_job",
        serde_json::json!({"action": "stop", "id": pid}),
    )
    .await;

    // A job started by another session (no sink scoped around its tool call) stays silent.
    let other = shell_session(&base).await;
    let other_wake = other.job_wake();
    forge_tools::Tool::run(
        &forge_tools::ShellTool::default(),
        &serde_json::json!({"command": "true", "background": true, "cwd": base}),
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert!(
        !wake.has_pending(),
        "a deliberate stop must not wake the model"
    );
    assert!(!other_wake.has_pending());
    let _ = std::fs::remove_dir_all(base);
}
