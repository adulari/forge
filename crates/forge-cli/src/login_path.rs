//! Give a headless `forge serve` the user's login `PATH`.
//!
//! A `systemd --user` unit on a server has no desktop session to import a `PATH` from, so the
//! daemon sees only `/usr/local/sbin:...:/snap/bin`. Everything the daemon spawns inherits that:
//! the `claude` CLI in `~/.local/bin` is "not installed" (so model discovery caches a catalog
//! without any `claude-cli::` models), and agent shell tools cannot find `cargo` or `node`.
//! Asking the user's login shell what its `PATH` is fixes both at the root.

use std::io::{IsTerminal, Read};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Login entries first, then the existing entries, each directory once. Returns the merged value
/// and the login entries that were not already present.
pub fn merge_paths(login: &str, current: &str) -> (String, Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    let mut merged: Vec<&str> = Vec::new();
    let mut added = Vec::new();
    let current_set: std::collections::HashSet<&str> = current.split(':').collect();
    for dir in login.split(':').chain(current.split(':')) {
        if dir.is_empty() || !seen.insert(dir) {
            continue;
        }
        if !current_set.contains(dir) {
            added.push(dir.to_string());
        }
        merged.push(dir);
    }
    (merged.join(":"), added)
}

/// A login shell may print a banner before our `printf`; the PATH is the last non-empty line.
fn parse_shell_output(out: &str) -> Option<&str> {
    out.lines().rev().map(str::trim).find(|l| !l.is_empty())
}

fn login_shell_path(shell: &str, timeout: Duration) -> Option<String> {
    let mut child = Command::new(shell)
        .args(["-l", "-c", "printf %s \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let out = reader.join().ok()?;
    parse_shell_output(&out).map(str::to_string)
}

/// Whether `args` (argv, program name first) invoke the long-lived daemon. `forge serve` only —
/// `mcp-serve` is spawned by a CLI that already has the user's environment.
pub fn is_daemon_invocation(args: &[String]) -> bool {
    args.iter()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .is_some_and(|a| a == "serve")
}

/// Hydrate `PATH` from the login shell for `forge serve`. MUST run before the tokio runtime or any
/// other thread exists: `set_var` is not sound against concurrent environment reads. Returns the
/// entries added, for logging once tracing is up.
#[cfg(unix)]
pub fn hydrate_for_daemon() -> Vec<String> {
    let args: Vec<String> = std::env::args().collect();
    if !is_daemon_invocation(&args)
        || std::io::stdout().is_terminal()
        || std::env::var_os("FORGE_NO_LOGIN_PATH").is_some_and(|v| v == "1")
    {
        return Vec::new();
    }
    let current = std::env::var("PATH").unwrap_or_default();
    let shells = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .into_iter()
        .chain(std::iter::once("/bin/sh".to_string()));
    for shell in shells {
        if let Some(login) = login_shell_path(&shell, Duration::from_secs(3)) {
            let (merged, added) = merge_paths(&login, &current);
            if !added.is_empty() {
                std::env::set_var("PATH", merged);
            }
            return added;
        }
    }
    Vec::new()
}

#[cfg(not(unix))]
pub fn hydrate_for_daemon() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_entries_come_first_and_are_deduped() {
        let (merged, added) = merge_paths(
            "/home/u/.local/bin:/usr/bin:/home/u/.cargo/bin",
            "/usr/local/bin:/usr/bin:/bin",
        );
        assert_eq!(
            merged,
            "/home/u/.local/bin:/usr/bin:/home/u/.cargo/bin:/usr/local/bin:/bin"
        );
        assert_eq!(added, vec!["/home/u/.local/bin", "/home/u/.cargo/bin"]);
    }

    #[test]
    fn empty_and_identical_inputs_are_stable() {
        assert_eq!(
            merge_paths("", "/usr/bin"),
            ("/usr/bin".to_string(), vec![])
        );
        assert_eq!(
            merge_paths("/a::/b", ""),
            ("/a:/b".to_string(), vec!["/a".into(), "/b".into()])
        );
        let (m, added) = merge_paths("/usr/bin:/bin", "/usr/bin:/bin");
        assert_eq!(m, "/usr/bin:/bin");
        assert!(added.is_empty());
    }

    #[test]
    fn banner_noise_before_the_path_is_ignored() {
        assert_eq!(parse_shell_output("Welcome!\n/a:/b"), Some("/a:/b"));
        assert_eq!(parse_shell_output("\n\n"), None);
    }

    #[test]
    fn detects_only_the_serve_subcommand() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_daemon_invocation(&a(&["forge", "serve", "--tunnel"])));
        assert!(!is_daemon_invocation(&a(&["forge", "mcp-serve"])));
        assert!(!is_daemon_invocation(&a(&["forge", "run", "serve"])));
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_shell_yields_none() {
        assert_eq!(
            login_shell_path("/nonexistent/shell", Duration::from_secs(1)),
            None
        );
    }
}
