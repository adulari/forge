//! Running hook handlers: a shell command, an HTTP endpoint or a model, each reported the way a
//! command hook would be (`exit code, stdout, stderr`). The hooks matched by one event run
//! concurrently, as in Claude Code; callers fold the results in declaration order, so the outcome
//! does not depend on which hook finished first.

use std::process::Stdio;
use std::time::Duration;

use forge_config::{HookConfig, HookEvent, HookKind};
use tokio::io::AsyncWriteExt;

/// `(exit code, stdout, stderr)`, or why the hook could not run (launch failure, timeout, an
/// unusable HTTP response). A failure to run never blocks anything.
pub(crate) type HookRun = Result<(i32, String, String), String>;

/// Model hooks are the expensive ones; Claude Code runs at most 12 at once.
const MAX_CONCURRENT_HOOKS: usize = 12;

/// Run every `(hook, payload)` job concurrently and return the results in job order.
pub(crate) async fn run_all(jobs: Vec<(&HookConfig, String)>) -> Vec<(&HookConfig, HookRun)> {
    let mut results = Vec::with_capacity(jobs.len());
    let mut jobs = jobs.into_iter().peekable();
    // A plain loop over named futures, not a stream of closures: closure-built futures trip
    // rustc's higher-ranked `Send` check in the spawned subagent and turn futures that await this.
    while jobs.peek().is_some() {
        let mut wave = Vec::new();
        for (hook, payload) in jobs.by_ref().take(MAX_CONCURRENT_HOOKS) {
            wave.push(run_job(hook, payload));
        }
        results.extend(futures::future::join_all(wave).await);
    }
    results
}

async fn run_job(hook: &HookConfig, payload: String) -> (&HookConfig, HookRun) {
    (hook, run_one(hook, &payload).await)
}

/// Run one hook and report it the way a command hook would: `(exit code, stdout, stderr)`. A
/// `prompt` / `agent` hook's `{"ok": false, "reason"}` becomes exit 2 with `reason` on stderr, so
/// every event interprets it exactly like a blocking command hook; `ok: true` is a clean exit
/// (and, for `PermissionRequest`, an explicit allow). An `async` command hook is started in the
/// background and reported as a clean, silent exit.
pub(crate) async fn run_one(h: &HookConfig, payload: &str) -> HookRun {
    match h.kind {
        HookKind::Command if h.run_async => {
            crate::hooks_async::spawn(h.clone(), payload.to_string());
            return Ok((0, String::new(), String::new()));
        }
        HookKind::Command => return run_command(h, payload).await,
        HookKind::Http => return crate::hooks_http::run_http(h, payload).await,
        HookKind::Prompt | HookKind::Agent => {}
    }
    let runner = crate::hooks_llm::current_runner()
        .ok_or("prompt/agent hook skipped: no model is available on this surface")?;
    let input = serde_json::from_str::<serde_json::Value>(payload)
        .unwrap_or_else(|_| serde_json::Value::String(payload.to_string()));
    let verdict = tokio::time::timeout(
        Duration::from_secs(h.timeout_secs),
        runner.evaluate(h, &input),
    )
    .await
    .map_err(|_| format!("timed out after {}s", h.timeout_secs))??;
    Ok(if verdict.ok {
        let stdout = if h.event == HookEvent::PermissionRequest {
            serde_json::json!({"hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {"behavior": "allow"}
            }})
            .to_string()
        } else {
            String::new()
        };
        (0, stdout, String::new())
    } else {
        let reason = verdict
            .reason
            .unwrap_or_else(|| format!("blocked by {} hook", hook_kind_label(h)));
        (2, String::new(), reason)
    })
}

fn hook_kind_label(h: &HookConfig) -> &'static str {
    match h.kind {
        HookKind::Command => "command",
        HookKind::Http => "http",
        HookKind::Prompt => "prompt",
        HookKind::Agent => "agent",
    }
}

fn hook_shell() -> (&'static str, &'static str) {
    #[cfg(windows)]
    return ("cmd", "/C");
    #[cfg(not(windows))]
    ("sh", "-c")
}

/// The project directory a Claude-Code hook starts in (and finds in `CLAUDE_PROJECT_DIR`): the
/// payload's `cwd` when it names a directory.
pub(crate) fn payload_project_dir(payload: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("cwd").and_then(|c| c.as_str()).map(String::from))
        .filter(|dir| std::path::Path::new(dir).is_dir())
}

pub(crate) async fn run_command(h: &HookConfig, payload: &str) -> HookRun {
    let (sh, sh_flag) = hook_shell();
    let mut cmd = tokio::process::Command::new(sh);
    cmd.arg(sh_flag).arg(&h.command);
    if h.cc_compat {
        if let Some(dir) = payload_project_dir(payload) {
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

    #[cfg(unix)]
    #[tokio::test]
    async fn matched_hooks_run_concurrently_and_report_in_declaration_order() {
        let slow = HookConfig::new(HookEvent::PostToolUse, "sleep 1; echo first");
        let fast = HookConfig::new(HookEvent::PostToolUse, "echo second");
        let started = std::time::Instant::now();
        let runs = run_all(vec![
            (&slow, String::new()),
            (&slow, String::new()),
            (&fast, String::new()),
        ])
        .await;
        assert!(
            started.elapsed() < Duration::from_millis(1900),
            "two 1 s hooks ran one after the other: {:?}",
            started.elapsed()
        );
        let outs: Vec<String> = runs
            .into_iter()
            .map(|(_, r)| r.unwrap().1.trim().to_string())
            .collect();
        assert_eq!(outs, vec!["first", "first", "second"]);
    }
}
