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
use forge_types::{EffortLevel, PermissionMode, SideEffect, TaskTier};
use serde_json::Value;

use super::{GENERAL_PURPOSE_AGENT, SPAWN_AGENTS_TOOL, SUBAGENT_SYSTEM};

/// [`spawn_agents_spec`] plus the named agent types loaded for this workspace, so the model sees
/// each type's description and tool set and can pick one with `agent` (Claude Code's
/// `subagent_type`).
pub fn spawn_agents_spec_for(max_agents: usize, project_root: &Path, agents_dir: &str) -> ToolSpec {
    let agents = forge_config::load_agents_layered(project_root, agents_dir);
    spawn_agents_spec_with(max_agents, &agents)
}

/// [`spawn_agents_spec_for`] over an already-loaded agent map.
pub fn spawn_agents_spec_with(max_agents: usize, agents: &HashMap<String, AgentDef>) -> ToolSpec {
    let mut spec = spawn_agents_spec(max_agents);
    let catalog = forge_config::agent_catalog(agents);
    if !catalog.is_empty() {
        spec.description
            .push_str(&format!("\nAvailable `agent` types:{catalog}"));
    }
    spec
}

/// The `ToolSpec` advertised to the parent so the model can call `spawn_agents`.
pub fn spawn_agents_spec(max_agents: usize) -> ToolSpec {
    ToolSpec {
        name: SPAWN_AGENTS_TOOL.to_string(),
        description: format!(
            "Delegate independent deliverables to child agents that work in isolated contexts and \
             are routed to the cheapest capable model. Use only when at least two child results \
             are independently useful. Do not use for routine repository exploration, code \
             search, test discovery, or review within one bug, feature, or refactor; direct tools \
             share context and are faster. Up to {max_agents} agents per call. Each agent has the \
             tools its `agent` type lists (see below; omitting `agent` gives general-purpose, with \
             every tool) under your own permission mode, and returns a concise result. Returns all results, labeled. \
             Set `detached: true` to admit the agents WITHOUT waiting: the call returns \
             immediately with each child's id/name/model, the children keep running in the \
             background, and their results are delivered as a labeled message the next time you \
             speak — use this for long-running work you don't need to block on. Check on a \
             detached batch with `list_subagents`, block on it with `wait_for_subagents`, \
             redirect a running one with `steer_subagent`, or stop one early with \
             `cancel_subagent`."
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
                                "description": "optional named agent type (also accepted as `subagent_type`); omit for general-purpose (all tools, can edit files and run commands under the session's permission mode); `explore` and `plan` are read-only"
                            },
                            "task": {
                                "type": "string",
                                "description": "the self-contained subtask for this agent"
                            },
                            "tags": {
                                "type": "array",
                                "maxItems": MAX_TAGS,
                                "items": {"type": "string"},
                                "description": "optional short labels for the kind of work (for example `refactor`, `tests`, `docs`); the router learns which models suit which kinds"
                            }
                        },
                        "required": ["task"]
                    }
                },
                "detached": {
                    "type": "boolean",
                    "description": "admit these agents immediately and return without waiting for them to finish (default false = block until all finish)"
                }
            },
            "required": ["agents"]
        }),
    }
}

/// Whether a `spawn_agents` call requested detached (fire-and-forget) admission. Applies to the
/// WHOLE call — every agent in one `spawn_agents(detached: true, agents: [...])` call is admitted
/// detached, not a per-agent choice.
pub fn parse_detached_flag(args: &serde_json::Value) -> bool {
    args.get("detached")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub const LIST_SUBAGENTS_TOOL: &str = "list_subagents";

/// The `ToolSpec` for listing every agent spawned this session — including detached children
/// still running in the background, with their live status.
pub fn list_subagents_spec() -> ToolSpec {
    ToolSpec {
        name: LIST_SUBAGENTS_TOOL.to_string(),
        description: "List the agents spawned this session via spawn_agents, including detached \
             children still running in the background. Shows each child's id, name, model, and \
             status (running/done/failed/cancelled) — use this to check on detached work before \
             deciding whether to wait, follow up, or cancel it."
            .to_string(),
        schema: serde_json::json!({ "type": "object", "properties": {} }),
    }
}

pub const CANCEL_SUBAGENT_TOOL: &str = "cancel_subagent";

/// The `ToolSpec` for stopping a still-running detached child early.
pub fn cancel_subagent_spec() -> ToolSpec {
    ToolSpec {
        name: CANCEL_SUBAGENT_TOOL.to_string(),
        description: "Cancel a detached agent spawned earlier with spawn_agents(detached: true) \
             that's still running. Address it by name or by the id prefix from its admission \
             handle or from list_subagents. A no-op (reported, not an error) if it already \
             finished."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": "the child's agent name from spawn_agents, or its session-id prefix"
                }
            },
            "required": ["agent"]
        }),
    }
}

