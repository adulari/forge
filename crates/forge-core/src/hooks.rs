//! Shell hooks (docs/features/hooks.md). Each `[[hooks]]` entry runs via the OS shell around
//! tool calls and session lifecycle events. A `PreToolUse` hook that exits non-zero **blocks**
//! the tool; a `UserPromptSubmit` hook can rewrite the user's prompt (stdout replaces it on
//! exit 0) or block the turn (non-zero). `SessionStart` output can be injected as model context
//! (CC-compat hooks); `SessionEnd` observes only. Hooks are time-bounded so a wedged hook can't hang the agent.

use std::process::Stdio;
use std::time::Duration;

use forge_config::{HookConfig, HookEvent};
use forge_types::truncate_ellipsis as truncate;

use crate::hooks_cc::{cc_input_to_forge, parse_cc_output, to_cc_payload};
use tokio::io::AsyncWriteExt;

/// The combined effect of running the hooks that matched one tool call + event.
#[derive(Debug, Default)]
pub struct HookOutcome {
    /// `Some(reason)` if a `PreToolUse` hook blocked the call.
    pub blocked: Option<String>,
    /// Lines to surface to the user (hook stdout / errors).
    pub notes: Vec<String>,
    /// Rewritten tool args from a `PreToolUse` hook that exited 0 and emitted a JSON object on
    /// stdout. The core substitutes these args for the model's original args before running the
    /// tool. `None` means use the original args unchanged.
    pub rewritten_args: Option<serde_json::Value>,
    /// Context strings a hook asked to inject into the transcript (`{"action":"inject",
    /// "context":"…"}`). The core queues each as a model-visible system hint after the tool runs —
    /// so a hook can feed the model extra context (lint output, "this file is generated", a policy
    /// reminder) without blocking or rewriting. Works for both `PreToolUse` and `PostToolUse`.
    pub injected_context: Vec<String>,
}

/// A structured directive a hook can emit on stdout as a JSON object with an `"action"` field.
/// This is the richer protocol on top of the legacy "bare JSON object = rewritten args" behavior:
/// a `PreToolUse` hook that emits a JSON object WITHOUT an `action` still rewrites args as before.
enum HookDirective {
    /// `{"action":"rewrite","args":{…}}` — replace the tool's args (PreToolUse).
    Rewrite(serde_json::Value),
    /// `{"action":"inject","context":"…"}` — add model-visible context after the call.
    Inject(String),
    /// `{"action":"block","reason":"…"}` — block the call (PreToolUse; downgraded to a note elsewhere).
    Block(String),
    /// `{"action":"allow"}` — explicit no-op (the hook approves without changing anything).
    Noop,
    /// Anything else (non-JSON, or a JSON object that isn't a recognised directive) → a user note.
    Note(String),
}

/// Interpret a hook's exit-0 stdout. The structured `action` protocol takes precedence; a bare JSON
/// object (no `action`) keeps the legacy meaning (rewrite args, but only for `PreToolUse`); anything
/// else is a note. A malformed structured directive (missing `args`/`context`) degrades to a note so
/// the author sees their output rather than it silently vanishing.
fn parse_hook_directive(stdout: &str, event: HookEvent) -> HookDirective {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(stdout)
    else {
        return HookDirective::Note(stdout.to_string());
    };
    if let Some(action) = map.get("action").and_then(serde_json::Value::as_str) {
        return match action {
            "rewrite" => map
                .get("args")
                .cloned()
                .map(HookDirective::Rewrite)
                .unwrap_or_else(|| HookDirective::Note(stdout.to_string())),
            "inject" => map
                .get("context")
                .and_then(serde_json::Value::as_str)
                .filter(|c| !c.trim().is_empty())
                .map(|c| HookDirective::Inject(c.to_string()))
                .unwrap_or_else(|| HookDirective::Note(stdout.to_string())),
            "block" => HookDirective::Block(
                map.get("reason")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("blocked by hook")
                    .to_string(),
            ),
            "allow" => HookDirective::Noop,
            _ => HookDirective::Note(stdout.to_string()),
        };
    }
    // Legacy: a bare JSON object rewrites args on PreToolUse; elsewhere it's just a note.
    if event == HookEvent::PreToolUse {
        HookDirective::Rewrite(serde_json::Value::Object(map))
    } else {
        HookDirective::Note(stdout.to_string())
    }
}

