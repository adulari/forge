//! The deferred tool registry shared by the direct path and the CLI bridge
//! (docs/features/deferred-tools.md).
//!
//! Every advertised schema is resent on every request. Most turns use the core coding tools, so
//! only those stay advertised; the rest (device, browser, proxy, web, heartbeats, fleet, ...) are
//! listed by name with a short gloss inside `tool_search`'s description. `tool_search` returns the
//! full description, schema and an example call for the matching tools, and `tool_call` runs one by
//! name through the normal dispatch, so permissions, hooks, task scope and audit see the real tool.

use forge_provider::ToolSpec;
use serde_json::{json, Map, Value};

pub use forge_types::deferred_call::{TOOL_CALL, TOOL_SEARCH};

/// Advertised up front on both paths. Everything else is deferred behind [`TOOL_SEARCH`] /
/// [`TOOL_CALL`].
pub const CORE_TOOLS: &[&str] = &[
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
    crate::worktree_ops::WORKTREE_TOOL,
    TOOL_SEARCH,
    TOOL_CALL,
];

/// Also core on the direct path. The bridge has no `ask_user` (the CLI owns the conversation) and
/// its measured lean surface keeps `spawn_agents` deferred; FORGE_SYSTEM tells a direct model to
/// ask before pushing, and spawn_agents is the most-used platform tool (23 of 407 sessions).
/// `run_code` is direct-only too, and hiding it behind `tool_search` would defeat its purpose.
pub const DIRECT_CORE_EXTRA: &[&str] = &[
    "ask_user",
    "spawn_agents",
    crate::codemode::RUN_CODE_TOOL,
    forge_tools::PYTHON_TOOL,
];

/// External-MCP meta-tools that `tool_search`/`tool_call` replace on the direct path. They are
/// dropped from the catalog but stay callable by name (old transcripts, the orchestrate skill).
pub const MCP_ALIASES: &[&str] = &[forge_mcp::MCP_SEARCH_TOOLS, forge_mcp::MCP_CALL];

/// `FORGE_DIRECT_LEAN=1` forces the lean direct surface on, `0` off; unset defers to
/// `mesh.direct_lean`.
pub const DIRECT_LEAN_ENV: &str = "FORGE_DIRECT_LEAN";

pub fn lean_enabled_with(config_flag: bool, env_override: Option<&str>) -> bool {
    match env_override {
        Some("1") => true,
        Some("0") => false,
        _ => config_flag,
    }
}

pub fn is_core(name: &str) -> bool {
    CORE_TOOLS.contains(&name)
}

/// Split a full tool list into the advertised core and the deferred rest.
pub fn partition_by(
    specs: Vec<ToolSpec>,
    core: impl Fn(&str) -> bool,
) -> (Vec<ToolSpec>, Vec<ToolSpec>) {
    specs.into_iter().partition(|s| core(&s.name))
}

/// Longest gloss of a deferred tool in the catalog. Long enough to say what it drives, short enough
/// that ~25 deferred tools stay well under the cost of one advertised schema.
const GLOSS_CAP: usize = 64;

/// The two meta-tools that make `deferred` reachable. `None` when nothing is deferred.
/// `external_mcp` adds one sentence pointing at connected MCP servers' `server__tool` names.
pub fn meta_specs(deferred: &[ToolSpec], external_mcp: bool) -> Option<[ToolSpec; 2]> {
    if deferred.is_empty() && !external_mcp {
        return None;
    }
    let mut entries: Vec<(&str, String)> = deferred
        .iter()
        .map(|s| (s.name.as_str(), gloss(&s.description)))
        .collect();
    entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
    let catalog = entries
        .iter()
        .map(|(name, gloss)| {
            if gloss.is_empty() {
                (*name).to_string()
            } else {
                format!("{name} ({gloss})")
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    let mcp_note = if external_mcp {
        " Tools on connected MCP servers are named server__tool; search and run them the same way."
    } else {
        ""
    };
    let search = ToolSpec {
        name: TOOL_SEARCH.to_string(),
        description: format!(
            "Look up Forge tools that are not listed here. Returns each match's description, input \
             schema and an example; then run it with tool_call.{mcp_note} Not-listed tools: \
             {catalog}."
        ),
        schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Tool name or keywords; comma-separate several names."
                }
            },
            "required": ["query"]
        }),
    };
    let call = ToolSpec {
        name: TOOL_CALL.to_string(),
        description: "Run a tool found with tool_search by its exact name.".to_string(),
        schema: json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "arguments": { "type": "object", "description": "Per the tool's input schema." }
            },
            "required": ["name"]
        }),
    };
    Some([search, call])
}

