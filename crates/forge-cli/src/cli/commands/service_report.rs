//! What the daemon is ACTUALLY running, and how `forge service` reports it.
//!
//! `install` used to print the unit's `ExecStart` (`runs: …`) as if it described the live
//! process. It does not: the unit is what will run next time, and `enable --now` never restarts a
//! unit that is already up. This module answers the other question — which binary the running
//! process is executing right now — so install and status can both tell the truth.

use super::service::{run_capture, SYSTEMD_UNIT_NAME};

/// What `install` did to the service manager, and whether it worked.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Activation {
    Started,
    Restarted,
    Failed(String),
}

/// The binary a live daemon process is actually executing — not the one the unit names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunningDaemon {
    pub(crate) exe: String,
    pub(crate) version: Option<String>,
}

impl std::fmt::Display for RunningDaemon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.version {
            Some(v) => write!(f, "{} ({v})", self.exe),
            None => write!(f, "{} (version unknown)", self.exe),
        }
    }
}

/// `forge 2.13.5\n` → `2.13.5`.
fn parse_version_output(stdout: &str) -> Option<String> {
    let last = stdout.lines().next()?.split_whitespace().next_back()?;
    (!last.is_empty()).then(|| last.to_string())
}

fn binary_version(exe: &str) -> Option<String> {
    let (ok, stdout, _) = run_capture(exe, &["--version"]).ok()?;
    ok.then(|| parse_version_output(&stdout)).flatten()
}

fn linux_daemon() -> Option<RunningDaemon> {
    let (_, stdout, _) = run_capture(
        "systemctl",
        &[
            "--user",
            "show",
            "-p",
            "MainPID",
            "--value",
            SYSTEMD_UNIT_NAME,
        ],
    )
    .ok()?;
    let pid: u32 = stdout.trim().parse().ok()?;
    if pid == 0 {
        return None;
    }
    let proc_exe = format!("/proc/{pid}/exe");
    let exe = std::fs::read_link(&proc_exe)
        .ok()?
        .to_string_lossy()
        .trim_end_matches(" (deleted)")
        .to_string();
    let version = binary_version(&proc_exe);
    Some(RunningDaemon { exe, version })
}

fn process_daemon() -> Option<RunningDaemon> {
    let system = sysinfo::System::new_all();
    let process = system.processes().values().find(|process| {
        process
            .cmd()
            .windows(2)
            .any(|args| args[0] == "serve" && args[1].to_string_lossy().starts_with("--"))
    })?;
    let exe = process.exe()?.to_string_lossy().to_string();
    let version = binary_version(&exe);
    Some(RunningDaemon { exe, version })
}

/// The daemon's live process image. On Linux, the service manager's main PID and `/proc/<pid>/exe`
/// give an answer that cannot be fooled by a re-rendered unit or a replaced binary. Other platforms
/// identify the supervised `forge serve` process from the process table.
pub(crate) fn running_daemon() -> Option<RunningDaemon> {
    if cfg!(target_os = "linux") {
        linux_daemon()
    } else {
        process_daemon()
    }
}

pub(crate) fn wait_for_running_daemon() -> Option<RunningDaemon> {
    for _ in 0..20 {
        if let Some(daemon) = running_daemon() {
            return Some(daemon);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

pub(crate) fn activation_report(
    activation: Activation,
    before: Option<&RunningDaemon>,
    after: Option<&RunningDaemon>,
) -> String {
    match activation {
        Activation::Failed(err) => format!("! daemon NOT running the new unit: {err}"),
        Activation::Started => match after {
            Some(d) => format!("started: daemon now {d}"),
            None => "started: running process could not be identified".to_string(),
        },
        Activation::Restarted => {
            let was = before
                .and_then(|d| d.version.clone())
                .map(|v| format!(" (was {v})"))
                .unwrap_or_default();
            match after {
                Some(d) => format!("restarted: daemon now {d}{was}"),
                None => format!("restarted: running process could not be identified{was}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Install's output must describe the PROCESS, not the unit it just wrote — reporting
    /// `runs: <ExecStart>` is what made the stale daemon invisible.
    #[test]
    fn activation_report_names_the_running_binary_and_the_version_it_replaced() {
        let before = RunningDaemon {
            exe: "/home/me/.cache/forge-target/release/forge".into(),
            version: Some("2.13.2".into()),
        };
        let after = RunningDaemon {
            exe: "/home/me/.local/bin/forge".into(),
            version: Some("2.13.5".into()),
        };
        assert_eq!(
            activation_report(Activation::Restarted, Some(&before), Some(&after)),
            "restarted: daemon now /home/me/.local/bin/forge (2.13.5) (was 2.13.2)"
        );
        assert_eq!(
            activation_report(Activation::Started, None, Some(&after)),
            "started: daemon now /home/me/.local/bin/forge (2.13.5)"
        );
        assert!(
            activation_report(Activation::Failed("boom".into()), None, None)
                .starts_with("! daemon NOT running")
        );
    }

    #[test]
    fn version_output_parses_the_trailing_semver() {
        assert_eq!(
            parse_version_output("forge 2.13.5\n").as_deref(),
            Some("2.13.5")
        );
        assert_eq!(parse_version_output("").as_deref(), None);
    }

    #[test]
    fn a_daemon_without_a_readable_version_is_reported_as_unknown() {
        let d = RunningDaemon {
            exe: "/opt/forge".into(),
            version: None,
        };
        assert_eq!(d.to_string(), "/opt/forge (version unknown)");
    }
}
