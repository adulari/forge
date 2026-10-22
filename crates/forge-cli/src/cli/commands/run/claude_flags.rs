//! Applies the Claude Code flags that change the config a `forge run` session is built from:
//! `--max-turns`, `--allowedTools`/`--disallowedTools`, `--system-prompt[-file]`, `--add-dir`,
//! `--mcp-config`/`--strict-mcp-config` and `--setting-sources`.

use std::sync::OnceLock;

use anyhow::{Context, Result};
use forge_config::{Config, OneOrMany, RuleConfig};

use super::mcp_overlay::McpOverlay;
use crate::cli::args::claude_compat::ClaudeCompat;
use crate::cli::args::SessionFlags;

#[derive(Debug, Default)]
pub(crate) struct RunOverrides {
    max_turns: Option<usize>,
    allow: Vec<String>,
    deny: Vec<String>,
    /// Replaces the base coding-agent prompt for every provider (`--system-prompt`).
    system_prompt: Option<String>,
    /// Absolute directories added to `[tools] extra_roots` (`--add-dir`).
    extra_roots: Vec<String>,
    mcp: Option<McpOverlay>,
}

static OVERRIDES: OnceLock<RunOverrides> = OnceLock::new();

/// Record this invocation's overrides. `forge run` builds exactly one session per process, so a
/// process-wide value reaches `build_session_with_self_mcp` without threading it through every
/// caller. Fails on a flag whose input cannot be used (a missing prompt file or directory, an
/// unreadable MCP config, an unknown setting source), before any session work starts.
pub(crate) fn install(print: &ClaudeCompat, session: &SessionFlags) -> Result<()> {
    if let Some(list) = &session.setting_sources {
        let sources = forge_config::setting_sources::SettingSources::parse(list)
            .map_err(anyhow::Error::msg)?;
        forge_config::setting_sources::set_setting_sources(sources);
    }
    let system_prompt = match (&session.system_prompt, &session.system_prompt_file) {
        (Some(text), _) => Some(text.clone()),
        (None, Some(path)) => Some(
            std::fs::read_to_string(path)
                .with_context(|| format!("reading --system-prompt-file {}", path.display()))?,
        ),
        (None, None) => None,
    };
    let overrides = RunOverrides {
        max_turns: print.max_turns.map(|n| n as usize),
        allow: split_tool_list(&session.allowed_tools),
        deny: split_tool_list(&session.disallowed_tools),
        system_prompt,
        extra_roots: added_dirs(&session.add_dir)?,
        mcp: (!session.mcp_config.is_empty())
            .then(|| McpOverlay::load(&session.mcp_config, session.strict_mcp_config))
            .transpose()?,
    };
    let _ = OVERRIDES.set(overrides);
    Ok(())
}

/// `--add-dir` paths as the absolute directories `[tools] extra_roots` requires.
pub(crate) fn added_dirs(dirs: &[std::path::PathBuf]) -> Result<Vec<String>> {
    dirs.iter()
        .map(|dir| {
            let path = dir
                .canonicalize()
                .with_context(|| format!("--add-dir {}", dir.display()))?;
            anyhow::ensure!(
                path.is_dir(),
                "--add-dir {}: not a directory",
                dir.display()
            );
            Ok(path.display().to_string())
        })
        .collect()
}

pub(crate) fn apply(config: &mut Config) {
    if let Some(overrides) = OVERRIDES.get() {
        apply_overrides(config, overrides);
    }
}

fn apply_overrides(config: &mut Config, overrides: &RunOverrides) {
    if let Some(turns) = overrides.max_turns {
        config.mesh.max_steps = turns;
        config.mesh.max_steps_unattended = turns;
    }
    let mut rules = Vec::new();
    for spec in &overrides.deny {
        rules.extend(rules_for(spec, Decision::Deny));
    }
    for spec in &overrides.allow {
        rules.extend(rules_for(spec, Decision::Allow));
    }
    config.permissions.rules.extend(rules);
    if let Some(prompt) = &overrides.system_prompt {
        // The command line wins over every configured per-provider prompt.
        config.system_prompt_overrides = [("*".to_string(), prompt.clone())].into();
    }
    for root in &overrides.extra_roots {
        if !config.tools.extra_roots.contains(root) {
            config.tools.extra_roots.push(root.clone());
        }
    }
    if let Some(mcp) = &overrides.mcp {
        mcp.apply(&mut config.mcp);
        if mcp.strict() {
            // "Only the servers from --mcp-config" includes Forge's own self-MCP server.
            config.self_mcp = false;
        }
    }
}