/// What `spawn_agents(detached: true)` hands back immediately for one admitted child — enough for
/// the parent model to address it later via `send_to_agent`/`cancel_subagent`/`list_subagents`,
/// without waiting for it to run.
#[derive(Debug, Clone)]
pub struct AdmissionHandle {
    pub child_id: String,
    pub name: String,
    pub model: String,
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
    if arr.len() > max_agents {
        return Err(format!(
            "spawn_agents accepts at most {max_agents} agents per call (got {}); split the rest into a later call",
            arr.len()
        ));
    }
    let mut out = Vec::new();
    for entry in arr.iter() {
        let task = entry
            .get("task")
            .and_then(|t| t.as_str())
            .filter(|t| !t.trim().is_empty())
            .ok_or("each agent needs a non-empty `task`")?;
        let agent = entry
            .get("agent")
            .or_else(|| entry.get("subagent_type"))
            .and_then(|a| a.as_str())
            .filter(|a| !a.trim().is_empty())
            .unwrap_or(GENERAL_PURPOSE_AGENT)
            .to_string();
        let tags = parse_tags(entry);
        let task = if tags.is_empty() {
            task.to_string()
        } else {
            // The router classifies and learns from the task text, so the tags travel in it.
            format!("{task}\n\n(Task tags: {})", tags.join(", "))
        };
        out.push(AgentRequest { agent, task });
    }
    Ok(out)
}

/// Most tags kept per agent, and the longest one: labels, not sentences.
const MAX_TAGS: usize = 4;
const MAX_TAG_LEN: usize = 24;

/// The optional `tags` of one `spawn_agents` entry: lowercase `[a-z0-9_-]` words, deduplicated.
pub fn parse_tags(entry: &serde_json::Value) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for raw in entry
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let tag: String = raw
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .split('-')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-")
            .chars()
            .take(MAX_TAG_LEN)
            .collect();
        if !tag.is_empty() && !tags.contains(&tag) {
            tags.push(tag);
        }
        if tags.len() == MAX_TAGS {
            break;
        }
    }
    tags
}

/// A request resolved against the loaded agent types — owned so it can move into a spawned
/// task. A named agent supplies its system prompt / tool subset / pinned tier; an unknown or
/// inline (`general`) agent falls back to the `general-purpose` type: every tool, the parent's
/// temper.
#[derive(Debug, Clone, Default)]
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
    /// Tools the agent type removes from its tool set (`disallowedTools`).
    pub disallowed_tools: Vec<String>,
    /// The temper the agent type asks for; applied only when stricter than the parent's
    /// ([`narrowed_mode`]).
    pub mode: Option<PermissionMode>,
    /// The agent type's step cap (`maxTurns`), never above `[mesh] max_steps` when that is set.
    pub max_turns: Option<usize>,
    /// The reasoning rung for the child's model calls.
    pub effort: Option<EffortLevel>,
    /// `isolation: worktree`: isolate this child even when worktree isolation is off.
    pub worktree: bool,
    /// Skills to load into the child's system prompt ([`system_prompt_with_skills`]).
    pub skills: Vec<String>,
}

/// The agent type `name` refers to: an exact match, else a case-insensitive one, so Claude Code's
/// `Explore` and `Plan` find Forge's built-in `explore` and `plan`.
pub fn find_agent<'a>(agents: &'a HashMap<String, AgentDef>, name: &str) -> Option<&'a AgentDef> {
    agents.get(name).or_else(|| {
        agents
            .values()
            .find(|def| def.name.eq_ignore_ascii_case(name))
    })
}

/// The temper a child runs under: the agent type's mode when it is stricter than the parent's,
/// else the parent's. An agent file never widens what a child may do unprompted.
pub fn narrowed_mode(parent: PermissionMode, requested: Option<PermissionMode>) -> PermissionMode {
    let rank = |mode: PermissionMode| match mode {
        PermissionMode::Plan => 0,
        PermissionMode::Default => 1,
        PermissionMode::Auto | PermissionMode::AcceptEdits => 2,
        PermissionMode::Bypass => 3,
    };
    match requested {
        Some(mode) if rank(mode) < rank(parent) => mode,
        _ => parent,
    }
}

