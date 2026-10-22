//! Running the `` !`cmd` `` / ` ```! ` injections in a skill or command body before the model
//! sees it (Claude Code's dynamic context injection; `forge_skills::inline_shell` finds them).
//!
//! A command runs only when all of these hold, otherwise the invocation is refused with the
//! reason:
//! - the project is trusted (`forge trust`). In an untrusted project each command is replaced by a
//!   note instead of running, since a skill command such as `npm test` executes the repo's code;
//! - Forge's safety floor and the configured deny rules allow it;
//! - a rule allows it (the skill's own `allowed-tools` count, which is how Claude Code skills
//!   pre-approve their commands), the session's temper allows it, or it is a read-only command
//!   such as `git status` or `git log`. A command that would need approval is refused with a hint,
//!   because the dispatcher has no prompt to ask on.
//!
//! As in Claude Code, a failing command aborts the whole invocation; exit code 1 from search and
//! compare commands (`grep`, `rg`, `diff`, `git diff`) is a normal "no match" result.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use forge_skills::inline_shell::{self, ShellSnippet};
use forge_types::{PermissionDecision, PermissionMode, PermissionRule, SideEffect};

use crate::permission;

const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_OUTPUT_CHARS: usize = 40_000;

/// What an injected command is checked against.
#[derive(Debug, Clone)]
pub struct ShellPolicy {
    pub mode: PermissionMode,
    pub rules: Vec<PermissionRule>,
    pub workspace: PathBuf,
    pub trusted: bool,
}

/// What replaces each command in a project that is not trusted.
pub const UNTRUSTED_NOTE: &str =
    "[shell command not run: this project is not trusted — `forge trust` allows skill commands]";

impl crate::Session {
    /// The policy for this session's injected commands, with `extra` rules (the invoked skill's
    /// own turn grant) on top of the session's.
    pub fn skill_shell_policy(&self, extra: &[PermissionRule]) -> ShellPolicy {
        let workspace = self.workspace_root().to_path_buf();
        let mut rules = self.permission_rules().to_vec();
        rules.extend_from_slice(extra);
        ShellPolicy {
            mode: self.mode(),
            rules,
            trusted: forge_config::project_trust::is_trusted(&workspace),
            workspace,
        }
    }
}

/// `text` with every injected command replaced by its output. `Err` carries the message that
/// aborts the invocation. Text without injections is returned unchanged without any checks.
pub fn render(text: &str, policy: &ShellPolicy) -> Result<String, String> {
    let snippets = inline_shell::find(text);
    if snippets.is_empty() {
        return Ok(text.to_string());
    }
    let mut outputs = Vec::with_capacity(snippets.len());
    for snippet in &snippets {
        if !policy.trusted {
            outputs.push(UNTRUSTED_NOTE.to_string());
            continue;
        }
        permit(snippet, policy)?;
        outputs.push(run(snippet, &policy.workspace)?);
    }
    Ok(inline_shell::render(text, &snippets, &outputs))
}

/// Whether `text` contains any injected command (so the caller can skip snapshotting a policy).
pub fn has_injections(text: &str) -> bool {
    !inline_shell::find(text).is_empty()
}

fn permit(snippet: &ShellSnippet, policy: &ShellPolicy) -> Result<(), String> {
    let args = serde_json::json!({ "command": snippet.command });
    if let Some(note) = permission::denial_note("shell", &args) {
        return Err(format!("{} — {note}", snippet.pattern));
    }
    let decision = permission::decide_in(
        policy.mode,
        SideEffect::Shell,
        "shell",
        &args,
        &policy.rules,
        Some(&policy.workspace),
    );
    match decision {
        PermissionDecision::Allow => Ok(()),
        PermissionDecision::Ask if permission::is_read_only_shell(&snippet.command) => Ok(()),
        PermissionDecision::Ask => Err(format!(
            "{} needs approval before a skill may run it: add it to the skill's `allowed-tools` \
             (e.g. `Bash({} *)`) or to a permission allow rule",
            snippet.pattern,
            snippet.command.split_whitespace().next().unwrap_or("cmd")
        )),
        PermissionDecision::Deny => Err(format!(
            "{} is refused by a permission rule",
            snippet.pattern
        )),
    }
}

