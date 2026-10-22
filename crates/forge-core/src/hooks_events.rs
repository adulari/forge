//! One runner for the Claude-Code events that are neither tool calls nor the original lifecycle
//! set: `PostToolBatch`, `UserPromptExpansion`, `InstructionsLoaded`, `StopFailure`,
//! `SubagentStart`, `CwdChanged`, `WorktreeCreate`, `WorktreeRemove`, `PreModelSwitch` and
//! `PostModelSwitch`. It runs the matching hooks concurrently and reports everything they said;
//! each seam decides what that means for its event (see docs/features/hooks.md).

use forge_config::{HookConfig, HookEvent};
use forge_types::truncate_ellipsis as truncate;
use serde_json::{Map, Value};

use crate::hooks_cc::{parse_cc_output, to_cc_payload};
use crate::hooks_exec::run_all;
use crate::hooks_match::{event_value_matches, model_matches, runs_off_tool};

/// What a hook matcher is tested against for this firing.
#[derive(Debug, Clone, Copy)]
pub enum EventMatch<'a> {
    /// The event has no matcher support: every hook for it runs.
    Any,
    /// An event value (`load_reason`, `error`, `agent_type`, a command name …).
    Value(&'a str),
    /// A model id; a matcher may name it with or without the `provider::` prefix.
    Model(&'a str),
}

/// Everything the hooks for one event said, in declaration order.
#[derive(Debug, Default)]
pub struct EventOutcome {
    /// How many hooks ran.
    pub ran: usize,
    /// Exit 2, `decision: "block"` or `permissionDecision: "deny"`: the first reason.
    pub blocked: Option<String>,
    /// `continue: false`, with its `stopReason` when given.
    pub halt: Option<String>,
    /// `permissionDecision: "ask"` (only `PreModelSwitch` reads it).
    pub ask: Option<String>,
    /// `additionalContext`, plus plain stdout on events whose stdout is context.
    pub context: Vec<String>,
    /// `systemMessage` and plain stdout for the user.
    pub notes: Vec<String>,
    /// Hooks that failed: could not start, timed out, or exited non-zero.
    pub failures: Vec<String>,
    /// A hook timed out.
    pub timed_out: bool,
    /// `WorktreeCreate`: the directory the hook made (its last stdout line, or an HTTP hook's
    /// `hookSpecificOutput.worktreePath`).
    pub worktree_path: Option<String>,
}

/// Events whose plain stdout (exit 0) is context for the model, as in Claude Code.
fn stdout_is_context(event: HookEvent) -> bool {
    matches!(event, HookEvent::PostModelSwitch)
}

/// Run the hooks for `event`. `fields` join the payload (`{session_id, event, …fields}` natively;
/// the Claude-Code shape for a CC-compat hook). A hook carrying `if` never runs here.
pub async fn run_event_hooks(
    hooks: &[HookConfig],
    event: HookEvent,
    session_id: &str,
    matcher: EventMatch<'_>,
    fields: Value,
) -> EventOutcome {
    let mut outcome = EventOutcome::default();
    let matched: Vec<&HookConfig> = hooks
        .iter()
        .filter(|h| h.event == event && runs_off_tool(h))
        .filter(|h| match matcher {
            EventMatch::Any => true,
            EventMatch::Value(value) => event_value_matches(h, value),
            EventMatch::Model(model) => model_matches(h, model),
        })
        .collect();
    if matched.is_empty() {
        return outcome;
    }
    let mut base = Map::new();
    base.insert("session_id".into(), session_id.into());
    base.insert("event".into(), event.cc_name().into());
    if let Value::Object(fields) = fields {
        base.extend(fields);
    }
    let native = Value::Object(base).to_string();
    let jobs = matched
        .into_iter()
        .map(|h| {
            let sent = if h.cc_compat {
                to_cc_payload(&native, event, session_id)
            } else {
                native.clone()
            };
            (h, sent)
        })
        .collect();
    for (h, run) in run_all(jobs).await {
        outcome.ran += 1;
        let (code, stdout, stderr) = match run {
            Ok(result) => result,
            Err(e) => {
                outcome.timed_out |= e.starts_with("timed out");
                outcome.failures.push(e);
                continue;
            }
        };
        if code != 0 && code != 2 {
            let err = stderr.trim();
            outcome.failures.push(if err.is_empty() {
                format!("`{}` exited {code}", h.command)
            } else {
                truncate(err, 800)
            });
        }
        if event == HookEvent::WorktreeCreate && code == 0 && outcome.worktree_path.is_none() {
            outcome.worktree_path = last_line(&stdout);
        }
        let out = parse_cc_output(code, &stdout, &stderr);
        if let Some(reason) = out.block {
            outcome.blocked.get_or_insert(reason);
        }
        if out.halt {
            outcome.halt.get_or_insert(
                out.stop_reason
                    .unwrap_or_else(|| "a hook asked to stop".into()),
            );
        }
        if let Some(ask) = out.ask {
            outcome.ask.get_or_insert(ask);
        }
        if let Some(path) = out.worktree_path {
            outcome.worktree_path = Some(path);
        }
        outcome.context.extend(out.context);
        if let Some(text) = out.note {
            let plain = serde_json::from_str::<Value>(stdout.trim()).is_err();
            if plain && stdout_is_context(event) {
                outcome.context.push(text);
            } else if event != HookEvent::WorktreeCreate || !plain {
                outcome.notes.push(text);
            }
        }
    }
    outcome
}

/// The last non-empty stdout line with ANSI escapes removed: where a `WorktreeCreate` command hook
/// prints the directory it made (a shell banner before it is ignored).
fn last_line(stdout: &str) -> Option<String> {
    let ansi = regex::Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").ok()?;
    stdout
        .lines()
        .map(|line| ansi.replace_all(line, "").trim().to_string())
        .rfind(|line| !line.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc(event: HookEvent, command: &str) -> HookConfig {
        HookConfig {
            cc_compat: true,
            timeout_secs: 10,
            ..HookConfig::new(event, command)
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn matcher_values_select_hooks_and_outputs_are_collected() {
        let hooks = vec![
            HookConfig {
                matcher: Some("rate_limit".into()),
                ..cc(HookEvent::StopFailure, "cat")
            },
            HookConfig {
                matcher: Some("billing_error".into()),
                ..cc(HookEvent::StopFailure, "echo never")
            },
        ];
        let out = run_event_hooks(
            &hooks,
            HookEvent::StopFailure,
            "s1",
            EventMatch::Value("rate_limit"),
            serde_json::json!({"error": "rate_limit", "cwd": "/"}),
        )
        .await;
        assert_eq!(out.ran, 1);
        let echoed = out.notes.join(" ");
        assert!(
            echoed.contains("\"hook_event_name\":\"StopFailure\""),
            "{echoed}"
        );
        assert!(echoed.contains("\"error\":\"rate_limit\""), "{echoed}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worktree_path_is_the_last_stdout_line_and_post_switch_stdout_is_context() {
        let create = cc(
            HookEvent::WorktreeCreate,
            r"echo banner; printf '\033[32m/tmp/wt-x\033[0m\n'",
        );
        let out = run_event_hooks(
            &[create],
            HookEvent::WorktreeCreate,
            "s",
            EventMatch::Any,
            serde_json::json!({"name": "x", "cwd": "/"}),
        )
        .await;
        assert_eq!(out.worktree_path.as_deref(), Some("/tmp/wt-x"));
        assert!(out.notes.is_empty());

        let post = cc(HookEvent::PostModelSwitch, "echo 'delegate more on opus'");
        let out = run_event_hooks(
            &[post],
            HookEvent::PostModelSwitch,
            "s",
            EventMatch::Model("claude-cli::claude-opus-5-5"),
            serde_json::json!({"cwd": "/"}),
        )
        .await;
        assert_eq!(out.context, vec!["delegate more on opus"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn blocks_asks_and_failures_are_reported() {
        let hooks = vec![
            cc(HookEvent::PreModelSwitch, "echo retired >&2; exit 2"),
            cc(
                HookEvent::PreModelSwitch,
                r#"echo '{"hookSpecificOutput":{"permissionDecision":"ask","permissionDecisionReason":"re-sends 180k tokens"}}'"#,
            ),
            cc(HookEvent::PreModelSwitch, "exit 3"),
        ];
        let out = run_event_hooks(
            &hooks,
            HookEvent::PreModelSwitch,
            "s",
            EventMatch::Model("x::y"),
            serde_json::json!({"cwd": "/"}),
        )
        .await;
        assert_eq!(out.blocked.as_deref(), Some("retired"));
        assert_eq!(out.ask.as_deref(), Some("re-sends 180k tokens"));
        assert_eq!(out.failures.len(), 1);
    }
}