/// Run one CC-compat hook and fold its decision into `outcome`. Returns `true` if the call should
/// be blocked + short-circuited (PreToolUse only; PostToolUse downgrades a block to a note).
/// `updatedInput` replaces the tool's args (PreToolUse), translated from Claude-Code names.
async fn run_cc_hook(
    h: &HookConfig,
    event: HookEvent,
    tool: &str,
    payload: &str,
    outcome: &mut HookOutcome,
) -> bool {
    let cc_payload = to_cc_payload(payload, event, "");
    match run_one(h, &cc_payload).await {
        Ok((code, stdout, stderr)) => {
            let out = parse_cc_output(code, &stdout, &stderr);
            if let Some(reason) = out.block {
                if event == HookEvent::PreToolUse {
                    outcome.blocked = Some(reason);
                    return true;
                }
                outcome.notes.push(format!("⎇ hook: {reason}"));
            }
            if event == HookEvent::PreToolUse {
                if let Some(input) = out.updated_input {
                    outcome.rewritten_args = Some(cc_input_to_forge(tool, &input));
                }
            }
            outcome.injected_context.extend(out.context);
            if let Some(text) = out.note {
                outcome.notes.push(format!("⎇ hook: {text}"));
            }
        }
        Err(e) => outcome.notes.push(format!("⎇ hook error: {e}")),
    }
    false
}

/// Run every hook matching `event` + `tool`, in declaration order. The first `PreToolUse` hook
/// that exits non-zero blocks and short-circuits. A hook that fails to launch is noted, not fatal.
/// CC-compat hooks ([`HookConfig::cc_compat`]) speak the Claude-Code protocol (CC stdin payload +
/// `decision`/exit-2 output); native hooks keep Forge's directive protocol.
pub async fn run_hooks(
    hooks: &[HookConfig],
    event: HookEvent,
    tool: &str,
    payload: &str,
) -> HookOutcome {
    let mut outcome = HookOutcome::default();
    let payload = payload.to_string();
    for h in hooks.iter().filter(|h| h.event == event && h.matches(tool)) {
        if h.cc_compat {
            if run_cc_hook(h, event, tool, &payload, &mut outcome).await {
                break;
            }
            continue;
        }
        match run_one(h, &payload).await {
            Ok((code, stdout, stderr)) => {
                let trimmed = stdout.trim();
                if event == HookEvent::PreToolUse && code != 0 {
                    let err = stderr.trim();
                    let reason = if !err.is_empty() {
                        truncate(err, 800)
                    } else if !trimmed.is_empty() {
                        truncate(trimmed, 800)
                    } else {
                        format!("{tool} blocked by hook (exit {code})")
                    };
                    outcome.blocked = Some(reason);
                    break;
                }
                // exit 0 + non-empty stdout: interpret the structured directive protocol (rewrite /
                // inject / block / allow), falling back to the legacy "bare object = rewrite" and to
                // a plain note. `block` only blocks on PreToolUse (Post can't unwind a finished call).
                if !trimmed.is_empty() {
                    match parse_hook_directive(trimmed, event) {
                        HookDirective::Rewrite(args) => outcome.rewritten_args = Some(args),
                        HookDirective::Inject(ctx) => outcome.injected_context.push(ctx),
                        HookDirective::Block(reason) => {
                            if event == HookEvent::PreToolUse {
                                outcome.blocked = Some(truncate(&reason, 800));
                                break;
                            }
                            outcome
                                .notes
                                .push(format!("⎇ hook: {}", truncate(&reason, 800)));
                        }
                        HookDirective::Noop => {}
                        HookDirective::Note(text) => outcome
                            .notes
                            .push(format!("⎇ hook: {}", truncate(&text, 800))),
                    }
                }
            }
            Err(e) => outcome.notes.push(format!("⎇ hook error: {e}")),
        }
    }
    outcome
}

/// Run `user_prompt_submit` hooks in declaration order.
///
/// Returns `Ok(prompt)` where `prompt` is either the original (no hook rewrote it) or the
/// stdout from the first hook that exited 0 and produced non-empty output.
/// Returns `Err(reason)` if any hook exits non-zero — the turn should be blocked.
pub async fn run_prompt_hooks_in(
    hooks: &[HookConfig],
    prompt: &str,
    cwd: Option<&std::path::Path>,
) -> Result<String, String> {
    let payload = serde_json::json!({
        "prompt": prompt,
        "cwd": cwd.map(|cwd| cwd.display().to_string()),
    })
    .to_string();
    let mut current = prompt.to_string();
    for h in hooks
        .iter()
        .filter(|h| h.event == HookEvent::UserPromptSubmit)
    {
        if h.cc_compat {
            // CC UserPromptSubmit: a block decision / exit-2 blocks the turn; otherwise stdout (and
            // `additionalContext`) is APPENDED as extra context (CC semantics — it doesn't replace
            // the prompt the way a native prompt hook does).
            let cc_payload = to_cc_payload(&payload, HookEvent::UserPromptSubmit, "");
            match run_one(h, &cc_payload).await {
                Ok((code, stdout, stderr)) => {
                    let out = parse_cc_output(code, &stdout, &stderr);
                    if let Some(reason) = out.block {
                        return Err(reason);
                    }
                    for ctx in out.context.into_iter().chain(out.note) {
                        current = format!("{current}\n\n{ctx}");
                    }
                }
                Err(e) => eprintln!("⎇ hook error: {e}"),
            }
            continue;
        }
        match run_one(h, &payload).await {
            Ok((code, stdout, stderr)) => {
                if code != 0 {
                    let reason = if !stderr.trim().is_empty() {
                        truncate(stderr.trim(), 800)
                    } else if !stdout.trim().is_empty() {
                        truncate(stdout.trim(), 800)
                    } else {
                        format!("prompt blocked by hook (exit {code})")
                    };
                    return Err(reason);
                }
                let out = stdout.trim().to_string();
                if !out.is_empty() {
                    current = out;
                }
            }
            Err(e) => {
                // Launch failure is noted but doesn't block the turn.
                eprintln!("⎇ hook error: {e}");
            }
        }
    }
    Ok(current)
}

