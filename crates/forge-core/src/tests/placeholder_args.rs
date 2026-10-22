//! A model that must fill in every property of a tool sends `commands: []`, `path: ""`,
//! `timeout_secs: 0` for the ones it is not using. The call it meant is what the permission rules,
//! the loop guards and the tool see; the transcript keeps what was sent.

use super::*;
use crate::deferred_tools::TOOL_CALL;

fn session_in(dir: &std::path::Path, mode: forge_types::PermissionMode) -> Session {
    let config = Config {
        permission_mode: mode,
        ..Config::default()
    };
    Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(CapturePresenter::default()),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap()
}

fn call(name: &str, args: serde_json::Value) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: "call-1".into(),
        name: name.into(),
        args,
    }
}

#[test]
fn a_shell_call_loses_its_placeholders_and_keeps_its_command() {
    let dir = tempfile::tempdir().unwrap();
    let session = session_in(dir.path(), forge_types::PermissionMode::Bypass);
    let sent = call(
        "shell",
        serde_json::json!({
            "command": "git status --short",
            "commands": [],
            "cwd": "",
            "timeout_secs": 0,
            "poll_interval_secs": 0,
            "background": false,
            "pty": false,
            "poll_until_exit_zero": false,
            "stop_on_error": true
        }),
    );
    let resolved = session.resolve_model_call(&sent);
    assert_eq!(
        resolved.args,
        serde_json::json!({
            "command": "git status --short",
            "background": false,
            "pty": false,
            "poll_until_exit_zero": false,
            "stop_on_error": true
        })
    );
    assert_eq!(resolved.id, sent.id);
}

#[tokio::test]
async fn a_shell_call_that_used_to_be_rejected_as_both_now_runs() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path(), forge_types::PermissionMode::Bypass);
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "run it", None)
        .unwrap();
    let out = session
        .invoke_tool(
            &msg_id,
            &call(
                "shell",
                serde_json::json!({
                    "command": "echo placeholder-ok",
                    "commands": [],
                    "cwd": ".",
                    "background": false,
                    "poll_until_exit_zero": false,
                    "poll_interval_secs": 5,
                    "pty": false,
                    "stop_on_error": true,
                    "timeout_secs": 120
                }),
            ),
        )
        .await
        .unwrap();
    assert!(!out.starts_with("error:"), "{out}");
    assert!(out.contains("placeholder-ok"), "{out}");
}

#[tokio::test]
async fn a_tool_call_wrapper_is_cleaned_like_the_call_it_wraps() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path(), forge_types::PermissionMode::Bypass);
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "run it", None)
        .unwrap();
    let wrapped = call(
        TOOL_CALL,
        serde_json::json!({
            "name": "shell",
            "arguments": { "command": "echo wrapped-ok", "commands": [], "cwd": "" }
        }),
    );
    let resolved = session.resolve_model_call(&wrapped);
    assert_eq!(resolved.name, "shell");
    assert_eq!(
        resolved.args,
        serde_json::json!({ "command": "echo wrapped-ok" })
    );
    let out = session.invoke_tool(&msg_id, &wrapped).await.unwrap();
    assert!(out.contains("wrapped-ok"), "{out}");
}

#[test]
fn core_owned_tools_are_cleaned_against_their_own_spec() {
    let dir = tempfile::tempdir().unwrap();
    let session = session_in(dir.path(), forge_types::PermissionMode::Default);
    let tasks = session.resolve_model_call(&call(
        "update_tasks",
        serde_json::json!({
            "tasks": [
                { "title": "a", "status": "pending", "assignee": "" },
                { "title": "b", "status": "done", "assignee": "worker" }
            ]
        }),
    ));
    assert_eq!(
        tasks.args,
        serde_json::json!({
            "tasks": [
                { "title": "a", "status": "pending" },
                { "title": "b", "status": "done", "assignee": "worker" }
            ]
        })
    );
    let spawn = session.resolve_model_call(&call(
        "spawn_agents",
        serde_json::json!({
            "agents": [{ "task": "look", "agent": "" }],
            "detached": false
        }),
    ));
    assert_eq!(
        spawn.args,
        serde_json::json!({ "agents": [{ "task": "look" }], "detached": false })
    );
}

#[test]
fn an_unknown_tool_keeps_its_arguments_as_sent() {
    let dir = tempfile::tempdir().unwrap();
    let session = session_in(dir.path(), forge_types::PermissionMode::Default);
    let args = serde_json::json!({ "anything": "", "list": [] });
    assert_eq!(
        session
            .resolve_model_call(&call("no_such_tool", args.clone()))
            .args,
        args
    );
}

#[test]
fn a_blank_command_cannot_hide_a_batch_from_the_permission_rules() {
    use forge_types::{PermissionDecision, PermissionMode, PermissionRule, RuleSource, SideEffect};
    let deny_touch = PermissionRule {
        tool: "shell".into(),
        patterns: vec!["touch *".into()],
        decision: PermissionDecision::Deny,
        source: RuleSource::Configured,
        reason: None,
    };
    let decide = |args: serde_json::Value| {
        permission::decide(
            PermissionMode::Bypass,
            SideEffect::Shell,
            "shell",
            &args,
            std::slice::from_ref(&deny_touch),
        )
    };
    let batch = serde_json::json!({ "commands": ["touch made-by-the-batch"] });
    let with_blank = serde_json::json!({ "command": "", "commands": ["touch made-by-the-batch"] });
    assert_eq!(
        decide(batch),
        PermissionDecision::Deny,
        "the rule matches the batch"
    );
    assert_eq!(
        decide(with_blank),
        PermissionDecision::Deny,
        "a blank `command` next to the batch must not hide it from the rule"
    );
}
