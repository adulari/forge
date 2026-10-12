//! Where Claude Code keeps a session, and whether one is in use right now.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};

/// Claude Code's own limit on a project directory name; longer ones get a hash suffix this
/// module cannot reproduce.
const SLUG_MAX: usize = 200;
/// How much of a transcript's start is read to learn who wrote it.
const HEAD_BYTES: u64 = 256 * 1024;
/// How much of its end is read to find the last permission mode.
const TAIL_BYTES: u64 = 4 * 1024 * 1024;
/// A transcript written this recently while a `claude` runs in its directory is taken as open.
const HOT_WINDOW: Duration = Duration::from_secs(5);
const FALLBACK_VERSION: &str = "2.1.296";

/// `~/.claude/projects`, or `$CLAUDE_CONFIG_DIR/projects` when the variable is set (the `claude`
/// binary this process starts reads the same variable, so both always agree).
pub(crate) fn projects_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir).join("projects"));
    }
    Ok(forge_config::claude_dir()
        .context("no home directory: cannot locate ~/.claude")?
        .join("projects"))
}

/// The directory name Claude Code gives a working directory: every character that is not an ASCII
/// letter or digit becomes `-` (one per UTF-16 unit, as the JavaScript original does).
pub(crate) fn slug(cwd: &str) -> String {
    let mut out = String::with_capacity(cwd.len());
    for c in cwd.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.extend(std::iter::repeat_n('-', c.len_utf16()));
        }
    }
    out
}

/// The project directory for `cwd` under `projects`, whether or not it exists yet.
pub(crate) fn project_dir(projects: &Path, cwd: &str) -> Result<PathBuf> {
    let slug = slug(cwd);
    if slug.len() <= SLUG_MAX {
        return Ok(projects.join(slug));
    }
    let prefix: String = slug.chars().take(SLUG_MAX).collect();
    for entry in std::fs::read_dir(projects).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            return Ok(entry.path());
        }
    }
    bail!("{cwd} is too long for Claude Code's project directory naming; open it once in claude first")
}

/// A Claude Code transcript of a person's own session.
#[derive(Debug, Clone)]
pub(crate) struct CcSession {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    pub(crate) modified: SystemTime,
}

/// The session id a transcript path stands for.
pub(crate) fn session_id_of(path: &Path) -> Option<String> {
    Some(path.file_stem()?.to_str()?.to_string())
}

/// Whether the transcript belongs to an interactive Claude Code session rather than to a
/// headless run. Forge's own claude-cli bridge writes `sdk-cli` transcripts (calling Forge's tools
/// as `mcp__forge__*`) into the very same directory; they are never "the user's session".
pub(crate) fn is_interactive(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut reader = BufReader::new(file.take(HEAD_BYTES));
    let mut line = String::new();
    let mut entrypoint: Option<String> = None;
    let mut bridge_tools = false;
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        if line.contains("mcp__forge__") {
            bridge_tools = true;
        }
        if entrypoint.is_none() && line.contains("\"entrypoint\"") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                entrypoint = v["entrypoint"].as_str().map(str::to_string);
            }
        }
        line.clear();
    }
    !bridge_tools && entrypoint.is_some_and(|e| !e.starts_with("sdk"))
}

/// Interactive sessions of `cwd`, most recently written first.
pub(crate) fn interactive_sessions(projects: &Path, cwd: &str) -> Vec<CcSession> {
    let Ok(dir) = project_dir(projects, cwd) else {
        return Vec::new();
    };
    let mut found: Vec<CcSession> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension()? != "jsonl" {
                return None;
            }
            let modified = e.metadata().ok()?.modified().ok()?;
            Some(CcSession {
                id: session_id_of(&path)?,
                path,
                modified,
            })
        })
        .collect();
    found.sort_by_key(|s| std::cmp::Reverse(s.modified));
    found.retain(|s| is_interactive(&s.path));
    found
}

/// The newest `permissionMode` in the last few MB of a transcript.
pub(crate) fn last_permission_mode(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
        .ok()?;
    let mut text = String::new();
    BufReader::new(file).read_to_string(&mut text).ok()?;
    text.lines().rev().find_map(|l| {
        if !l.contains("\"permissionMode\"") {
            return None;
        }
        serde_json::from_str::<serde_json::Value>(l)
            .ok()?
            .get("permissionMode")?
            .as_str()
            .map(str::to_string)
    })
}

/// Forge's permission mode for a Claude Code `permissionMode`; `None` leaves Forge's own.
pub(crate) fn forge_mode_for(cc_mode: &str) -> Option<&'static str> {
    match cc_mode {
        "bypassPermissions" => Some("bypass"),
        "acceptEdits" => Some("accept-edits"),
        "plan" => Some("plan"),
        _ => None,
    }
}