/// The tool names the child is offered: the agent's own list (or, when it names none, every tool in
/// `registry`, as a Claude Code agent file without `tools:` inherits them all) minus its
/// `disallowedTools`. `spawn_agents` is never in it; recursion is offered separately.
pub fn effective_tools(agent: &ResolvedAgent, registry: &ToolRegistry) -> Vec<String> {
    let base: Vec<String> = if agent.tools.is_empty() {
        let mut all: Vec<String> = registry.names().map(str::to_string).collect();
        all.sort();
        all
    } else {
        agent.tools.clone()
    };
    base.into_iter()
        .filter(|t| t != SPAWN_AGENTS_TOOL && !agent.disallowed_tools.iter().any(|d| d == t))
        .collect()
}

/// The child's system prompt: the agent type's prompt followed by each of its `skills` (Claude
/// Code's preloaded skills). A skill the model may not load (`disable-model-invocation`, or a
/// project skill in an untrusted project) is skipped, as it would be for `use_skill`.
pub fn system_prompt_with_skills(
    agent: &ResolvedAgent,
    repo_root: &Path,
    allow_project: bool,
) -> String {
    let mut prompt = agent.system_prompt.clone();
    if agent.skills.is_empty() {
        return prompt;
    }
    let catalog = forge_skills::Catalog::load(&forge_config::command_sources_in(repo_root));
    for name in &agent.skills {
        match catalog.skill_guidance(name, allow_project) {
            Some(guidance) => prompt.push_str(&format!("\n\n--- skill: {name} ---\n{guidance}")),
            None => tracing::warn!("agent {}: skill '{name}' is not available", agent.name),
        }
    }
    prompt
}

/// Resolve a parsed request against the loaded agent-type map (RFC subagent-orchestration Ph2).
pub fn resolve(req: &AgentRequest, agents: &HashMap<String, AgentDef>) -> ResolvedAgent {
    let named = find_agent(agents, &req.agent);
    let def = named.or_else(|| agents.get(GENERAL_PURPOSE_AGENT));
    match def {
        Some(def) => ResolvedAgent {
            name: if named.is_some() {
                def.name.clone()
            } else {
                req.agent.clone()
            },
            task: req.task.clone(),
            system_prompt: if def.system_prompt.is_empty() {
                SUBAGENT_SYSTEM.to_string()
            } else {
                def.system_prompt.clone()
            },
            tools: def.tools.clone(),
            tier: def.tier,
            pinned_model: def.pinned_model.clone(),
            disallowed_tools: def.disallowed_tools.clone(),
            mode: def.permission_mode,
            max_turns: def.max_turns,
            effort: def.effort,
            worktree: def.worktree,
            skills: def.skills.clone(),
        },
        None => ResolvedAgent {
            name: req.agent.clone(),
            task: req.task.clone(),
            system_prompt: SUBAGENT_SYSTEM.to_string(),
            ..ResolvedAgent::default()
        },
    }
}

