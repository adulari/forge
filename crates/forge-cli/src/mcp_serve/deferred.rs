//! Deferred tool surface for the CLI bridge: a thin `rmcp::Tool` adapter over the registry the
//! direct path uses ([`forge_core::deferred_tools`]), so both paths share the core set, the
//! catalog text and the search ranking.
//!
//! A bridged CLI re-ingests every advertised tool schema on every request of ITS loop. The core
//! coding tools stay advertised; everything else is listed inside `tool_search`'s description and
//! loaded on demand: `tool_search` returns the full description and schema of the matching tools,
//! `tool_call` invokes one by name through the normal dispatch (so the permission gate, hooks and
//! bridge caps apply exactly as for an advertised tool).

use forge_core::deferred_tools as shared;
use forge_provider::ToolSpec;
use rmcp::model::{JsonObject, Tool};
use serde_json::Value;
use std::sync::Arc;

pub(super) use shared::{is_core, TOOL_CALL, TOOL_SEARCH};

fn to_spec(tool: &Tool) -> ToolSpec {
    ToolSpec {
        name: tool.name.to_string(),
        description: tool.description.as_deref().unwrap_or_default().to_string(),
        schema: Value::Object((*tool.input_schema).clone()),
    }
}

fn to_tool(spec: ToolSpec) -> Tool {
    let schema: JsonObject = spec.schema.as_object().cloned().unwrap_or_default();
    Tool::new(spec.name, spec.description, Arc::new(schema))
}

/// Split a full tool list into the advertised core and the deferred rest.
pub(super) fn partition(tools: Vec<Tool>) -> (Vec<Tool>, Vec<Tool>) {
    tools.into_iter().partition(|t| is_core(&t.name))
}

/// The two meta-tools that make the deferred tools reachable. `None` when nothing is deferred.
pub(super) fn meta_tools(deferred: &[Tool]) -> Option<[Tool; 2]> {
    let specs: Vec<ToolSpec> = deferred.iter().map(to_spec).collect();
    shared::meta_specs(&specs, false).map(|pair| pair.map(to_tool))
}

/// The `(name, arguments)` a bridge `tool_call` forwards to, through the same unwrap as the direct
/// path: arguments may arrive as an object, an escaped JSON string, or under `args`/`input`.
pub(super) fn unwrap_call(args: Value) -> Result<(String, Option<JsonObject>), String> {
    let wrapped = forge_types::ToolCall {
        id: String::new(),
        name: TOOL_CALL.to_string(),
        args,
    };
    match shared::unwrap_tool_call(&wrapped) {
        Some(Ok(inner)) => Ok((
            inner.name,
            match inner.args {
                Value::Object(arguments) => Some(arguments),
                _ => None,
            },
        )),
        Some(Err(error)) => Err(error),
        None => Err(format!("error: not a {TOOL_CALL}")),
    }
}

/// Shorten every parameter description on `tool` to its first sentence (see [`shared::slim`]).
pub(super) fn slim(mut tool: Tool) -> Tool {
    let slimmed = shared::slim(to_spec(&tool));
    tool.input_schema = Arc::new(slimmed.schema.as_object().cloned().unwrap_or_default());
    tool
}

const JOB_TOOL: &str = "shell_job";

/// `shell` tells the model that background jobs are managed with `shell_job`, a tool this lean
/// surface defers behind `tool_search`/`tool_call`. A model that takes the sentence literally calls
/// `mcp__forge__shell_job` and is told no such tool exists (a haiku run did, on d05). Point the
/// description at the route that works. Only call this when `shell_job` is deferred.
pub(super) fn route_job_tool(mut tool: Tool) -> Tool {
    if tool.name == "shell" && !is_core(JOB_TOOL) {
        if let Some(description) = tool.description.take() {
            tool.description = Some(
                description
                    .replace(
                        "the `shell_job` tool",
                        "`shell_job` (not listed here: load it with tool_search, run it with tool_call)",
                    )
                    .into(),
            );
        }
    }
    tool
}

/// The same correction for a started job's result, which names the follow-up call.
pub(super) fn route_job_hint(tool: &str, text: String) -> String {
    if tool != "shell" || is_core(JOB_TOOL) || !text.contains(forge_tools::JOB_HINT) {
        return text;
    }
    text.replace(
        forge_tools::JOB_HINT,
        &format!(
            "{JOB_TOOL} is not in your tool list: run it as tool_call {{\"name\": \"{JOB_TOOL}\", \
             \"arguments\": ...}}. Check on it with arguments "
        ),
    )
}

