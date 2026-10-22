//! Which hooks an event runs, beyond the tool-name matcher in `hooks.rs`:
//!
//! - the `if` field (Claude-Code parity): one permission rule (`Bash(git *)`, `Edit(*.ts)`,
//!   `WebFetch(domain:docs.rs)`) the tool call must match. Only tool events evaluate it; on any
//!   other event a hook carrying `if` never runs, as in Claude Code. Like Claude Code's, the
//!   filter is best effort: a shell line Forge cannot split confidently runs the hook anyway, so a
//!   hook is never a substitute for a deny rule.
//! - the matcher of a non-tool event, tested against that event's value (`source`, `load_reason`,
//!   `error`, the model id …) with Claude Code's rules: plain names separated by `|` or `,` are an
//!   exact list, anything else is an unanchored regular expression.

use std::path::{Path, PathBuf};

use forge_config::{HookConfig, HookEvent};
use serde_json::Value;

/// Whether `h` may run for this tool call under its `if` rule.
pub(crate) fn if_allows(
    h: &HookConfig,
    event: HookEvent,
    tool: &str,
    args: &Value,
    cwd: Option<&str>,
) -> bool {
    match h
        .if_rule
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        None => true,
        Some(rule) => event.is_tool_event() && rule_matches_call(rule, tool, args, cwd),
    }
}

/// A hook on a non-tool event: one carrying `if` never runs.
pub(crate) fn runs_off_tool(h: &HookConfig) -> bool {
    h.if_rule.as_deref().is_none_or(|r| r.trim().is_empty())
}

/// `Tool(pattern)` → (`Tool`, `Some(pattern)`); a bare `Tool` → (`Tool`, `None`).
fn split_rule(rule: &str) -> (&str, Option<&str>) {
    match rule.find('(') {
        Some(open) if rule.ends_with(')') => {
            let pattern = rule[open + 1..rule.len() - 1].trim();
            (
                rule[..open].trim(),
                (!pattern.is_empty()).then_some(pattern),
            )
        }
        _ => (rule, None),
    }
}

pub(crate) fn rule_matches_call(rule: &str, tool: &str, args: &Value, cwd: Option<&str>) -> bool {
    let (rule_tool, pattern) = split_rule(rule);
    if !tool_name_matches(rule_tool, tool) {
        return false;
    }
    let Some(pattern) = pattern else {
        return true;
    };
    if pattern == "*" {
        return true;
    }
    if crate::permission::is_shell_tool(tool) {
        return forge_types::shell_command_text(args)
            .is_none_or(|command| shell_matches(pattern, &command));
    }
    if let Some(domain) = pattern.strip_prefix("domain:") {
        return args
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|url| domain_matches(domain, url));
    }
    let paths = forge_types::all_path_args(args);
    if !paths.is_empty() {
        return paths.iter().any(|p| path_matches(pattern, p, cwd));
    }
    args.as_object().is_some_and(|map| {
        map.values()
            .filter_map(Value::as_str)
            .any(|v| crate::permission::shell_glob(pattern, v))
    })
}

/// The rule's tool position against the Forge tool name: the Forge name itself, its Claude-Code
/// name (`Bash` for `shell`, `Edit` for `apply_patch` …), `*`, or an MCP name. Forge calls an MCP
/// tool `server__tool`; Claude Code `mcp__server__tool`, and `mcp__server` covers a whole server.
fn tool_name_matches(rule_tool: &str, tool: &str) -> bool {
    if rule_tool == "*" || rule_tool.eq_ignore_ascii_case(tool) {
        return true;
    }
    if rule_tool.eq_ignore_ascii_case(forge_config::cc_tool_alias(tool))
        || forge_config::forge_tools_from_cc(rule_tool).contains(&tool)
    {
        return true;
    }
    let Some(mcp) = rule_tool.strip_prefix("mcp__") else {
        return rule_tool.contains('*') && crate::permission::shell_glob(rule_tool, tool);
    };
    let bare = tool.strip_prefix("mcp__").unwrap_or(tool);
    if mcp.contains('*') {
        return crate::permission::shell_glob(mcp, bare);
    }
    bare == mcp || bare.starts_with(&format!("{mcp}__"))
}

