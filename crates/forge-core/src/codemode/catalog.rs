//! Name resolution and `tools.$search` for `run_code`.
//!
//! [`ToolDirectory`] is the one seam between codemode and "which tools exist". The session
//! implements it over its tool registry plus connected MCP tools. Deferred tools stay registered
//! when the lean surface hides them, so a script reaches them by name. The `tool_call` unwrap and
//! the search ranking are the shared deferred registry's ([`crate::deferred_tools`]), so a script
//! and a plain `tool_search`/`tool_call` resolve the same names the same way.

use forge_provider::ToolSpec;
use serde_json::Value;

use crate::{deferred_tools, Session};

/// Tools a script may not call: they drive the session itself (prompts, plans, child agents,
/// scheduling) or would recurse. Everything else in the directory is callable, and so is
/// `update_tasks`: it only rewrites the model's own checklist, and "tick this task off, then run
/// the next calls" is exactly the step a script exists to batch. Denying it made a script that led
/// with a task update die on its first line, discarding every call batched after it.
const SCRIPT_DENY: &[&str] = &[
    super::RUN_CODE_TOOL,
    crate::workflow::RUN_WORKFLOW_TOOL,
    crate::subagent::SPAWN_AGENTS_TOOL,
    crate::subagent::SEND_TO_AGENT_TOOL,
    crate::subagent::LIST_SUBAGENTS_TOOL,
    crate::subagent::CANCEL_SUBAGENT_TOOL,
    crate::ASK_USER_TOOL,
    crate::PRESENT_PLAN_TOOL,
    crate::USE_SKILL_TOOL,
    crate::REMEMBER_TOOL,
    crate::fleet::MESSAGE_SESSION_TOOL,
    crate::fleet::LIST_SESSIONS_TOOL,
    crate::dispatch::DISPATCH_SESSIONS_TOOL,
    crate::heartbeat::MANAGE_HEARTBEATS_TOOL,
    crate::wakeup::SCHEDULE_WAKEUP_TOOL,
    crate::push_notify::PUSH_NOTIFICATION_TOOL,
    deferred_tools::TOOL_SEARCH,
];

/// Wrapper prefixes models copy from the bridge or SDK spellings.
const ALIAS_PREFIXES: &[&str] = &["mcp__forge__", "default_api:", "default_api.", "functions."];

/// The tools a script can reach. See the module doc for the deferred-registry seam.
pub(crate) trait ToolDirectory {
    /// Every callable tool's spec, deferred ones included, minus [`SCRIPT_DENY`].
    fn catalog(&self) -> Vec<ToolSpec>;
    /// Whether `name` is a callable tool (before the deny list is applied).
    fn knows(&self, name: &str) -> bool;
}