/// Split `"Bash(git log:*) Read"` / `"Read,Edit"` into entries, keeping spaces and commas that sit
/// inside parentheses.
pub(crate) fn split_tool_list(values: &[String]) -> Vec<String> {
    let mut entries = Vec::new();
    for value in values {
        let mut depth = 0usize;
        let mut current = String::new();
        for ch in value.chars() {
            match ch {
                '(' => {
                    depth += 1;
                    current.push(ch);
                }
                ')' => {
                    depth = depth.saturating_sub(1);
                    current.push(ch);
                }
                ' ' | ',' if depth == 0 => {
                    if !current.is_empty() {
                        entries.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(ch),
            }
        }
        if !current.is_empty() {
            entries.push(current);
        }
    }
    entries
}

#[derive(Clone, Copy)]
enum Decision {
    Allow,
    Deny,
}

/// `Bash(git log:*)` -> a rule on Forge's `shell` tool matching `git log *`; a bare `Read` -> every
/// call of `read_file`. Tools with no Forge equivalent (e.g. an MCP tool) keep their own name.
fn rules_for(spec: &str, decision: Decision) -> Vec<RuleConfig> {
    let spec = spec.trim();
    let (cc_tool, pattern) = match spec.split_once('(') {
        Some((tool, rest)) if rest.ends_with(')') => {
            let pattern = rest[..rest.len() - 1].trim();
            (
                tool.trim(),
                (!pattern.is_empty()).then(|| pattern.to_string()),
            )
        }
        _ => (spec, None),
    };
    if cc_tool.is_empty() {
        return Vec::new();
    }
    let pattern = pattern
        .map(|p| match p.strip_suffix(":*") {
            Some(prefix) => format!("{prefix} *"),
            None => p,
        })
        .unwrap_or_else(|| "*".to_string());
    let forge_tools = forge_config::forge_tools_from_cc(cc_tool);
    let tools: Vec<&str> = if forge_tools.is_empty() {
        vec![cc_tool]
    } else {
        forge_tools
    };
    tools
        .into_iter()
        .map(|tool| {
            let patterns = Some(OneOrMany::One(pattern.clone()));
            RuleConfig {
                tool: tool.to_string(),
                allow: matches!(decision, Decision::Allow)
                    .then(|| patterns.clone())
                    .flatten(),
                ask: None,
                deny: matches!(decision, Decision::Deny)
                    .then(|| patterns.clone())
                    .flatten(),
                reason: Some("from --allowedTools/--disallowedTools".to_string()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(values: &[&str]) -> Vec<String> {
        split_tool_list(&values.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn tool_lists_split_on_spaces_and_commas_outside_parentheses() {
        assert_eq!(
            list(&["Bash(git log:*) Read,Edit", "Write"]),
            ["Bash(git log:*)", "Read", "Edit", "Write"]
        );
        assert_eq!(list(&["Bash(a, b)"]), ["Bash(a, b)"]);
        assert!(list(&[" , "]).is_empty());
    }

    #[test]
    fn claude_tool_names_become_forge_rules() {
        let rules = rules_for("Bash(git log:*)", Decision::Allow);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].tool, "shell");
        assert!(matches!(&rules[0].allow, Some(OneOrMany::One(p)) if p == "git log *"));
        assert!(rules[0].deny.is_none());

        let rules = rules_for("Edit", Decision::Deny);
        assert_eq!(
            rules.iter().map(|r| r.tool.as_str()).collect::<Vec<_>>(),
            ["edit_file", "apply_patch"]
        );
        assert!(matches!(&rules[0].deny, Some(OneOrMany::One(p)) if p == "*"));

        let rules = rules_for("mcp__git__status", Decision::Allow);
        assert_eq!(rules[0].tool, "mcp__git__status");
    }

    #[test]
    fn overrides_cap_steps_and_add_rules() {
        let mut config = Config::default();
        let overrides = RunOverrides {
            max_turns: Some(3),
            allow: vec!["Read".into()],
            deny: vec!["Bash(rm:*)".into()],
            system_prompt: Some("You are a Python expert".into()),
            extra_roots: vec!["/srv/shared".into()],
            ..RunOverrides::default()
        };
        apply_overrides(&mut config, &overrides);
        assert_eq!(config.mesh.max_steps, 3);
        assert_eq!(config.mesh.max_steps_unattended, 3);
        assert_eq!(config.permissions.rules[0].tool, "shell");
        assert!(
            config.permissions.rules[0].deny.is_some(),
            "denies come first"
        );
        assert_eq!(config.permissions.rules[1].tool, "read_file");
        assert_eq!(config.system_prompt_overrides.len(), 1);
        assert_eq!(
            config.system_prompt_overrides["*"],
            "You are a Python expert"
        );
        assert_eq!(config.tools.extra_roots, ["/srv/shared"]);
    }

    #[test]
    fn added_dirs_must_exist_and_become_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let roots = added_dirs(&[dir.path().to_path_buf()]).unwrap();
        assert!(std::path::Path::new(&roots[0]).is_absolute());
        assert!(added_dirs(&[dir.path().join("missing")]).is_err());
    }
}