/// Render the deferred tools matching `query` (exact names, or keywords against name/description).
pub(super) fn search(deferred: &[Tool], query: &str) -> String {
    let specs: Vec<ToolSpec> = deferred.iter().map(to_spec).collect();
    shared::search(&specs, query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(value: Value) -> Arc<JsonObject> {
        Arc::new(value.as_object().cloned().unwrap_or_default())
    }

    fn tool(name: &'static str, desc: &'static str) -> Tool {
        Tool::new(name, desc, schema(json!({"type": "object"})))
    }

    #[test]
    fn partition_keeps_core_and_defers_the_rest() {
        let (core, deferred) = partition(vec![
            tool("read_file", "r"),
            tool("device", "d"),
            tool("browser", "b"),
            tool("shell", "s"),
        ]);
        let core: Vec<_> = core.iter().map(|t| t.name.to_string()).collect();
        assert_eq!(core, ["read_file", "shell"]);
        assert_eq!(deferred.len(), 2);
    }

    #[test]
    fn open_pr_is_reachable_on_the_bridge_but_deferred() {
        let registry = forge_tools::ToolRegistry::with_core_tools();
        assert!(registry.get(forge_tools::OPEN_PR_TOOL).is_some());
        assert!(!is_core(forge_tools::OPEN_PR_TOOL));
    }

    #[test]
    fn slim_cuts_parameter_descriptions_to_the_first_sentence() {
        let long = "Run it in the background. The job runs in its own session with a very long \
                    explanation that keeps going and going and going well past the cap.";
        let t = Tool::new(
            "shell",
            "d",
            schema(json!({"type": "object", "properties": {
                "background": {"type": "boolean", "description": long},
                "command": {"type": "string", "description": "POSIX command"},
                "bare": {"type": "string"}
            }})),
        );
        let slimmed = slim(t);
        let props = &slimmed.input_schema["properties"];
        assert_eq!(
            props["background"]["description"],
            "Run it in the background."
        );
        assert_eq!(props["command"]["description"], "POSIX command");
        assert!(props["bare"].get("description").is_none());
    }

    #[test]
    fn meta_tools_name_every_deferred_tool_and_are_absent_when_none() {
        assert!(meta_tools(&[]).is_none());
        let [search, call] = meta_tools(&[tool("device", "d"), tool("browser", "b")]).unwrap();
        assert_eq!(search.name, TOOL_SEARCH);
        assert_eq!(call.name, TOOL_CALL);
        let desc = search.description.unwrap();
        assert!(desc.contains("browser (b); device (d)"), "{desc}");
    }

    #[test]
    fn unwrap_call_matches_the_direct_path() {
        let (name, args) =
            unwrap_call(json!({"name": "web_fetch", "arguments": "{\"url\":\"https://x\"}"}))
                .unwrap();
        assert_eq!(name, "web_fetch");
        assert_eq!(args.unwrap()["url"], "https://x");
        let (_, args) = unwrap_call(json!({"name": "device", "args": {"action": "ui"}})).unwrap();
        assert_eq!(args.unwrap()["action"], "ui");
        assert!(unwrap_call(json!({"name": TOOL_SEARCH})).is_err());
        assert!(unwrap_call(json!({})).is_err());
    }

    #[test]
    fn search_prefers_exact_names_then_falls_back_to_keywords() {
        let deferred = [
            tool("browser", "drive a headless web browser"),
            tool("device", "control an android emulator"),
        ];
        let exact = search(&deferred, "device");
        assert!(exact.contains("## device") && !exact.contains("## browser"));
        let kw = search(&deferred, "headless");
        assert!(kw.contains("## browser"));
        let multi = search(&deferred, "device, browser");
        assert!(multi.contains("## device") && multi.contains("## browser"));
        assert!(search(&deferred, "nonsense").contains("Available: browser, device"));
    }

    fn real_shell() -> Tool {
        let registry = forge_tools::ToolRegistry::with_core_tools();
        let shell = registry.get("shell").unwrap();
        Tool::new(
            "shell",
            shell.description().to_string(),
            schema(shell.schema()),
        )
    }

    #[test]
    fn the_deferred_job_tool_is_reached_through_tool_call_not_by_name() {
        let original = real_shell().description.unwrap().to_string();
        assert!(
            original.contains("the `shell_job` tool"),
            "shell's own wording changed; update route_job_tool: {original}"
        );
        let routed = route_job_tool(slim(real_shell()))
            .description
            .unwrap()
            .to_string();
        assert!(!routed.contains("the `shell_job` tool"), "{routed}");
        assert!(
            routed.contains("tool_search") && routed.contains("tool_call"),
            "{routed}"
        );
        assert!(routed.contains("`shell_job`"), "{routed}");
    }

    #[test]
    fn only_shell_is_rewritten() {
        let other = tool("read_file", "manage it with the `shell_job` tool");
        assert_eq!(
            route_job_tool(other).description.unwrap(),
            "manage it with the `shell_job` tool"
        );
        let result = format!("note: {}x", forge_tools::JOB_HINT);
        assert_eq!(route_job_hint("read_file", result.clone()), result);
    }

    #[test]
    fn a_started_jobs_result_names_the_call_that_works() {
        let started = format!(
            "shell: started background job 7 (pid 7)\n  log: /tmp/7.log\n  {}{{\"action\": \"log\", \"id\": 7}}, stop it with {{\"action\": \"stop\", \"id\": 7}}. It keeps running until stopped.",
            forge_tools::JOB_HINT
        );
        let routed = route_job_hint("shell", started.clone());
        assert!(!routed.contains(forge_tools::JOB_HINT), "{routed}");
        assert!(routed.contains("tool_call"), "{routed}");
        assert!(routed.contains(r#"{"action": "log", "id": 7}"#), "{routed}");
        assert!(
            routed.contains(r#"{"action": "stop", "id": 7}"#),
            "{routed}"
        );
        assert!(
            routed.starts_with("shell: started background job 7"),
            "{routed}"
        );
        assert_eq!(
            route_job_hint("shell", "shell: exit 0".to_string()),
            "shell: exit 0"
        );
    }
}
