//! The Claude-Code hook events added for parity, fired from their real seams in a session:
//! `PostToolUseFailure`, `PermissionDenied`, the `if` filter, `SessionStart`'s `source`, and the
//! `WorktreeCreate` / `CwdChanged` / `WorktreeRemove` trio on the `worktree` tool.

use super::*;
use forge_config::{HookConfig, HookEvent};

fn cc(event: HookEvent, command: &str) -> HookConfig {
    HookConfig {
        cc_compat: true,
        timeout_secs: 10,
        ..HookConfig::new(event, command)
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("forge-hook-{tag}-{}", forge_types::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn session_in(
    base: &std::path::Path,
    mode: forge_types::PermissionMode,
    hooks: Vec<HookConfig>,
) -> Session {
    let config = Config {
        permission_mode: mode,
        hooks,
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

fn read(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_shell_call_fires_post_tool_use_failure_instead_of_post_tool_use() {
    let base = temp_dir("failure");
    let post = base.join("post.json");
    let failure = base.join("failure.json");
    let hooks = vec![
        cc(HookEvent::PostToolUse, &format!("cat > {}", post.display())),
        cc(
            HookEvent::PostToolUseFailure,
            &format!(
                r#"cat > {}; echo '{{"hookSpecificOutput":{{"hookEventName":"PostToolUseFailure","additionalContext":"check the exit code"}}}}'"#,
                failure.display()
            ),
        ),
    ];
    let mut session = session_in(&base, forge_types::PermissionMode::Bypass, hooks);
    call(
        &mut session,
        "shell",
        serde_json::json!({"command": "exit 3"}),
    )
    .await;
    let payload = read(&failure);
    assert!(
        payload.contains(r#""hook_event_name":"PostToolUseFailure""#),
        "{payload}"
    );
    assert!(payload.contains(r#""tool_name":"Bash""#), "{payload}");
    assert!(payload.contains(r#""error":"#), "{payload}");
    assert!(payload.contains(r#""tool_use_id":"#), "{payload}");
    assert!(!post.exists(), "PostToolUse fires only after a success");
    assert!(session
        .pending_hints
        .iter()
        .any(|h| h.contains("check the exit code")));

    call(
        &mut session,
        "shell",
        serde_json::json!({"command": "echo fine"}),
    )
    .await;
    assert!(read(&post).contains(r#""hook_event_name":"PostToolUse""#));
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[tokio::test]
async fn an_unattended_refusal_fires_permission_denied_and_retry_reaches_the_model() {
    let base = temp_dir("denied");
    let seen = base.join("denied.json");
    let hooks = vec![cc(
        HookEvent::PermissionDenied,
        &format!(
            r#"cat > {}; echo '{{"hookSpecificOutput":{{"hookEventName":"PermissionDenied","retry":true}}}}'"#,
            seen.display()
        ),
    )];
    let mut session = session_in(&base, forge_types::PermissionMode::Default, hooks);
    let out = call(
        &mut session,
        "write_file",
        serde_json::json!({"path": "x.txt", "content": "hi"}),
    )
    .await;
    assert!(!base.join("x.txt").exists(), "{out}");
    let payload = read(&seen);
    assert!(
        payload.contains(r#""hook_event_name":"PermissionDenied""#),
        "{payload}"
    );
    assert!(payload.contains(r#""tool_name":"Write""#), "{payload}");
    assert!(payload.contains(r#""reason":"#), "{payload}");
    assert!(session
        .pending_hints
        .iter()
        .any(|h| h.contains("may retry the refused `write_file` call")));
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[tokio::test]
async fn the_if_rule_runs_a_hook_only_for_matching_commands() {
    let base = temp_dir("if");
    let hooks = vec![HookConfig {
        if_rule: Some("Bash(git push*)".into()),
        ..cc(
            HookEvent::PreToolUse,
            "echo 'no pushing from here' >&2; exit 2",
        )
    }];
    let mut session = session_in(&base, forge_types::PermissionMode::Bypass, hooks);
    let ran = call(
        &mut session,
        "shell",
        serde_json::json!({"command": "echo ok"}),
    )
    .await;
    assert!(ran.contains("ok") && !ran.contains("no pushing"), "{ran}");
    let blocked = call(
        &mut session,
        "shell",
        serde_json::json!({"command": "cd . && git push origin main"}),
    )
    .await;
    assert!(blocked.contains("no pushing from here"), "{blocked}");
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[tokio::test]
async fn session_start_reports_how_the_session_began_and_its_matcher_filters_on_it() {
    let base = temp_dir("source");
    let log = base.join("starts.log");
    let hooks = vec![
        cc(
            HookEvent::SessionStart,
            &format!("cat >> {}; echo >> {}", log.display(), log.display()),
        ),
        HookConfig {
            matcher: Some("clear".into()),
            ..cc(
                HookEvent::SessionStart,
                "echo 'fresh conversation after /new'",
            )
        },
    ];
    let mut session = session_in(&base, forge_types::PermissionMode::Bypass, hooks);
    session.fire_session_start(None).await;
    assert!(read(&log).contains(r#""source":"startup""#));
    assert!(!session
        .transcript
        .iter()
        .any(|m| m.content.contains("fresh conversation")));

    session
        .reset_fresh_hooked(base.to_str().unwrap())
        .await
        .unwrap();
    let starts = read(&log);
    assert!(starts.contains(r#""source":"clear""#), "{starts}");
    assert!(
        session
            .transcript
            .iter()
            .any(|m| m.content.contains("fresh conversation after /new")),
        "the clear-only hook ran and its stdout became context"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[tokio::test]
async fn a_worktree_create_hook_replaces_git_and_cwd_changed_follows_the_move() {
    let base = temp_dir("worktree");
    let moves = base.join("moves.log");
    let removed = base.join("removed.json");
    let hooks = vec![
        cc(
            HookEvent::WorktreeCreate,
            "mkdir -p wt-made && echo \"$PWD/wt-made\"",
        ),
        cc(
            HookEvent::CwdChanged,
            &format!("cat >> {}; echo >> {}", moves.display(), moves.display()),
        ),
        cc(
            HookEvent::WorktreeRemove,
            &format!("cat > {}; rm -rf wt-made", removed.display()),
        ),
    ];
    let mut session = session_in(&base, forge_types::PermissionMode::Bypass, hooks);
    let made = base.join("wt-made");
    let out = call(
        &mut session,
        "worktree",
        serde_json::json!({"action": "create", "name": "feature"}),
    )
    .await;
    assert!(out.contains("made by the WorktreeCreate hook"), "{out}");
    assert_eq!(session.workspace_root(), made);
    let log = read(&moves);
    assert!(log.contains(r#""hook_event_name":"CwdChanged""#), "{log}");
    assert!(
        log.contains(&format!(r#""new_cwd":"{}""#, made.display())),
        "{log}"
    );

    let out = call(
        &mut session,
        "worktree",
        serde_json::json!({"action": "exit", "remove": true}),
    )
    .await;
    assert!(out.contains("removed worktree"), "{out}");
    assert_eq!(session.workspace_root(), base);
    assert!(!made.exists());
    assert!(read(&removed).contains(r#""worktree_path":"#));
    let _ = std::fs::remove_dir_all(base);
}
