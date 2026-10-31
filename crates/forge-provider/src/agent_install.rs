//! Installing and updating the subscription CLIs Forge bridges to (`claude`, `codex`, `agy`).
//!
//! Every action runs the vendor's own installer or updater; Forge never unpacks a binary itself.
//! The planning half is pure (`plan_install`, `plan_update`, version parsing, runtime labels) so it
//! is testable without touching the machine, and the running half takes a line callback so the CLI
//! and the daemon can both show progress.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use crate::CliKind;

/// Overrides the npm registry base used to look up the newest published version, for mirrors and
/// for tests that must never reach the network.
pub const REGISTRY_ENV: &str = "FORGE_AGENT_REGISTRY_URL";
const DEFAULT_REGISTRY: &str = "https://registry.npmjs.org";
const LATEST_TIMEOUT: Duration = Duration::from_secs(6);
const RUN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const LINE_CAP: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InstallPlan {
    pub program: String,
    pub args: Vec<String>,
    /// What to show a person (and copy into a terminal): the command as typed.
    pub display: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Install,
    Update,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgentStatus {
    /// The bridge id (`claude-cli`), which is also the provider row id.
    pub id: &'static str,
    pub name: &'static str,
    pub installed: bool,
    pub version: Option<String>,
    pub latest: Option<String>,
    pub outdated: bool,
    /// `native`, `wsl` or `windows_interop`.
    pub runtime: &'static str,
    /// What Forge would run, or `None` when the vendor ships no command Forge can drive.
    pub action: Option<Action>,
    pub command: Option<String>,
    /// Why there is nothing to run, and where to go instead.
    pub manual_hint: Option<String>,
}

pub fn display_name(kind: CliKind) -> &'static str {
    match kind {
        CliKind::ClaudeCode => "Claude Code",
        CliKind::Codex => "OpenAI Codex",
        CliKind::Antigravity => "Google Antigravity",
    }
}

fn npm_package(kind: CliKind) -> Option<&'static str> {
    match kind {
        CliKind::ClaudeCode => Some("@anthropic-ai/claude-code"),
        CliKind::Codex => Some("@openai/codex"),
        // No public registry entry we can verify; "outdated" stays unknown rather than guessed.
        CliKind::Antigravity => None,
    }
}

fn npm_program() -> &'static str {
    if cfg!(windows) {
        "npm.cmd"
    } else {
        "npm"
    }
}

fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let cand = dir.join(name);
        cand.is_file() || (cfg!(windows) && dir.join(format!("{name}.exe")).is_file())
    })
}

/// The vendor-sanctioned update command for an installed CLI. All three ship a self-updater.
pub fn plan_update(kind: CliKind) -> InstallPlan {
    let binary = kind.binary_name();
    let program = kind
        .binary_path()
        .map_or_else(|| binary.to_string(), |p| p.to_string_lossy().into_owned());
    InstallPlan {
        program,
        args: vec!["update".to_string()],
        display: format!("{binary} update"),
    }
}

/// The vendor-sanctioned first install, or `Err(hint)` when Forge has no command to drive.
/// `have` answers whether a helper tool (`curl`, `npm`) is available.
pub fn plan_install(kind: CliKind, have: &dyn Fn(&str) -> bool) -> Result<InstallPlan, String> {
    let npm = |package: &str| {
        if have("npm") {
            Ok(InstallPlan {
                program: npm_program().to_string(),
                args: vec![
                    "install".to_string(),
                    "-g".to_string(),
                    format!("{package}@latest"),
                ],
                display: format!("npm install -g {package}@latest"),
            })
        } else {
            Err(format!(
                "needs Node.js and npm; install them, then run `npm install -g {package}`"
            ))
        }
    };
    match kind {
        CliKind::ClaudeCode => {
            if !cfg!(windows) && have("curl") && have("bash") {
                let script = "curl -fsSL https://claude.ai/install.sh | bash";
                Ok(InstallPlan {
                    program: "sh".to_string(),
                    args: vec!["-c".to_string(), script.to_string()],
                    display: script.to_string(),
                })
            } else {
                npm("@anthropic-ai/claude-code")
            }
        }
        CliKind::Codex => npm("@openai/codex"),
        CliKind::Antigravity => {
            Err("install Antigravity from its download page, then run `agy` once".to_string())
        }
    }
}

