//! Deferred tool surface for the CLI bridge.
//!
//! A bridged CLI re-ingests every advertised tool schema on every request of ITS loop, and Forge's
//! full surface (device/browser/proxy/subagents/MCP meta-tools/…) is ~15K tokens. Most turns use
//! six of them. The core coding tools stay advertised; everything else is listed by name inside
//! `tool_search`'s description and loaded on demand: `tool_search` returns the full description and
//! schema of the matching tools, `tool_call` invokes one by name through the normal dispatch (so
//! the permission gate, hooks and bridge caps apply exactly as for an advertised tool).

use rmcp::model::{JsonObject, Tool};
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) const TOOL_SEARCH: &str = "tool_search";
pub(super) const TOOL_CALL: &str = "tool_call";

/// Tools advertised up front on a lean bridge. Everything else is deferred behind
/// [`TOOL_SEARCH`]/[`TOOL_CALL`].
pub(super) const CORE_TOOLS: &[&str] = &[
    "shell",
    "read_file",
    "edit_file",
    "multi_edit",
    "apply_patch",
    "write_file",
    "append_file",
    "delete_file",
    "search",
    "glob",
    "list_dir",
    "update_tasks",
    "present_plan",
    "use_skill",
    "dispatch_sessions",
    TOOL_SEARCH,
    TOOL_CALL,
];

pub(super) fn is_core(name: &str) -> bool {
    CORE_TOOLS.contains(&name)
}

/// Split a full tool list into the advertised core and the deferred rest.
pub(super) fn partition(tools: Vec<Tool>) -> (Vec<Tool>, Vec<Tool>) {
    tools.into_iter().partition(|t| is_core(&t.name))
}

/// The two meta-tools that make the deferred tools reachable. `None` when nothing is deferred.
pub(super) fn meta_tools(deferred: &[Tool]) -> Option<[Tool; 2]> {
    if deferred.is_empty() {
        return None;
    }
    let mut names: Vec<&str> = deferred.iter().map(|t| t.name.as_ref()).collect();
    names.sort_unstable();
    let search = Tool::new(
        TOOL_SEARCH,
        format!(
            "Look up Forge tools that are not listed here. Returns each match's description and \
             input schema; then run it with tool_call. Not-listed tools: {}.",
            names.join(", ")
        ),
        schema(json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Tool name or keywords; comma-separate several names."
                }
            },
            "required": ["query"]
        })),
    );
    let call = Tool::new(
        TOOL_CALL,
        "Run a tool found with tool_search by its exact name.",
        schema(json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "arguments": { "type": "object", "description": "Per the tool's input schema." }
            },
            "required": ["name"]
        })),
    );
    Some([search, call])
}

/// Cap on a per-parameter description on an advertised core tool. The full text of rarely used
/// parameters (shell's poll/background modes) is re-ingested on every request for no benefit.
const PARAM_DESC_CAP: usize = 110;

/// Shorten every parameter description on `tool` to its first sentence, at most
/// [`PARAM_DESC_CAP`] bytes.
pub(super) fn slim(mut tool: Tool) -> Tool {
    let mut schema = (*tool.input_schema).clone();
    if let Some(Value::Object(props)) = schema.get_mut("properties") {
        for prop in props.values_mut() {
            if let Some(Value::String(desc)) = prop.get_mut("description") {
                *desc = first_sentence(desc);
            }
        }
    }
    tool.input_schema = Arc::new(schema);
    tool
}

fn first_sentence(desc: &str) -> String {
    let end = desc
        .match_indices(". ")
        .map(|(i, _)| i + 1)
        .next()
        .unwrap_or(desc.len());
    let mut cut = end.min(PARAM_DESC_CAP).min(desc.len());
    while !desc.is_char_boundary(cut) {
        cut -= 1;
    }
    desc[..cut].trim_end().to_string()
}

fn schema(value: Value) -> Arc<JsonObject> {
    Arc::new(value.as_object().cloned().unwrap_or_default())
}

/// Render the deferred tools matching `query` (exact names, or keywords against name/description).
pub(super) fn search(deferred: &[Tool], query: &str) -> String {
    let terms: Vec<String> = query
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let exact: Vec<&Tool> = deferred
        .iter()
        .filter(|t| terms.iter().any(|q| t.name.eq_ignore_ascii_case(q)))
        .collect();
    let hits: Vec<&Tool> = if exact.is_empty() {
        deferred
            .iter()
            .filter(|t| {
                let hay = format!(
                    "{} {}",
                    t.name.to_lowercase(),
                    t.description.as_deref().unwrap_or_default().to_lowercase()
                );
                terms.iter().any(|q| hay.contains(q.as_str()))
            })
            .collect()
    } else {
        exact
    };
    if hits.is_empty() {
        let names: Vec<&str> = deferred.iter().map(|t| t.name.as_ref()).collect();
        return format!("no tool matches '{query}'. Available: {}", names.join(", "));
    }
    hits.iter()
        .take(8)
        .map(|t| {
            format!(
                "## {}\n{}\ninput schema: {}",
                t.name,
                t.description.as_deref().unwrap_or_default(),
                Value::Object((*t.input_schema).clone())
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(desc.contains("browser, device"), "{desc}");
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