fn run(snippet: &ShellSnippet, cwd: &Path) -> Result<String, String> {
    let mut child = shell_command(&snippet.command)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            format!(
                "Shell command failed for pattern \"{}\": {e}",
                snippet.pattern
            )
        })?;
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Shell command failed for pattern \"{}\": timed out after {}s",
                    snippet.pattern,
                    TIMEOUT.as_secs()
                ));
            }
            Err(e) => {
                return Err(format!(
                    "Shell command failed for pattern \"{}\": {e}",
                    snippet.pattern
                ))
            }
        }
    };
    let output = clip(String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned());
    let code = status.code().unwrap_or(-1);
    if status.success() || (code == 1 && no_match_is_normal(&snippet.command)) {
        Ok(output)
    } else {
        Err(format!(
            "Shell command failed for pattern \"{}\" (exit {code}): {}",
            snippet.pattern,
            output.trim_end()
        ))
    }
}

/// The platform shell, with stderr merged into stdout as Claude Code's bash runner does.
fn shell_command(command: &str) -> std::process::Command {
    if cfg!(windows) {
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg(format!("({command}) 2>&1"));
        cmd
    } else {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(format!("exec 2>&1\n{command}"));
        cmd
    }
}

/// Exit code 1 means "no match / differs" for these, not failure.
fn no_match_is_normal(command: &str) -> bool {
    let mut words = command.split_whitespace();
    match words.next() {
        Some("grep" | "egrep" | "fgrep" | "rg" | "diff" | "cmp" | "comm") => true,
        Some("git") => words.next() == Some("diff"),
        _ => false,
    }
}

fn clip(mut output: String) -> String {
    if output.chars().count() > MAX_OUTPUT_CHARS {
        output = output.chars().take(MAX_OUTPUT_CHARS).collect();
        output.push_str("\n[output truncated]");
    }
    output
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn policy(mode: PermissionMode, rules: Vec<PermissionRule>, trusted: bool) -> ShellPolicy {
        ShellPolicy {
            mode,
            rules,
            workspace: std::env::temp_dir(),
            trusted,
        }
    }

    fn allow(spec: &str) -> Vec<PermissionRule> {
        forge_config::cc_rules::rules_for_spec(
            spec,
            PermissionDecision::Allow,
            forge_types::RuleSource::Turn,
            "test",
        )
    }

    #[test]
    fn allowed_commands_run_and_replace_their_placeholder() {
        let p = policy(PermissionMode::Default, allow("Bash(echo *)"), true);
        let out = render("value: !`echo hi; echo err >&2`", &p).unwrap();
        assert_eq!(out, "value: hi\nerr");
    }

    #[test]
    fn read_only_commands_run_without_a_rule() {
        let p = policy(PermissionMode::Default, Vec::new(), true);
        assert!(!render("!`pwd`", &p).unwrap().is_empty());
    }

    #[test]
    fn unapproved_commands_are_refused_with_a_hint() {
        let p = policy(PermissionMode::Default, Vec::new(), true);
        let err = render("x !`touch /tmp/forge-skill-shell-should-not-exist`", &p).unwrap_err();
        assert!(err.contains("allowed-tools"), "{err}");
        assert!(!Path::new("/tmp/forge-skill-shell-should-not-exist").exists());
    }

    #[test]
    fn untrusted_projects_never_run_commands() {
        let p = policy(PermissionMode::Bypass, allow("Bash"), false);
        assert_eq!(
            render("a !`echo hi` b", &p).unwrap(),
            format!("a {UNTRUSTED_NOTE} b")
        );
    }

    #[test]
    fn the_safety_floor_and_deny_rules_win() {
        let p = policy(PermissionMode::Bypass, allow("Bash"), true);
        assert!(render("!`sudo ls`", &p).is_err());
        let mut rules = allow("Bash");
        rules.extend(forge_config::cc_rules::rules_for_spec(
            "Bash(echo *)",
            PermissionDecision::Deny,
            forge_types::RuleSource::Configured,
            "test",
        ));
        let p = policy(PermissionMode::Default, rules, true);
        assert!(render("!`echo hi`", &p).unwrap_err().contains("refused"));
    }

    #[test]
    fn failures_abort_except_no_match_from_search_commands() {
        let p = policy(PermissionMode::Bypass, Vec::new(), true);
        let err = render("!`echo boom; exit 3`", &p).unwrap_err();
        assert!(err.contains("exit 3") && err.contains("boom"), "{err}");
        assert_eq!(render("[ !`grep zzz /dev/null` ]", &p).unwrap(), "[  ]");
    }
}
