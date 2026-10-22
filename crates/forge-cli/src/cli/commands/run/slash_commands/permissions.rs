//! `/permissions` (alias `/allowed-tools`): the rules in force, and session rules written in
//! Claude Code's syntax (`/permissions allow Bash(git log:*) Read`).

use forge_core::session_slash_controls::SESSION_RULE_REASON;
use forge_tui::App;
use forge_types::{PermissionDecision, PermissionRule, RuleSource};

use super::SharedSession;

const USAGE: &str = "usage: /permissions [allow|ask|deny <Tool(pattern)>…] — e.g. /permissions allow Bash(npm test:*) Read";

pub(super) async fn run(session: &SharedSession, app: &mut App, arg: &str) {
    let arg = arg.trim();
    if arg.is_empty() {
        let s = session.lock().await;
        let temper = s.temper();
        app.note(&listing(temper.label(), temper.description(), s.permission_rules()).join("\n"));
        return;
    }
    let (verb, specs) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
    let decision = match verb.to_ascii_lowercase().as_str() {
        "allow" => PermissionDecision::Allow,
        "ask" => PermissionDecision::Ask,
        "deny" => PermissionDecision::Deny,
        _ => {
            app.note(USAGE);
            return;
        }
    };
    let rules = parse_specs(specs);
    if rules.is_empty() {
        app.note(USAGE);
        return;
    }
    let mut s = session.lock().await;
    let mut added = Vec::new();
    for (tool, patterns) in rules {
        added.push(format!("{tool} {}", shown_patterns(&patterns)));
        s.add_session_rule(tool, patterns, decision);
    }
    app.note(&format!(
        "✓ {} for this session: {} — keep a rule with [[permissions.rules]] in .forge/config.toml",
        decision_word(decision),
        added.join(", ")
    ));
}

fn decision_word(decision: PermissionDecision) -> &'static str {
    match decision {
        PermissionDecision::Allow => "allow",
        PermissionDecision::Ask => "ask",
        PermissionDecision::Deny => "deny",
    }
}

fn shown_patterns(patterns: &[String]) -> String {
    if patterns.is_empty() {
        "(any call)".to_string()
    } else {
        patterns.join(", ")
    }
}

fn origin(rule: &PermissionRule) -> &'static str {
    match (rule.source, rule.reason.as_deref()) {
        (RuleSource::Builtin, _) => "built-in",
        (_, Some(SESSION_RULE_REASON)) => "this session",
        (_, Some(reason)) if reason.starts_with("user answered 'always'") => "always-allow answer",
        _ => "config",
    }
}

/// The `/permissions` listing. Built-in safety rules (secret paths, catastrophic commands) are
/// counted rather than listed: no setting can lift them, and they run to dozens of lines.
fn listing(temper: &str, description: &str, rules: &[PermissionRule]) -> Vec<String> {
    let builtin = rules
        .iter()
        .filter(|r| r.source == RuleSource::Builtin)
        .count();
    let mut lines = vec![format!("permissions · temper {temper}: {description}")];
    let mut own: Vec<&PermissionRule> = rules
        .iter()
        .filter(|r| r.source != RuleSource::Builtin)
        .collect();
    own.sort_by_key(|r| match r.decision {
        PermissionDecision::Deny => 0,
        PermissionDecision::Ask => 1,
        PermissionDecision::Allow => 2,
    });
    if own.is_empty() {
        lines.push("  no allow/ask/deny rules beyond the temper".to_string());
    }
    for rule in own {
        lines.push(format!(
            "  {:<6} {:<14} {:<34} {}",
            decision_word(rule.decision),
            rule.tool,
            shown_patterns(&rule.patterns),
            origin(rule)
        ));
    }
    lines.push(format!(
        "  + {builtin} built-in safety rules (secret files, catastrophic deletes, privilege escalation) that no temper lifts"
    ));
    lines.push(format!("  {USAGE}; Shift+Tab or /mode changes the temper"));
    lines
}

