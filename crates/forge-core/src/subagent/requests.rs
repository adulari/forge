//! What the parent asked for, and what it resolves to.
//!
//! Delegation starts as a model-authored `spawn_agents` / `send_to_agent` call, which is untrusted
//! input: the count, the agent type, and the addressed child all have to be validated before any
//! child session exists. This module owns that front half — the advertised tool specs, request
//! parsing and clamping, agent-type resolution (including the read-only default tool set), the
//! write-capability check that decides whether a child needs its own worktree, and the path
//! rewriting that keeps a worktree child's tool arguments inside its own root. Running the child
//! is somebody else's job.

use std::collections::HashMap;
use std::path::Path;

use forge_config::AgentDef;
use forge_provider::ToolSpec;
use forge_tools::ToolRegistry;
use forge_types::{SideEffect, TaskTier};
use serde_json::Value;

use super::{SPAWN_AGENTS_TOOL, SUBAGENT_SYSTEM, SUBAGENT_TOOLS};

/// The `ToolSpec` advertised to the parent so the model can call `spawn_agents`.
pub fn spawn_agents_spec(max_agents: usize) -> ToolSpec {
    ToolSpec {
        name: SPAWN_AGENTS_TOOL.to_string(),
        description: format!(
            "Delegate independent deliverables to child agents that work in isolated contexts and \
             are routed to the cheapest capable model. Use only when at least two child results \
             are independently useful. Do not use for routine repository exploration, code \
             search, test discovery, or review within one bug, feature, or refactor; direct tools \
             share context and are faster. Up to {max_agents} agents per call. Each agent gets \
             read-only tools and returns a concise result. Returns all results, labeled."
        ),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "agents": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": max_agents,
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent": {
                                "type": "string",
                                "description": "optional named agent type; omit for a general read-only agent"
                            },
                            "task": {
                                "type": "string",
                                "description": "the self-contained subtask for this agent"
                            }
                        },
                        "required": ["task"]
                    }
                }
            },
            "required": ["agents"]
        }),
    }
}

pub const SEND_TO_AGENT_TOOL: &str = "send_to_agent";

/// The `ToolSpec` for following up with a child agent spawned earlier this session (or in a
/// previous turn of it) — the persistent-subagents half of the orchestration surface.
pub fn send_to_agent_spec() -> ToolSpec {
    ToolSpec {
        name: SEND_TO_AGENT_TOOL.to_string(),
        description: "Send a follow-up message to a child agent you spawned earlier with \
             spawn_agents. The child keeps its full previous context (its investigation, its \
             findings), so use this for iterative refinement — a clarifying question, a deeper \
             dive on one finding — instead of re-spawning and re-explaining from scratch. \
             Address it by the agent name used at spawn time (e.g. 'researcher') or a prefix \
             of its child-session id. Returns the child's answer."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "the child's agent name from spawn_agents, or its session-id prefix; with duplicate names the most recent child answers"
                },
                "message": {
                    "type": "string",
                    "description": "the follow-up message for the child"
                }
            },
            "required": ["agent", "message"]
        }),
    }
}

/// Resolve a `send_to_agent` address against a parent's children: exact title (agent-name)
/// match wins with most-recent-first tiebreak, then an id-prefix match. Pure — unit-testable.
pub fn resolve_child_address(
    children: &[(String, Option<String>)],
    address: &str,
) -> Option<(String, String)> {
    if let Some((id, name)) = children
        .iter()
        .rev()
        .find(|(_, title)| title.as_deref() == Some(address))
    {
        return Some((id.clone(), name.clone().unwrap_or_default()));
    }
    let mut prefix_matches = children.iter().filter(|(id, _)| id.starts_with(address));
    match (prefix_matches.next(), prefix_matches.next()) {
        (Some((id, name)), None) => Some((id.clone(), name.clone().unwrap_or_default())),
        _ => None,
    }
}

/// One requested child agent, parsed from the `spawn_agents` arguments.
#[derive(Debug, Clone)]
pub struct AgentRequest {
    pub agent: String,
    pub task: String,
}

