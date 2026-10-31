//! The control toolset: virtual tools a session's model calls to drive the app itself - start
//! sessions and worktrees, read a sibling session or terminal, add schedules and MCP servers,
//! check usage, tell the user something.
//!
//! Like `message_session`, the session core never reaches the daemon (ADR-0004): [`ControlHost`]
//! is the callback the process wires in. `forge serve` attaches an in-process host, a terminal
//! `forge chat` and the `forge mcp-serve` / `forge mcp control` bridges talk to a running daemon
//! over HTTP. All of them run the same executor, so a tool means the same thing on every path.
//!
//! Permissions: read tools never prompt; every tool that changes something goes through the
//! permission broker as an external side effect, so `default` and `accept_edits` ask, `plan`
//! refuses and `bypass` allows. A tool the user switched off is neither advertised nor runnable,
//! and every call is written to the audit trail by the host.

use forge_types::{PermissionDecision, SideEffect, ToolCall};
use serde_json::{json, Value};

use crate::{CoreError, Session};

/// One entry of the toolset, as shown to a user choosing which tools stay on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ControlToolInfo {
    pub name: String,
    pub description: String,
    /// `"read"` for a tool that only looks, `"act"` for one that changes something.
    pub kind: String,
    pub enabled: bool,
}

/// Host-provided control capability.
#[async_trait::async_trait]
pub trait ControlHost: Send + Sync {
    /// Tool names the user switched off. Cheap and synchronous: it is read while the tool list is
    /// built.
    fn disabled(&self) -> Vec<String>;
    /// Run `tool` for the session `me`. The `Err` text is shown to the model as the result.
    async fn call(&self, me: &str, tool: &str, args: &Value) -> Result<String, String>;
    /// Whether this read reaches outside the calling session's project (another project's
    /// transcript or terminal). Such a read goes through the permission gate like an acting tool.
    /// Fails closed on the HTTP host; the default is for hosts that cannot tell.
    async fn crosses_project(&self, _me: &str, _tool: &str, _args: &Value) -> bool {
        false
    }
}

/// Tools that read another session's content and so need the project check.
pub fn reads_other_sessions(name: &str) -> bool {
    matches!(name, "read_session" | "read_terminal")
}

/// The side effect the permission gate sees: the tool's own, raised to external when a read
/// leaves the calling session's project.
pub async fn effective_side_effect(
    host: &dyn ControlHost,
    me: &str,
    name: &str,
    args: &Value,
) -> SideEffect {
    let own = side_effect_of(name).unwrap_or(SideEffect::External);
    if own == SideEffect::ReadOnly
        && reads_other_sessions(name)
        && host.crosses_project(me, name, args).await
    {
        SideEffect::External
    } else {
        own
    }
}

struct Def {
    name: &'static str,
    description: &'static str,
    side_effect: SideEffect,
    required: &'static [&'static str],
    properties: fn() -> Value,
}