/// `2.1.314 (Claude Code)` / `codex-cli 0.166.0` / `1.4.12` -> the dotted version alone.
pub fn parse_version(raw: &str) -> Option<String> {
    raw.split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .map(|tok| tok.trim_start_matches('v'))
        .find(|tok| {
            tok.chars().next().is_some_and(|c| c.is_ascii_digit())
                && tok.contains('.')
                && tok
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+')
        })
        .map(str::to_string)
}

fn numeric_parts(version: &str) -> Vec<u64> {
    version
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<Vec<u64>>>()
        .unwrap_or_default()
}

/// True when `installed` is strictly older than `latest`. At equal numbers a prerelease
/// (`1.0.0-beta`) is older than the stable `1.0.0`; unparseable input is never "outdated".
pub fn is_older(installed: &str, latest: &str) -> bool {
    let (a, b) = (numeric_parts(installed), numeric_parts(latest));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let len = a.len().max(b.len());
    let prerelease = |v: &str| v.split('+').next().unwrap_or("").contains('-');
    for i in 0..len {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x < y;
        }
    }
    // Same numbers: an installed prerelease is behind the stable release it leads up to.
    prerelease(installed) && !prerelease(latest)
}

/// Whether the CLI Forge would launch runs on this OS (`native`), inside WSL, or is a Windows
/// executable reached from WSL through interop (`/mnt/<drive>/...`).
pub fn runtime_label(path: Option<&Path>, proc_version: Option<&str>) -> &'static str {
    let in_wsl = proc_version
        .map(str::to_ascii_lowercase)
        .is_some_and(|v| v.contains("microsoft") || v.contains("wsl"));
    if !in_wsl {
        return "native";
    }
    let interop = path.is_some_and(|p| {
        let mut parts = p
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned());
        parts.next();
        parts.next().as_deref() == Some("mnt") && parts.next().is_some_and(|drive| drive.len() == 1)
    });
    if interop {
        "windows_interop"
    } else {
        "wsl"
    }
}

fn host_runtime(path: Option<&Path>) -> &'static str {
    let proc_version = if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/version").ok()
    } else {
        None
    };
    runtime_label(path, proc_version.as_deref())
}

