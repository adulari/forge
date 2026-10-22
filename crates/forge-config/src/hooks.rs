//! `[[hooks]]` configuration (docs/features/hooks.md): the events a hook can fire on, how it is
//! evaluated, and the translation of a Claude-Code `settings.json` `hooks` object into Forge hooks.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::cc_tool_alias;

/// When a hook fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Before the tool runs; a non-zero exit blocks the call (stderr/stdout = the reason).
    PreToolUse,
    /// After the tool returns; output is surfaced as a note, exit code is advisory.
    PostToolUse,
    /// After a tool that started running failed (Claude-Code `PostToolUseFailure`). The payload
    /// carries `error`; `additionalContext` reaches the model next to the failed result.
    PostToolUseFailure,
    /// Once per model step, after every tool call of the batch resolved and before the next model
    /// request (Claude-Code `PostToolBatch`). A block ends the turn before that request.
    PostToolBatch,
    /// After the user submits a message, before the agent turn starts.
    /// Hook receives `{"prompt": "<user message>"}` on stdin.
    /// Exit 0 + non-empty stdout → stdout replaces the prompt.
    /// Exit non-zero → turn is blocked; stderr/stdout shown as the reason.
    UserPromptSubmit,
    /// A typed `/command` or skill expanded into a prompt (Claude-Code `UserPromptExpansion`). A
    /// block cancels it; `additionalContext` joins the expanded prompt.
    UserPromptExpansion,
    /// When a session starts (`source`: startup, resume, clear or compact). Output from a
    /// Claude-Code hook is added to the model's context.
    SessionStart,
    /// When the session loop exits cleanly. Observe-only.
    /// Receives `{"session_id": "<id>", "event": "session_end"}` on stdin.
    SessionEnd,
    /// An instruction file (`AGENTS.md`, `CLAUDE.md`) was loaded into context. Observe-only.
    InstructionsLoaded,
    /// The agent emitted a user-facing notification (e.g. a tool needs permission). Observe-only.
    /// Claude-Code parity event (`Notification`).
    Notification,
    /// Just before context compaction runs (Claude-Code `PreCompact`). Observe-only.
    PreCompact,
    /// Just after context compaction completes. Observe-only.
    PostCompact,
    /// A turn is about to end (Claude-Code `Stop`). A hook that exits 2 or prints
    /// `{"decision":"block","reason":…}` keeps the agent going with `reason` as its next
    /// instruction, bounded by `stop_hook_max_blocks`.
    Stop,
    /// The turn ended on a provider error instead of an answer (Claude-Code `StopFailure`; `error`
    /// is `rate_limit`, `authentication_failed`, …). Observe-only.
    StopFailure,
    /// A subagent is starting (Claude-Code `SubagentStart`). `additionalContext` is added to the
    /// subagent's task.
    SubagentStart,
    /// A subagent finished (Claude-Code `SubagentStop`). A block appends its reason to the
    /// `spawn_agents` result the parent model reads.
    SubagentStop,
    /// A tool call is about to ask the user for permission (Claude-Code `PermissionRequest`). The
    /// hook can answer for the user: allow (optionally with rewritten input) or deny (with a
    /// message for the model, optionally interrupting the turn). No answer shows the prompt.
    PermissionRequest,
    /// Forge refused a call on its own (no one could answer the prompt, the temper forbids it, the
    /// safety floor). `hookSpecificOutput.retry: true` tells the model it may try again.
    PermissionDenied,
    /// The session's working directory moved (a worktree switch). Observe-only.
    CwdChanged,
    /// The `worktree` tool is creating a worktree: the hook REPLACES `git worktree add` and prints
    /// the new directory as its last stdout line.
    WorktreeCreate,
    /// A worktree a `WorktreeCreate` hook made is being removed; the hook does the removal.
    WorktreeRemove,
    /// Before a model switch the user asked for (`/model`, the model picker). A block keeps the
    /// current model.
    PreModelSwitch,
    /// After the session's model changed. Output is added to the model's context.
    PostModelSwitch,
}

