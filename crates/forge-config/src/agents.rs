//! Named subagent types loaded from `.forge/agents/<name>.md` (RFC subagent-orchestration,
//! Phase 2). Each file is a small front-matter block followed by the agent's system prompt:
//!
//! ```text
//! ---
//! name: reviewer
//! description: Reviews a code change for bugs and risk.
//! tools: [read_file, list_dir, search]   # optional; omit → every tool
//! tier: standard                          # optional; omit → mesh-routed per task
//! model: sonnet                           # optional; haiku/sonnet/opus pick a tier, `provider::model` pins
//! ---
//! You are a meticulous code reviewer. ...
//! ```
//!
//! Claude Code agent files (`.claude/agents/*.md`) load unchanged: `tools: Read, Grep, Bash` is a
//! bare comma list of Claude tool names, which [`map_tool_name`] translates to Forge's. The rest of
//! Claude Code's agent frontmatter is read too: `disallowedTools`, `permissionMode`, `maxTurns`,
//! `skills`, `mcpServers`, `effort`, `isolation` and `color` (see [`AgentDef`] for what each does
//! here). [`load_agents_layered`] starts from Forge's built-in `explore` and `plan` agents, then
//! merges `~/.claude/agents`, `~/.forge/agents`, the project's `.claude/agents` and the project's
//! configured `agents_dir`, later layers overriding earlier ones by name. The project layers repeat
//! for each parent up to the repository top ([`crate::project_resource_dirs`]), farthest first, so a
//! session in `repo/pkg/` sees `repo/.forge/agents` and `pkg`'s own definitions override the
//! repository's.
//!
//! The front matter goes through the lenient reader `forge_skills::frontmatter` (YAML lists, folded
//! scalars), so a malformed file degrades to being skipped rather than failing the whole load.

pub mod crews;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use forge_skills::frontmatter::{self, Frontmatter};
use forge_types::{EffortLevel, PermissionMode, TaskTier};

/// A reusable subagent type. The `system_prompt` is the file body; `tools`/`tier` are optional
/// overrides resolved by the orchestrator (empty `tools` → every tool, as in Claude Code).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    /// `disallowedTools`: removed from the child's tool set, including from the default one.
    pub disallowed_tools: Vec<String>,
    /// `permissionMode`. A child can only be made stricter than its parent session: `plan` or
    /// `default` narrow it, a wider mode is ignored (a cloned repo's agent file must not widen
    /// what Forge lets a child do without asking).
    pub permission_mode: Option<PermissionMode>,
    /// `maxTurns`: the child's model-step cap, never above `[mesh] max_steps` when that is set.
    pub max_turns: Option<usize>,
    /// `skills`: skills whose methodology is loaded into the child's system prompt.
    pub skills: Vec<String>,
    /// `mcpServers`: the MCP servers the agent asks for, by name. Listed by `/agents`; children
    /// have no MCP tool surface yet, so they are not connected.
    pub mcp_servers: Vec<String>,
    /// `effort`: the reasoning rung sent with the child's model calls.
    pub effort: Option<EffortLevel>,
    /// `isolation: worktree`: run the child in its own git worktree even when
    /// `[mesh.subagents] worktree_isolation` is off.
    pub worktree: bool,
    /// `color`: the agent's display colour (shown by `/agents`).
    pub color: Option<String>,
    /// The file the definition came from; `None` for a built-in agent.
    pub path: Option<PathBuf>,
}