/// The newest published version of the CLI, or `None` when unknown (offline, no registry entry).
pub async fn latest_version(kind: CliKind) -> Option<String> {
    let package = npm_package(kind)?;
    let base = std::env::var(REGISTRY_ENV).unwrap_or_else(|_| DEFAULT_REGISTRY.to_string());
    let url = format!("{}/{package}/latest", base.trim_end_matches('/'));
    let response = crate::bundled_http_client()
        .get(url)
        .timeout(LATEST_TIMEOUT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let body: serde_json::Value = response.json().await.ok()?;
    parse_version(body.get("version")?.as_str()?)
}

/// Installed-vs-latest for one bridge CLI.
pub async fn status(kind: CliKind, check_latest: bool) -> AgentStatus {
    let path = kind.binary_path();
    let installed = path.is_some();
    let (version, latest) = tokio::join!(
        async {
            if installed {
                kind.cli_version().await.and_then(|raw| parse_version(&raw))
            } else {
                None
            }
        },
        async {
            if check_latest {
                latest_version(kind).await
            } else {
                None
            }
        },
    );
    let outdated = match (&version, &latest) {
        (Some(v), Some(l)) => is_older(v, l),
        _ => false,
    };
    let (action, command, manual_hint) = if installed {
        (Some(Action::Update), Some(plan_update(kind).display), None)
    } else {
        match plan_install(kind, &on_path) {
            Ok(plan) => (Some(Action::Install), Some(plan.display), None),
            Err(hint) => (None, None, Some(hint)),
        }
    };
    AgentStatus {
        id: kind.prefix(),
        name: display_name(kind),
        installed,
        version,
        latest,
        outdated,
        runtime: host_runtime(path.as_deref()),
        action,
        command,
        manual_hint,
    }
}

pub async fn statuses(check_latest: bool) -> Vec<AgentStatus> {
    futures::future::join_all(CliKind::all().map(|kind| status(kind, check_latest))).await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub action: Action,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// Run the vendor's installer (not installed yet) or updater (installed), streaming each output
/// line to `on_line`, then re-read the installed version.
pub async fn apply(kind: CliKind, mut on_line: impl FnMut(String)) -> Result<Applied, String> {
    let installed = kind.binary_path().is_some();
    let before = if installed {
        kind.cli_version().await.and_then(|raw| parse_version(&raw))
    } else {
        None
    };
    let (action, plan) = if installed {
        (Action::Update, plan_update(kind))
    } else {
        (Action::Install, plan_install(kind, &on_path)?)
    };
    on_line(format!("$ {}", plan.display));
    run_plan(&plan, &mut on_line).await?;
    let after = kind.cli_version().await.and_then(|raw| parse_version(&raw));
    if after.is_none() {
        return Err(format!(
            "`{}` finished but {} is still not runnable; check that its install directory is on PATH",
            plan.display,
            kind.binary_name()
        ));
    }
    Ok(Applied {
        action,
        before,
        after,
    })
}

fn clean_line(raw: &str) -> Option<String> {
    let line = raw.rsplit('\r').next().unwrap_or(raw).trim_end();
    if line.trim().is_empty() {
        return None;
    }
    Some(line.chars().take(LINE_CAP).collect())
}

pub async fn run_plan(plan: &InstallPlan, on_line: &mut impl FnMut(String)) -> Result<(), String> {
    use tokio::io::{AsyncRead, AsyncReadExt};

    /// Split output on `\n` and `\r` (progress bars redraw with `\r`), never holding more than a
    /// few lines' worth of bytes for one line.
    fn forward<R: AsyncRead + Unpin + Send + 'static>(
        mut reader: R,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) {
        tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            let mut line: Vec<u8> = Vec::new();
            loop {
                let n = match reader.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                for &byte in &chunk[..n] {
                    if byte == b'\n' || byte == b'\r' {
                        if !line.is_empty() {
                            let text = String::from_utf8_lossy(&line).into_owned();
                            line.clear();
                            if tx.send(text).is_err() {
                                return;
                            }
                        }
                    } else if line.len() < LINE_CAP * 4 {
                        line.push(byte);
                    }
                }
            }
            if !line.is_empty() {
                let _ = tx.send(String::from_utf8_lossy(&line).into_owned());
            }
        });
    }

    // A neutral directory: a project `.npmrc` in the daemon's cwd must not steer a global install.
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let mut command = tokio::process::Command::new(&plan.program);
    command
        .args(&plan.args)
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("FORGE_DB")
        .env("CI", "1")
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start `{}`: {e}", plan.display))?;
    let group = child.id();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    if let Some(out) = child.stdout.take() {
        forward(out, tx.clone());
    }
    if let Some(err) = child.stderr.take() {
        forward(err, tx.clone());
    }
    drop(tx);

    let kill_group = |group: Option<u32>| {
        #[cfg(unix)]
        if let Some(pid) = group {
            // SAFETY: signalling the process group this function created.
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }
        #[cfg(not(unix))]
        let _ = group;
    };

    let drive = async {
        // The job ends when the child exits; a grandchild holding the pipes open must not keep it
        // alive, so after exit the remaining output gets a short, bounded drain.
        let mut open = true;
        let status = loop {
            tokio::select! {
                line = rx.recv(), if open => match line {
                    Some(line) => {
                        if let Some(line) = clean_line(&line) {
                            on_line(line);
                        }
                    }
                    None => open = false,
                },
                status = child.wait() => break status,
            }
        };
        let drain = async {
            while let Some(line) = rx.recv().await {
                if let Some(line) = clean_line(&line) {
                    on_line(line);
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(2), drain).await;
        status
    };
    match tokio::time::timeout(RUN_TIMEOUT, drive).await {
        Err(_) => {
            kill_group(group);
            Err(format!("`{}` timed out after 10 minutes", plan.display))
        }
        Ok(result) => {
            // Anything the installer left running in its group goes with it.
            kill_group(group);
            match result {
                Err(e) => Err(format!("`{}` failed: {e}", plan.display)),
                Ok(status) if status.success() => Ok(()),
                Ok(status) => Err(format!("`{}` exited with {status}", plan.display)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_from_each_vendors_format() {
        assert_eq!(
            parse_version("2.1.314 (Claude Code)").as_deref(),
            Some("2.1.314")
        );
        assert_eq!(
            parse_version("codex-cli 0.166.0").as_deref(),
            Some("0.166.0")
        );
        assert_eq!(parse_version("1.4.12").as_deref(), Some("1.4.12"));
        assert_eq!(
            parse_version("v3.0.0-beta.1").as_deref(),
            Some("3.0.0-beta.1")
        );
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn older_compares_numerically_not_lexically() {
        assert!(is_older("2.1.9", "2.1.10"));
        assert!(is_older("0.9.0", "0.166.0"));
        assert!(!is_older("2.1.314", "2.1.314"));
        assert!(!is_older("2.2", "2.1.314"));
        assert!(is_older("2.1", "2.1.1"));
        assert!(is_older("0.166.0-alpha.3", "0.166.0"));
        assert!(!is_older("0.166.0", "0.166.0-alpha.3"));
        assert!(!is_older("1.0.0-beta", "1.0.0-beta"));
        assert!(!is_older("garbage", "1.0.0"));
    }

    #[test]
    fn runtime_labels_separate_native_wsl_and_interop() {
        let wsl = Some("Linux version 5.15 microsoft-standard-WSL2");
        assert_eq!(
            runtime_label(
                Some(Path::new("/usr/bin/claude")),
                Some("Linux version 7.2 arch")
            ),
            "native"
        );
        assert_eq!(
            runtime_label(Some(Path::new("/home/a/.local/bin/claude")), wsl),
            "wsl"
        );
        assert_eq!(
            runtime_label(Some(Path::new("/mnt/c/Users/a/claude.exe")), wsl),
            "windows_interop"
        );
        assert_eq!(runtime_label(Some(Path::new("/mnt/wsl/x")), wsl), "wsl");
        assert_eq!(runtime_label(None, None), "native");
    }

    #[test]
    fn install_plans_follow_the_vendor_routes() {
        let all = |_: &str| true;
        let none = |_: &str| false;
        if !cfg!(windows) {
            let claude = plan_install(CliKind::ClaudeCode, &all).unwrap();
            assert_eq!(claude.program, "sh");
            assert!(claude.display.contains("claude.ai/install.sh"));
        }
        let codex = plan_install(CliKind::Codex, &all).unwrap();
        assert_eq!(codex.display, "npm install -g @openai/codex@latest");
        assert!(plan_install(CliKind::Codex, &none)
            .unwrap_err()
            .contains("npm"));
        // Without curl, Claude falls back to its npm package rather than failing.
        let only_npm = |name: &str| name == "npm";
        let claude = plan_install(CliKind::ClaudeCode, &only_npm).unwrap();
        assert_eq!(
            claude.display,
            "npm install -g @anthropic-ai/claude-code@latest"
        );
        assert!(plan_install(CliKind::Antigravity, &all).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_plan_streams_lines_and_reports_failure() {
        let plan = InstallPlan {
            program: "sh".into(),
            args: vec!["-c".into(), "echo one; echo two >&2; exit 3".into()],
            display: "fake".into(),
        };
        let mut lines = Vec::new();
        let err = run_plan(&plan, &mut |l| lines.push(l)).await.unwrap_err();
        lines.sort();
        assert_eq!(lines, ["one", "two"]);
        assert!(err.contains("exited"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_job_ends_with_its_child_even_if_a_grandchild_holds_the_pipes() {
        let plan = InstallPlan {
            program: "sh".into(),
            args: vec!["-c".into(), "sleep 30 & echo started".into()],
            display: "test".into(),
        };
        let mut lines = Vec::new();
        let started = std::time::Instant::now();
        run_plan(&plan, &mut |l| lines.push(l)).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(lines, vec!["started".to_string()]);
    }
}
