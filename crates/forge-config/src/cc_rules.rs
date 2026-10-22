//! Claude Code permission specs (`Bash(git log:*)`, `Read`, `Edit(src/**)`, `mcp__x__y`) as
//! Forge permission rules, for skill/command `allowed-tools` and `disallowed-tools`.

use forge_types::{PermissionDecision, PermissionRule, RuleSource};

/// Forge's name(s) for a Claude Code tool, covering the agent-loop tools [`forge_tools_from_cc`]
/// leaves out. Unknown names (Forge's own, MCP tools) pass through unchanged.
///
/// [`forge_tools_from_cc`]: crate::forge_tools_from_cc
pub fn forge_tool_names(cc_tool: &str) -> Vec<String> {
    let mapped = crate::forge_tools_from_cc(cc_tool);
    if !mapped.is_empty() {
        return mapped.into_iter().map(str::to_string).collect();
    }
    let name = match cc_tool {
        "AskUserQuestion" => "ask_user",
        "Task" | "Agent" => "spawn_agents",
        "TodoWrite" => "update_tasks",
        "Skill" => "use_skill",
        "NotebookEdit" => "notebook_edit",
        other => other,
    };
    vec![name.to_string()]
}

/// The rules one spec stands for. `Bash(git log:*)` becomes a `shell` rule on `git log *`; a bare
/// `Read` matches every `read_file` call. An empty or malformed spec yields nothing.
pub fn rules_for_spec(
    spec: &str,
    decision: PermissionDecision,
    source: RuleSource,
    reason: &str,
) -> Vec<PermissionRule> {
    let spec = spec.trim();
    let (tool, pattern) = match spec.split_once('(') {
        Some((tool, rest)) if rest.ends_with(')') => {
            let pattern = rest[..rest.len() - 1].trim();
            (tool.trim(), (!pattern.is_empty()).then_some(pattern))
        }
        Some(_) => return Vec::new(),
        None => (spec, None),
    };
    if tool.is_empty() {
        return Vec::new();
    }
    let patterns: Vec<String> = pattern
        .filter(|p| *p != "*")
        .map(|p| match p.strip_suffix(":*") {
            Some(prefix) => format!("{prefix} *"),
            None => p.to_string(),
        })
        .into_iter()
        .collect();
    forge_tool_names(tool)
        .into_iter()
        .map(|tool| PermissionRule {
            tool,
            patterns: patterns.clone(),
            decision,
            source,
            reason: Some(reason.to_string()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow(spec: &str) -> Vec<PermissionRule> {
        rules_for_spec(spec, PermissionDecision::Allow, RuleSource::Turn, "test")
    }

    #[test]
    fn bash_prefix_specs_become_shell_globs() {
        let rules = allow("Bash(git log:*)");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].tool, "shell");
        assert_eq!(rules[0].patterns, ["git log *"]);
        assert_eq!(rules[0].source, RuleSource::Turn);
        assert_eq!(allow("Bash(gh *)")[0].patterns, ["gh *"]);
    }

    #[test]
    fn bare_and_star_specs_match_every_call() {
        let edit = allow("Edit");
        assert_eq!(
            edit.iter().map(|r| r.tool.as_str()).collect::<Vec<_>>(),
            ["edit_file", "apply_patch"]
        );
        assert!(edit.iter().all(|r| r.patterns.is_empty()));
        assert!(allow("Bash(*)")[0].patterns.is_empty());
    }

    #[test]
    fn agent_loop_tools_and_unknown_names_map() {
        assert_eq!(allow("AskUserQuestion")[0].tool, "ask_user");
        assert_eq!(allow("Task")[0].tool, "spawn_agents");
        assert_eq!(
            allow("mcp__github__get_issue")[0].tool,
            "mcp__github__get_issue"
        );
        assert_eq!(allow("read_file")[0].tool, "read_file");
        assert_eq!(allow("Read(src/**)")[0].patterns, ["src/**"]);
    }

    #[test]
    fn malformed_specs_yield_nothing() {
        assert!(allow("").is_empty());
        assert!(allow("Bash(unclosed").is_empty());
        assert!(allow("(x)").is_empty());
    }
}