/// Merge every agent-definition directory a user might keep, lowest precedence first:
/// `~/.claude/agents`, `~/.forge/agents`, `<config>/forge/agents`, then for each project directory
/// (the repository top down to `project_root`) its `.claude/agents` and `agents_dir` (relative to
/// that directory unless absolute). A later definition replaces an earlier one with the same name,
/// so a project file overrides a personal one and a nearer directory overrides the repository's.
pub fn load_agents_layered(project_root: &Path, agents_dir: &str) -> HashMap<String, AgentDef> {
    let dirs = layer_dirs(
        crate::home_dir().as_deref(),
        crate::config_dir().as_deref(),
        &crate::project_resource_dirs(project_root),
        agents_dir,
    );
    let mut agents = merge_layers(dirs);
    // Saved crews contribute one `<crew>-<role>` type per member; an agent file of the same name wins.
    for (name, def) in crews::crew_agents(&crews::load_crews(project_root)) {
        agents.entry(name).or_insert(def);
    }
    agents
}

/// The built-in agents overlaid by each directory in order, later ones winning by name.
fn merge_layers(dirs: Vec<PathBuf>) -> HashMap<String, AgentDef> {
    let mut out = crate::agent_builtins::builtin_agents();
    for dir in dirs {
        out.extend(load_agents(&dir));
    }
    out
}

/// `project_dirs` is nearest first, as [`crate::project_resource_dirs`] returns it.
fn layer_dirs(
    home: Option<&Path>,
    config_dir: Option<&Path>,
    project_dirs: &[std::path::PathBuf],
    agents_dir: &str,
) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home {
        dirs.push(home.join(".claude/agents"));
        dirs.push(home.join(".forge/agents"));
    }
    // `forge import claude` and `forge skill import` write user-scope agents here.
    if let Some(config_dir) = config_dir {
        dirs.push(config_dir.join("agents"));
    }
    for project in project_dirs.iter().rev() {
        dirs.push(project.join(".claude/agents"));
        if !Path::new(agents_dir).is_absolute() {
            dirs.push(project.join(agents_dir));
        }
    }
    if Path::new(agents_dir).is_absolute() {
        dirs.push(agents_dir.into());
    }
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
        let tools = if def.permission_mode == Some(PermissionMode::Plan) {
            "read-only".to_string()
        } else if def.tools.is_empty() {
            "all tools".to_string()
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
        if let Some(mut def) = parse_agent(&text, &stem) {
            def.path = Some(path.clone());
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
    let fm = lenient_frontmatter(front);
    let (model_tier, pinned_model) = parse_model(&fm.scalar("model").unwrap_or_default());
    let tools = |keys: &[&str]| -> Vec<String> {
        keys.iter()
            .flat_map(|k| fm.tool_list(k))
            .filter_map(|t| map_tool_name(&t))
            .collect()
    };
    Some(AgentDef {
        name: fm
            .scalar("name")
            .unwrap_or_else(|| default_name.to_string()),
        description: fm.scalar("description").unwrap_or_default(),
        tools: tools(&["tools"]),
        tier: fm
            .scalar("tier")
            .and_then(|t| parse_tier(&t))
            .or(model_tier),
        pinned_model,
        system_prompt: body.trim().to_string(),
        disallowed_tools: tools(&["disallowedtools", "disallowed-tools", "disallowed_tools"]),
        permission_mode: first_scalar(&fm, &["permissionmode", "permission-mode"])
            .and_then(|m| parse_permission_mode(&m)),
        max_turns: first_scalar(&fm, &["maxturns", "max-turns", "max_turns"])
            .and_then(|n| n.trim().parse().ok())
            .filter(|n| *n > 0),
        skills: names(&fm, "skills"),
        mcp_servers: first_list(&fm, &["mcpservers", "mcp-servers", "mcp_servers"]),
        effort: fm.scalar("effort").and_then(|e| parse_effort(&e)),
        worktree: fm
            .scalar("isolation")
            .is_some_and(|i| i.trim().eq_ignore_ascii_case("worktree")),
        color: fm.scalar("color"),
        path: None,
    })
}

/// Parse front matter, dropping unindented lines that are not `key: value` (the reader rejects
/// them, and an agent file has always tolerated stray lines) and YAML ` # comments`.
fn lenient_frontmatter(front: &str) -> Frontmatter {
    let kept: String = front
        .lines()
        .map(|line| match line.find(" #") {
            Some(i) => &line[..i],
            None => line,
        })
        .filter(|line| {
            let t = line.trim();
            t.is_empty()
                || t.starts_with('#')
                || t.starts_with('-')
                || line.starts_with([' ', '\t'])
                || t.contains(':')
        })
        .map(|line| format!("{line}\n"))
        .collect();
    frontmatter::parse(&kept).unwrap_or_default()
}

fn first_scalar(fm: &Frontmatter, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| fm.scalar(k))
}