/// Every event with its Claude-Code name, in one table so the two directions cannot drift.
const EVENT_NAMES: &[(HookEvent, &str)] = &[
    (HookEvent::PreToolUse, "PreToolUse"),
    (HookEvent::PostToolUse, "PostToolUse"),
    (HookEvent::PostToolUseFailure, "PostToolUseFailure"),
    (HookEvent::PostToolBatch, "PostToolBatch"),
    (HookEvent::UserPromptSubmit, "UserPromptSubmit"),
    (HookEvent::UserPromptExpansion, "UserPromptExpansion"),
    (HookEvent::SessionStart, "SessionStart"),
    (HookEvent::SessionEnd, "SessionEnd"),
    (HookEvent::InstructionsLoaded, "InstructionsLoaded"),
    (HookEvent::Notification, "Notification"),
    (HookEvent::PreCompact, "PreCompact"),
    (HookEvent::PostCompact, "PostCompact"),
    (HookEvent::Stop, "Stop"),
    (HookEvent::StopFailure, "StopFailure"),
    (HookEvent::SubagentStart, "SubagentStart"),
    (HookEvent::SubagentStop, "SubagentStop"),
    (HookEvent::PermissionRequest, "PermissionRequest"),
    (HookEvent::PermissionDenied, "PermissionDenied"),
    (HookEvent::CwdChanged, "CwdChanged"),
    (HookEvent::WorktreeCreate, "WorktreeCreate"),
    (HookEvent::WorktreeRemove, "WorktreeRemove"),
    (HookEvent::PreModelSwitch, "PreModelSwitch"),
    (HookEvent::PostModelSwitch, "PostModelSwitch"),
];

impl HookEvent {
    /// The Claude-Code event name for this hook (`PreToolUse`, `Notification`, …). Used to build
    /// the CC stdin payload's `hook_event_name` and to translate CC settings.json config.
    pub fn cc_name(self) -> &'static str {
        EVENT_NAMES
            .iter()
            .find(|(event, _)| *event == self)
            .map_or("Unknown", |(_, name)| name)
    }

    /// Parse a Claude-Code event name into a [`HookEvent`]. `None` for an unknown name.
    pub fn from_cc_name(name: &str) -> Option<HookEvent> {
        EVENT_NAMES
            .iter()
            .find(|(_, cc)| *cc == name)
            .map(|(event, _)| *event)
    }

    /// The events whose matcher filters the tool name and whose hooks may carry an `if` rule.
    pub fn is_tool_event(self) -> bool {
        matches!(
            self,
            HookEvent::PreToolUse
                | HookEvent::PostToolUse
                | HookEvent::PostToolUseFailure
                | HookEvent::PermissionRequest
                | HookEvent::PermissionDenied
        )
    }

    /// Claude Code's default timeout for a `command` / `http` hook on this event: 600 s, lowered
    /// to 30 s where the user is waiting on the answer.
    fn cc_command_timeout(self) -> u64 {
        match self {
            HookEvent::UserPromptSubmit
            | HookEvent::PreModelSwitch
            | HookEvent::PostModelSwitch => 30,
            _ => 600,
        }
    }
}

/// How a hook is evaluated (Claude-Code `type`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    /// Run `command` through the OS shell (the default).
    #[default]
    Command,
    /// POST the event JSON to `url`; the response body is read like a command hook's stdout.
    Http,
    /// Ask a model to judge the event: `prompt` (with `$ARGUMENTS` replaced by the event JSON) is
    /// sent to a fast side-call model, which answers `{"ok": true}` or `{"ok": false, "reason"}`.
    Prompt,
    /// Like `prompt`, but a read-only subagent (read_file/search/glob/list_dir in the workspace)
    /// investigates before answering.
    Agent,
}

/// One `[[hooks]]` entry: a shell command (or a model, see [`HookKind`]) run around tool calls
/// matching `matcher` and around session lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    pub event: HookEvent,
    /// `command` (default), `http`, `prompt` or `agent`.
    #[serde(default, rename = "type")]
    pub kind: HookKind,
    /// Tool-name filter for tool events: absent or `"*"` = every tool; otherwise a comma-separated
    /// list of exact tool names (e.g. `"shell"`). On the other events with Claude-Code matcher
    /// support it filters that event's value (`source`, `load_reason`, `error`, the model id …).
    #[serde(default)]
    pub matcher: Option<String>,
    /// Claude-Code `if`: one permission rule (`Bash(git *)`, `Edit(*.ts)`) the tool call must
    /// match for the hook to run. Only tool events evaluate it; elsewhere such a hook never runs.
    #[serde(default, rename = "if", skip_serializing_if = "Option::is_none")]
    pub if_rule: Option<String>,
    /// Shell command line (`command` hooks). Receives event data as JSON on stdin.
    #[serde(default)]
    pub command: String,
    /// The endpoint of an `http` hook; the event JSON is the POST body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Extra request headers for an `http` hook. `$VAR` / `${VAR}` expand only for variables
    /// listed in `allowed_env_vars`; any other reference becomes empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_env_vars: Vec<String>,
    /// Run a `command` hook in the background: the event never waits for it, and its
    /// `additionalContext` / `systemMessage` reach the model at the next turn.
    #[serde(default, rename = "async")]
    pub run_async: bool,
    /// The instruction for a `prompt` / `agent` hook. `$ARGUMENTS` is replaced by the event JSON;
    /// without it the JSON is appended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Model for a `prompt` / `agent` hook (`provider::model`). Default: the session's cheap
    /// side-call model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Kill the hook after this many seconds (default 30).
    #[serde(default = "default_hook_timeout")]
    pub timeout_secs: u64,
    /// Claude-Code compatibility mode. When true the hook speaks the CC protocol: it receives the
    /// CC JSON payload on stdin (`{session_id, transcript_path, cwd, hook_event_name, tool_name,
    /// tool_input, …}`) and its output is parsed CC-style (`{"decision":"block|approve","reason":…}`
    /// / `hookSpecificOutput`) with exit-code 2 = block. The `matcher` is interpreted as a CC tool
    /// matcher (`|`-separated names / `*`) against the CC tool-name alias. When false (default) the
    /// hook keeps Forge's native protocol. Set automatically for hooks loaded from a CC settings.json.
    #[serde(default)]
    pub cc_compat: bool,
}

