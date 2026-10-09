//! Named subagent types loaded from `.forge/agents/<name>.md` (RFC subagent-orchestration,
//! Phase 2). Each file is a small front-matter block followed by the agent's system prompt:
//!
//! ```text
//! ---
//! name: reviewer
//! description: Reviews a code change for bugs and risk.
//! tools: [read_file, list_dir, search]   # optional; omit → default read-only set
//! tier: standard                          # optional; omit → mesh-routed per task
//! model: sonnet                           # optional; haiku/sonnet/opus pick a tier, `provider::model` pins
//! ---
//! You are a meticulous code reviewer. ...
//! ```
//!
//! Claude Code agent files (`.claude/agents/*.md`) load unchanged: `tools: Read, Grep, Bash` is a
//! bare comma list of Claude tool names, which [`map_tool_name`] translates to Forge's.
//! [`load_agents_layered`] merges `~/.claude/agents`, `~/.forge/agents`, the project's
//! `.claude/agents` and the project's configured `agents_dir`, later layers overriding earlier
//! ones by name.
//!
//! Parsing is dependency-free (a tiny front-matter reader, not full YAML) so a malformed file
//! degrades to being skipped rather than failing the whole load.

use std::collections::HashMap;
use std::path::Path;

use forge_types::TaskTier;

/// A reusable subagent type. The `system_prompt` is the file body; `tools`/`tier` are optional
/// overrides resolved by the orchestrator (empty `tools` → the default read-only set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDef {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub tier: Option<TaskTier>,
    /// A hard model pin from `model: provider::model`. Claude's `haiku`/`sonnet`/`opus` aliases
    /// become a [`tier`](Self::tier) instead (the mesh still picks the model), and `inherit` is
    /// the default.
    pub pinned_model: Option<String>,
    pub system_prompt: String,
}

/// Merge every agent-definition directory a user might keep, lowest precedence first:
/// `~/.claude/agents`, `~/.forge/agents`, `<project>/.claude/agents`, then `agents_dir`
/// (relative to `project_root` unless absolute). A later definition replaces an earlier one with
/// the same name, so a project file overrides a personal one.
pub fn load_agents_layered(project_root: &Path, agents_dir: &str) -> HashMap<String, AgentDef> {
    let dirs = layer_dirs(crate::home_dir().as_deref(), project_root, agents_dir);
    let mut out = HashMap::new();
    for dir in dirs {
        out.extend(load_agents(&dir));
    }
    out
}

fn layer_dirs(
    home: Option<&Path>,
    project_root: &Path,
    agents_dir: &str,
) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home {
        dirs.push(home.join(".claude/agents"));
        dirs.push(home.join(".forge/agents"));
    }
    dirs.push(project_root.join(".claude/agents"));
    dirs.push(project_root.join(agents_dir));
    dirs
}

/// The agent-type list appended to the `spawn_agents` tool description, so the model knows which
/// `agent` values exist and when to pick each. Empty when no named agent is loaded.
pub fn agent_catalog(agents: &HashMap<String, AgentDef>) -> String {
    let mut defs: Vec<&AgentDef> = agents.values().collect();
    defs.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = String::new();
    for def in defs {
        let desc = if def.description.is_empty() {
            "(no description)"
        } else {
            def.description.as_str()
        };
        let tools = if def.tools.is_empty() {
            "read-only tools".to_string()
        } else {
            def.tools.join(", ")
        };
        out.push_str(&format!("\n- {}: {desc} [{tools}]", def.name));
    }
    out
}

/// Translate a Claude Code tool name to Forge's. Forge names pass through; Claude tools Forge has
/// no equivalent for (`Task`, `TodoWrite`, `mcp__…`) map to `None` and are dropped.
pub fn map_tool_name(name: &str) -> Option<String> {
    let mapped = match name.trim().to_ascii_lowercase().as_str() {
        "read" => "read_file",
        "grep" => "search",
        "glob" => "glob",
        "ls" => "list_dir",
        "bash" => "shell",
        "write" => "write_file",
        "edit" => "edit_file",
        "multiedit" => "multi_edit",
        "notebookedit" => "notebook_edit",
        "webfetch" => "web_fetch",
        "websearch" => "web_search",
        "task" | "todowrite" | "todoread" | "exitplanmode" | "bashoutput" | "killshell" => {
            return None
        }
        n if n.starts_with("mcp__") => return None,
        _ => return Some(name.trim().to_string()),
    };
    Some(mapped.to_string())
}