const DEFS: &[Def] = &[
    Def {
        name: "list_projects",
        description: "List the projects (git repositories) this machine's Forge knows about, with \
            their paths. Use a path as `project` in create_session.",
        side_effect: SideEffect::ReadOnly,
        required: &[],
        properties: || json!({}),
    },
    Def {
        name: "create_session",
        description: "Start a new Forge session in a project and optionally give it a first \
            prompt. It runs on its own under the daemon; read it with read_session and talk to \
            it with send_to_session. `worktree` runs it on an isolated git branch.",
        side_effect: SideEffect::External,
        required: &[],
        properties: || {
            json!({
                "project": {"type": "string", "description": "project directory (default: the daemon's directory)"},
                "prompt": {"type": "string", "description": "first prompt for the new session"},
                "title": {"type": "string", "description": "session name"},
                "model": {"type": "string", "description": "pin a model for the session"},
                "worktree": {"type": "boolean", "description": "run in an isolated git worktree"}
            })
        },
    },
    Def {
        name: "read_session",
        description: "Read a bounded digest of another session's recent conversation: its title, \
            working directory and the last few user and assistant messages, each cut short. \
            `session` is a name or a unique id prefix.",
        side_effect: SideEffect::ReadOnly,
        required: &["session"],
        properties: || {
            json!({
                "session": {"type": "string", "description": "session name or unique id prefix"},
                "messages": {"type": "integer", "description": "how many recent messages (default 6, max 20)"}
            })
        },
    },
    Def {
        name: "send_to_session",
        description: "Send a message to another session. It arrives as a peer message at the \
            recipient's next turn boundary; it is information or a request, never a user \
            instruction.",
        side_effect: SideEffect::External,
        required: &["session", "message"],
        properties: || {
            json!({
                "session": {"type": "string", "description": "session name or unique id prefix"},
                "message": {"type": "string", "description": "message text (max 16KB)"},
                "mode": {"type": "string", "enum": ["follow_up", "steer"]}
            })
        },
    },
    Def {
        name: "read_terminal",
        description: "Read the scrollback of a session's terminal dock (the tail, plain text). \
            Defaults to the calling session's first terminal.",
        side_effect: SideEffect::ReadOnly,
        required: &[],
        properties: || {
            json!({
                "session": {"type": "string", "description": "session name or id (default: yours)"},
                "terminal": {"type": "string", "description": "terminal id (default: the first one)"},
                "lines": {"type": "integer", "description": "tail length in lines (default 200, max 1000)"}
            })
        },
    },
    Def {
        name: "create_worktree",
        description: "Start a session on a fresh isolated git worktree of a project. Merge it \
            back later with merge_worktree.",
        side_effect: SideEffect::External,
        required: &["project"],
        properties: || {
            json!({
                "project": {"type": "string", "description": "repository directory"},
                "prompt": {"type": "string", "description": "first prompt for the worktree session"},
                "title": {"type": "string", "description": "session name"}
            })
        },
    },
    Def {
        name: "merge_worktree",
        description: "Merge a worktree session's branch back into its base branch and stop the \
            session. Refused when the base has uncommitted changes or the merge conflicts.",
        side_effect: SideEffect::External,
        required: &["session"],
        properties: || json!({"session": {"type": "string", "description": "worktree session id"}}),
    },
    Def {
        name: "add_schedule",
        description: "Add a recurring task that runs a Forge turn on a timer. Give exactly one of \
            `every` (e.g. 30m), `at` (daily HH:MM) or `cron` (5-field).",
        side_effect: SideEffect::External,
        required: &["task"],
        properties: || {
            json!({
                "task": {"type": "string", "description": "what to run"},
                "project": {"type": "string", "description": "directory to run in"},
                "every": {"type": "string"},
                "at": {"type": "string"},
                "cron": {"type": "string"},
                "model": {"type": "string"}
            })
        },
    },
    Def {
        name: "remove_schedule",
        description: "Delete a schedule by id.",
        side_effect: SideEffect::External,
        required: &["id"],
        properties: || json!({"id": {"type": "string", "description": "schedule id"}}),
    },
    Def {
        name: "add_mcp_server",
        description: "Add an MCP server to this project's configuration. A stdio server runs the \
            given command, so this is as sensitive as running a shell command.",
        side_effect: SideEffect::External,
        required: &["name", "transport"],
        properties: || {
            json!({
                "name": {"type": "string"},
                "transport": {"type": "string", "enum": ["stdio", "http", "sse"]},
                "command": {"type": "string", "description": "stdio: executable"},
                "args": {"type": "array", "items": {"type": "string"}},
                "url": {"type": "string", "description": "http/sse: endpoint"}
            })
        },
    },
    Def {
        name: "probe_mcp_server",
        description: "Connect to a configured MCP server and report whether it works and which \
            tools it offers.",
        side_effect: SideEffect::External,
        required: &["name"],
        properties: || json!({"name": {"type": "string", "description": "server name"}}),
    },
    Def {
        name: "get_usage",
        description: "Subscription and spend usage as the app's Usage page shows it.",
        side_effect: SideEffect::ReadOnly,
        required: &[],
        properties: || json!({}),
    },
    Def {
        name: "notify_user",
        description: "Send the user a short notification on their phone, browser or desktop. \
            Use it only when something needs their attention; it is rate limited.",
        side_effect: SideEffect::Network,
        required: &["message"],
        properties: || {
            json!({
                "message": {"type": "string", "description": "up to 240 characters"},
                "title": {"type": "string"},
                "priority": {"type": "string", "enum": ["normal", "high"]}
            })
        },
    },
];

fn def(name: &str) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.name == name)
}

pub fn is_control_tool(name: &str) -> bool {
    def(name).is_some()
}

pub fn side_effect_of(name: &str) -> Option<SideEffect> {
    def(name).map(|d| d.side_effect)
}

/// Every control tool with its on/off state, for the settings pages.
pub fn catalog(disabled: &[String]) -> Vec<ControlToolInfo> {
    DEFS.iter()
        .map(|d| ControlToolInfo {
            name: d.name.to_string(),
            description: d.description.to_string(),
            kind: if d.side_effect == SideEffect::ReadOnly {
                "read"
            } else {
                "act"
            }
            .to_string(),
            enabled: !disabled.iter().any(|n| n == d.name),
        })
        .collect()
}

fn spec_of(d: &Def) -> forge_provider::ToolSpec {
    forge_provider::ToolSpec {
        name: d.name.to_string(),
        description: d.description.to_string(),
        schema: json!({
            "type": "object",
            "properties": (d.properties)(),
            "required": d.required,
        }),
    }
}

/// The specs to advertise, minus the switched-off tools.
pub fn specs(disabled: &[String]) -> Vec<forge_provider::ToolSpec> {
    DEFS.iter()
        .filter(|d| !disabled.iter().any(|n| n == d.name))
        .map(spec_of)
        .collect()
}