pub async fn run_prompt_hooks(hooks: &[HookConfig], prompt: &str) -> Result<String, String> {
    run_prompt_hooks_in(hooks, prompt, None).await
}

/// Run session lifecycle hooks (`session_start` / `session_end`). Exit code is advisory.
///
/// Returns the context a `SessionStart` hook asked to put in front of the model, for the caller to
/// inject (`Session::inject_hook_context`). Claude-Code semantics apply to `cc_compat` hooks: plain
/// stdout and `hookSpecificOutput.additionalContext` are context, not a visible note, and the hook
/// gets the CC payload (`hook_event_name`, `source`). Native hooks keep printing stdout to stderr.
pub async fn run_session_hooks_in(
    hooks: &[HookConfig],
    event: HookEvent,
    session_id: &str,
    cwd: Option<&std::path::Path>,
) -> Vec<String> {
    debug_assert!(
        matches!(event, HookEvent::SessionStart | HookEvent::SessionEnd),
        "run_session_hooks called with non-session event"
    );
    let event_str = match event {
        HookEvent::SessionStart => "session_start",
        HookEvent::SessionEnd => "session_end",
        _ => return Vec::new(),
    };
    let payload = serde_json::json!({
        "session_id": session_id,
        "event": event_str,
        "cwd": cwd.map(|cwd| cwd.display().to_string()),
        "source": "startup",
    })
    .to_string();
    let mut context = Vec::new();
    for h in hooks.iter().filter(|h| h.event == event) {
        let sent = if h.cc_compat {
            to_cc_payload(&payload, event, session_id)
        } else {
            payload.clone()
        };
        match run_one(h, &sent).await {
            Ok((code, stdout, stderr)) if h.cc_compat => {
                let out = parse_cc_output(code, &stdout, &stderr);
                if event == HookEvent::SessionStart {
                    context.extend(out.context.into_iter().chain(out.note));
                }
            }
            Ok((_, stdout, _)) => {
                let out = stdout.trim();
                if !out.is_empty() {
                    eprintln!("⎇ hook: {}", truncate(out, 800));
                }
            }
            Err(e) => eprintln!("⎇ hook error: {e}"),
        }
    }
    context
}

/// The combined effect of running lifecycle hooks (`notification`, `pre_compact`, `post_compact`,
/// `stop`, `subagent_stop`) for one event.
#[derive(Debug, Default)]
pub struct LifecycleOutcome {
    /// `Some(reason)` if a hook asked to block (exit 2 / `decision:block`). For `stop`/`subagent_stop`
    /// this is the "keep going, don't stop yet" signal, and a bare `additionalContext` counts too
    /// (Claude Code continues the conversation with it); the caller decides whether to honor it.
    pub blocked: Option<String>,
    /// A hook printed `{"continue":false}`: end now, overriding any other hook's block.
    pub halt: bool,
    /// Lines to surface to the user (hook stdout / decision reasons).
    pub notes: Vec<String>,
}