/// Returns `true` when any of the agent's resolved tools is write- or shell-capable, meaning
/// concurrent execution could corrupt the shared working tree.
/// A child in `plan` mode cannot write whatever its tools are, so it needs no worktree.
pub fn is_write_capable(agent: &ResolvedAgent, registry: &ToolRegistry) -> bool {
    if agent.mode == Some(PermissionMode::Plan) {
        return false;
    }
    effective_tools(agent, registry).iter().any(|name| {
        registry
            .get(name.as_str())
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

    // Multi-file edits carry a path per edit.
    if let Some(Value::Array(edits)) = out.get_mut("edits") {
        for edit in edits {
            if let Some(Value::String(path)) = edit.get_mut("path") {
                if Path::new(path.as_str()).is_relative() {
                    *path = root.join(path.as_str()).display().to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn agents() -> HashMap<String, AgentDef> {
        forge_config::load_agents_layered(Path::new("/nonexistent-forge-root"), ".forge/agents")
    }

    #[test]
    fn claude_code_type_names_find_the_builtins() {
        let agents = agents();
        let explore = resolve(
            &AgentRequest {
                agent: "Explore".into(),
                task: "where is main".into(),
            },
            &agents,
        );
        assert_eq!(explore.name, "explore");
        assert_eq!(explore.mode, Some(PermissionMode::Plan));
        assert_eq!(explore.tier, Some(TaskTier::Trivial));
        assert!(find_agent(&agents, "PLAN").is_some());
        let general = resolve(
            &AgentRequest {
                agent: "general-purpose".into(),
                task: "t".into(),
            },
            &agents,
        );
        assert!(general.tools.is_empty());
        assert_eq!(general.mode, None);
    }

    #[test]
    fn agent_modes_only_narrow() {
        use PermissionMode::*;
        assert_eq!(narrowed_mode(AcceptEdits, Some(Plan)), Plan);
        assert_eq!(narrowed_mode(Bypass, Some(Default)), Default);
        assert_eq!(narrowed_mode(Default, Some(Bypass)), Default);
        assert_eq!(narrowed_mode(Auto, Some(AcceptEdits)), Auto);
        assert_eq!(narrowed_mode(Plan, Some(Auto)), Plan);
        assert_eq!(narrowed_mode(AcceptEdits, None), AcceptEdits);
    }

    #[test]
    fn disallowed_tools_leave_the_tool_set() {
        let mut agent = ResolvedAgent {
            tools: vec!["read_file".into(), "shell".into(), SPAWN_AGENTS_TOOL.into()],
            disallowed_tools: vec!["shell".into()],
            ..ResolvedAgent::default()
        };
        let registry = ToolRegistry::with_core_tools();
        assert_eq!(effective_tools(&agent, &registry), ["read_file"]);
        agent.tools.clear();
        agent.disallowed_tools = vec!["write_file".into(), "edit_file".into()];
        let tools = effective_tools(&agent, &registry);
        assert!(tools.iter().any(|t| t == "read_file"));
        assert!(!tools.iter().any(|t| t == "write_file" || t == "edit_file"));
        assert!(tools.iter().any(|t| t == "shell"));
    }

    #[test]
    fn plan_mode_agents_never_need_a_worktree() {
        let registry = ToolRegistry::with_core_tools();
        let mut agent = ResolvedAgent {
            tools: vec!["write_file".into()],
            ..ResolvedAgent::default()
        };
        assert!(is_write_capable(&agent, &registry));
        agent.mode = Some(PermissionMode::Plan);
        assert!(!is_write_capable(&agent, &registry));
        agent.mode = None;
        agent.disallowed_tools = vec!["write_file".into()];
        assert!(!is_write_capable(&agent, &registry));
    }

    #[test]
    fn resolve_carries_the_claude_code_fields() {
        let mut agents = HashMap::new();
        agents.insert(
            "auditor".to_string(),
            AgentDef {
                name: "auditor".into(),
                max_turns: Some(4),
                effort: Some(EffortLevel::High),
                worktree: true,
                skills: vec!["no-such-skill-for-this-test".into()],
                ..AgentDef::default()
            },
        );
        let r = resolve(
            &AgentRequest {
                agent: "auditor".into(),
                task: "t".into(),
            },
            &agents,
        );
        assert_eq!(r.max_turns, Some(4));
        assert_eq!(r.effort, Some(EffortLevel::High));
        assert!(r.worktree);
        assert_eq!(r.skills, ["no-such-skill-for-this-test"]);
        // An unknown skill is skipped: the prompt is the agent's own, unchanged.
        assert_eq!(
            system_prompt_with_skills(&r, Path::new("/nonexistent-forge-root"), false),
            r.system_prompt
        );
    }

    #[test]
    fn tags_are_normalized_bounded_and_ride_in_the_task() {
        let args = serde_json::json!({"agents": [{
            "task": "tidy the parser",
            "tags": ["Refactor", "refactor", " Tests & Docs ", "a", "b", "c", ""]
        }]});
        assert_eq!(
            parse_tags(&args["agents"][0]),
            ["refactor", "tests-docs", "a", "b"]
        );
        let reqs = parse_requests(&args, 4).unwrap();
        assert!(reqs[0].task.starts_with("tidy the parser"));
        assert!(reqs[0]
            .task
            .contains("Task tags: refactor, tests-docs, a, b"));
        let plain = parse_requests(&serde_json::json!({"agents": [{"task": "x"}]}), 4).unwrap();
        assert_eq!(plain[0].task, "x");
    }
}
