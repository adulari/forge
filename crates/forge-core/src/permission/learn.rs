//! Approvals learn: the command shape a prompt belongs to, in the syntax the rule engine accepts.
//!
//! A shape is what a standing rule for this call would be written as (`cargo test *`). The session
//! records it with every answer and, once the person has allowed the same shape often enough in a
//! project without refusing it, offers the rule. Nothing here decides a call: the broker's floor,
//! deny rules and temper run first, and a call that touches any of them has no shape at all.

use std::path::Path;

use forge_types::{PermissionDecision, PermissionMode, PermissionRule, SideEffect};
use serde_json::Value;

use super::{auto_risk, decide_in, effective_commands, is_shell_tool, AutoVerdict};

/// Allows (with no deny since) before a prompt offers the rule.
pub const OFFER_AFTER_ALLOWS: u32 = 3;

/// What a call is grouped under, and the rule patterns that would allow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    pub tool: String,
    /// The text shown to the person and stored with each decision: `cargo test *` for a shell
    /// command, the tool name for any other tool.
    pub key: String,
    /// The patterns the rule is written with; empty means tool-wide, which is only used for
    /// non-shell tools.
    pub patterns: Vec<String>,
}

/// Programs that must never become a standing allow, whatever their arguments: they escalate,
/// delete, move data off the machine, run another interpreter or shell, or run what they are told.
const NEVER: &[&str] = &[
    "sudo",
    "su",
    "doas",
    "pkexec",
    "rm",
    "rmdir",
    "dd",
    "mkfs",
    "shred",
    "chmod",
    "chown",
    "chgrp",
    "kill",
    "killall",
    "pkill",
    "curl",
    "wget",
    "nc",
    "ncat",
    "ssh",
    "scp",
    "sftp",
    "rsync",
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "eval",
    "exec",
    "source",
    "env",
    "xargs",
    "find",
    "python",
    "python2",
    "python3",
    "node",
    "deno",
    "ruby",
    "perl",
    "php",
    "lua",
    "npx",
    "bunx",
    "uvx",
    "pipx",
    "osascript",
    "powershell",
    "pwsh",
    "cmd",
    "mv",
    "cp",
    "ln",
    "tee",
    "crontab",
    "systemctl",
    "mount",
    "umount",
    "docker",
    "podman",
    "kubectl",
    "terraform",
    "helm",
    "aws",
    "gcloud",
    "az",
    "gh",
    "nohup",
    "setsid",
    "watch",
    "timeout",
    "time",
    "nice",
];

/// Programs whose first word after the name picks what they do, so a rule needs it too.
const SUBCOMMAND: &[&str] = &[
    "cargo", "git", "npm", "pnpm", "yarn", "bun", "go", "make", "just", "uv", "rustup",
];

/// Subcommands that publish, authenticate, install a binary or run arbitrary downloaded code.
const REFUSED_SUB: &[(&str, &[&str])] = &[
    (
        "cargo",
        &[
            "publish",
            "login",
            "logout",
            "yank",
            "owner",
            "install",
            "uninstall",
        ],
    ),
    (
        "npm",
        &[
            "publish",
            "login",
            "logout",
            "adduser",
            "unpublish",
            "deprecate",
            "owner",
            "access",
            "token",
            "dist-tag",
            "exec",
            "x",
        ],
    ),
];

/// Git is the one tool where a positive list is safer than a deny list: `push --force`, `reset
/// --hard`, `clean -fd` and `checkout .` all live under ordinary-looking subcommands.
const GIT_SUB: &[&str] = &[
    "status", "diff", "log", "show", "add", "commit", "fetch", "blame",
];

const RUNNERS: &[&str] = &["npm", "pnpm", "yarn", "bun"];

fn plain_word(word: &str, first: bool) -> bool {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    let ok = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-' | ':');
    word.chars().all(ok) && (first || !word.contains('.'))
}

