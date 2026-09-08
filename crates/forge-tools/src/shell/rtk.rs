//! RTK (Rust Token Killer) rewrite for the `shell` tool — docs/features/token-savings.md.
//!
//! `rtk` is a CLI proxy that runs a command and prints a compacted rendering of its output: a
//! `cargo test` run collapses to one line per suite plus the failures verbatim, `ls -la` to names
//! and sizes, `find` to a tree. When it is installed, the `shell` tool prefixes eligible commands
//! with it so the model reads the compact form. Measured on this repository (o200k tokens):
//!
//! | command                         | raw   | rtk  | saved |
//! |---------------------------------|-------|------|-------|
//! | `cargo test -p <crate> --lib`   | 2551  | 16   | 99%   |
//! | `cargo check -p <crate>`        | 95    | 30   | 68%   |
//! | `ls -la crates`                 | 597   | 75   | 87%   |
//! | `find <dir> -name '*.rs'`       | 447   | 130  | 71%   |
//! | a compile error (2 diagnostics) | full text preserved, plus a full-output tee file |
//!
//! Only commands whose RTK rendering was verified lossless-enough for an agent are rewritten.
//! `grep`/`rg` are NOT (`rtk grep` reinterprets flags such as `-h`, and it saved nothing on real
//! searches), nor are `cat`/`git diff`/`git show` (`rtk read` filters file contents and the diff
//! renderer drops hunk context — an agent editing code needs both), nor anything whose output the
//! model parses structurally (`gh --json`, `curl`, `psql`). The raw form stays one prefix away:
//! `rtk proxy <cmd>` runs the command unfiltered, and the tool result header says when a
//! rewrite happened so the model knows.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RtkRewriter {
    binary: PathBuf,
    skip: Vec<String>,
}

/// First-word programs RTK renders well for an agent, with the subcommands that qualify
/// (`None` = any invocation of the program).
const ELIGIBLE: &[(&str, Option<&[&str]>)] = &[
    ("ls", None),
    ("tree", None),
    ("find", None),
    ("wc", None),
    ("git", Some(&["status"])),
    ("cargo", Some(&["build", "check", "test", "clippy"])),
    ("npm", Some(&["test", "run"])),
    ("pnpm", Some(&["test", "run", "build", "lint"])),
    (
        "npx",
        Some(&["tsc", "eslint", "vitest", "jest", "prettier", "playwright"]),
    ),
    ("vitest", None),
    ("jest", None),
    ("tsc", None),
    ("eslint", None),
    ("prettier", None),
    ("playwright", None),
    ("dotnet", Some(&["build", "test", "restore"])),
];

impl RtkRewriter {
    /// Find a genuine `rtk` on `PATH`. `None` when absent, or when the `rtk` on PATH is some other
    /// program of the same name (the "Rust Type Kit" collision RTK's own docs warn about) — the
    /// version banner has to start with `rtk `.
    pub fn detect() -> Option<Self> {
        let path = std::env::var_os("PATH")?;
        let binary = std::env::split_paths(&path)
            .map(|dir| dir.join(if cfg!(windows) { "rtk.exe" } else { "rtk" }))
            .find(|candidate| candidate.is_file())?;
        let output = std::process::Command::new(&binary)
            .arg("--version")
            .output()
            .ok()?;
        let banner = String::from_utf8_lossy(&output.stdout);
        banner
            .trim_start()
            .starts_with("rtk ")
            .then(|| Self::at(binary))
    }

    /// A rewriter for an `rtk` at a known path (tests; no version probe).
    pub fn at(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            skip: Vec::new(),
        }
    }

    /// Programs (first word, e.g. `"cargo"`) the user asked to leave unfiltered.
    pub fn skipping(mut self, skip: Vec<String>) -> Self {
        self.skip = skip;
        self
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// The command line to run in place of `command`, or `None` when RTK has nothing to add.
    pub fn rewrite(&self, command: &str) -> Option<String> {
        rewrite_with(command, &self.skip, &self.binary.to_string_lossy())
    }
}