/// Claude Code's Bash rule semantics over every command the line runs: `*` is any text, a
/// trailing ` *` (or `:*`) that is the only wildcard also matches the bare command.
fn shell_matches(pattern: &str, command: &str) -> bool {
    let pattern = match pattern.strip_suffix(":*") {
        Some(prefix) => format!("{prefix} *"),
        None => pattern.to_string(),
    };
    let bare = pattern
        .strip_suffix(" *")
        .filter(|prefix| !prefix.contains('*'))
        .map(str::to_string);
    let (segments, confident) = crate::permission::shell_segments(command);
    if !confident {
        return true;
    }
    segments.iter().any(|segment| {
        let segment = segment.trim();
        crate::permission::shell_glob(&pattern, segment) || bare.as_deref() == Some(segment)
    })
}

fn domain_matches(domain: &str, url: &str) -> bool {
    let host = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("");
    let host = host.split(':').next().unwrap_or("").to_ascii_lowercase();
    let domain = domain.to_ascii_lowercase();
    match domain.strip_prefix("*.") {
        Some(parent) => host.ends_with(&format!(".{parent}")),
        None => domain == "*" || host == domain,
    }
}

/// gitignore-style path rules as Claude Code reads them: `//abs` is absolute, `~/x` is under the
/// home directory, `/x` and `./x` are anchored at the session's directory, a pattern with no `/`
/// (a file name or `*.ts`) matches at any depth, and any other pattern is anchored at the session's
/// directory. `*` stays within one path segment; `**` crosses directories.
fn path_matches(pattern: &str, path: &str, cwd: Option<&str>) -> bool {
    let cwd = cwd.map(PathBuf::from);
    let absolute = absolutize(path, cwd.as_deref());
    let relative = cwd
        .as_deref()
        .and_then(|cwd| absolute.strip_prefix(cwd).ok())
        .map(Path::to_path_buf);
    let glob = |pattern: &str, candidate: &Path| {
        globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .is_ok_and(|g| g.compile_matcher().is_match(candidate))
    };
    if let Some(abs) = pattern.strip_prefix("//") {
        return glob(&format!("/{abs}"), &absolute);
    }
    if let Some(rest) = pattern.strip_prefix("~/") {
        return forge_config::home_dir()
            .is_some_and(|home| glob(&home.join(rest).to_string_lossy(), &absolute));
    }
    let Some(relative) = relative else {
        return false;
    };
    let anchored = pattern
        .strip_prefix("./")
        .or_else(|| pattern.strip_prefix('/'));
    match anchored {
        Some(anchored) => glob(anchored, &relative),
        None if !pattern.contains('/') => glob(&format!("**/{pattern}"), &relative),
        None => glob(pattern, &relative),
    }
}

fn absolutize(path: &str, cwd: Option<&Path>) -> PathBuf {
    let expanded = match path.strip_prefix("~/") {
        Some(rest) => {
            forge_config::home_dir().map_or_else(|| PathBuf::from(path), |h| h.join(rest))
        }
        None => PathBuf::from(path),
    };
    let joined = match cwd {
        Some(cwd) if expanded.is_relative() => cwd.join(expanded),
        _ => expanded,
    };
    let mut clean = PathBuf::new();
    for part in joined.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    clean
}

/// Whether `h`'s matcher selects `value` on a non-tool event. A hook without a matcher (or `*`)
/// always runs.
pub(crate) fn event_value_matches(h: &HookConfig, value: &str) -> bool {
    let Some(pattern) = h.matcher.as_deref().map(str::trim) else {
        return true;
    };
    if pattern.is_empty() || pattern == "*" {
        return true;
    }
    let plain = pattern
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ' ' | ',' | '|'));
    if plain {
        return pattern
            .split(['|', ','])
            .map(str::trim)
            .any(|name| name == value);
    }
    regex::Regex::new(pattern).is_ok_and(|re| re.is_match(value))
}

