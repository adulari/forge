//! Daemon unit/binary drift checks, kept beside the other daemon probes so `doctor.rs` stays
//! reviewable.
//!
//! The version in "running X" must describe the DAEMON, never the binary printing the report:
//! `forge doctor` is the one command users paste into bug reports, and the unit's `ExecStart` very
//! often points at a different (older or newer) binary than the one being run from the shell.

use crate::doctor::{check, Check, Status};

/// Where a daemon version came from, so the report can be honest about its evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DaemonVersion {
    /// The running daemon reported it over its own API — the only source that describes the
    /// process actually serving right now.
    Reported(String),
    /// The binary `ExecStart` names, asked with `--version`. Works while the daemon is stopped and
    /// is what systemd would launch next.
    Binary(String),
    /// Neither source answered. Say so rather than substituting this binary's version.
    Unknown,
}

impl DaemonVersion {
    fn value(&self) -> Option<&str> {
        match self {
            DaemonVersion::Reported(v) | DaemonVersion::Binary(v) => Some(v),
            DaemonVersion::Unknown => None,
        }
    }

    fn describe(&self) -> String {
        match self {
            DaemonVersion::Reported(v) | DaemonVersion::Binary(v) => {
                format!("daemon binary {v}")
            }
            DaemonVersion::Unknown => "daemon binary unknown (daemon not reachable)".to_string(),
        }
    }
}

/// The absolute path `ExecStart` launches, if the unit names one.
fn unit_exec_start(unit: &str) -> &str {
    unit.lines()
        .find_map(|l| l.trim().strip_prefix("ExecStart="))
        .and_then(|cmd| cmd.split_whitespace().next())
        .unwrap_or_default()
}

/// Ask the running daemon over `/api/identity`, then fall back to running the unit's binary. The
/// live daemon is preferred because an upgraded-but-not-restarted daemon still serves the OLD
/// version, which no on-disk inspection can see.
pub(crate) async fn daemon_version(port: u16, unit_exe: &str) -> DaemonVersion {
    if let Some(v) = reported_version(port).await {
        return DaemonVersion::Reported(v);
    }
    if let Some(v) = binary_version(unit_exe) {
        return DaemonVersion::Binary(v);
    }
    DaemonVersion::Unknown
}

async fn reported_version(port: u16) -> Option<String> {
    let state = crate::serve::read_state()
        .ok()
        .flatten()
        .filter(|state| state.port == port && state.process_is_alive());
    let token = match state.as_ref().map(|state| state.token.clone()) {
        Some(token) => token,
        None => crate::serve::read_daemon_token().ok()?,
    };
    let tls = state.as_ref().is_some_and(|state| state.exposure == "lan");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .danger_accept_invalid_certs(tls)
        .build()
        .ok()?;
    let scheme = if tls { "https" } else { "http" };
    let url = format!("{scheme}://127.0.0.1:{port}/{token}/api/identity");
    let response = client.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.json::<serde_json::Value>().await.ok()?;
    body.get("version")?.as_str().map(str::to_string)
}

fn binary_version(unit_exe: &str) -> Option<String> {
    if unit_exe.is_empty() || !std::path::Path::new(unit_exe).exists() {
        return None;
    }
    let out = std::process::Command::new(unit_exe)
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_version_output(&String::from_utf8_lossy(&out.stdout))
}

/// `forge --version` prints `forge X.Y.Z`; take the last whitespace-separated token so a future
/// prefix change does not silently turn into a bogus version string.
fn parse_version_output(text: &str) -> Option<String> {
    let line = text.lines().next()?.trim();
    let token = line.split_whitespace().next_back()?;
    token
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
        .then(|| token.to_string())
}

