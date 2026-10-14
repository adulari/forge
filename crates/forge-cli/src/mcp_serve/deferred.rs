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

/// Shorten every parameter description on `tool` to its first sentence (see [`shared::slim`]).
pub(super) fn slim(mut tool: Tool) -> Tool {
    let slimmed = shared::slim(to_spec(&tool));
    tool.input_schema = Arc::new(slimmed.schema.as_object().cloned().unwrap_or_default());
    tool
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
}