/// [`event_value_matches`] for a model id: Forge spells models `provider::model`, Claude Code
/// matches the bare model name, so either spelling selects the hook.
pub(crate) fn model_matches(h: &HookConfig, model: &str) -> bool {
    let bare = model.rsplit("::").next().unwrap_or(model);
    let bare = bare.strip_suffix("[1m]").unwrap_or(bare);
    event_value_matches(h, model) || event_value_matches(h, bare)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shell(command: &str) -> Value {
        json!({ "command": command })
    }

    #[test]
    fn bash_rules_check_every_subcommand() {
        let m = |rule: &str, command: &str| rule_matches_call(rule, "shell", &shell(command), None);
        assert!(m("Bash(git *)", "FOO=bar git push"));
        assert!(m("Bash(git *)", "npm test && git push"));
        assert!(m("Bash(rm *)", "echo $(rm -rf /tmp/x)"));
        assert!(!m("Bash(rm *)", "echo $(date)"));
        assert!(m("Bash(ls *)", "ls"));
        assert!(!m("Bash(ls *)", "lsof"));
        assert!(m("Bash(ls*)", "lsof"));
        assert!(m("Bash(npm run test:*)", "npm run test -- --watch"));
        assert!(m("shell(cargo *)", "cargo build"));
        assert!(!m("Edit(*.ts)", "git push"), "the tool must match too");
    }

    #[test]
    fn path_rules_follow_gitignore_shapes() {
        let cwd = "/work/repo";
        let m = |rule: &str, tool: &str, path: &str| {
            rule_matches_call(rule, tool, &json!({ "path": path }), Some(cwd))
        };
        assert!(m("Edit(*.ts)", "edit_file", "src/deep/a.ts"));
        assert!(!m("Edit(*.ts)", "edit_file", "src/a.rs"));
        assert!(m("Edit(src/**)", "edit_file", "src/a/b.rs"));
        assert!(!m("Edit(src/**)", "edit_file", "lib/src/b.rs"));
        assert!(m("Edit(**/src/**)", "edit_file", "lib/src/b.rs"));
        assert!(m("Write(/docs/*.md)", "write_file", "/work/repo/docs/x.md"));
        assert!(!m(
            "Write(/docs/*.md)",
            "write_file",
            "/work/repo/docs/sub/x.md"
        ));
        assert!(m("Read(//etc/hosts)", "read_file", "/etc/hosts"));
        assert!(m("Read(.env)", "read_file", "app/.env"));
        assert!(m("Edit", "apply_patch", "x"));
    }

    #[test]
    fn domain_mcp_and_unscoped_rules() {
        let fetch = json!({ "url": "https://api.docs.rs:443/a?b" });
        assert!(rule_matches_call(
            "WebFetch(domain:*.docs.rs)",
            "web_fetch",
            &fetch,
            None
        ));
        assert!(!rule_matches_call(
            "WebFetch(domain:docs.rs)",
            "web_fetch",
            &fetch,
            None
        ));
        assert!(rule_matches_call(
            "mcp__github",
            "github__get_issue",
            &json!({}),
            None
        ));
        assert!(rule_matches_call(
            "mcp__github__get_*",
            "github__get_issue",
            &json!({}),
            None
        ));
        assert!(!rule_matches_call(
            "mcp__github",
            "helm__get_today",
            &json!({}),
            None
        ));
    }

    #[test]
    fn if_only_runs_on_tool_events() {
        let hook = HookConfig {
            if_rule: Some("Bash(git *)".into()),
            ..HookConfig::new(HookEvent::Stop, "x")
        };
        assert!(!if_allows(
            &hook,
            HookEvent::Stop,
            "shell",
            &shell("git push"),
            None
        ));
        assert!(if_allows(
            &hook,
            HookEvent::PreToolUse,
            "shell",
            &shell("git push"),
            None
        ));
        assert!(!runs_off_tool(&hook));
    }

    #[test]
    fn event_matchers_use_claude_codes_two_paths() {
        let with = |matcher: &str| HookConfig {
            matcher: Some(matcher.into()),
            ..HookConfig::new(HookEvent::SessionStart, "x")
        };
        assert!(event_value_matches(&with("resume"), "resume"));
        assert!(!event_value_matches(&with("resume"), "startup"));
        assert!(event_value_matches(&with("startup, clear"), "clear"));
        assert!(event_value_matches(
            &with("rate_limit|overloaded"),
            "overloaded"
        ));
        assert!(event_value_matches(&with("^auth"), "authentication_failed"));
        assert!(model_matches(
            &with(".*opus.*"),
            "claude-cli::claude-opus-5-5"
        ));
        assert!(model_matches(&with("gpt-5.6-luna"), "codex::gpt-5.6-luna"));
        assert!(!model_matches(
            &with("claude-opus-5"),
            "codex::gpt-5.6-luna"
        ));
    }
}