pub(crate) fn default_hook_timeout() -> u64 {
    30
}

/// Claude Code treats a hook matcher as a regex. A matcher of plain names joined by `|` keeps its
/// exact-name meaning; anything with another regex metacharacter (`.*`, `^Read$`, `mcp__.*`) is
/// compiled and searched by `forge_core::hooks` (this crate has no regex dependency).
pub fn is_regex_matcher(pattern: &str) -> bool {
    pattern.chars().any(|c| ".*+?^$()[]{}\\".contains(c)) && pattern.trim() != "*"
}

impl HookConfig {
    /// A native `command` hook for `event` with every optional field at its default.
    pub fn new(event: HookEvent, command: impl Into<String>) -> Self {
        HookConfig {
            event,
            kind: HookKind::Command,
            matcher: None,
            if_rule: None,
            command: command.into(),
            url: None,
            headers: BTreeMap::new(),
            allowed_env_vars: Vec::new(),
            run_async: false,
            prompt: None,
            model: None,
            timeout_secs: default_hook_timeout(),
            cc_compat: false,
        }
    }

    /// Whether this hook applies to `tool_name`. Native hooks use Forge's exact/comma-separated
    /// matcher; CC-compat hooks treat the matcher as a `|`-separated CC matcher tested against both
    /// the Forge tool name and its [`cc_tool_alias`] (so `"Write|Edit"` fires on `edit_file`).
    pub fn matches(&self, tool_name: &str) -> bool {
        match self.matcher.as_deref() {
            None | Some("") | Some("*") => true,
            Some(pattern) => {
                if self.cc_compat {
                    let alias = cc_tool_alias(tool_name);
                    pattern.split('|').map(str::trim).any(|m| {
                        m == "*"
                            || m.eq_ignore_ascii_case(tool_name)
                            || m.eq_ignore_ascii_case(alias)
                    })
                } else {
                    pattern.split(',').any(|m| m.trim() == tool_name)
                }
            }
        }
    }
}

/// Translate a Claude-Code `settings.json` `hooks` object into a flat list of [`HookConfig`]s in
/// CC-compat mode, so existing CC hook scripts run under Forge unmodified. The CC shape is:
/// ```json
/// { "PreToolUse": [ { "matcher": "Write|Edit",
///                     "hooks": [ { "type": "command", "command": "./h.sh", "timeout": 10 } ] } ],
///   "Notification": [ { "hooks": [ { "type": "command", "command": "notify" } ] } ] }
/// ```
/// `value` may be either the top-level settings object (we look up its `hooks` key) or the `hooks`
/// object itself. `command`, `http`, `prompt` and `agent` hook types are read (with `if`, `async`,
/// `headers`, `allowedEnvVars`); unknown event names and other hook types are skipped.
pub fn cc_hooks_from_settings(value: &serde_json::Value) -> Vec<HookConfig> {
    let hooks_obj = value
        .get("hooks")
        .and_then(|h| h.as_object())
        .or_else(|| value.as_object());
    let Some(obj) = hooks_obj else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (event_name, groups) in obj {
        let Some(event) = HookEvent::from_cc_name(event_name) else {
            continue;
        };
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for group in groups {
            let matcher = group
                .get("matcher")
                .and_then(|m| m.as_str())
                .filter(|m| !m.is_empty() && *m != "*")
                .map(str::to_string);
            let Some(entries) = group.get("hooks").and_then(|h| h.as_array()) else {
                continue;
            };
            for entry in entries {
                if let Some(hook) = cc_hook_entry(event, matcher.clone(), entry) {
                    out.push(hook);
                }
            }
        }
    }
    out
}

