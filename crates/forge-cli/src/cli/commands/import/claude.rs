//! Claude settings and MCP import policy.

use anyhow::{Context, Result};

/// Translate a Claude-Code `settings.json` (user `~/.claude/settings.json` + project `.claude/`)
/// into Forge: permission allow/ask/deny rules → `[[permissions.rules]]`, and hooks → a
/// CC-compatible `settings.json` Forge loads natively (item: CC-compatible hooks). Prints a summary.
pub(super) fn import_claude_settings(claude_home: &std::path::Path, project: bool) -> Result<()> {
    let sources = if project {
        vec![
            std::path::PathBuf::from("./.claude/settings.json"),
            std::path::PathBuf::from("./.claude/settings.local.json"),
        ]
    } else {
        vec![claude_home.join("settings.json")]
    };
    let mut values = Vec::new();
    let mut found_source = false;
    for path in &sources {
        if !path.exists() {
            continue;
        }
        found_source = true;
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading Claude settings {}", path.display()))?;
        values.push(
            serde_json::from_str::<serde_json::Value>(&text)
                .with_context(|| format!("parsing Claude settings {}", path.display()))?,
        );
    }
    if values.is_empty() && found_source {
        return Ok(());
    }
    if values.is_empty() {
        // No settings source remains: reconcile away the importer-owned permission block.
    }
    let (settings_dst, config_dst) = if project {
        (
            std::path::PathBuf::from("./.forge/settings.json"),
            std::path::PathBuf::from("./.forge/config.toml"),
        )
    } else {
        let dir = forge_config::config_dir().context("no user config directory")?;
        (dir.join("settings.json"), dir.join("config.toml"))
    };
    let hooks_n = count_cc_hooks(&values);
    let perms_n = transfer_permissions(&values, &config_dst)?;
    if hooks_n > 0 {
        println!("• found {hooks_n} hook(s) in settings.json — NOT imported; review {} manually (docs/features/hooks.md).", settings_dst.display());
    }
    if perms_n > 0 {
        println!(
            "✓ imported {perms_n} permission rule(s) from settings.json → {}",
            config_dst.display()
        );
    }
    if hooks_n == 0 && perms_n == 0 {
        println!("• no hooks or permission rules found in settings.json to import");
    }
    Ok(())
}

/// Count the hook command entries present across the CC settings sources — for the import
/// summary only, nothing is written. Hooks are deliberately NOT auto-imported (unlike commands/
/// skills/agents/permissions): a CC hook script is written against Claude Code's specific
/// behavior — many exist purely to inject text into an LLM's SYSTEM PROMPT (mode trackers,
/// project-context primers) or to talk to Claude-Code-only tooling, and assume nobody but an
/// LLM ever sees their raw stdout. Forge has its own session lifecycle and currently renders a
/// CC-compatible hook's stdout directly as a visible chat note — so blindly importing someone's
/// personal Claude Code hook set silently turned every new Forge session into a wall of garbled,
/// context-injection-style text (found via a real user report; the hooks were never meant to be
/// shown to a human). Permissions stay auto-imported below — `allow`/`deny`/`ask` tool rules are
/// data, not arbitrary code, so they carry no equivalent execution-context mismatch risk.
pub(super) fn count_cc_hooks(values: &[serde_json::Value]) -> usize {
    use serde_json::Value;
    let mut merged: serde_json::Map<String, Value> = serde_json::Map::new();
    for v in values {
        let Some(hooks) = v.get("hooks").and_then(|h| h.as_object()) else {
            continue;
        };
        for (event, groups) in hooks {
            let Some(groups) = groups.as_array() else {
                continue;
            };
            let entry = merged
                .entry(event.clone())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Some(arr) = entry.as_array_mut() {
                arr.extend(groups.iter().cloned());
            }
        }
    }
    if merged.is_empty() {
        return 0;
    }
    forge_config::cc_hooks_from_settings(&Value::Object(merged)).len()
}