/// Load every `*.md` agent definition in `dir`, keyed by `name` (falling back to the file stem).
/// A missing directory or an unparseable file is skipped, never an error — agent types are
/// optional convenience config.
pub fn load_agents(dir: &Path) -> HashMap<String, AgentDef> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("agent")
            .to_string();
        if let Some(def) = parse_agent(&text, &stem) {
            out.insert(def.name.clone(), def);
        }
    }
    out
}

/// Parse one agent file's text. `default_name` is used when the front matter omits `name`.
/// Returns `None` if there is no front-matter block.
pub fn parse_agent(text: &str, default_name: &str) -> Option<AgentDef> {
    let rest = text.strip_prefix("---")?;
    // Split on the closing fence; everything after it is the system prompt body.
    let (front, body) = split_front_matter(rest)?;

    let mut name = default_name.to_string();
    let mut description = String::new();
    let mut tools = Vec::new();
    let mut tier = None;
    let mut model = None;

    for line in front.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "name" if !value.is_empty() => name = unquote(value),
            "description" => description = unquote(value),
            "tools" => tools = parse_list(value),
            "tier" => tier = parse_tier(value),
            "model" => model = Some(unquote(value)),
            _ => {}
        }
    }

    let (model_tier, pinned_model) = parse_model(model.as_deref().unwrap_or(""));
    Some(AgentDef {
        name,
        description,
        tools: tools.iter().filter_map(|t| map_tool_name(t)).collect(),
        tier: tier.or(model_tier),
        pinned_model,
        system_prompt: body.trim().to_string(),
    })
}

/// Split `---`-prefixed-stripped text into (front_matter, body) on the next `---` line.
fn split_front_matter(after_open: &str) -> Option<(&str, &str)> {
    // The content after the opening fence; find a line that is exactly `---`.
    let mut idx = 0;
    for line in after_open.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']).trim() == "---" {
            let front = &after_open[..idx];
            let body = &after_open[idx + line.len()..];
            return Some((front, body));
        }
        idx += line.len();
    }
    None
}

fn unquote(s: &str) -> String {
    s.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

/// Parse `[a, b, c]` or a bare comma list into items.
fn parse_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(unquote)
        .filter(|s| !s.is_empty())
        .collect()
}

/// `model:` → (tier, pin). Claude's aliases choose a tier; `provider::model` pins; anything else
/// (`inherit`, a bare Anthropic id Forge cannot route) leaves the child to the mesh.
fn parse_model(value: &str) -> (Option<TaskTier>, Option<String>) {
    let v = value.trim();
    if v.contains("::") {
        return (None, Some(v.to_string()));
    }
    match v.to_ascii_lowercase().as_str() {
        "haiku" => (Some(TaskTier::Trivial), None),
        "sonnet" => (Some(TaskTier::Standard), None),
        "opus" => (Some(TaskTier::Complex), None),
        _ => (None, None),
    }
}

