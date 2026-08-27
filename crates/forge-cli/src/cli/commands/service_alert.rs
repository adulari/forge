//! A deadman signal for the daemon, deliberately local and credential-free.
//!
//! `RestartPreventExitStatus=78` already stops the unit on a permanent failure instead of looping
//! forever, but stopping is not the same as telling anyone. In the observed outages the unit sat in
//! `failed`/`activating` for days and was found only because a person went looking — which is not a
//! monitoring strategy.
//!
//! The alarm must not share fate with what it monitors, which rules out routing it through the
//! daemon or the Anywhere relay: both are silent exactly when it matters. This path is a systemd
//! `OnFailure=` handler, so systemd — not Forge — delivers it.
//!
//! Deliberately NOT an external dead-man endpoint (Healthchecks.io and friends). That covers a
//! failure this has not yet had — a host that is off or unreachable — at the cost of an external
//! dependency and a stored token. Every observed failure so far has been a live host with a dead
//! unit, which this catches. The external endpoint remains the right *second* step.

use anyhow::Result;

/// Companion unit named by `OnFailure=` in the main unit.
pub(crate) const ALERT_UNIT_NAME: &str = "forge-serve-alert.service";

pub(crate) fn render_systemd_alert_unit(forge_exe: &str) -> String {
    format!(
        "[Unit]\nDescription=Forge serve failed — local notification\n\
         # Started by OnFailure= in forge-serve.service. Kept as its own unit rather than an\n\
         # ExecStopPost= so it runs when the service FAILS, not on every ordinary stop.\n\n\
         [Service]\nType=oneshot\nExecStart={forge_exe} service alert\n"
    )
}

/// The notification text. Pure so the wording is testable without a systemd session.
pub(crate) fn alert_message(result: &str, exit_status: &str) -> String {
    // 78 is EX_CONFIG — the one failure retrying can never fix, and the one that produced the long
    // silent loops. Naming the remedy here means the notification is actionable on its own.
    let cause = if exit_status == "78" {
        "the store is newer than this build — install a newer Forge (`forge update`)"
    } else if result == "oom-kill" {
        "the kernel OOM-killed it"
    } else {
        "run `forge doctor` for a health check"
    };
    format!("Forge daemon stopped (result={result}, exit={exit_status}). Anywhere is offline — {cause}.")
}

pub(crate) fn alert_cmd() -> Result<()> {
    let (result, exit_status) = failure_detail();
    let message = alert_message(&result, &exit_status);

    // stderr first and unconditionally: under systemd this lands in the journal, so the signal
    // survives even where no desktop session exists to receive a notification.
    eprintln!("{message}");
    notify_desktop(&message);
    Ok(())
}

fn failure_detail() -> (String, String) {
    let output = std::process::Command::new("systemctl")
        .args([
            "--user",
            "show",
            super::service::SYSTEMD_UNIT_NAME,
            "-p",
            "Result",
            "-p",
            "ExecMainStatus",
        ])
        .output();
    let text = match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Err(_) => String::new(),
    };
    (
        property(&text, "Result").unwrap_or_else(|| "unknown".to_string()),
        property(&text, "ExecMainStatus").unwrap_or_else(|| "unknown".to_string()),
    )
}

fn property(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Best effort. A headless box has no notification daemon, and that must not turn the alert into
/// its own failure — the journal line above is the guaranteed channel.
fn notify_desktop(message: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["-u", "critical", "Forge daemon stopped", message])
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alert_unit_runs_forge_and_is_oneshot() {
        let unit = render_systemd_alert_unit("/usr/bin/forge");
        assert!(unit.contains("ExecStart=/usr/bin/forge service alert"));
        assert!(unit.contains("Type=oneshot"));
    }

    /// The failure that caused every long silent loop must name its remedy, or the notification is
    /// just a louder version of "something broke".
    #[test]
    fn exit_78_names_the_remedy() {
        let message = alert_message("exit-code", "78");
        assert!(message.contains("forge update"), "got: {message}");
        assert!(message.contains("exit=78"), "got: {message}");
    }

    #[test]
    fn other_failures_fall_back_to_doctor() {
        assert!(alert_message("exit-code", "1").contains("forge doctor"));
        assert!(alert_message("oom-kill", "137").contains("OOM-killed"));
    }

    #[test]
    fn properties_parse_from_systemctl_show_output() {
        let text = "Result=exit-code\nExecMainStatus=78\n";
        assert_eq!(property(text, "Result").as_deref(), Some("exit-code"));
        assert_eq!(property(text, "ExecMainStatus").as_deref(), Some("78"));
        assert_eq!(property(text, "Missing"), None);
    }

    /// An empty value is what `systemctl show` prints for a unit that does not exist; treating it
    /// as present would put `result=` with nothing after it into the notification.
    #[test]
    fn an_empty_property_is_not_a_value() {
        assert_eq!(property("Result=\n", "Result"), None);
        assert!(alert_message("unknown", "unknown").contains("forge doctor"));
    }
}