/// Does the installed unit match the daemon binary it launches, and does it point at this binary?
///
/// `forge service install` writes the unit once; upgrading Forge never rewrites it. So a fix that
/// lives in the unit rather than the binary silently never reaches an existing install. That is not
/// hypothetical: #996 stops the daemon restart-looping on an unfixable failure via BOTH an exit code
/// (binary) and `RestartPreventExitStatus=78` (unit). Upgrade alone delivers half of it, and the
/// daemon keeps looping — 25,218 consecutive restarts on the author's machine, silently.
pub(crate) async fn unit_drift_checks(unit: &str, current_exe: &str, port: u16) -> Vec<Check> {
    let daemon = daemon_version(port, unit_exec_start(unit)).await;
    unit_drift_checks_at(unit, current_exe, &daemon, env!("CARGO_PKG_VERSION"))
}

/// Pure so every branch is testable without installing or breaking a real service.
pub(crate) fn unit_drift_checks_at(
    unit: &str,
    current_exe: &str,
    daemon: &DaemonVersion,
    this_version: &str,
) -> Vec<Check> {
    use crate::cli::commands::service::UNIT_VERSION_PREFIX;
    let mut out = Vec::new();

    // The general case: any future change to the rendered unit becomes visible, not just the
    // directives someone thought to grep for. A unit with no marker predates the stamp entirely.
    let unit_version = unit
        .lines()
        .find_map(|l| l.trim().strip_prefix(UNIT_VERSION_PREFIX))
        .map(str::trim);
    if let Some(v) = unit_version {
        // Re-rendering the unit stamps it with the version of the binary that renders it, so the
        // stamp's counterpart is the DAEMON binary — not whichever Forge happens to run doctor.
        let drifted = match daemon.value() {
            Some(daemon_version) => daemon_version != v,
            // Without daemon evidence there is no truthful equality comparison to make. Surface
            // that uncertainty instead of silently treating this Forge's version as the daemon's.
            None => true,
        };
        let detail = format!(
            "unit rendered by Forge {v} · {} · this forge {this_version}",
            daemon.describe()
        );
        if drifted {
            out.push(check(
                Status::Warn,
                "daemon unit",
                detail,
                Some("`forge service install` to re-render it (installing a new Forge does not)"),
            ));
        } else {
            out.push(check(Status::Ok, "daemon unit", detail, None));
        }
    }

    if !unit.contains("RestartPreventExitStatus") {
        out.push(check(
            Status::Warn,
            "daemon unit",
            "predates the permanent-failure guard — a failure retrying cannot fix will restart forever",
            Some("`forge service install` to re-render the unit (it is rewritten only on install)"),
        ));
    }

    // ExecStart pins an absolute path. Install Forge somewhere else afterwards (brew, cargo) and
    // systemd keeps launching the OLD binary, so `forge --version` in a terminal can disagree with
    // what the daemon is actually running, with nothing anywhere saying so.
    let unit_exe = unit_exec_start(unit);
    // A dev build is EXPECTED to differ from the installed unit — warning about it every time
    // someone runs `cargo run -- doctor` would be pure noise, and noise is how the real signal
    // above gets ignored.
    let from_build_tree =
        current_exe.contains("/target/debug/") || current_exe.contains("/target/release/");
    if !unit_exe.is_empty()
        && !current_exe.is_empty()
        && unit_exe != current_exe
        && !from_build_tree
    {
        out.push(check(
            Status::Warn,
            "daemon binary",
            format!("unit runs {unit_exe}, but this is {current_exe}"),
            Some("`forge service install` to point the unit at this binary"),
        ));
    }

    if out.is_empty() {
        out.push(check(
            Status::Ok,
            "daemon unit",
            "matches this binary",
            None,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{unit_drift_checks_at, DaemonVersion, Status};

    fn at(unit: &str, exe: &str) -> Vec<super::Check> {
        unit_drift_checks_at(
            unit,
            exe,
            &DaemonVersion::Unknown,
            env!("CARGO_PKG_VERSION"),
        )
    }

    /// The unit actually installed on the author's machine on 2026-08-10, verbatim. Its daemon had
    /// restarted 25,218 times; #996's exit-code half would have arrived with an upgrade and changed
    /// nothing, because this unit has no `RestartPreventExitStatus`.
    const REAL_STALE_UNIT: &str = "[Unit]\nDescription=Forge serve\n\n[Service]\n\
        ExecStart=/home/floris/.local/bin/forge serve --tunnel --port 7420\nRestart=on-failure\n";

    #[test]
    fn a_unit_without_the_permanent_failure_guard_is_flagged() {
        let out = at(REAL_STALE_UNIT, "/home/floris/.local/bin/forge");
        let guard = out
            .iter()
            .find(|c| c.label == "daemon unit")
            .expect("the missing guard must be reported");
        assert_eq!(guard.status, Status::Warn);
        assert!(guard.fix.is_some(), "must say how to fix it");
        // Same binary path, so only the guard should be reported — not a spurious binary mismatch.
        assert!(out.iter().all(|c| c.label != "daemon binary"));
    }

    #[test]
    fn a_unit_pointing_at_another_binary_is_flagged() {
        let unit = "[Service]\nExecStart=/usr/bin/forge serve --port 7420\n\
            RestartPreventExitStatus=78\nRestart=on-failure\n";
        let out = at(unit, "/home/floris/.cargo/bin/forge");
        let drift = out
            .iter()
            .find(|c| c.label == "daemon binary")
            .expect("a different ExecStart path must be reported");
        assert_eq!(drift.status, Status::Warn);
        assert!(drift.detail.contains("/usr/bin/forge"), "{}", drift.detail);
        assert!(drift.detail.contains("cargo"), "{}", drift.detail);
    }

    #[test]
    fn a_current_unit_reports_ok_and_nothing_else() {
        let unit = "[Service]\nExecStart=/usr/bin/forge serve --port 7420\n\
            RestartPreventExitStatus=78\nRestart=on-failure\n";
        let out = at(unit, "/usr/bin/forge");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, Status::Ok);
    }

    /// An unknown current-exe path must not manufacture a mismatch — reporting drift against an
    /// empty string would fire on every install that cannot resolve its own binary.
    #[test]
    fn an_unresolvable_current_binary_does_not_report_drift() {
        let unit = "[Service]\nExecStart=/usr/bin/forge serve\nRestartPreventExitStatus=78\n";
        let out = at(unit, "");
        assert!(out.iter().all(|c| c.label != "daemon binary"));
    }

    /// A unit rendered by an older Forge is reported even when it carries every directive this
    /// version happens to check for — the point of the stamp is catching drift nobody grepped for.
    #[test]
    fn a_unit_stamped_by_an_older_forge_is_reported() {
        let unit = "[Unit]\n# forge-unit-version: 2.9.0\n[Service]\n\
            ExecStart=/usr/bin/forge serve\nRestartPreventExitStatus=78\n";
        let out = unit_drift_checks_at(
            unit,
            "/usr/bin/forge",
            &DaemonVersion::Reported("2.13.0".into()),
            "2.13.0",
        );
        let stale = out
            .iter()
            .find(|c| c.label == "daemon unit")
            .expect("a version gap must be reported");
        assert_eq!(stale.status, Status::Warn);
        assert!(stale.detail.contains("2.9.0"), "{}", stale.detail);
        assert!(stale.detail.contains("2.13.0"), "{}", stale.detail);
    }

    #[test]
    fn a_unit_stamped_by_this_forge_is_not_reported() {
        let unit = "[Unit]\n# forge-unit-version: 2.13.0\n[Service]\n\
            ExecStart=/usr/bin/forge serve\nRestartPreventExitStatus=78\n";
        let out = unit_drift_checks_at(
            unit,
            "/usr/bin/forge",
            &DaemonVersion::Reported("2.13.0".into()),
            "2.13.0",
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, Status::Ok);
        assert!(
            out[0].detail.contains("daemon binary 2.13.0"),
            "{}",
            out[0].detail
        );
    }

    /// A dev build always differs from the installed unit. Warning every `cargo run -- doctor` is
    /// noise, and noise is how the genuine warning above gets ignored.
    #[test]
    fn a_build_tree_binary_does_not_report_drift() {
        let unit = "[Service]\nExecStart=/usr/bin/forge serve\nRestartPreventExitStatus=78\n";
        for exe in [
            "/home/me/src/forge/target/debug/forge",
            "/home/me/src/forge/target/release/forge",
        ] {
            let out = at(unit, exe);
            assert!(
                out.iter().all(|c| c.label != "daemon binary"),
                "{exe} must not warn"
            );
        }
    }

    /// The defect: the unit is stamped 2.12.2, ExecStart points at binary A running 2.13.5, and
    /// doctor is a DIFFERENT binary at 2.13.2. The old code printed "running 2.13.2" — doctor's own
    /// version, which describes nothing about the daemon.
    #[test]
    fn running_version_is_the_daemon_not_the_doctor_binary() {
        let unit = "[Unit]\n# forge-unit-version: 2.12.2\n[Service]\n\
            ExecStart=/home/floris/.cache/forge-target/release/forge serve --port 7420\n\
            RestartPreventExitStatus=78\n";
        let out = unit_drift_checks_at(
            unit,
            "/home/floris/.local/bin/forge",
            &DaemonVersion::Reported("2.13.5".into()),
            "2.13.2",
        );
        let stale = out
            .iter()
            .find(|c| c.label == "daemon unit")
            .expect("a version gap must be reported");
        assert!(
            stale.detail.contains("unit rendered by Forge 2.12.2"),
            "{}",
            stale.detail
        );
        assert!(
            stale.detail.contains("daemon binary 2.13.5"),
            "{}",
            stale.detail
        );
        assert!(
            stale.detail.contains("this forge 2.13.2"),
            "{}",
            stale.detail
        );
        assert!(
            !stale.detail.contains("running 2.13.2"),
            "doctor's own version must never be presented as the daemon's: {}",
            stale.detail
        );
    }

    /// No daemon and no runnable ExecStart binary: say unknown rather than substitute a version.
    #[test]
    fn an_unreachable_daemon_reports_unknown() {
        let unit = "[Unit]\n# forge-unit-version: 2.12.2\n[Service]\n\
            ExecStart=/usr/bin/forge serve\nRestartPreventExitStatus=78\n";
        let out = unit_drift_checks_at(unit, "/usr/bin/forge", &DaemonVersion::Unknown, "2.13.2");
        let stale = out
            .iter()
            .find(|c| c.label == "daemon unit")
            .expect("a version gap must still be reported");
        assert!(
            stale
                .detail
                .contains("daemon binary unknown (daemon not reachable)"),
            "{}",
            stale.detail
        );
    }

    /// No daemon and no runnable ExecStart binary: say unknown rather than substitute a version,
    /// even when the unit stamp happens to equal the doctor binary's version.
    #[test]
    fn an_unreachable_daemon_never_substitutes_the_doctor_version() {
        let unit = "[Unit]\n# forge-unit-version: 2.13.2\n[Service]\n\
            ExecStart=/usr/bin/forge serve\nRestartPreventExitStatus=78\n";
        let out = unit_drift_checks_at(unit, "/usr/bin/forge", &DaemonVersion::Unknown, "2.13.2");
        let stale = out
            .iter()
            .find(|c| c.label == "daemon unit")
            .expect("an unknown daemon version must be reported honestly");
        assert!(
            stale
                .detail
                .contains("daemon binary unknown (daemon not reachable)"),
            "{}",
            stale.detail
        );
    }

    #[test]
    fn version_output_is_parsed_and_garbage_is_rejected() {
        assert_eq!(
            super::parse_version_output("forge 2.13.5\n").as_deref(),
            Some("2.13.5")
        );
        assert_eq!(super::parse_version_output("error: no such file"), None);
        assert_eq!(super::parse_version_output(""), None);
    }
}