/// The `claude` flags for a stored Forge permission mode.
pub(crate) fn claude_mode_flags(forge_mode: &str) -> Vec<&'static str> {
    match forge_mode {
        "Bypass" => vec!["--dangerously-skip-permissions"],
        "AcceptEdits" => vec!["--permission-mode", "acceptEdits"],
        "Plan" => vec!["--permission-mode", "plan"],
        _ => Vec::new(),
    }
}

/// A running `claude` process, as far as `/proc` or `ps` shows.
struct Proc {
    args: Vec<String>,
    cwd: Option<PathBuf>,
}

fn is_claude(args: &[String]) -> bool {
    let name = |s: &str| {
        Path::new(s)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string()
    };
    let first = args.first().map(|a| name(a)).unwrap_or_default();
    first == "claude"
        || (matches!(first.as_str(), "node" | "bun")
            && args
                .get(1)
                .is_some_and(|a| matches!(name(a).as_str(), "claude" | "cli.js")))
}

/// Interactive means not print mode: Forge's own bridge runs `claude -p` for every turn.
fn is_interactive_process(args: &[String]) -> bool {
    !args.iter().any(|a| a == "-p" || a == "--print")
}

#[cfg(target_os = "linux")]
fn claude_processes() -> Vec<Proc> {
    let me = std::process::id().to_string();
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == me || !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let args: Vec<String> = raw
            .split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).to_string())
            .collect();
        if is_claude(&args) {
            out.push(Proc {
                args,
                cwd: std::fs::read_link(entry.path().join("cwd")).ok(),
            });
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn claude_processes() -> Vec<Proc> {
    let Ok(out) = std::process::Command::new("ps")
        .args(["-axo", "args="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .filter(|args| is_claude(args))
        .map(|args| Proc { args, cwd: None })
        .collect()
}

/// Why this Claude Code session looks open right now, if it does: a running `claude` names its
/// id on the command line, or its transcript was just written while a `claude` runs in its
/// working directory.
pub(crate) fn open_reason(session_id: &str, path: &Path, cwd: &str) -> Option<String> {
    let procs: Vec<Proc> = claude_processes()
        .into_iter()
        .filter(|p| is_interactive_process(&p.args))
        .collect();
    open_reason_in(&procs, session_id, path, cwd)
}

fn open_reason_in(procs: &[Proc], session_id: &str, path: &Path, cwd: &str) -> Option<String> {
    if procs
        .iter()
        .any(|p| p.args.iter().any(|a| a.contains(session_id)))
    {
        return Some(format!("a running `claude` has session {session_id} open"));
    }
    let hot = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < HOT_WINDOW);
    let in_cwd = procs
        .iter()
        .any(|p| p.cwd.as_deref().is_none_or(|c| c == Path::new(cwd)));
    (hot && in_cwd).then(|| {
        format!(
            "{} was written moments ago and `claude` is running in {cwd}",
            path.display()
        )
    })
}

/// The installed Claude Code version, for the `version` field of entries Forge writes.
pub(crate) fn claude_version() -> String {
    let out = std::process::Command::new("claude")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    out.ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.split_whitespace().next().map(str::to_string))
        .filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or_else(|| FALLBACK_VERSION.to_string())
}

/// The branch checked out in `cwd`, read from `.git/HEAD` without running git.
pub(crate) fn git_branch(cwd: &str) -> Option<String> {
    let mut dir = Some(Path::new(cwd));
    while let Some(d) = dir {
        let git = d.join(".git");
        let head = if git.is_dir() {
            git.join("HEAD")
        } else if git.is_file() {
            // A worktree: `.git` is a file `gitdir: <path>`.
            let target = std::fs::read_to_string(&git).ok()?;
            PathBuf::from(target.trim().strip_prefix("gitdir:")?.trim()).join("HEAD")
        } else {
            dir = d.parent();
            continue;
        };
        let text = std::fs::read_to_string(head).ok()?;
        return Some(
            text.trim()
                .strip_prefix("ref: refs/heads/")
                .unwrap_or("HEAD")
                .to_string(),
        );
    }
    None
}

/// Locate a session's transcript by id under every project directory.
pub(crate) fn find_by_id(session: &str) -> Option<PathBuf> {
    let projects = projects_dir().ok()?;
    let wanted = format!("{session}.jsonl");
    let mut found: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(&projects)
        .ok()?
        .flatten()
        .filter_map(|dir| {
            let candidate = dir.path().join(&wanted);
            let meta = std::fs::metadata(&candidate).ok()?;
            Some((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), candidate))
        })
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_replaces_every_non_alphanumeric_character() {
        assert_eq!(
            slug("/home/floris/.cache/forge-x"),
            "-home-floris--cache-forge-x"
        );
        assert_eq!(
            slug("/home/forge/Documents/Adulari/Trading Bots/vectra-bot"),
            "-home-forge-Documents-Adulari-Trading-Bots-vectra-bot"
        );
        assert_eq!(slug("/tmp/ünï"), "-tmp--n-");
        assert_eq!(slug("/a/😀"), "-a---");
    }

    fn write(dir: &Path, name: &str, lines: &[&str]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        p
    }

    #[test]
    fn the_picker_ignores_bridge_and_headless_transcripts() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(slug("/w/proj"));
        std::fs::create_dir_all(&dir).unwrap();
        write(
            &dir,
            "human.jsonl",
            &[
                r#"{"type":"user","uuid":"u","entrypoint":"cli","message":{"role":"user","content":"hi"}}"#,
            ],
        );
        write(
            &dir,
            "headless.jsonl",
            &[
                r#"{"type":"user","uuid":"u","entrypoint":"sdk-cli","message":{"role":"user","content":"hi"}}"#,
            ],
        );
        write(
            &dir,
            "bridge.jsonl",
            &[
                r#"{"type":"assistant","uuid":"a","entrypoint":"cli","message":{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"mcp__forge__shell","input":{}}]}}"#,
            ],
        );
        write(
            &dir,
            "empty.jsonl",
            &[r#"{"type":"last-prompt","lastPrompt":"x"}"#],
        );
        let found = interactive_sessions(root.path(), "/w/proj");
        let ids: Vec<&str> = found.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["human"]);
    }

    #[test]
    fn modes_map_both_ways() {
        assert_eq!(forge_mode_for("bypassPermissions"), Some("bypass"));
        assert_eq!(forge_mode_for("acceptEdits"), Some("accept-edits"));
        assert_eq!(forge_mode_for("plan"), Some("plan"));
        assert_eq!(forge_mode_for("default"), None);
        assert_eq!(forge_mode_for("auto"), None);
        assert_eq!(
            claude_mode_flags("Bypass"),
            vec!["--dangerously-skip-permissions"]
        );
        assert_eq!(
            claude_mode_flags("AcceptEdits"),
            vec!["--permission-mode", "acceptEdits"]
        );
        assert!(claude_mode_flags("Default").is_empty());
    }

    #[test]
    fn last_permission_mode_is_the_newest_one() {
        let root = tempfile::tempdir().unwrap();
        let p = write(
            root.path(),
            "s.jsonl",
            &[
                r#"{"type":"permission-mode","permissionMode":"plan","sessionId":"s"}"#,
                r#"{"type":"user","uuid":"u"}"#,
                r#"{"type":"permission-mode","permissionMode":"bypassPermissions","sessionId":"s"}"#,
            ],
        );
        assert_eq!(
            last_permission_mode(&p).as_deref(),
            Some("bypassPermissions")
        );
    }

    #[test]
    fn an_open_session_is_found_by_command_line_or_recent_write() {
        let root = tempfile::tempdir().unwrap();
        let p = write(root.path(), "abc.jsonl", &[r#"{"type":"user"}"#]);
        let proc_for = |args: &[&str], cwd: &str| Proc {
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: Some(PathBuf::from(cwd)),
        };
        let old = std::time::SystemTime::now() - Duration::from_secs(600);
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let by_id = [proc_for(&["claude", "--resume", "abc"], "/elsewhere")];
        assert!(open_reason_in(&by_id, "abc", &p, "/w").is_some());
        let idle = [proc_for(&["claude"], "/w")];
        assert!(
            open_reason_in(&idle, "abc", &p, "/w").is_none(),
            "old file, other id"
        );
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(std::time::SystemTime::now())
            .unwrap();
        assert!(open_reason_in(&idle, "abc", &p, "/w").is_some());
        assert!(
            open_reason_in(&[], "abc", &p, "/w").is_none(),
            "no claude running"
        );
        let elsewhere = [proc_for(&["claude"], "/other")];
        assert!(open_reason_in(&elsewhere, "abc", &p, "/w").is_none());
    }

    #[test]
    fn claude_processes_are_recognised_but_print_mode_is_not_interactive() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_claude(&a(&[
            "/home/u/.local/bin/claude",
            "--resume",
            "x"
        ])));
        assert!(is_claude(&a(&[
            "node",
            "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"
        ])));
        assert!(!is_claude(&a(&["vim", "claude.md"])));
        assert!(!is_interactive_process(&a(&["claude", "-p", "hi"])));
        assert!(is_interactive_process(&a(&["claude", "--resume", "x"])));
    }
}