/// Pure rewrite rule: prefix the FIRST pipeline segment with `rtk` when its program (and, where
/// RTK's filter is subcommand-specific, its subcommand) is eligible. Everything else — env
/// assignments, `cd … &&` chains, `sudo`, a command already routed through rtk, multi-line
/// scripts — is left alone; the fallback is always the unmodified command.
pub(crate) fn rewrite_with(command: &str, skip: &[String], rtk: &str) -> Option<String> {
    let trimmed = command.trim_start();
    if trimmed.contains('\n') || trimmed.is_empty() {
        return None;
    }
    let mut words = trimmed.split_whitespace();
    let program = words.next()?;
    let program_name = Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(program);
    if program_name == "rtk" || program_name == "sudo" || program.contains('=') {
        return None;
    }
    if skip.iter().any(|s| s == program_name) {
        return None;
    }
    let (_, subcommands) = ELIGIBLE.iter().find(|(name, _)| *name == program_name)?;
    if let Some(allowed) = subcommands {
        // The first non-flag word after the program is the subcommand.
        let sub = words.find(|w| !w.starts_with('-'))?;
        if !allowed.contains(&sub) {
            return None;
        }
    }
    let leading = &command[..command.len() - trimmed.len()];
    Some(format!("{leading}{rtk} {trimmed}"))
}

/// Mark a tool result header so the model knows the output was RTK-filtered and how to get the
/// raw form. The header is the first line ("shell: exit 0 in 132ms"); the body follows a blank.
pub(crate) fn tag_result(result: String) -> String {
    match result.split_once('\n') {
        Some((header, rest)) => {
            format!("{header}  (rtk-filtered; `rtk proxy <cmd>` for raw)\n{rest}")
        }
        None => format!("{result}  (rtk-filtered; `rtk proxy <cmd>` for raw)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rw(cmd: &str) -> Option<String> {
        rewrite_with(cmd, &[], "rtk")
    }

    #[test]
    fn prefixes_eligible_programs_and_subcommands() {
        assert_eq!(rw("ls -la crates").as_deref(), Some("rtk ls -la crates"));
        assert_eq!(
            rw("cargo test -p forge-agent-config --lib 2>&1 | tail -5").as_deref(),
            Some("rtk cargo test -p forge-agent-config --lib 2>&1 | tail -5")
        );
        assert_eq!(rw("git status").as_deref(), Some("rtk git status"));
        assert_eq!(
            rw("  find . -name '*.rs'").as_deref(),
            Some("  rtk find . -name '*.rs'")
        );
        assert_eq!(
            rw("/usr/bin/wc -l src/lib.rs").as_deref(),
            Some("rtk /usr/bin/wc -l src/lib.rs")
        );
    }

    #[test]
    fn leaves_lossy_or_unknown_commands_alone() {
        for cmd in [
            "grep -h '^name' crates/*/Cargo.toml",
            "rg foo",
            "cat Cargo.toml",
            "git diff HEAD~1",
            "git log --oneline -5",
            "cargo run --bin forge",
            "cargo tree -p forge-types",
            "gh pr view 12 --json state",
            "rtk cargo test",
            "sudo ls",
            "FOO=bar ls",
            "cd crates && ls",
            "ls\ncargo test",
            "",
        ] {
            assert_eq!(rw(cmd), None, "{cmd:?} must not be rewritten");
        }
    }

    #[test]
    fn user_skip_list_wins() {
        assert_eq!(
            rewrite_with("cargo test", &["cargo".to_string()], "rtk"),
            None
        );
        assert_eq!(
            rewrite_with("ls", &["cargo".to_string()], "rtk").as_deref(),
            Some("rtk ls")
        );
    }

    #[test]
    fn tags_the_header_line_only() {
        let tagged = tag_result("shell: exit 0 in 12ms\n\nbody".to_string());
        assert!(tagged.starts_with("shell: exit 0 in 12ms  (rtk-filtered;"));
        assert!(tagged.ends_with("\n\nbody"));
        assert!(tag_result("shell: exit 0 in 1ms".to_string()).contains("rtk-filtered"));
    }
}
