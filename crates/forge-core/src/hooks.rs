//! Shell hooks (docs/features/hooks.md). Each `[[hooks]]` entry runs via the OS shell (or as an
//! HTTP call or a model judgement, see `hooks_exec.rs`) around tool calls and session lifecycle
//! events. A `PreToolUse` hook that exits non-zero **blocks** the tool; a `UserPromptSubmit` hook
//! can rewrite the user's prompt (stdout replaces it on exit 0) or block the turn (non-zero).
//! `SessionStart` output can be injected as model context (CC-compat hooks); `SessionEnd` observes
//! only. The hooks one event matches run concurrently and are folded in declaration order. Hooks
//! are time-bounded so a wedged hook can't hang the agent.

use forge_config::{HookConfig, HookEvent};
use forge_types::truncate_ellipsis as truncate;
use serde_json::Value;

use crate::hooks_cc::{cc_input_to_forge, parse_cc_output, to_cc_payload};
use crate::hooks_exec::{run_all, HookRun};
use crate::hooks_match::{event_value_matches, if_allows, runs_off_tool};

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
    /// A `PermissionDenied` hook answered `hookSpecificOutput.retry: true`.
    pub retry: bool,
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

/// Whether `h` applies to `tool`. Claude Code treats a matcher as a regex, so a CC-compat hook with
/// regex syntax (`.*`, `Rea.*`, `^Read$`, `mcp__.*`) is searched case-insensitively against the
/// Forge name and its CC alias; every other matcher keeps [`HookConfig::matches`] semantics.
pub(crate) fn hook_applies(h: &HookConfig, tool: &str) -> bool {
    match h.matcher.as_deref() {
        Some(pattern) if h.cc_compat && forge_config::is_regex_matcher(pattern) => {
            let alias = forge_config::cc_tool_alias(tool);
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .is_ok_and(|re| re.is_match(tool) || re.is_match(alias))
        }
        _ => h.matches(tool),
    }
}

/// The tool-event hooks that run for this call: event, tool-name matcher and `if` rule all agree.
fn tool_event_hooks<'a>(
    hooks: &'a [HookConfig],
    event: HookEvent,
    tool: &str,
    payload: &str,
) -> Vec<&'a HookConfig> {
    let parsed: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    let args = parsed.get("args").cloned().unwrap_or(Value::Null);
    let cwd = parsed.get("cwd").and_then(Value::as_str);
    hooks
        .iter()
        .filter(|h| h.event == event && hook_applies(h, tool))
        .filter(|h| if_allows(h, event, tool, &args, cwd))
        .collect()
}

/// The payload each hook reads: the Claude-Code shape for a CC-compat hook, Forge's own otherwise.
fn jobs_for<'a>(
    hooks: Vec<&'a HookConfig>,
    payload: &str,
    event: HookEvent,
    session_id: &str,
) -> Vec<(&'a HookConfig, String)> {
    hooks
        .into_iter()
        .map(|h| {
            let sent = if h.cc_compat {
                to_cc_payload(payload, event, session_id)
            } else {
                payload.to_string()
            };
            (h, sent)
        })
        .collect()
}

/// Fold one CC-compat hook's result into `outcome`. Returns `true` if the call should be blocked
/// (PreToolUse only; the after-call events turn a block into feedback for the model).
/// `updatedInput` replaces the tool's args (PreToolUse), translated from Claude-Code names.
fn fold_cc(event: HookEvent, tool: &str, run: HookRun, outcome: &mut HookOutcome) -> bool {
    let (code, stdout, stderr) = match run {
        Ok(result) => result,
        Err(e) => {
            outcome.notes.push(format!("⎇ hook error: {e}"));
            return false;
        }
    };
    let out = parse_cc_output(code, &stdout, &stderr);
    if let Some(reason) = out.block {
        if event == HookEvent::PreToolUse {
            outcome.blocked = Some(reason);
            return true;
        }
        // Claude Code shows an after-call block reason to the model (the tool already ran).
        if after_call(event) {
            outcome.injected_context.push(reason.clone());
        }
        outcome.notes.push(format!("⎇ hook: {reason}"));
    }
    if event == HookEvent::PreToolUse {
        if let Some(input) = out.updated_input {
            outcome.rewritten_args = Some(cc_input_to_forge(tool, &input));
        }
    }
    outcome.retry |= out.retry;
    outcome.injected_context.extend(out.context);
    if let Some(text) = out.note {
        outcome.notes.push(format!("⎇ hook: {text}"));
    }
    false
}

