//! Name resolution and `tools.$search` for `run_code`.
//!
//! [`ToolDirectory`] is the one seam between codemode and "which tools exist". Today the session
//! implements it over its tool registry plus connected MCP tools, which is every tool a direct
//! model can call. When the shared deferred-tool registry lands (Build-later item 1), point the
//! `Session` impl at it: `catalog()` returns core plus deferred specs and `knows()` accepts a
//! deferred name, and nothing else in codemode changes.

use forge_provider::ToolSpec;
use serde_json::Value;

use crate::Session;

/// Tools a script may not call: they drive the session itself (prompts, plans, tasks, child
/// agents, scheduling) or would recurse. Everything else in the directory is callable.
const SCRIPT_DENY: &[&str] = &[
    super::RUN_CODE_TOOL,
    crate::workflow::RUN_WORKFLOW_TOOL,
    crate::subagent::SPAWN_AGENTS_TOOL,
    crate::subagent::SEND_TO_AGENT_TOOL,
    crate::subagent::LIST_SUBAGENTS_TOOL,
    crate::subagent::CANCEL_SUBAGENT_TOOL,
    crate::ASK_USER_TOOL,
    crate::PRESENT_PLAN_TOOL,
    crate::UPDATE_TASKS_TOOL,
    crate::USE_SKILL_TOOL,
    crate::REMEMBER_TOOL,
    crate::fleet::MESSAGE_SESSION_TOOL,
    crate::fleet::LIST_SESSIONS_TOOL,
    crate::dispatch::DISPATCH_SESSIONS_TOOL,
    crate::heartbeat::MANAGE_HEARTBEATS_TOOL,
    crate::wakeup::SCHEDULE_WAKEUP_TOOL,
    crate::push_notify::PUSH_NOTIFICATION_TOOL,
    "tool_search",
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

/// Resolve a script's call: unwrap `tool_call{name, arguments}` (the deferred registry's call
/// form), strip SDK prefixes when the bare name is a real tool, then apply the deny list.
pub(crate) fn resolve(name: &str, args: Value, dir: &impl ToolDirectory) -> Resolved {
    let (name, args) = if name == "tool_call" {
        let inner = args
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let inner_args = args
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        (inner, inner_args)
    } else {
        (name.to_string(), args)
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
    Resolved::Call(name, args)
}

/// `tools.$search(query)`: exact names first (comma or space separated), else keyword matches
/// against name and description. Returns up to eight `{name, description, schema}` objects.
pub(crate) fn search(catalog: &[ToolSpec], query: &str) -> Value {
    let terms: Vec<String> = query
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let exact: Vec<&ToolSpec> = catalog
        .iter()
        .filter(|t| terms.iter().any(|q| t.name.eq_ignore_ascii_case(q)))
        .collect();
    let hits: Vec<&ToolSpec> = if exact.is_empty() {
        catalog
            .iter()
            .filter(|t| {
                let hay = format!("{} {}", t.name, t.description).to_lowercase();
                terms.iter().any(|q| hay.contains(q.as_str()))
            })
            .collect()
    } else {
        exact
    };
    Value::Array(
        hits.into_iter()
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
                    schema: serde_json::json!({"type": "object"}),
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
}