fn parse_tier(value: &str) -> Option<TaskTier> {
    match unquote(value).to_lowercase().as_str() {
        "trivial" => Some(TaskTier::Trivial),
        "standard" => Some(TaskTier::Standard),
        "complex" => Some(TaskTier::Complex),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_front_matter_and_body() {
        let text = "---\nname: reviewer\ndescription: Reviews a change.\ntools: [read_file, search]\ntier: standard\n---\nYou are a reviewer.\nBe terse.";
        let def = parse_agent(text, "fallback").unwrap();
        assert_eq!(def.name, "reviewer");
        assert_eq!(def.description, "Reviews a change.");
        assert_eq!(def.tools, vec!["read_file", "search"]);
        assert_eq!(def.tier, Some(TaskTier::Standard));
        assert_eq!(def.system_prompt, "You are a reviewer.\nBe terse.");
    }

    #[test]
    fn omitted_fields_default_sensibly() {
        let text = "---\ndescription: just a body\n---\nDo the thing.";
        let def = parse_agent(text, "myagent").unwrap();
        assert_eq!(def.name, "myagent"); // falls back to file stem
        assert!(def.tools.is_empty()); // → default read-only set, decided in core
        assert_eq!(def.tier, None); // → mesh-routed
        assert_eq!(def.system_prompt, "Do the thing.");
    }

    #[test]
    fn no_front_matter_is_none() {
        assert!(parse_agent("just a plain file", "x").is_none());
    }

    #[test]
    fn claude_code_agent_file_loads_with_tool_and_model_translation() {
        let text = "---\nname: code-reviewer\ndescription: Reviews code. Use after edits.\ntools: Read, Grep, Glob, Bash, Task, mcp__x__y\nmodel: sonnet\n---\nReview.";
        let def = parse_agent(text, "f").unwrap();
        assert_eq!(def.tools, vec!["read_file", "search", "glob", "shell"]);
        assert_eq!(def.tier, Some(TaskTier::Standard));
        assert_eq!(def.pinned_model, None);
    }

    #[test]
    fn model_field_variants() {
        let parse = |m: &str| parse_agent(&format!("---\nmodel: {m}\n---\nx"), "a").unwrap();
        assert_eq!(parse("haiku").tier, Some(TaskTier::Trivial));
        assert_eq!(parse("opus").tier, Some(TaskTier::Complex));
        let pinned = parse("codex-oauth::gpt-5.6-luna");
        assert_eq!(
            pinned.pinned_model.as_deref(),
            Some("codex-oauth::gpt-5.6-luna")
        );
        assert_eq!(pinned.tier, None);
        let inherit = parse("inherit");
        assert_eq!((inherit.tier, inherit.pinned_model), (None, None));
        // An explicit `tier:` wins over the model alias.
        let both = parse_agent("---\ntier: trivial\nmodel: opus\n---\nx", "a").unwrap();
        assert_eq!(both.tier, Some(TaskTier::Trivial));
    }

    #[test]
    fn tool_names_translate_and_unknown_claude_tools_drop() {
        assert_eq!(map_tool_name("Read").as_deref(), Some("read_file"));
        assert_eq!(map_tool_name(" bash ").as_deref(), Some("shell"));
        assert_eq!(map_tool_name("read_file").as_deref(), Some("read_file"));
        assert_eq!(map_tool_name("TodoWrite"), None);
        assert_eq!(map_tool_name("mcp__github__get_issue"), None);
    }

    #[test]
    fn layers_merge_with_project_overriding_personal() {
        let tmp = std::env::temp_dir().join(format!("forge-agents-{}", std::process::id()));
        let (home, proj) = (tmp.join("home"), tmp.join("proj"));
        let write = |dir: &Path, name: &str, desc: &str| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(
                dir.join(format!("{name}.md")),
                format!("---\nname: {name}\ndescription: {desc}\n---\nbody"),
            )
            .unwrap();
        };
        write(&home.join(".claude/agents"), "shared", "home-claude");
        write(&home.join(".claude/agents"), "personal", "only-home");
        write(&proj.join(".claude/agents"), "shared", "project-claude");
        write(&proj.join(".forge/agents"), "shared", "project-forge");
        write(
            &proj.join(".claude/agents"),
            "claude-only",
            "from claude dir",
        );
        let mut out = HashMap::new();
        for dir in layer_dirs(Some(&home), &proj, ".forge/agents") {
            out.extend(load_agents(&dir));
        }
        assert_eq!(out["shared"].description, "project-forge");
        assert_eq!(out["personal"].description, "only-home");
        assert_eq!(out["claude-only"].description, "from claude dir");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn catalog_lists_descriptions_and_tools_sorted() {
        let mut agents = HashMap::new();
        for (name, desc, tools) in [
            ("b", "second", vec![]),
            ("a", "first", vec!["shell".to_string()]),
        ] {
            agents.insert(
                name.to_string(),
                AgentDef {
                    name: name.into(),
                    description: desc.into(),
                    tools,
                    tier: None,
                    pinned_model: None,
                    system_prompt: String::new(),
                },
            );
        }
        let cat = agent_catalog(&agents);
        assert_eq!(cat, "\n- a: first [shell]\n- b: second [read-only tools]");
        assert!(agent_catalog(&HashMap::new()).is_empty());
    }
}