/// One handler object of a CC hook group, or `None` when its type is unsupported or a required
/// field is missing.
fn cc_hook_entry(
    event: HookEvent,
    matcher: Option<String>,
    entry: &serde_json::Value,
) -> Option<HookConfig> {
    let text = |key: &str| entry.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let kind = match entry.get("type").and_then(|t| t.as_str()) {
        Some("command") => HookKind::Command,
        Some("http") => HookKind::Http,
        Some("prompt") => HookKind::Prompt,
        Some("agent") => HookKind::Agent,
        _ => return None,
    };
    let required = match kind {
        HookKind::Command => "command",
        HookKind::Http => "url",
        HookKind::Prompt | HookKind::Agent => "prompt",
    };
    text(required)?;
    let timeout_secs = entry
        .get("timeout")
        .and_then(|t| t.as_u64())
        .unwrap_or(match kind {
            HookKind::Agent => 60,
            HookKind::Prompt => default_hook_timeout(),
            HookKind::Command | HookKind::Http => event.cc_command_timeout(),
        });
    let headers = entry
        .get("headers")
        .and_then(|h| h.as_object())
        .map(|h| {
            h.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let allowed_env_vars = entry
        .get("allowedEnvVars")
        .and_then(|v| v.as_array())
        .map(|names| {
            names
                .iter()
                .filter_map(|n| n.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(HookConfig {
        kind,
        matcher,
        if_rule: text("if").filter(|rule| !rule.trim().is_empty()),
        url: text("url"),
        headers,
        allowed_env_vars,
        run_async: kind == HookKind::Command
            && entry.get("async").and_then(|a| a.as_bool()) == Some(true),
        prompt: text("prompt"),
        model: text("model"),
        timeout_secs,
        cc_compat: true,
        ..HookConfig::new(event, text("command").unwrap_or_default())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_round_trips_through_its_claude_code_name() {
        for (event, name) in EVENT_NAMES {
            assert_eq!(event.cc_name(), *name);
            assert_eq!(HookEvent::from_cc_name(name), Some(*event));
        }
        assert_eq!(HookEvent::from_cc_name("TeammateIdle"), None);
    }

    #[test]
    fn cc_http_if_and_async_fields_are_read() {
        let settings = serde_json::json!({"hooks": {
            "PreToolUse": [{"matcher": "Bash", "hooks": [
                {"type": "http", "url": "http://localhost:8080/h", "headers": {"Authorization": "Bearer $TOKEN"},
                 "allowedEnvVars": ["TOKEN"], "if": "Bash(git *)"},
                {"type": "command", "command": "./bg.sh", "async": true},
                {"type": "http"}
            ]}],
            "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "x"}]}],
            "WorktreeCreate": [{"hooks": [{"type": "command", "command": "y", "timeout": 5}]}]
        }});
        let hooks = cc_hooks_from_settings(&settings);
        assert_eq!(hooks.len(), 4, "{hooks:?}");
        let http = hooks.iter().find(|h| h.kind == HookKind::Http).unwrap();
        assert_eq!(http.url.as_deref(), Some("http://localhost:8080/h"));
        assert_eq!(http.headers["Authorization"], "Bearer $TOKEN");
        assert_eq!(http.allowed_env_vars, vec!["TOKEN"]);
        assert_eq!(http.if_rule.as_deref(), Some("Bash(git *)"));
        assert_eq!(http.timeout_secs, 600, "Claude Code's command/http default");
        assert!(hooks.iter().any(|h| h.run_async && h.command == "./bg.sh"));
        let prompt = hooks
            .iter()
            .find(|h| h.event == HookEvent::UserPromptSubmit)
            .unwrap();
        assert_eq!(prompt.timeout_secs, 30, "lowered where the user waits");
        let worktree = hooks
            .iter()
            .find(|h| h.event == HookEvent::WorktreeCreate)
            .unwrap();
        assert_eq!(worktree.timeout_secs, 5);
    }

    #[test]
    fn native_hooks_parse_the_new_fields_from_toml() {
        #[derive(Deserialize)]
        struct Wrap {
            hooks: Vec<HookConfig>,
        }
        let parsed: Wrap = toml::from_str(
            "[[hooks]]\nevent = \"post_tool_use_failure\"\ntype = \"http\"\nurl = \"http://x\"\n\
             if = \"shell(cargo *)\"\n[hooks.headers]\nX-Key = \"$K\"\n",
        )
        .unwrap();
        let hook = &parsed.hooks[0];
        assert_eq!(hook.event, HookEvent::PostToolUseFailure);
        assert_eq!(hook.kind, HookKind::Http);
        assert_eq!(hook.if_rule.as_deref(), Some("shell(cargo *)"));
        assert_eq!(hook.headers["X-Key"], "$K");
        assert_eq!(hook.timeout_secs, 30);
    }
}