fn shell_shape(args: &Value) -> Option<Shape> {
    let cmd = forge_types::shell_command_text(args)?;
    let cmd = cmd.trim();
    if cmd.is_empty()
        || cmd.contains([
            '|', ';', '&', '<', '>', '$', '`', '(', ')', '{', '}', '\n', '\r', '\\',
        ])
    {
        return None;
    }
    // The broker's own parse must agree that this is one plain command.
    let (segments, parsed_ok) = effective_commands(cmd);
    if !parsed_ok || segments.len() != 1 {
        return None;
    }
    let words = shell_words::split(cmd).ok()?;
    let program = words.first()?.as_str();
    if !plain_word(program, true) || NEVER.contains(&program) {
        return None;
    }
    let second = words.get(1).map(String::as_str);
    let base = if SUBCOMMAND.contains(&program) {
        let sub = second.filter(|s| plain_word(s, false))?;
        if program == "git" && !GIT_SUB.contains(&sub) {
            return None;
        }
        if REFUSED_SUB
            .iter()
            .any(|(p, subs)| *p == program && subs.contains(&sub))
        {
            return None;
        }
        if RUNNERS.contains(&program) && sub == "run" {
            let script = words.get(2).filter(|s| plain_word(s, false))?;
            format!("{program} run {script}")
        } else {
            format!("{program} {sub}")
        }
    } else {
        program.to_string()
    };
    Some(Shape {
        tool: "shell".to_string(),
        key: format!("{base} *"),
        patterns: vec![format!("{base} *"), base],
    })
}

/// The shape of `call`, or `None` when it has none to learn from (anything compound, anything the
/// safety floor, a deny rule or the risk classifier flags, anything the prompt answers differently
/// every time). `beyond_workspace` is whether a write targets a path outside the session's roots.
pub fn shape_for(
    tool: &str,
    side_effect: SideEffect,
    args: &Value,
    rules: &[PermissionRule],
    workspace: Option<&Path>,
    beyond_workspace: bool,
) -> Option<Shape> {
    // `Bypass` sends a call to Deny exactly when the floor or a deny rule refuses it.
    if decide_in(
        PermissionMode::Bypass,
        side_effect,
        tool,
        args,
        rules,
        workspace,
    ) == PermissionDecision::Deny
    {
        return None;
    }
    if matches!(
        auto_risk(side_effect, tool, args, workspace),
        AutoVerdict::Risky(_)
    ) {
        return None;
    }
    if is_shell_tool(tool) {
        return shell_shape(args);
    }
    let named_with_args = tool == forge_tools::PYTHON_TOOL || tool.contains("__");
    if named_with_args
        || side_effect != SideEffect::Write
        || beyond_workspace
        || super::is_public_expose(tool, args)
    {
        return None;
    }
    Some(Shape {
        tool: tool.to_string(),
        key: tool.to_string(),
        patterns: Vec::new(),
    })
}