fn first_list(fm: &Frontmatter, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .map(|k| names(fm, k))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

/// A list of plain names (`skills`, `mcpServers`), accepting a YAML list or a comma/space
/// separated string. Inline server definitions (`name: {...}`) contribute just their name.
fn names(fm: &Frontmatter, key: &str) -> Vec<String> {
    fm.tool_list(key)
        .into_iter()
        .map(|n| n.split(':').next().unwrap_or("").trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

/// Claude Code's `permissionMode` values (and Forge's own keys) onto Forge's modes. `dontAsk`
/// is Forge's default for a child: it has no one to ask, so an `ask` already resolves to deny.
pub fn parse_permission_mode(value: &str) -> Option<PermissionMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "plan" => Some(PermissionMode::Plan),
        "default" | "dontask" | "dont-ask" | "ask" => Some(PermissionMode::Default),
        "acceptedits" | "accept-edits" | "auto-edit" => Some(PermissionMode::AcceptEdits),
        "auto" => Some(PermissionMode::Auto),
        "bypasspermissions" | "bypass" | "full" => Some(PermissionMode::Bypass),
        _ => None,
    }
}

/// Claude Code's effort names. `max` is Forge's highest provider rung, `xhigh`: Forge's
/// `whitehot` adds workflow orchestration guidance an agent file never asked for.
pub fn parse_effort(value: &str) -> Option<EffortLevel> {
    match value.trim().to_ascii_lowercase().as_str() {
        "max" => Some(EffortLevel::XHigh),
        other => EffortLevel::parse(other),
    }
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

/// `model:` → (tier, pin). Claude's aliases choose a tier; `provider::model` pins; anything else
/// (`inherit`, a bare Anthropic id Forge cannot route) leaves the child to the mesh.
pub fn parse_model(value: &str) -> (Option<TaskTier>, Option<String>) {
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

pub(crate) fn parse_tier(value: &str) -> Option<TaskTier> {
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
        assert!(def.tools.is_empty()); // → every tool, decided in core
        assert_eq!(def.tier, None); // → mesh-routed
        assert_eq!(def.system_prompt, "Do the thing.");
    }

    #[test]
    fn claude_code_agent_fields_parse() {
        let text = concat!(
            "---\n",
            "name: auditor\n",
            "description: Audits code\n",
            "tools:\n  - Read\n  - Bash\n",
            "disallowedTools: Write, Edit\n",
            "permissionMode: plan\n",
            "maxTurns: 7\n",
            "skills:\n  - honest-review\n  - rust\n",
            "mcpServers:\n  - github\n",
            "effort: max\n",
            "isolation: worktree\n",
            "color: purple\n",
            "---\nAudit.",
        );
        let def = parse_agent(text, "f").unwrap();
        assert_eq!(def.tools, ["read_file", "shell"]);
        assert_eq!(def.disallowed_tools, ["write_file", "edit_file"]);
        assert_eq!(def.permission_mode, Some(PermissionMode::Plan));
        assert_eq!(def.max_turns, Some(7));
        assert_eq!(def.skills, ["honest-review", "rust"]);
        assert_eq!(def.mcp_servers, ["github"]);
        assert_eq!(def.effort, Some(EffortLevel::XHigh));
        assert!(def.worktree);
        assert_eq!(def.color.as_deref(), Some("purple"));
    }

    #[test]
    fn permission_modes_and_stray_lines_and_comments() {
        for (raw, want) in [
            ("dontAsk", PermissionMode::Default),
            ("acceptEdits", PermissionMode::AcceptEdits),
            ("bypassPermissions", PermissionMode::Bypass),
            ("auto", PermissionMode::Auto),
        ] {
            assert_eq!(parse_permission_mode(raw), Some(want), "{raw}");
        }
        assert_eq!(parse_permission_mode("sometimes"), None);
        let text = "---\nname: x\nthis line has no colon\ntools: [read_file, search]   # comment\nmaxTurns: 0\n---\nbody";
        let def = parse_agent(text, "x").unwrap();
        assert_eq!(def.tools, ["read_file", "search"]);
        assert_eq!(def.max_turns, None, "zero turns is not a cap");
    }

    #[test]
    fn layered_load_starts_from_builtins_and_records_paths() {
        let tmp = std::env::temp_dir().join(format!("forge-agents-b-{}", std::process::id()));
        let dir = tmp.join(".forge/agents");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plan.md"), "---\ndescription: mine\n---\nMy plan.").unwrap();
        let agents = merge_layers(layer_dirs(
            None,
            None,
            std::slice::from_ref(&tmp),
            ".forge/agents",
        ));
        assert!(
            agents["explore"].path.is_none(),
            "builtin explore is present"
        );
        assert_eq!(
            agents["plan"].description, "mine",
            "a file overrides a builtin"
        );
        assert_eq!(
            agents["plan"].path.as_deref(),
            Some(dir.join("plan.md").as_path())
        );
        std::fs::remove_dir_all(&tmp).ok();
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
        let cfg = tmp.join("cfg");
        write(&cfg.join("agents"), "imported", "from config dir");
        write(&proj.join(".claude/agents"), "shared", "project-claude");
        write(&proj.join(".forge/agents"), "shared", "project-forge");
        write(
            &proj.join(".claude/agents"),
            "claude-only",
            "from claude dir",
        );
        let mut out = HashMap::new();
        for dir in layer_dirs(
            Some(&home),
            Some(&cfg),
            std::slice::from_ref(&proj),
            ".forge/agents",
        ) {
            out.extend(load_agents(&dir));
        }
        assert_eq!(out["shared"].description, "project-forge");
        assert_eq!(out["personal"].description, "only-home");
        assert_eq!(out["imported"].description, "from config dir");
        assert_eq!(out["claude-only"].description, "from claude dir");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn a_subdirectory_sees_the_repositorys_agents_and_overrides_them() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let pkg = repo.join("pkg");
        let write = |dir: &Path, name: &str, desc: &str| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(
                dir.join(format!("{name}.md")),
                format!("---\nname: {name}\ndescription: {desc}\n---\nbody"),
            )
            .unwrap();
        };
        write(&repo.join(".forge/agents"), "reviewer", "repo");
        write(&repo.join(".forge/agents"), "planner", "repo-only");
        write(&pkg.join(".claude/agents"), "reviewer", "pkg");
        let mut out = HashMap::new();
        for dir in layer_dirs(None, None, &[pkg.clone(), repo.clone()], ".forge/agents") {
            out.extend(load_agents(&dir));
        }
        assert_eq!(out["reviewer"].description, "pkg");
        assert_eq!(out["planner"].description, "repo-only");
        let absolute = layer_dirs(None, None, &[pkg, repo], "/opt/agents");
        assert_eq!(
            absolute
                .iter()
                .filter(|d| d.ends_with("opt/agents"))
                .count(),
            1
        );
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
                    ..AgentDef::default()
                },
            );
        }
        let cat = agent_catalog(&agents);
        assert_eq!(cat, "\n- a: first [shell]\n- b: second [all tools]");
        assert!(agent_catalog(&HashMap::new()).is_empty());
    }
}