/// Run the Claude-Code lifecycle hooks Forge previously lacked: `Notification`, `PreCompact`,
/// `PostCompact`, `Stop`, `SubagentStop`. `fields` are merged into the stdin payload (e.g.
/// `{"message":…}` for a notification, `{"trigger":…}` for compaction). Native hooks receive
/// `{session_id, event, …fields}`; CC-compat hooks receive the CC shape with `hook_event_name`.
/// Output is collected as notes; acting on `blocked` is the caller's job (only `Stop` and
/// `SubagentStop` do, see `stop_hook.rs`).
pub async fn run_lifecycle_hooks(
    hooks: &[HookConfig],
    event: HookEvent,
    session_id: &str,
    fields: serde_json::Value,
) -> LifecycleOutcome {
    let mut outcome = LifecycleOutcome::default();
    // Native payload: {session_id, event, ...fields}.
    let mut base = serde_json::Map::new();
    base.insert("session_id".into(), session_id.into());
    base.insert("event".into(), event.cc_name().into());
    if let Some(map) = fields.as_object() {
        for (k, v) in map {
            base.insert(k.clone(), v.clone());
        }
    }
    let native_payload = serde_json::Value::Object(base).to_string();
    let continues = matches!(event, HookEvent::Stop | HookEvent::SubagentStop);

    for h in hooks.iter().filter(|h| h.event == event) {
        let payload = if h.cc_compat {
            to_cc_payload(&native_payload, event, session_id)
        } else {
            native_payload.clone()
        };
        match run_one(h, &payload).await {
            Ok((code, stdout, stderr)) => {
                let out = parse_cc_output(code, &stdout, &stderr);
                outcome.halt |= out.halt;
                if let Some(reason) = out.block {
                    outcome.notes.push(format!("⎇ hook: {reason}"));
                    outcome.blocked.get_or_insert(reason);
                } else if let (true, Some(ctx)) = (continues, &out.context) {
                    outcome.blocked.get_or_insert_with(|| ctx.clone());
                } else if let Some(ctx) = out.context {
                    outcome.notes.push(format!("⎇ hook: {ctx}"));
                }
                if let Some(text) = out.note {
                    outcome.notes.push(format!("⎇ hook: {text}"));
                }
            }
            Err(e) => outcome.notes.push(format!("⎇ hook error: {e}")),
        }
    }
    outcome
}

fn hook_shell() -> (&'static str, &'static str) {
    #[cfg(windows)]
    return ("cmd", "/C");
    #[cfg(not(windows))]
    ("sh", "-c")
}

pub async fn run_session_hooks(
    hooks: &[HookConfig],
    event: HookEvent,
    session_id: &str,
) -> Vec<String> {
    run_session_hooks_in(hooks, event, session_id, None).await
}