/// The advertised surface: core tools slimmed, plus the meta-tools over the deferred rest.
pub fn lean_surface(
    core: Vec<ToolSpec>,
    deferred: &[ToolSpec],
    external_mcp: bool,
) -> Vec<ToolSpec> {
    let mut out: Vec<ToolSpec> = core.into_iter().map(slim).collect();
    out.extend(meta_specs(deferred, external_mcp).into_iter().flatten());
    out
}

/// Cap on a per-parameter description on an advertised core tool. The full text of rarely used
/// parameters (shell's poll/background modes) is re-ingested on every request for no benefit.
const PARAM_DESC_CAP: usize = 110;

/// Shorten every parameter description to its first sentence, at most [`PARAM_DESC_CAP`] bytes.
pub fn slim(mut spec: ToolSpec) -> ToolSpec {
    if let Some(Value::Object(props)) = spec.schema.get_mut("properties") {
        for prop in props.values_mut() {
            if let Some(Value::String(desc)) = prop.get_mut("description") {
                *desc = first_sentence(desc, PARAM_DESC_CAP);
            }
        }
    }
    spec
}

pub fn first_sentence(desc: &str, cap: usize) -> String {
    let end = desc
        .match_indices(". ")
        .map(|(i, _)| i + 1)
        .next()
        .unwrap_or(desc.len());
    let mut cut = end.min(cap).min(desc.len());
    while !desc.is_char_boundary(cut) {
        cut -= 1;
    }
    desc[..cut].trim_end().to_string()
}

fn gloss(desc: &str) -> String {
    let sentence = first_sentence(desc.trim(), usize::MAX);
    let sentence = sentence.trim_end_matches('.');
    if sentence.len() <= GLOSS_CAP {
        return sentence.to_string();
    }
    let mut cut = GLOSS_CAP;
    while !sentence.is_char_boundary(cut) {
        cut -= 1;
    }
    let head = &sentence[..cut];
    let head = head.rfind(' ').map_or(head, |i| &head[..i]);
    format!("{}…", head.trim_end_matches([',', ';', ':', ' ']))
}

/// Render the deferred tools matching `query`: exact names first, else keywords against
/// name/description. At most 8 blocks.
pub fn search(deferred: &[ToolSpec], query: &str) -> String {
    let hits = matches(deferred, query);
    if hits.is_empty() {
        let names: Vec<&str> = deferred.iter().map(|s| s.name.as_str()).collect();
        return format!("no tool matches '{query}'. Available: {}", names.join(", "));
    }
    hits.iter()
        .take(8)
        .map(|s| {
            format!(
                "## {}\n{}\ninput schema: {}\nexample: {}",
                s.name,
                s.description,
                s.schema,
                example_call(s)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The deferred tools `query` names or describes; empty when none match.
pub fn matches<'a>(deferred: &'a [ToolSpec], query: &str) -> Vec<&'a ToolSpec> {
    let terms: Vec<String> = query
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let exact: Vec<&ToolSpec> = deferred
        .iter()
        .filter(|s| terms.iter().any(|q| s.name.eq_ignore_ascii_case(q)))
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    deferred
        .iter()
        .filter(|s| {
            let hay = format!("{} {}", s.name, s.description).to_lowercase();
            terms.iter().any(|q| hay.contains(q.as_str()))
        })
        .collect()
}

/// `tool_call{"name":"web_fetch","arguments":{"url":"…"}}`, filled from the required fields.
fn example_call(spec: &ToolSpec) -> String {
    let props = spec.schema.get("properties").and_then(Value::as_object);
    let mut args = Map::new();
    for field in spec
        .schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let kind = props
            .and_then(|p| p.get(field))
            .and_then(|p| p.get("type"))
            .and_then(Value::as_str);
        let placeholder = match kind {
            Some("integer" | "number") => json!(0),
            Some("boolean") => json!(false),
            Some("object") => json!({}),
            Some("array") => json!([]),
            _ => json!("…"),
        };
        args.insert(field.to_string(), placeholder);
    }
    format!(
        "{TOOL_CALL}{}",
        json!({ "name": spec.name, "arguments": Value::Object(args) })
    )
}

/// The call a `tool_call{name, arguments}` stands for, keeping the original id so the provider's
/// tool_use/tool_result pairing holds. `None` when `call` is not a `tool_call`; `Some(Err)` when
/// it names nothing runnable.
pub fn unwrap_tool_call(
    call: &forge_types::ToolCall,
) -> Option<Result<forge_types::ToolCall, String>> {
    if call.name != TOOL_CALL {
        return None;
    }
    Some(
        forge_types::deferred_call::inner_call(&call.args).map(|(name, args)| {
            forge_types::ToolCall {
                id: call.id.clone(),
                name,
                args,
            }
        }),
    )
}

#[cfg(test)]
#[path = "tests/deferred_tools.rs"]
mod tests;