/// Parse `spawn_agents` arguments into child requests, capped at `max_agents`. Returns an
/// `Err(message)` describing the problem in model-readable form if the shape is wrong.
pub fn parse_requests(
    args: &serde_json::Value,
    max_agents: usize,
) -> Result<Vec<AgentRequest>, String> {
    let arr = args
        .get("agents")
        .and_then(|a| a.as_array())
        .ok_or("spawn_agents requires an `agents` array")?;
    if arr.is_empty() {
        return Err("spawn_agents `agents` must not be empty".into());
    }
    let mut out = Vec::new();
    for entry in arr.iter().take(max_agents) {
        let task = entry
            .get("task")
            .and_then(|t| t.as_str())
            .filter(|t| !t.trim().is_empty())
            .ok_or("each agent needs a non-empty `task`")?;
        let agent = entry
            .get("agent")
            .and_then(|a| a.as_str())
            .filter(|a| !a.trim().is_empty())
            .unwrap_or("general")
            .to_string();
        out.push(AgentRequest {
            agent,
            task: task.to_string(),
        });
    }
    Ok(out)
}

/// A request resolved against the loaded agent types — owned so it can move into a spawned
/// task. A named agent supplies its system prompt / tool subset / pinned tier; an unknown or
/// inline (`general`) agent falls back to the default read-only investigator.
#[derive(Debug, Clone)]
pub struct ResolvedAgent {
    pub name: String,
    pub task: String,
    pub system_prompt: String,
    pub tools: Vec<String>,
    pub tier: Option<TaskTier>,
    /// A specific model pinned for this child, bypassing the mesh entirely (used by `/duel`, where
    /// each candidate MUST run the exact model the arena picked, not whatever the mesh would
    /// independently route the task to). `None` = route normally (the `tier` pin still applies).
    pub pinned_model: Option<String>,
}

/// Resolve a parsed request against the loaded agent-type map (RFC subagent-orchestration Ph2).
pub fn resolve(req: &AgentRequest, agents: &HashMap<String, AgentDef>) -> ResolvedAgent {
    match agents.get(&req.agent) {
        Some(def) => ResolvedAgent {
            name: def.name.clone(),
            task: req.task.clone(),
            system_prompt: if def.system_prompt.is_empty() {
                SUBAGENT_SYSTEM.to_string()
            } else {
                def.system_prompt.clone()
            },
            tools: def.tools.clone(),
            tier: def.tier,
            pinned_model: None,
        },
        None => ResolvedAgent {
            name: req.agent.clone(),
            task: req.task.clone(),
            system_prompt: SUBAGENT_SYSTEM.to_string(),
            tools: Vec::new(),
            tier: None,
            pinned_model: None,
        },
    }
}

/// Returns `true` when any of the agent's resolved tools is write- or shell-capable, meaning
/// concurrent execution could corrupt the shared working tree.
pub fn is_write_capable(agent: &ResolvedAgent, registry: &ToolRegistry) -> bool {
    let tool_names: Vec<&str> = if agent.tools.is_empty() {
        SUBAGENT_TOOLS.to_vec()
    } else {
        agent.tools.iter().map(String::as_str).collect()
    };
    tool_names.iter().any(|name| {
        registry
            .get(name)
            .map(|t| matches!(t.side_effect(), SideEffect::Write | SideEffect::Shell))
            .unwrap_or(false)
    })
}

/// Rewrite tool call arguments so that relative or absent paths/cwd are rooted inside `root`.
/// This is used for every child tool call: read-only children use the parent session's repository
/// root, while isolated write-capable children use their dedicated worktree. Absolute paths are
/// left alone.
/// - For `path` args: if the value is relative, make it absolute under `root`.
/// - For `cwd` args on shell calls: if absent or relative, set it to `root`.
pub fn rewrite_args_for_root(args: &Value, root: &Path) -> Value {
    let Some(map) = args.as_object() else {
        return args.clone();
    };
    let mut out = map.clone();

    // Rewrite "path" field.
    if let Some(Value::String(p)) = out.get("path") {
        let pb = Path::new(p);
        if pb.is_relative() {
            let abs = root.join(pb);
            out.insert(
                "path".into(),
                Value::String(abs.to_string_lossy().into_owned()),
            );
        }
    }

    // Rewrite batched read paths too.
    if let Some(Value::Array(paths)) = out.get_mut("paths") {
        for path in paths {
            if let Value::String(path) = path {
                let pb = Path::new(path);
                if pb.is_relative() {
                    *path = root.join(pb).display().to_string();
                }
            }
        }
    }

    // Rewrite "cwd" field (shell tool); inject worktree_root when absent.
    match out.get("cwd") {
        None => {
            out.insert(
                "cwd".into(),
                Value::String(root.to_string_lossy().into_owned()),
            );
        }
        Some(Value::String(cwd)) if Path::new(cwd).is_relative() => {
            let abs = root.join(cwd);
            out.insert(
                "cwd".into(),
                Value::String(abs.to_string_lossy().into_owned()),
            );
        }
        _ => {}
    }

    Value::Object(out)
}