async fn run_one(h: &HookConfig, payload: &str) -> Result<(i32, String, String), String> {
    let (sh, sh_flag) = hook_shell();
    let mut cmd = tokio::process::Command::new(sh);
    cmd.arg(sh_flag).arg(&h.command);
    if h.cc_compat {
        // Claude-Code scripts expect to start in the project and find it in CLAUDE_PROJECT_DIR.
        let project = serde_json::from_str::<serde_json::Value>(payload)
            .ok()
            .and_then(|v| v.get("cwd").and_then(|c| c.as_str()).map(String::from))
            .filter(|dir| std::path::Path::new(dir).is_dir());
        if let Some(dir) = project {
            cmd.env("CLAUDE_PROJECT_DIR", &dir).current_dir(dir);
        }
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true) // a timeout drops the future → the child is killed, not orphaned
        .spawn()
        .map_err(|e| e.to_string())?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes()).await;
        // Dropping `stdin` here sends EOF so a hook that reads to end returns.
    }

    let out = match tokio::time::timeout(
        Duration::from_secs(h.timeout_secs),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => return Err(format!("timed out after {}s", h.timeout_secs)),
    };

    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(event: HookEvent, command: &str) -> HookConfig {
        HookConfig {
            event,
            matcher: None,
            command: command.into(),
            timeout_secs: 10,
            cc_compat: false,
        }
    }

    fn cc_hook(event: HookEvent, command: &str) -> HookConfig {
        HookConfig {
            event,
            matcher: None,
            command: command.into(),
            timeout_secs: 10,
            cc_compat: true,
        }
    }

    #[test]
    fn cc_payload_prefers_explicit_workspace_cwd() {
        let payload = to_cc_payload(
            r#"{"tool":"shell","args":{},"cwd":"/workspace-b"}"#,
            HookEvent::PreToolUse,
            "session-b",
        );
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["cwd"], "/workspace-b");
        assert_eq!(parsed["session_id"], "session-b");
    }

    #[tokio::test]
    async fn pretooluse_nonzero_exit_blocks_with_stderr_reason() {
        #[cfg(not(windows))]
        let cmd = "echo nope 1>&2; exit 1";
        #[cfg(windows)]
        let cmd = "echo nope 1>&2 & exit /b 1";
        let hooks = vec![hook(HookEvent::PreToolUse, cmd)];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        assert_eq!(o.blocked.as_deref(), Some("nope"));
    }

    #[tokio::test]
    async fn pretooluse_zero_exit_does_not_block_and_stdout_is_a_note() {
        let hooks = vec![hook(HookEvent::PreToolUse, "echo looks-good")];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        assert!(o.blocked.is_none());
        assert!(o.notes.iter().any(|n| n.contains("looks-good")));
    }

    #[tokio::test]
    async fn hook_receives_payload_on_stdin() {
        // The hook echoes back stdin; we assert the payload round-trips.
        let hooks = vec![hook(HookEvent::PostToolUse, "cat")];
        let o = run_hooks(
            &hooks,
            HookEvent::PostToolUse,
            "shell",
            "{\"tool\":\"shell\"}",
        )
        .await;
        assert!(o.notes.iter().any(|n| n.contains("\"tool\":\"shell\"")));
    }

    #[tokio::test]
    async fn matcher_skips_non_matching_tools() {
        let mut h = hook(HookEvent::PreToolUse, "exit 1");
        h.matcher = Some("edit_file".into());
        // Tool is "shell", hook matches only "edit_file" → not run → not blocked.
        let o = run_hooks(&[h], HookEvent::PreToolUse, "shell", "{}").await;
        assert!(o.blocked.is_none());
    }

    #[tokio::test]
    async fn a_wedged_hook_times_out_instead_of_hanging() {
        let mut h = hook(HookEvent::PreToolUse, "sleep 30");
        h.timeout_secs = 1;
        let o = run_hooks(&[h], HookEvent::PreToolUse, "shell", "{}").await;
        // Timeout is a launch error (noted), not a block.
        assert!(o.blocked.is_none());
        assert!(o.notes.iter().any(|n| n.contains("timed out")));
    }

    #[tokio::test]
    async fn prompt_hook_exit_zero_passthrough_when_no_stdout() {
        let hooks = vec![hook(HookEvent::UserPromptSubmit, "true")];
        let result = run_prompt_hooks(&hooks, "hello world").await;
        assert_eq!(result.unwrap(), "hello world");
    }

    #[tokio::test]
    async fn prompt_hook_exit_zero_with_stdout_rewrites_prompt() {
        let hooks = vec![hook(HookEvent::UserPromptSubmit, "echo rewritten")];
        let result = run_prompt_hooks(&hooks, "original").await;
        assert_eq!(result.unwrap(), "rewritten");
    }

    #[tokio::test]
    async fn prompt_hook_nonzero_exit_blocks_turn() {
        #[cfg(not(windows))]
        let cmd = "echo blocked reason 1>&2; exit 1";
        #[cfg(windows)]
        let cmd = "echo blocked reason 1>&2 & exit /b 1";
        let hooks = vec![hook(HookEvent::UserPromptSubmit, cmd)];
        let result = run_prompt_hooks(&hooks, "hello").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("blocked reason"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn prompt_and_session_hooks_preserve_explicit_workspace_cwd() {
        let base =
            std::env::temp_dir().join(format!("forge-prompt-hook-cwd-{}", std::process::id()));
        let workspace = base.join("workspace");
        let sentinel = base.join("sentinel");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&sentinel).unwrap();
        let _cwd_guard = crate::test_cwd_guard(&sentinel);

        for (cc_compat, capture) in [
            (false, base.join("native-prompt.json")),
            (true, base.join("cc-prompt.json")),
        ] {
            let mut prompt_hook = hook(
                HookEvent::UserPromptSubmit,
                &format!("cat > {}", capture.display()),
            );
            prompt_hook.cc_compat = cc_compat;
            let _ = run_prompt_hooks_in(&[prompt_hook], "hello", Some(&workspace))
                .await
                .unwrap();
            let payload: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&capture).unwrap()).unwrap();
            assert_eq!(payload["cwd"], workspace.to_string_lossy().as_ref());
        }

        for event in [HookEvent::SessionStart, HookEvent::SessionEnd] {
            for (cc_compat, capture) in [
                (false, base.join(format!("native-{event:?}.json"))),
                (true, base.join(format!("cc-{event:?}.json"))),
            ] {
                let mut session_hook = hook(event, &format!("cat > {}", capture.display()));
                session_hook.cc_compat = cc_compat;
                run_session_hooks_in(&[session_hook], event, "session-b", Some(&workspace)).await;
                let payload: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&capture).unwrap()).unwrap();
                assert_eq!(payload["cwd"], workspace.to_string_lossy().as_ref());
            }
        }

        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn session_hooks_observe_only_do_not_panic() {
        let hooks = vec![hook(HookEvent::SessionStart, "true")];
        run_session_hooks(&hooks, HookEvent::SessionStart, "test-session-id").await;
        // No assertion needed — observe-only hooks must not panic or hang.
    }

    // Windows cmd.exe mangles double-quoted JSON in `echo` output; the JSON-detection
    // logic is pure Rust and is exercised on Linux + macOS.
    #[cfg(not(windows))]
    #[tokio::test]
    async fn pretooluse_exit_zero_json_object_stdout_rewrites_args() {
        let hooks = vec![hook(
            HookEvent::PreToolUse,
            "echo '{\"path\":\"rewritten.rs\"}'",
        )];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        assert!(o.blocked.is_none());
        assert!(o.notes.is_empty(), "json stdout should not become a note");
        let rewritten = o.rewritten_args.expect("should have rewritten args");
        assert_eq!(rewritten["path"], "rewritten.rs");
    }

    #[tokio::test]
    async fn pretooluse_exit_zero_plain_text_stdout_is_a_note_not_rewrite() {
        let hooks = vec![hook(HookEvent::PreToolUse, "echo 'just a message'")];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        assert!(o.blocked.is_none());
        assert!(o.rewritten_args.is_none());
        assert!(o.notes.iter().any(|n| n.contains("just a message")));
    }

    #[tokio::test]
    async fn prompt_hooks_not_fired_for_tool_events() {
        // A pre_tool_use hook must not fire when run_prompt_hooks is called.
        let hooks = vec![hook(HookEvent::PreToolUse, "exit 1")];
        let result = run_prompt_hooks(&hooks, "hello").await;
        assert_eq!(result.unwrap(), "hello"); // no hook matched → prompt unchanged
    }

    // --- Structured directive protocol (completes the hooks system: rewrite / inject / block) ---

    // Windows cmd.exe preserves the single quotes in `echo '{…}'`, so the JSON directive can't be
    // emitted from a hook this way; the directive parsing is pure Rust, exercised on Linux + macOS.
    #[cfg(not(windows))]
    #[tokio::test]
    async fn inject_action_queues_context_not_a_note() {
        let hooks = vec![hook(
            HookEvent::PreToolUse,
            "echo '{\"action\":\"inject\",\"context\":\"this file is auto-generated\"}'",
        )];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        assert!(o.blocked.is_none());
        assert!(o.rewritten_args.is_none());
        assert!(o.notes.is_empty(), "an inject directive is not a user note");
        assert_eq!(o.injected_context, vec!["this file is auto-generated"]);
    }

    #[cfg(not(windows))] // Windows cmd echo keeps the single quotes around JSON (see note above).
    #[tokio::test]
    async fn inject_action_works_on_posttooluse_too() {
        let hooks = vec![hook(
            HookEvent::PostToolUse,
            "echo '{\"action\":\"inject\",\"context\":\"lint: 2 warnings\"}'",
        )];
        let o = run_hooks(&hooks, HookEvent::PostToolUse, "shell", "{}").await;
        assert_eq!(o.injected_context, vec!["lint: 2 warnings"]);
        assert!(o.notes.is_empty());
    }

    #[cfg(not(windows))] // Windows cmd echo keeps the single quotes around JSON (see note above).
    #[tokio::test]
    async fn rewrite_action_replaces_args() {
        let hooks = vec![hook(
            HookEvent::PreToolUse,
            "echo '{\"action\":\"rewrite\",\"args\":{\"path\":\"safe.rs\"}}'",
        )];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        let rewritten = o.rewritten_args.expect("rewrite action sets args");
        assert_eq!(rewritten["path"], "safe.rs");
        assert!(o.injected_context.is_empty());
    }

    #[cfg(not(windows))] // Windows cmd echo keeps the single quotes around JSON (see note above).
    #[tokio::test]
    async fn block_action_blocks_pretooluse_with_reason() {
        let hooks = vec![hook(
            HookEvent::PreToolUse,
            "echo '{\"action\":\"block\",\"reason\":\"writes outside the project are denied\"}'",
        )];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        assert_eq!(
            o.blocked.as_deref(),
            Some("writes outside the project are denied")
        );
    }

    #[tokio::test]
    async fn block_action_downgrades_to_note_on_posttooluse() {
        // PostToolUse can't unwind a finished call, so a block directive becomes a note.
        let hooks = vec![hook(
            HookEvent::PostToolUse,
            "echo '{\"action\":\"block\",\"reason\":\"too late\"}'",
        )];
        let o = run_hooks(&hooks, HookEvent::PostToolUse, "shell", "{}").await;
        assert!(o.blocked.is_none());
        assert!(o.notes.iter().any(|n| n.contains("too late")));
    }

    #[cfg(not(windows))] // Windows cmd echo keeps the single quotes around JSON (see note above).
    #[tokio::test]
    async fn allow_action_is_a_clean_noop() {
        let hooks = vec![hook(HookEvent::PreToolUse, "echo '{\"action\":\"allow\"}'")];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        assert!(o.blocked.is_none());
        assert!(o.rewritten_args.is_none());
        assert!(o.injected_context.is_empty());
        assert!(o.notes.is_empty(), "allow approves without any side effect");
    }

    #[tokio::test]
    async fn unknown_action_falls_back_to_a_note() {
        let hooks = vec![hook(
            HookEvent::PreToolUse,
            "echo '{\"action\":\"frobnicate\"}'",
        )];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        // Not a recognised directive AND has an `action` key → surfaced as a note, NOT rewritten args.
        assert!(o.rewritten_args.is_none());
        assert!(o.notes.iter().any(|n| n.contains("frobnicate")));
    }

    // --- Claude-Code-compatible hook mode (run unmodified CC hook scripts) ---

    #[tokio::test]
    async fn cc_pretooluse_exit_2_blocks_with_stderr_reason() {
        // CC protocol: exit code 2 = block, stderr fed back as the reason. Cross-platform.
        #[cfg(not(windows))]
        let cmd = "echo cc-denied 1>&2; exit 2";
        #[cfg(windows)]
        let cmd = "echo cc-denied 1>&2 & exit /b 2";
        let hooks = vec![cc_hook(HookEvent::PreToolUse, cmd)];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        assert_eq!(o.blocked.as_deref(), Some("cc-denied"));
    }

    #[tokio::test]
    async fn cc_pretooluse_nonblocking_exit_1_does_not_block() {
        // A non-2 non-zero exit is a non-blocking error under CC semantics (unlike native hooks,
        // where any non-zero PreToolUse exit blocks).
        #[cfg(not(windows))]
        let cmd = "echo oops 1>&2; exit 1";
        #[cfg(windows)]
        let cmd = "echo oops 1>&2 & exit /b 1";
        let hooks = vec![cc_hook(HookEvent::PreToolUse, cmd)];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        assert!(o.blocked.is_none(), "exit 1 is non-blocking in CC mode");
    }

    // The CC JSON-decision scripts echo a single-quoted JSON object, which Windows cmd.exe mangles;
    // `parse_cc_output` is pure Rust and is exercised on Linux + macOS.
    #[cfg(not(windows))]
    #[tokio::test]
    async fn cc_pretooluse_decision_block_is_honored() {
        let hooks = vec![cc_hook(
            HookEvent::PreToolUse,
            "echo '{\"decision\":\"block\",\"reason\":\"policy: no writes to /etc\"}'",
        )];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        assert_eq!(o.blocked.as_deref(), Some("policy: no writes to /etc"));
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn cc_posttooluse_additional_context_is_injected() {
        let hooks = vec![cc_hook(
            HookEvent::PostToolUse,
            "echo '{\"hookSpecificOutput\":{\"additionalContext\":\"ran clippy: 0 warnings\"}}'",
        )];
        let o = run_hooks(&hooks, HookEvent::PostToolUse, "shell", "{}").await;
        assert_eq!(o.injected_context, vec!["ran clippy: 0 warnings"]);
        assert!(o.notes.is_empty(), "additionalContext is not a user note");
    }

    #[tokio::test]
    async fn cc_hook_receives_cc_shaped_payload_on_stdin() {
        // The hook echoes its stdin back; assert the CC fields are present (translated from Forge's
        // native {tool, args} payload). `cat` is available on the project's CI on every OS.
        let hooks = vec![cc_hook(HookEvent::PreToolUse, "cat")];
        let o = run_hooks(
            &hooks,
            HookEvent::PreToolUse,
            "shell",
            "{\"tool\":\"shell\",\"args\":{\"command\":\"ls\"}}",
        )
        .await;
        let joined = o.notes.join(" ");
        assert!(
            joined.contains("\"hook_event_name\":\"PreToolUse\""),
            "{joined}"
        );
        assert!(joined.contains("\"tool_name\":\"Bash\""), "{joined}");
        assert!(joined.contains("\"tool_input\""), "{joined}");
    }

    #[tokio::test]
    async fn cc_matcher_uses_cc_tool_alias() {
        // A CC matcher written against CC tool names ("Write|Edit") fires on Forge's edit tool.
        let mut h = cc_hook(HookEvent::PreToolUse, "exit 2");
        h.matcher = Some("Write|Edit".into());
        let o = run_hooks(&[h.clone()], HookEvent::PreToolUse, "edit_file", "{}").await;
        assert!(o.blocked.is_some(), "Edit alias should match edit_file");
        let o2 = run_hooks(&[h], HookEvent::PreToolUse, "shell", "{}").await;
        assert!(o2.blocked.is_none(), "Bash is not in the matcher");
    }

    // --- New lifecycle events (notification / pre_compact / post_compact / stop / subagent_stop) ---

    #[tokio::test]
    async fn each_new_lifecycle_event_fires_its_hook() {
        for event in [
            HookEvent::Notification,
            HookEvent::PreCompact,
            HookEvent::PostCompact,
            HookEvent::Stop,
            HookEvent::SubagentStop,
        ] {
            // `cat` echoes the payload so we can prove the hook actually ran for THIS event.
            let hooks = vec![hook(event, "cat")];
            let o = run_lifecycle_hooks(&hooks, event, "sess-1", serde_json::json!({})).await;
            assert!(
                o.notes.iter().any(|n| n.contains(event.cc_name())),
                "{:?} hook must fire and echo its event; notes: {:?}",
                event,
                o.notes
            );
        }
    }

    #[tokio::test]
    async fn lifecycle_hook_only_fires_for_its_event() {
        // A Stop hook must not fire when a Notification event is dispatched.
        let hooks = vec![hook(HookEvent::Stop, "cat")];
        let o = run_lifecycle_hooks(
            &hooks,
            HookEvent::Notification,
            "sess-1",
            serde_json::json!({ "message": "hi" }),
        )
        .await;
        assert!(
            o.notes.is_empty(),
            "no Stop hook should run for Notification"
        );
    }

    #[tokio::test]
    async fn lifecycle_cc_hook_exit_2_reports_block() {
        #[cfg(not(windows))]
        let cmd = "echo stay 1>&2; exit 2";
        #[cfg(windows)]
        let cmd = "echo stay 1>&2 & exit /b 2";
        let hooks = vec![cc_hook(HookEvent::Stop, cmd)];
        let o = run_lifecycle_hooks(&hooks, HookEvent::Stop, "sess-1", serde_json::json!({})).await;
        assert_eq!(o.blocked.as_deref(), Some("stay"));
    }

    // --- Claude-Code parity: updatedInput, SessionStart / UserPromptSubmit context ---

    #[cfg(not(windows))]
    #[tokio::test]
    async fn cc_updated_input_becomes_rewritten_args_in_forge_names() {
        let hooks = vec![cc_hook(
            HookEvent::PreToolUse,
            "echo '{\"hookSpecificOutput\":{\"updatedInput\":{\"file_path\":\"b.rs\",\"old_string\":\"x\",\"new_string\":\"y\"}}}'",
        )];
        let o = run_hooks(&hooks, HookEvent::PreToolUse, "edit_file", "{}").await;
        assert_eq!(
            o.rewritten_args.unwrap(),
            serde_json::json!({"path":"b.rs","old":"x","new":"y"})
        );
        assert!(o.notes.is_empty());
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn cc_updated_input_is_ignored_outside_pretooluse() {
        let hooks = vec![cc_hook(
            HookEvent::PostToolUse,
            "echo '{\"hookSpecificOutput\":{\"updatedInput\":{\"command\":\"x\"}}}'",
        )];
        let o = run_hooks(&hooks, HookEvent::PostToolUse, "shell", "{}").await;
        assert!(o.rewritten_args.is_none());
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn cc_session_start_stdout_and_additional_context_are_returned_as_context() {
        let plain = cc_hook(HookEvent::SessionStart, "echo 'be terse'");
        let json = cc_hook(
            HookEvent::SessionStart,
            "echo '{\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\"additionalContext\":\"branch: main\"}}'",
        );
        let native = hook(HookEvent::SessionStart, "echo 'native stays a note'");
        let got =
            run_session_hooks_in(&[plain, json, native], HookEvent::SessionStart, "sid", None)
                .await;
        assert_eq!(got, vec!["be terse", "branch: main"]);
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn cc_session_hook_gets_the_cc_payload_and_project_dir() {
        let dir = std::env::temp_dir().join(format!("forge-sess-cc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let h = cc_hook(
            HookEvent::SessionStart,
            "printf '%s|%s' \"$CLAUDE_PROJECT_DIR\" \"$(cat)\"",
        );
        let got = run_session_hooks_in(&[h], HookEvent::SessionStart, "sid-9", Some(&dir)).await;
        let text = &got[0];
        assert!(text.starts_with(&dir.display().to_string()), "{text}");
        assert!(
            text.contains("\"hook_event_name\":\"SessionStart\""),
            "{text}"
        );
        assert!(text.contains("\"session_id\":\"sid-9\""), "{text}");
        assert!(text.contains("\"source\":\"startup\""), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn cc_user_prompt_additional_context_is_appended_not_replacing() {
        let hooks = vec![cc_hook(
            HookEvent::UserPromptSubmit,
            "echo '{\"hookSpecificOutput\":{\"additionalContext\":\"extra\"}}'",
        )];
        let got = run_prompt_hooks(&hooks, "hello").await.unwrap();
        assert_eq!(got, "hello\n\nextra");
    }

    #[tokio::test]
    async fn stop_additional_context_counts_as_a_continuation_and_continue_false_halts() {
        #[cfg(not(windows))]
        {
            let hooks = vec![cc_hook(
                HookEvent::Stop,
                "echo '{\"hookSpecificOutput\":{\"additionalContext\":\"keep going\"}}'",
            )];
            let o = run_lifecycle_hooks(&hooks, HookEvent::Stop, "s", serde_json::json!({})).await;
            assert_eq!(o.blocked.as_deref(), Some("keep going"));
            let hooks = vec![cc_hook(HookEvent::Stop, "echo '{\"continue\":false}'")];
            let o = run_lifecycle_hooks(&hooks, HookEvent::Stop, "s", serde_json::json!({})).await;
            assert!(o.halt && o.blocked.is_none());
            // Non-stop events never turn context into a block.
            let hooks = vec![cc_hook(
                HookEvent::Notification,
                "echo '{\"additionalContext\":\"fyi\"}'",
            )];
            let o =
                run_lifecycle_hooks(&hooks, HookEvent::Notification, "s", serde_json::json!({}))
                    .await;
            assert!(o.blocked.is_none());
        }
    }
}