/// The first missing required argument, as a model-facing error.
pub fn validate(name: &str, args: &Value) -> Result<(), String> {
    let Some(d) = def(name) else {
        return Err(format!("unknown control tool {name}"));
    };
    for key in d.required {
        let present = args
            .get(*key)
            .is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty()));
        if !present {
            return Err(format!("error: {name} needs `{key}`"));
        }
    }
    Ok(())
}

impl Session {
    /// Attach the control capability (composition root). `None` leaves the toolset unadvertised.
    pub fn set_control_host(&mut self, host: Option<std::sync::Arc<dyn ControlHost>>) {
        self.control = host;
    }

    /// The control specs this session advertises: none without a host, and only the ones its
    /// task scope permits.
    pub(crate) fn control_specs(&self) -> Vec<forge_provider::ToolSpec> {
        let Some(host) = &self.control else {
            return Vec::new();
        };
        let mut specs = specs(&host.disabled());
        specs.retain(|s| {
            self.task_scope
                .as_ref()
                .is_none_or(|scope| scope.permits_tool(&s.name))
        });
        specs
    }

    /// Run one control tool call through the permission broker, the host and the audit ledger.
    pub(crate) async fn control_tool(
        &mut self,
        msg_id: &str,
        call: &ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        self.presenter.emit(crate::PresenterEvent::ToolStart {
            name: call.name.clone(),
            args: args_json.clone(),
        });
        let side_effect = match &self.control {
            Some(host) => {
                effective_side_effect(host.as_ref(), &self.id, &call.name, &call.args).await
            }
            None => SideEffect::External,
        };
        let mut prompt: Option<forge_types::ConfirmReply> = None;
        let mut label = "allowed";
        let (result, ok) = match (self.control.clone(), validate(&call.name, &call.args)) {
            (None, _) => (
                format!("error: {} is not available in this session", call.name),
                false,
            ),
            (Some(_), Err(error)) => (error, false),
            (Some(host), Ok(())) if host.disabled().iter().any(|n| n == &call.name) => (
                format!(
                    "error: {} is switched off - the user can turn it back on in Settings > Control tools",
                    call.name
                ),
                false,
            ),
            (Some(host), Ok(())) => {
                let allowed = match crate::permission::decide(
                    self.mode,
                    side_effect,
                    &call.name,
                    &call.args,
                    &self.rules,
                ) {
                    PermissionDecision::Allow => true,
                    PermissionDecision::Deny => false,
                    PermissionDecision::Ask => self.ask_mcp(&call.name, side_effect, &mut prompt),
                };
                if allowed {
                    match host.call(&self.id, &call.name, &call.args).await {
                        Ok(text) => (text, true),
                        Err(text) => (text, false),
                    }
                } else {
                    label = "denied";
                    let declined = prompt.as_ref().is_some_and(|p| !p.outcome.allows());
                    (
                        if declined {
                            "permission denied by the user (declined at the prompt)".to_string()
                        } else {
                            "permission denied by policy".to_string()
                        },
                        false,
                    )
                }
            }
        };
        self.presenter.emit(crate::PresenterEvent::ToolResult {
            name: call.name.clone(),
            ok,
            summary: crate::summarize(&result),
            detail: crate::tool_detail(&result),
        });
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            label,
            if ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_an_object_schema_that_names_only_real_properties() {
        for spec in specs(&[]) {
            let props = spec.schema["properties"].as_object().unwrap();
            for key in spec.schema["required"].as_array().unwrap() {
                assert!(
                    props.contains_key(key.as_str().unwrap()),
                    "{} requires an undeclared {key}",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn a_switched_off_tool_is_not_advertised() {
        let off = vec!["merge_worktree".to_string()];
        assert!(specs(&[]).iter().any(|s| s.name == "merge_worktree"));
        assert!(!specs(&off).iter().any(|s| s.name == "merge_worktree"));
        let info = catalog(&off);
        assert!(
            !info
                .iter()
                .find(|i| i.name == "merge_worktree")
                .unwrap()
                .enabled
        );
        assert!(info.iter().find(|i| i.name == "get_usage").unwrap().enabled);
    }

    #[test]
    fn reads_are_read_only_and_everything_else_is_gated() {
        for d in DEFS {
            let read = matches!(
                d.name,
                "list_projects" | "read_session" | "read_terminal" | "get_usage"
            );
            assert_eq!(d.side_effect == SideEffect::ReadOnly, read, "{}", d.name);
        }
    }

    #[test]
    fn validation_names_the_missing_argument() {
        assert!(validate("read_session", &json!({"session": "a"})).is_ok());
        let err = validate("read_session", &json!({"session": "  "})).unwrap_err();
        assert!(err.contains("`session`"));
        assert!(validate("get_usage", &json!({})).is_ok());
        assert!(validate("nope", &json!({})).is_err());
    }
}