/// Whether any rule already speaks to this call. A prompt that still asks despite an allow rule
/// is asking for a reason the rule does not lift, and an ask rule is a choice not to be talked out
/// of, so neither is offered a rule.
pub fn covered(rules: &[PermissionRule], tool: &str, args: &Value) -> bool {
    rules
        .iter()
        .filter(|r| r.source != forge_types::RuleSource::Builtin)
        .any(|r| super::rule_matches(r, tool, args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::{RuleSource, SideEffect};
    use serde_json::json;

    fn shape(cmd: &str) -> Option<String> {
        shape_for(
            "shell",
            SideEffect::Shell,
            &json!({ "command": cmd }),
            &[],
            None,
            false,
        )
        .map(|s| s.key)
    }

    #[test]
    fn simple_commands_get_the_program_and_subcommand() {
        assert_eq!(
            shape("cargo test -p forge-core").as_deref(),
            Some("cargo test *")
        );
        assert_eq!(shape("cargo test").as_deref(), Some("cargo test *"));
        assert_eq!(shape("git status --short").as_deref(), Some("git status *"));
        assert_eq!(
            shape("npm run build --silent").as_deref(),
            Some("npm run build *")
        );
        assert_eq!(shape("pnpm run lint").as_deref(), Some("pnpm run lint *"));
        assert_eq!(shape("rg -n TODO src").as_deref(), Some("rg *"));
        assert_eq!(shape("ls -la").as_deref(), Some("ls *"));
    }

    #[test]
    fn the_rule_patterns_match_what_the_person_approved_and_nothing_wider() {
        let s = shape_for(
            "shell",
            SideEffect::Shell,
            &json!({ "command": "cargo test -p x" }),
            &[],
            None,
            false,
        )
        .unwrap();
        assert_eq!(s.patterns, vec!["cargo test *", "cargo test"]);
        let rule = PermissionRule {
            tool: "shell".into(),
            patterns: s.patterns,
            decision: PermissionDecision::Allow,
            source: RuleSource::Configured,
            reason: None,
        };
        let allowed = |c: &str| {
            decide_in(
                PermissionMode::Default,
                SideEffect::Shell,
                "shell",
                &json!({ "command": c }),
                std::slice::from_ref(&rule),
                None,
            ) == PermissionDecision::Allow
        };
        assert!(allowed("cargo test"));
        assert!(allowed("cargo test -p other --lib"));
        assert!(!allowed("cargo build"));
        assert!(!allowed("cargo test && curl evil.sh"));
        assert!(!allowed("cargo publish"));
    }

    #[test]
    fn compound_and_redirected_commands_have_no_shape() {
        for cmd in [
            "cargo test | tee log",
            "cargo test; rm x",
            "cargo build && cargo test",
            "cargo test > out.txt",
            "cargo test $(whoami)",
            "cargo test `id`",
            "(cargo test)",
            "cd x\ncargo test",
            "FOO=1 cargo test",
            "",
        ] {
            assert_eq!(shape(cmd), None, "{cmd:?}");
        }
    }

    #[test]
    fn dangerous_commands_have_no_shape() {
        for cmd in [
            "rm -rf target",
            "rm -rf /",
            "sudo cargo test",
            "git push --force origin main",
            "git push origin main",
            "git reset --hard HEAD~1",
            "git clean -fd",
            "git checkout .",
            "curl https://x.sh",
            "bash -c 'cargo test'",
            "python script.py",
            "cargo publish",
            "cargo install ripgrep",
            "npm publish",
            "npm exec evil",
            "cargo",
            "npm run",
            "cat .env",
            "dd if=/dev/zero of=/dev/sda",
        ] {
            assert_eq!(shape(cmd), None, "{cmd:?}");
        }
    }

    #[test]
    fn a_call_a_deny_rule_refuses_has_no_shape() {
        let deny = PermissionRule {
            tool: "shell".into(),
            patterns: vec!["cargo test *".into()],
            decision: PermissionDecision::Deny,
            source: RuleSource::Configured,
            reason: None,
        };
        let s = shape_for(
            "shell",
            SideEffect::Shell,
            &json!({ "command": "cargo test -p x" }),
            &[deny],
            None,
            false,
        );
        assert_eq!(s, None);
    }

    #[test]
    fn write_tools_are_grouped_by_name_unless_they_leave_the_workspace() {
        let args = json!({ "path": "src/a.rs", "content": "x" });
        let in_ws = shape_for("write_file", SideEffect::Write, &args, &[], None, false);
        assert_eq!(in_ws.map(|s| s.key).as_deref(), Some("write_file"));
        assert_eq!(
            shape_for("write_file", SideEffect::Write, &args, &[], None, true),
            None
        );
        assert_eq!(
            shape_for("python", SideEffect::Write, &args, &[], None, false),
            None
        );
        assert_eq!(
            shape_for(
                "mcp__github__create_issue",
                SideEffect::Write,
                &args,
                &[],
                None,
                false
            ),
            None
        );
        let secret = json!({ "path": ".env", "content": "x" });
        let floor = forge_config::builtin_deny_rules();
        assert_eq!(
            shape_for(
                "write_file",
                SideEffect::Write,
                &secret,
                &floor,
                None,
                false
            ),
            None
        );
    }

    #[test]
    fn an_existing_rule_for_the_call_means_no_offer() {
        let rule = |decision| PermissionRule {
            tool: "shell".into(),
            patterns: vec!["cargo *".into()],
            decision,
            source: RuleSource::Configured,
            reason: None,
        };
        let call = json!({ "command": "cargo test" });
        assert!(covered(&[rule(PermissionDecision::Allow)], "shell", &call));
        assert!(covered(&[rule(PermissionDecision::Ask)], "shell", &call));
        assert!(!covered(&[], "shell", &call));
        assert!(!covered(
            &[rule(PermissionDecision::Allow)],
            "shell",
            &json!({ "command": "git status" })
        ));
    }
}