fn after_call(event: HookEvent) -> bool {
    matches!(
        event,
        HookEvent::PostToolUse | HookEvent::PostToolUseFailure
    )
}

/// Fold one native hook's result into `outcome`; `true` blocks the call (PreToolUse only).
fn fold_native(event: HookEvent, tool: &str, run: HookRun, outcome: &mut HookOutcome) -> bool {
    let (code, stdout, stderr) = match run {
        Ok(result) => result,
        Err(e) => {
            outcome.notes.push(format!("⎇ hook error: {e}"));
            return false;
        }
    };
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
        return true;
    }
    // Exit 2 after the call (CC semantics): the reason is feedback for the model.
    if after_call(event) && code == 2 && !stderr.trim().is_empty() {
        let reason = truncate(stderr.trim(), 800);
        outcome.notes.push(format!("⎇ hook: {reason}"));
        outcome.injected_context.push(reason);
    }
    // exit 0 + non-empty stdout: interpret the structured directive protocol (rewrite / inject /
    // block / allow), falling back to the legacy "bare object = rewrite" and to a plain note.
    // `block` only blocks on PreToolUse (Post can't unwind a finished call).
    if !trimmed.is_empty() {
        match parse_hook_directive(trimmed, event) {
            HookDirective::Rewrite(args) => outcome.rewritten_args = Some(args),
            HookDirective::Inject(ctx) => outcome.injected_context.push(ctx),
            HookDirective::Block(reason) => {
                if event == HookEvent::PreToolUse {
                    outcome.blocked = Some(truncate(&reason, 800));
                    return true;
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
    false
}

/// Run every hook matching a tool `event` + `tool` (and its `if` rule), concurrently, then fold
/// the results in declaration order: the first `PreToolUse` block wins and later results are
/// ignored. A hook that fails to launch is noted, not fatal. CC-compat hooks
/// ([`HookConfig::cc_compat`]) speak the Claude-Code protocol (CC stdin payload + `decision` /
/// exit-2 output); native hooks keep Forge's directive protocol.
pub async fn run_hooks(
    hooks: &[HookConfig],
    event: HookEvent,
    tool: &str,
    payload: &str,
) -> HookOutcome {
    let mut outcome = HookOutcome::default();
    let matched = tool_event_hooks(hooks, event, tool, payload);
    if matched.is_empty() {
        return outcome;
    }
    for (h, run) in run_all(jobs_for(matched, payload, event, "")).await {
        let blocked = if h.cc_compat {
            fold_cc(event, tool, run, &mut outcome)
        } else {
            fold_native(event, tool, run, &mut outcome)
        };
        if blocked {
            break;
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
    run_prompt_hooks_for(hooks, prompt, cwd, "").await
}

/// [`run_prompt_hooks_in`] that also tells the hook which session the prompt belongs to. The
/// hooks run concurrently; their results are applied in declaration order.
pub async fn run_prompt_hooks_for(
    hooks: &[HookConfig],
    prompt: &str,
    cwd: Option<&std::path::Path>,
    session_id: &str,
) -> Result<String, String> {
    let payload = serde_json::json!({
        "prompt": prompt,
        "cwd": cwd.map(|cwd| cwd.display().to_string()),
        "session_id": (!session_id.is_empty()).then_some(session_id),
    })
    .to_string();
    let matched: Vec<&HookConfig> = hooks
        .iter()
        .filter(|h| h.event == HookEvent::UserPromptSubmit && runs_off_tool(h))
        .collect();
    let mut current = prompt.to_string();
    if matched.is_empty() {
        return Ok(current);
    }
    let jobs = jobs_for(matched, &payload, HookEvent::UserPromptSubmit, session_id);
    for (h, run) in run_all(jobs).await {
        let (code, stdout, stderr) = match run {
            Ok(result) => result,
            Err(e) => {
                // Launch failure is noted but doesn't block the turn.
                eprintln!("⎇ hook error: {e}");
                continue;
            }
        };
        if h.cc_compat {
            // CC UserPromptSubmit: a block decision / exit-2 blocks the turn; otherwise stdout (and
            // `additionalContext`) is APPENDED as extra context (CC semantics — it doesn't replace
            // the prompt the way a native prompt hook does).
            let out = parse_cc_output(code, &stdout, &stderr);
            if let Some(reason) = out.block {
                return Err(reason);
            }
            for ctx in out.context.into_iter().chain(out.note) {
                current = format!("{current}\n\n{ctx}");
            }
            continue;
        }
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
    Ok(current)
}

pub async fn run_prompt_hooks(hooks: &[HookConfig], prompt: &str) -> Result<String, String> {
    run_prompt_hooks_in(hooks, prompt, None).await
}

/// Run session lifecycle hooks (`session_start` / `session_end`). Exit code is advisory.
/// `SessionStart` reports `source: "startup"`; see [`run_session_hooks_with`] for the others.
pub async fn run_session_hooks_in(
    hooks: &[HookConfig],
    event: HookEvent,
    session_id: &str,
    cwd: Option<&std::path::Path>,
) -> Vec<String> {
    let value = if event == HookEvent::SessionStart {
        "startup"
    } else {
        "other"
    };
    run_session_hooks_with(hooks, event, session_id, cwd, value).await
}

/// Run `session_start` / `session_end` hooks. `value` is the start's `source` (`startup`,
/// `resume`, `clear`, `compact`) or the end's `reason` (`clear`, `resume`, `other` …); a CC-compat
/// hook's matcher is tested against it, as in Claude Code.
///
/// Returns the context a `SessionStart` hook asked to put in front of the model, for the caller to
/// inject (`Session::inject_hook_context`). Claude-Code semantics apply to `cc_compat` hooks: plain
/// stdout and `hookSpecificOutput.additionalContext` are context, not a visible note, and the hook
/// gets the CC payload (`hook_event_name`, `source`). Native hooks keep printing stdout to stderr.
pub async fn run_session_hooks_with(
    hooks: &[HookConfig],
    event: HookEvent,
    session_id: &str,
    cwd: Option<&std::path::Path>,
    value: &str,
) -> Vec<String> {
    debug_assert!(
        matches!(event, HookEvent::SessionStart | HookEvent::SessionEnd),
        "run_session_hooks called with non-session event"
    );
    let (event_str, key) = match event {
        HookEvent::SessionStart => ("session_start", "source"),
        HookEvent::SessionEnd => ("session_end", "reason"),
        _ => return Vec::new(),
    };
    let mut payload = serde_json::json!({
        "session_id": session_id,
        "event": event_str,
        "cwd": cwd.map(|cwd| cwd.display().to_string()),
    });
    payload[key] = value.into();
    let payload = payload.to_string();
    let matched: Vec<&HookConfig> = hooks
        .iter()
        .filter(|h| h.event == event && runs_off_tool(h))
        .filter(|h| !h.cc_compat || event_value_matches(h, value))
        .collect();
    let mut context = Vec::new();
    if matched.is_empty() {
        return context;
    }
    for (h, run) in run_all(jobs_for(matched, &payload, event, session_id)).await {
        match run {
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

/// The payload field a Claude-Code matcher filters on for a lifecycle event, if it has one.
fn lifecycle_matcher_value(event: HookEvent, fields: &Value) -> Option<String> {
    let key = match event {
        HookEvent::Notification => "notification_type",
        HookEvent::PreCompact | HookEvent::PostCompact => "trigger",
        HookEvent::SubagentStop => "agent_type",
        _ => return None,
    };
    fields.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Run the Claude-Code lifecycle hooks: `Notification`, `PreCompact`, `PostCompact`, `Stop`,
/// `SubagentStop`. `fields` are merged into the stdin payload (e.g. `{"message":…}` for a
/// notification, `{"trigger":…}` for compaction). Native hooks receive `{session_id, event,
/// …fields}`; CC-compat hooks receive the CC shape with `hook_event_name`, and their matcher is
/// tested against the event's value (`notification_type`, `trigger`). Output is collected as
/// notes; acting on `blocked` is the caller's job (only `Stop` and `SubagentStop` do, see
/// `stop_hook.rs`).
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
    let value = lifecycle_matcher_value(event, &fields);
    let matched: Vec<&HookConfig> = hooks
        .iter()
        .filter(|h| h.event == event && runs_off_tool(h))
        .filter(|h| {
            !h.cc_compat
                || value
                    .as_deref()
                    .is_none_or(|value| event_value_matches(h, value))
        })
        .collect();
    if matched.is_empty() {
        return outcome;
    }
    for (_, run) in run_all(jobs_for(matched, &native_payload, event, session_id)).await {
        match run {
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

/// A `PermissionRequest` hook's answer for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionHookDecision {
    /// Approve the call; `updated_input` (Forge argument names) replaces its arguments.
    Allow {
        updated_input: Option<serde_json::Value>,
    },
    /// Refuse the call; `message` is what the model reads. `interrupt` ends the agent's turn.
    Deny { message: String, interrupt: bool },
}

/// The combined effect of the `PermissionRequest` hooks for one call.
#[derive(Debug, Default)]
pub struct PermissionHookOutcome {
    /// `None` when no hook answered: the user is asked as usual.
    pub decision: Option<PermissionHookDecision>,
    pub notes: Vec<String>,
}

/// Run the `PermissionRequest` hooks matching `tool` (Claude-Code semantics): a hook answers with
/// `hookSpecificOutput.decision.behavior` = `allow` (optionally `updatedInput`) or `deny` (optional
/// `message`, `interrupt`), or blocks with exit 2 / `{"decision":"block"}`. Native hooks may also
/// print `{"action":"allow"}` / `{"action":"deny","reason":…}`, and a non-zero exit denies. A deny
/// from any hook wins; otherwise the first allow does. No answer leaves the prompt to the user.
pub async fn run_permission_hooks(
    hooks: &[HookConfig],
    tool: &str,
    payload: &str,
) -> PermissionHookOutcome {
    let mut outcome = PermissionHookOutcome::default();
    let event = HookEvent::PermissionRequest;
    let matched = tool_event_hooks(hooks, event, tool, payload);
    if matched.is_empty() {
        return outcome;
    }
    for (h, run) in run_all(jobs_for(matched, payload, event, "")).await {
        let (code, stdout, stderr) = match run {
            Ok(result) => result,
            Err(e) => {
                outcome.notes.push(format!("⎇ hook error: {e}"));
                continue;
            }
        };
        let decision = permission_decision_of(h, tool, code, &stdout, &stderr);
        match decision {
            Some(deny @ PermissionHookDecision::Deny { .. }) => {
                outcome.decision = Some(deny);
                break;
            }
            Some(allow) => {
                outcome.decision.get_or_insert(allow);
            }
            None => {
                let text = stdout.trim();
                if !text.is_empty() && serde_json::from_str::<serde_json::Value>(text).is_err() {
                    outcome
                        .notes
                        .push(format!("⎇ hook: {}", truncate(text, 800)));
                }
            }
        }
    }
    outcome
}

fn permission_decision_of(
    h: &HookConfig,
    tool: &str,
    code: i32,
    stdout: &str,
    stderr: &str,
) -> Option<PermissionHookDecision> {
    let deny = |message: String, interrupt: bool| PermissionHookDecision::Deny {
        message: truncate(&message, 800),
        interrupt,
    };
    let failure_reason = || {
        let err = stderr.trim();
        if !err.is_empty() {
            err.to_string()
        } else if !stdout.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            format!("{tool} denied by PermissionRequest hook (exit {code})")
        }
    };
    if code == 2 || (!h.cc_compat && code != 0) {
        return Some(deny(failure_reason(), false));
    }
    if code != 0 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    if let Some(decision) = value
        .get("hookSpecificOutput")
        .and_then(|h| h.get("decision"))
    {
        return match decision.get("behavior").and_then(serde_json::Value::as_str) {
            Some("allow") => Some(PermissionHookDecision::Allow {
                updated_input: decision
                    .get("updatedInput")
                    .filter(|i| i.is_object())
                    .map(|i| cc_input_to_forge(tool, i)),
            }),
            Some("deny") => Some(deny(
                decision
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("denied by PermissionRequest hook")
                    .to_string(),
                decision
                    .get("interrupt")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            )),
            _ => None,
        };
    }
    let field = |k: &str| value.get(k).and_then(serde_json::Value::as_str);
    match field("action").or_else(|| field("decision")) {
        Some("allow" | "approve") => Some(PermissionHookDecision::Allow {
            updated_input: None,
        }),
        Some("deny" | "block") => Some(deny(
            field("reason")
                .unwrap_or("denied by PermissionRequest hook")
                .to_string(),
            false,
        )),
        _ => None,
    }
}

pub async fn run_session_hooks(
    hooks: &[HookConfig],
    event: HookEvent,
    session_id: &str,
) -> Vec<String> {
    run_session_hooks_in(hooks, event, session_id, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc_regex_matchers_fire() {
        let hook = |matcher: &str| {
            let settings = serde_json::json!({"PreToolUse": [{"matcher": matcher,
                "hooks": [{"type": "command", "command": "true"}]}]});
            forge_config::cc_hooks_from_settings(&settings).remove(0)
        };
        assert!(hook_applies(&hook(".*"), "read_file"));
        assert!(hook_applies(&hook("Rea.*"), "read_file"));
        assert!(hook_applies(&hook("^Read$"), "read_file"));
        assert!(!hook_applies(&hook("^Read$"), "shell"));
        assert!(hook_applies(&hook("mcp__.*"), "mcp__github__get_issue"));
        assert!(!hook_applies(&hook("mcp__.*"), "read_file"));
        assert!(hook_applies(&hook("Read|Grep"), "read_file"));
    }

    fn hook(event: HookEvent, command: &str) -> HookConfig {
        HookConfig {
            timeout_secs: 10,
            ..HookConfig::new(event, command)
        }
    }

    fn cc_hook(event: HookEvent, command: &str) -> HookConfig {
        HookConfig {
            cc_compat: true,
            ..hook(event, command)
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

    // ---- PermissionRequest + prompt/agent hooks (CC parity) ----

    struct FixedVerdict(bool, Option<&'static str>);

    #[async_trait::async_trait]
    impl crate::hooks_llm::LlmHookRunner for FixedVerdict {
        async fn evaluate(
            &self,
            _hook: &HookConfig,
            input: &serde_json::Value,
        ) -> Result<crate::hooks_llm::LlmVerdict, String> {
            assert!(
                input.is_object(),
                "the event JSON reaches the model: {input}"
            );
            Ok(crate::hooks_llm::LlmVerdict {
                ok: self.0,
                reason: self.1.map(str::to_string),
            })
        }
    }

    fn prompt_hook(event: HookEvent) -> HookConfig {
        HookConfig {
            kind: forge_config::HookKind::Prompt,
            prompt: Some("judge $ARGUMENTS".into()),
            ..cc_hook(event, "")
        }
    }

    fn runner(
        ok: bool,
        reason: Option<&'static str>,
    ) -> Option<std::sync::Arc<dyn crate::hooks_llm::LlmHookRunner>> {
        Some(std::sync::Arc::new(FixedVerdict(ok, reason)))
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn permission_request_hooks_allow_deny_or_pass() {
        let payload = r#"{"tool":"shell","args":{"command":"ls"}}"#;
        let allow = cc_hook(
            HookEvent::PermissionRequest,
            r#"echo '{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedInput":{"command":"ls -la"}}}}'"#,
        );
        let out = run_permission_hooks(std::slice::from_ref(&allow), "shell", payload).await;
        assert_eq!(
            out.decision,
            Some(PermissionHookDecision::Allow {
                updated_input: Some(serde_json::json!({"command": "ls -la"}))
            })
        );

        let deny = cc_hook(
            HookEvent::PermissionRequest,
            r#"echo '{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"no","interrupt":true}}}'"#,
        );
        let out = run_permission_hooks(&[allow.clone(), deny], "shell", payload).await;
        assert_eq!(
            out.decision,
            Some(PermissionHookDecision::Deny {
                message: "no".into(),
                interrupt: true
            }),
            "a deny wins over an earlier allow"
        );

        let exit2 = cc_hook(HookEvent::PermissionRequest, "echo nope >&2; exit 2");
        assert!(matches!(
            run_permission_hooks(&[exit2], "shell", payload).await.decision,
            Some(PermissionHookDecision::Deny { message, .. }) if message == "nope"
        ));
        let native_allow = hook(HookEvent::PermissionRequest, r#"echo '{"action":"allow"}'"#);
        assert!(matches!(
            run_permission_hooks(&[native_allow], "shell", payload)
                .await
                .decision,
            Some(PermissionHookDecision::Allow {
                updated_input: None
            })
        ));
        let native_fail = hook(HookEvent::PermissionRequest, "exit 1");
        assert!(matches!(
            run_permission_hooks(&[native_fail], "shell", payload)
                .await
                .decision,
            Some(PermissionHookDecision::Deny { .. })
        ));
        let silent = cc_hook(HookEvent::PermissionRequest, "true");
        assert_eq!(
            run_permission_hooks(&[silent], "shell", payload)
                .await
                .decision,
            None
        );
        let other_tool = HookConfig {
            matcher: Some("Write".into()),
            ..cc_hook(HookEvent::PermissionRequest, "exit 2")
        };
        assert_eq!(
            run_permission_hooks(&[other_tool], "shell", payload)
                .await
                .decision,
            None,
            "the matcher scopes the hook"
        );
    }

    #[tokio::test]
    async fn prompt_hooks_block_or_pass_like_command_hooks() {
        let payload = r#"{"tool":"shell","args":{"command":"ls"}}"#;
        let hooks = [prompt_hook(HookEvent::PreToolUse)];
        let blocked = crate::hooks_llm::with_runner(
            runner(false, Some("tests first")),
            run_hooks(&hooks, HookEvent::PreToolUse, "shell", payload),
        )
        .await;
        assert_eq!(blocked.blocked.as_deref(), Some("tests first"));
        let passed = crate::hooks_llm::with_runner(
            runner(true, None),
            run_hooks(&hooks, HookEvent::PreToolUse, "shell", payload),
        )
        .await;
        assert!(
            passed.blocked.is_none() && passed.notes.is_empty(),
            "{passed:?}"
        );

        let permission = [prompt_hook(HookEvent::PermissionRequest)];
        let out = crate::hooks_llm::with_runner(
            runner(true, None),
            run_permission_hooks(&permission, "shell", payload),
        )
        .await;
        assert!(matches!(
            out.decision,
            Some(PermissionHookDecision::Allow { .. })
        ));

        let stop = [HookConfig {
            kind: forge_config::HookKind::Agent,
            ..prompt_hook(HookEvent::Stop)
        }];
        let out = crate::hooks_llm::with_runner(
            runner(false, Some("the build is red")),
            run_lifecycle_hooks(&stop, HookEvent::Stop, "s", serde_json::json!({})),
        )
        .await;
        assert_eq!(out.blocked.as_deref(), Some("the build is red"));
    }

    #[tokio::test]
    async fn a_model_hook_without_a_runner_is_skipped_not_guessed() {
        let hooks = [prompt_hook(HookEvent::PreToolUse)];
        let out = run_hooks(&hooks, HookEvent::PreToolUse, "shell", "{}").await;
        assert!(out.blocked.is_none());
        assert!(
            out.notes
                .iter()
                .any(|n| n.contains("no model is available")),
            "{:?}",
            out.notes
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_post_tool_use_block_reason_reaches_the_model() {
        let hooks = [cc_hook(
            HookEvent::PostToolUse,
            "echo lint failed >&2; exit 2",
        )];
        let out = run_hooks(&hooks, HookEvent::PostToolUse, "shell", "{}").await;
        assert_eq!(out.injected_context, vec!["lint failed".to_string()]);
    }
}