/// `Bash(git log:*) Read, WebFetch` → Forge `(tool, patterns)` pairs. A Claude Code tool name maps
/// onto Forge's equivalents (`Edit` covers `edit_file` and `apply_patch`); any other name, such
/// as `shell` or an MCP tool, is taken as it is. `prefix:*` becomes the glob `prefix *`.
fn parse_specs(specs: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for entry in split_entries(specs) {
        let (tool, pattern) = match entry.split_once('(') {
            Some((tool, rest)) if rest.ends_with(')') => {
                (tool.trim(), Some(rest[..rest.len() - 1].trim()))
            }
            _ => (entry.trim(), None),
        };
        if tool.is_empty() {
            continue;
        }
        let patterns: Vec<String> = pattern
            .filter(|p| !p.is_empty() && *p != "*")
            .map(|p| match p.strip_suffix(":*") {
                Some(prefix) => format!("{prefix} *"),
                None => p.to_string(),
            })
            .into_iter()
            .collect();
        let forge = forge_config::forge_tools_from_cc(tool);
        if forge.is_empty() {
            out.push((tool.to_string(), patterns));
        } else {
            out.extend(forge.into_iter().map(|t| (t.to_string(), patterns.clone())));
        }
    }
    out
}

/// Split on spaces and commas that are not inside parentheses.
fn split_entries(specs: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in specs.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ' ' | ',' if depth == 0 => {
                if !current.is_empty() {
                    entries.push(std::mem::take(&mut current));
                }
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    if !current.is_empty() {
        entries.push(current);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(
        tool: &str,
        decision: PermissionDecision,
        source: RuleSource,
        reason: Option<&str>,
    ) -> PermissionRule {
        PermissionRule {
            tool: tool.into(),
            patterns: vec![],
            decision,
            source,
            reason: reason.map(str::to_string),
        }
    }

    #[test]
    fn claude_code_rule_syntax_maps_onto_forge_tools() {
        assert_eq!(
            parse_specs("Bash(git log:*) Read"),
            vec![
                ("shell".to_string(), vec!["git log *".to_string()]),
                ("read_file".to_string(), vec![]),
            ]
        );
        assert_eq!(
            parse_specs("Edit(src/**), mcp__github__create_issue"),
            vec![
                ("edit_file".to_string(), vec!["src/**".to_string()]),
                ("apply_patch".to_string(), vec!["src/**".to_string()]),
                ("mcp__github__create_issue".to_string(), vec![]),
            ]
        );
        // Spaces inside the parentheses belong to the pattern.
        assert_eq!(
            parse_specs("Bash(npm run test:*)"),
            vec![("shell".to_string(), vec!["npm run test *".to_string()])]
        );
        assert!(parse_specs("  ").is_empty());
    }

    #[test]
    fn the_listing_counts_built_ins_and_names_where_each_rule_came_from() {
        let rules = vec![
            rule(
                "read_file",
                PermissionDecision::Deny,
                RuleSource::Builtin,
                None,
            ),
            rule(
                "shell",
                PermissionDecision::Allow,
                RuleSource::Configured,
                Some(SESSION_RULE_REASON),
            ),
            rule(
                "web_fetch",
                PermissionDecision::Deny,
                RuleSource::Configured,
                None,
            ),
            rule(
                "write_file",
                PermissionDecision::Allow,
                RuleSource::Configured,
                Some("user answered 'always' at runtime prompt"),
            ),
        ];
        let lines = listing("Auto-edit", "edits run", &rules);
        assert!(lines[0].contains("temper Auto-edit"));
        // Deny first, then allow.
        let deny = lines.iter().position(|l| l.contains("web_fetch")).unwrap();
        let allow = lines.iter().position(|l| l.contains("shell")).unwrap();
        assert!(deny < allow);
        assert!(lines[allow].contains("this session"));
        assert!(lines
            .iter()
            .any(|l| l.contains("write_file") && l.contains("always-allow answer")));
        assert!(lines
            .iter()
            .any(|l| l.contains("web_fetch") && l.ends_with("config")));
        assert!(lines
            .iter()
            .any(|l| l.contains("+ 1 built-in safety rules")));
        assert!(
            !lines.iter().any(|l| l.contains("read_file")),
            "built-ins are counted, not listed"
        );
    }
}