impl ToolDirectory for Session {
    fn catalog(&self) -> Vec<ToolSpec> {
        let scope = self.task_scope.as_ref();
        let mut specs: Vec<ToolSpec> = self
            .tools
            .names()
            .filter(|name| scope.is_none_or(|scope| scope.permits_tool(name)))
            .filter_map(|name| self.tools.get(name))
            .map(|t| ToolSpec {
                name: t.name().to_string(),
                description: t.description().to_string(),
                schema: t.schema(),
            })
            .collect();
        if let Some(mcp) = &self.mcp {
            specs.extend(mcp.advertised_specs().into_iter().map(|s| ToolSpec {
                name: s.name,
                description: s.description,
                schema: s.schema,
            }));
        }
        specs.retain(|s| !SCRIPT_DENY.contains(&s.name.as_str()));
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    fn knows(&self, name: &str) -> bool {
        self.tools.get(name).is_some() || self.mcp.as_ref().is_some_and(|m| m.knows_tool(name))
    }
}

/// What a script's `tools.<name>(args)` turns into.
#[derive(Debug, PartialEq)]
pub(crate) enum Resolved {
    /// Dispatch this (canonical) call through `invoke_tool`. An unknown name is dispatched too,
    /// so the model gets the broker's "unknown tool, available: …" answer and its audit row.
    Call(String, Value),
    Denied(String),
}

/// Resolve a script's call: unwrap `tool_call{name, arguments}` through the deferred registry's
/// unwrap, strip SDK prefixes when the bare name is a real tool, then apply the deny list.
pub(crate) fn resolve(name: &str, args: Value, dir: &impl ToolDirectory) -> Resolved {
    let wrapped = forge_types::ToolCall {
        id: String::new(),
        name: name.to_string(),
        args,
    };
    let (name, args) = match deferred_tools::unwrap_tool_call(&wrapped) {
        Some(Ok(inner)) => (inner.name, inner.args),
        Some(Err(error)) => return Resolved::Denied(error),
        None => (wrapped.name, wrapped.args),
    };
    let name = ALIAS_PREFIXES
        .iter()
        .find_map(|p| name.strip_prefix(p))
        .filter(|bare| dir.knows(bare))
        .map_or(name.clone(), str::to_string);
    if SCRIPT_DENY.contains(&name.as_str()) {
        return Resolved::Denied(format!(
            "error: `{name}` cannot be called from run_code; call it as a normal tool instead"
        ));
    }
    let args = positional(&name, args, dir);
    Resolved::Call(name, args)
}

/// `tools.read_file("a.py")`: a script that passes a bare string instead of the argument object.
/// A JSON object in the string is parsed; otherwise the string becomes the tool's first required
/// parameter (`path`, `command`, `query`, ...), so the common shorthand runs instead of failing.
fn positional(name: &str, args: Value, dir: &impl ToolDirectory) -> Value {
    let Value::String(text) = args else {
        return args;
    };
    if let Ok(object @ Value::Object(_)) = serde_json::from_str::<Value>(&text) {
        return object;
    }
    let key = dir
        .catalog()
        .into_iter()
        .find(|spec| spec.name == name)
        .and_then(|spec| {
            let schema = &spec.schema;
            schema
                .get("required")
                .and_then(Value::as_array)
                .and_then(|r| r.first())
                .and_then(Value::as_str)
                .or_else(|| {
                    let props = schema.get("properties")?.as_object()?;
                    ["path", "command", "query", "pattern"]
                        .into_iter()
                        .find(|k| props.contains_key(*k))
                })
                .map(str::to_string)
        });
    match key {
        Some(key) => serde_json::json!({ key: text }),
        None => Value::String(text),
    }
}

/// `tools.$search(query)`: the deferred registry's ranking (exact names first, comma or space
/// separated, else keyword matches) over the whole directory. Up to eight
/// `{name, description, schema}` objects.
pub(crate) fn search(catalog: &[ToolSpec], query: &str) -> Value {
    Value::Array(
        deferred_tools::matches(catalog, query)
            .into_iter()
            .take(8)
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "schema": t.schema,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<&'static str>);
    impl ToolDirectory for Fixed {
        fn catalog(&self) -> Vec<ToolSpec> {
            self.0
                .iter()
                .map(|n| ToolSpec {
                    name: n.to_string(),
                    description: format!("the {n} tool"),
                    schema: match *n {
                        "edit_file" => serde_json::json!({"type": "object", "required": ["path", "old", "new"]}),
                        "shell" => serde_json::json!({"type": "object", "properties": {"commands": {}, "command": {}}}),
                        _ => serde_json::json!({"type": "object"}),
                    },
                })
                .collect()
        }
        fn knows(&self, name: &str) -> bool {
            self.0.contains(&name)
        }
    }

    #[test]
    fn deferred_call_form_and_sdk_prefixes_resolve_to_the_real_tool() {
        let dir = Fixed(vec!["web_fetch", "read_file"]);
        assert_eq!(
            resolve(
                "tool_call",
                serde_json::json!({"name": "web_fetch", "arguments": {"url": "u"}}),
                &dir
            ),
            Resolved::Call("web_fetch".into(), serde_json::json!({"url": "u"}))
        );
        assert_eq!(
            resolve("mcp__forge__read_file", serde_json::json!({}), &dir),
            Resolved::Call("read_file".into(), serde_json::json!({}))
        );
        // An unknown bare name is kept as-is so the broker answers it with the tool list.
        assert_eq!(
            resolve("functions.nope", serde_json::json!({}), &dir),
            Resolved::Call("functions.nope".into(), serde_json::json!({}))
        );
    }

    #[test]
    fn tool_call_uses_the_shared_unwrap() {
        let dir = Fixed(vec!["web_fetch"]);
        assert_eq!(
            resolve(
                "tool_call",
                serde_json::json!({"name": "web_fetch", "arguments": "{\"url\":\"u\"}"}),
                &dir
            ),
            Resolved::Call("web_fetch".into(), serde_json::json!({"url": "u"})),
            "escaped-string arguments decode like a plain tool_call"
        );
        assert!(matches!(
            resolve(
                "tool_call",
                serde_json::json!({"name": "tool_search"}),
                &dir
            ),
            Resolved::Denied(_)
        ));
    }

    #[test]
    fn update_tasks_is_a_plain_call_inside_a_script() {
        let dir = Fixed(vec![]);
        let args =
            serde_json::json!({"tasks": [{"title": "Read the spec", "status": "in_progress"}]});
        assert_eq!(
            resolve("update_tasks", args.clone(), &dir),
            Resolved::Call("update_tasks".into(), args)
        );
    }

    #[test]
    fn session_control_tools_are_refused_inside_a_script() {
        let dir = Fixed(vec![]);
        for name in ["run_code", "ask_user", "spawn_agents", "present_plan"] {
            assert!(
                matches!(resolve(name, Value::Null, &dir), Resolved::Denied(_)),
                "{name} must be refused"
            );
        }
    }

    #[test]
    fn search_prefers_exact_names_then_keywords() {
        let catalog = Fixed(vec!["web_fetch", "web_search", "read_file"]).catalog();
        let exact = search(&catalog, "web_fetch");
        assert_eq!(exact.as_array().unwrap().len(), 1);
        assert_eq!(exact[0]["name"], "web_fetch");
        assert!(exact[0]["schema"].is_object());
        let keyword = search(&catalog, "web");
        assert_eq!(keyword.as_array().unwrap().len(), 2);
        assert!(search(&catalog, "zzz").as_array().unwrap().is_empty());
    }

    #[test]
    fn a_bare_string_argument_becomes_the_primary_parameter() {
        let dir = Fixed(vec!["edit_file", "shell", "web_fetch"]);
        assert_eq!(
            resolve("shell", Value::String("ls".into()), &dir),
            Resolved::Call("shell".into(), serde_json::json!({"command": "ls"}))
        );
        assert_eq!(
            resolve("edit_file", Value::String("a.py".into()), &dir),
            Resolved::Call("edit_file".into(), serde_json::json!({"path": "a.py"}))
        );
        assert_eq!(
            resolve("shell", Value::String("{\"command\":\"ls\"}".into()), &dir),
            Resolved::Call("shell".into(), serde_json::json!({"command": "ls"})),
            "a JSON object in the string is parsed"
        );
        assert_eq!(
            resolve("web_fetch", Value::String("u".into()), &dir),
            Resolved::Call("web_fetch".into(), Value::String("u".into())),
            "no known parameter: passed through for the broker to reject"
        );
    }
}
