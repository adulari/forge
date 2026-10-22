//! `/agents` — the subagent types `spawn_agents` can use in this session: Forge's built-ins and
//! the user's and project's agent files, with what each one changes (Claude Code's `/agents`).

use std::collections::HashMap;

use forge_config::AgentDef;

use super::*;

pub(crate) async fn list_agents(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
) {
    let (agents, workspace) = {
        let s = session.lock().await;
        (s.agent_types(), s.workspace_root().to_path_buf())
    };
    for line in agent_lines(&agents, &workspace) {
        app.note(&line);
    }
}

/// Two lines per agent, sorted by name: `name — description`, then where it comes from and what it
/// sets. Paths under the workspace or home directory are shortened.
pub(crate) fn agent_lines(
    agents: &HashMap<String, AgentDef>,
    workspace: &std::path::Path,
) -> Vec<String> {
    let mut defs: Vec<&AgentDef> = agents.values().collect();
    defs.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = vec![format!(
        "⛓ {} agent type(s) — spawn_agents picks one with `agent`; files in .forge/agents or .claude/agents add more",
        defs.len()
    )];
    let home = forge_config::home_dir();
    for def in defs {
        let description = if def.description.is_empty() {
            "(no description)"
        } else {
            def.description.as_str()
        };
        out.push(format!("  {} — {description}", def.name));
        out.push(format!(
            "      {}",
            details(def, workspace, home.as_deref())
        ));
    }
    out
}

fn details(def: &AgentDef, workspace: &std::path::Path, home: Option<&std::path::Path>) -> String {
    let source = match &def.path {
        None => "built-in".to_string(),
        Some(path) => {
            if let Ok(rel) = path.strip_prefix(workspace) {
                rel.display().to_string()
            } else if let Some(rel) = home.and_then(|h| path.strip_prefix(h).ok()) {
                format!("~/{}", rel.display())
            } else {
                path.display().to_string()
            }
        }
    };
    let mut parts = vec![source];
    parts.push(if def.tools.is_empty() {
        "tools: read-only default".to_string()
    } else {
        format!("tools: {}", def.tools.join(", "))
    });
    if !def.disallowed_tools.is_empty() {
        parts.push(format!("without: {}", def.disallowed_tools.join(", ")));
    }
    if let Some(mode) = def.permission_mode {
        parts.push(format!("mode: {}", mode.key()));
    }
    match (&def.pinned_model, def.tier) {
        (Some(model), _) => parts.push(format!("model: {model}")),
        (None, Some(tier)) => parts.push(format!("tier: {}", tier.as_str())),
        (None, None) => {}
    }
    if let Some(turns) = def.max_turns {
        parts.push(format!("max {turns} steps"));
    }
    if let Some(effort) = def.effort {
        parts.push(format!("effort: {}", effort.as_str()));
    }
    if def.worktree {
        parts.push("own worktree".to_string());
    }
    if !def.skills.is_empty() {
        parts.push(format!("skills: {}", def.skills.join(", ")));
    }
    if !def.mcp_servers.is_empty() {
        parts.push(format!(
            "mcp: {} (not connected for subagents)",
            def.mcp_servers.join(", ")
        ));
    }
    if let Some(color) = &def.color {
        parts.push(format!("color: {color}"));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_builtins_and_files_with_their_settings() {
        let workspace = std::path::Path::new("/repo");
        let mut agents = HashMap::new();
        agents.insert(
            "reviewer".to_string(),
            AgentDef {
                name: "reviewer".into(),
                description: "Reviews diffs".into(),
                tools: vec!["read_file".into(), "shell".into()],
                disallowed_tools: vec!["write_file".into()],
                permission_mode: Some(forge_types::PermissionMode::Plan),
                tier: Some(forge_types::TaskTier::Standard),
                max_turns: Some(5),
                mcp_servers: vec!["github".into()],
                color: Some("blue".into()),
                path: Some("/repo/.forge/agents/reviewer.md".into()),
                ..AgentDef::default()
            },
        );
        agents.insert(
            "explore".to_string(),
            AgentDef {
                name: "explore".into(),
                ..AgentDef::default()
            },
        );
        let lines = agent_lines(&agents, workspace);
        assert!(lines[0].contains("2 agent type(s)"));
        assert_eq!(lines[1], "  explore — (no description)");
        assert_eq!(lines[2], "      built-in · tools: read-only default");
        assert_eq!(lines[3], "  reviewer — Reviews diffs");
        assert_eq!(
            lines[4],
            "      .forge/agents/reviewer.md · tools: read_file, shell · without: write_file · \
             mode: plan · tier: standard · max 5 steps · mcp: github (not connected for subagents) \
             · color: blue"
        );
    }
}