/// Translate CC `permissions.{allow,ask,deny}` entries into Forge `[[permissions.rules]]` blocks,
/// appended to the target `config.toml`. Returns the number of rules written.
pub(super) fn transfer_permissions(
    values: &[serde_json::Value],
    config_dst: &std::path::Path,
) -> Result<usize> {
    let mut blocks = String::new();
    let mut count = 0usize;
    for v in values {
        let Some(perms) = v.get("permissions").and_then(|p| p.as_object()) else {
            continue;
        };
        for (kind, decision) in [("deny", "deny"), ("ask", "ask"), ("allow", "allow")] {
            let Some(arr) = perms.get(kind).and_then(|a| a.as_array()) else {
                continue;
            };
            for item in arr {
                let Some(s) = item.as_str() else { continue };
                let (cc_tool, pattern) = parse_cc_permission(s);
                let tools = forge_config::forge_tools_from_cc(&cc_tool);
                let tools = if tools.is_empty() {
                    vec![cc_tool.as_str()]
                } else {
                    tools
                };
                let pat = pattern.unwrap_or_else(|| "*".to_string());
                for tool in tools {
                    blocks.push_str(&format!(
                        "\n[[permissions.rules]]\ntool = {}\n{decision} = {}\nreason = \"imported from Claude Code settings.json\"\n",
                        toml_str(tool),
                        toml_str(&pat),
                    ));
                    count += 1;
                }
            }
        }
    }
    if let Some(parent) = config_dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    const START: &str = "# BEGIN Forge import: Claude Code permissions\n";
    const END: &str = "# END Forge import: Claude Code permissions\n";
    let existing = if config_dst.exists() {
        std::fs::read_to_string(config_dst)
            .with_context(|| format!("reading {}", config_dst.display()))?
    } else {
        "# Forge config\n".to_string()
    };
    let starts: Vec<_> = existing
        .match_indices(START)
        .map(|(index, _)| index)
        .collect();
    let ends: Vec<_> = existing
        .match_indices(END)
        .map(|(index, _)| index)
        .collect();
    let stripped = match (starts.as_slice(), ends.as_slice()) {
        ([start], [end]) if end >= start => {
            let mut text = existing[..*start].to_string();
            text.push_str(&existing[*end + END.len()..]);
            text
        }
        ([], []) => existing,
        _ => anyhow::bail!(
            "malformed or duplicate imported permission block in {}",
            config_dst.display()
        ),
    };
    let content = format!("{stripped}{START}{blocks}{END}");
    std::fs::write(config_dst, content)
        .with_context(|| format!("writing {}", config_dst.display()))?;
    Ok(count)
}

/// Parse a CC permission string `Tool(pattern)` → `(tool, Some(pattern))`; a bare `Tool` →
/// `(tool, None)`.
pub(super) fn parse_cc_permission(s: &str) -> (String, Option<String>) {
    let s = s.trim();
    if let Some(open) = s.find('(') {
        if s.ends_with(')') {
            let tool = s[..open].trim().to_string();
            let pat = s[open + 1..s.len() - 1].trim().to_string();
            return (tool, (!pat.is_empty()).then_some(pat));
        }
    }
    (s.to_string(), None)
}

/// Quote a string as a TOML basic string (escaping `\` and `"`).
fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// Fold the tool's MCP servers into `.forge/mcp.toml` (item: fold `forge mcp import` into
/// `forge import`). Non-interactive: imports every server discovered for this tool, storing secrets
/// in the OS keyring. `label` is `claude` or `codex`; other labels have no MCP sources here.
pub(super) fn import_tool_mcp_servers(label: &str, project: bool) -> Result<()> {
    let prefix = match label {
        "claude" => "claude",
        "codex" => "codex",
        _ => return Ok(()),
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let sources = forge_config::discover_import_sources(&cwd);
    let mut servers = Vec::new();
    let mut secrets = std::collections::HashMap::new();
    let mut seen = std::collections::HashSet::new();
    for s in &sources {
        if !s.label.starts_with(prefix)
            || (project && !s.label.contains("project") && !s.label.contains(".mcp.json"))
            || (!project && (s.label.contains("project") || s.label.contains(".mcp.json")))
        {
            continue;
        }
        for srv in &s.servers {
            if seen.insert(srv.name.clone()) {
                for key in srv.keyring_keys() {
                    if let Some(val) = s.secrets.get(&key) {
                        secrets.insert(key, val.clone());
                    }
                }
                servers.push(srv.clone());
            }
        }
    }
    if servers.is_empty() {
        return Ok(());
    }
    let out = if project {
        std::path::PathBuf::from(".forge/mcp.toml")
    } else {
        forge_config::config_dir()
            .context("no user config directory")?
            .join("mcp.toml")
    };
    crate::cli::commands::mcp::finish_import(&out, servers, secrets)
        .context("importing MCP servers")
}
