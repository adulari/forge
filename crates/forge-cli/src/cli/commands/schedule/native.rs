//! Native OS timer installation, removal, and pure renderers.

use anyhow::{Context, Result};

use forge_store::SchedulePolicy;

use super::{
    cron::{cron_to_launchd_intervals, cron_to_on_calendar, cron_to_schtasks_trigger},
    parse_posix_cron, ScheduleSpec,
};

fn systemd_user_dir() -> Result<std::path::PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Ok(std::path::PathBuf::from(xdg).join("systemd/user"));
    }
    let home =
        forge_config::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    Ok(home.join(".config/systemd/user"))
}

fn launchd_agents_dir() -> Result<std::path::PathBuf> {
    let home =
        forge_config::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    Ok(home.join("Library/LaunchAgents"))
}

pub(crate) fn install_timer(
    id: &str,
    spec: &ScheduleSpec,
    cwd: &str,
    forge_exe: &str,
    policy: &SchedulePolicy,
) -> Result<()> {
    if cfg!(target_os = "linux") {
        install_systemd(id, spec, cwd, forge_exe, policy)
    } else if cfg!(target_os = "macos") {
        install_launchd(id, spec, cwd, forge_exe, policy)
    } else if cfg!(target_os = "windows") {
        install_schtasks(id, spec, cwd, forge_exe, policy)
    } else {
        anyhow::bail!("forge schedule has no OS-timer backend for this platform")
    }
}

pub(crate) fn uninstall_timer(id: &str) -> Result<()> {
    if cfg!(target_os = "linux") {
        uninstall_systemd(id)
    } else if cfg!(target_os = "macos") {
        uninstall_launchd(id)
    } else if cfg!(target_os = "windows") {
        uninstall_schtasks(id)
    } else {
        Ok(())
    }
}

/// Run `cmd args…` directly (no shell), surfacing a non-zero exit or spawn failure with stderr
/// attached — the one place that actually touches the host's scheduler.
fn run_checked(cmd: &str, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("spawning `{cmd}`"))?;
    if !output.status.success() {
        anyhow::bail!(
            "`{cmd} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

// --- systemd (Linux) ---

/// The timer launches `forge schedule tick <id>`, which reads the task, mode and model from the
/// store at fire time - so an edit never needs the unit rewritten, and the tick can record history.
pub(super) fn render_systemd_service(id: &str, cwd: &str, forge_exe: &str) -> String {
    let exec = format!(
        "{} schedule tick {}",
        quote_unit_arg(forge_exe),
        quote_unit_arg(id)
    );
    format!(
        "[Unit]\nDescription=Forge scheduled task {id}\n\n\
         [Service]\nType=oneshot\nWorkingDirectory={}\nExecStart={exec}\n",
        // WorkingDirectory= is a bare path, not a command line: systemd does not unquote it, so a
        // quoted value is "not absolute" and the whole unit is refused. Only `%` specifiers apply.
        cwd.replace('%', "%%")
    )
}

pub(super) fn render_systemd_timer(
    id: &str,
    spec: &ScheduleSpec,
    policy: &SchedulePolicy,
) -> Result<String> {
    // systemd reads a trailing zone name on a calendar expression, which is what lets "3:00 in
    // Tokyo" fire at the right instant whatever the machine's own zone is.
    let zone = match policy
        .tz
        .as_deref()
        .map(str::trim)
        .filter(|tz| !tz.is_empty())
    {
        Some(tz) => format!(" {tz}"),
        None => String::new(),
    };
    let body = match spec {
        ScheduleSpec::Every(d) => {
            let secs = d.as_secs();
            format!("OnActiveSec={secs}s\nOnUnitActiveSec={secs}s")
        }
        ScheduleSpec::Daily { hour, minute } => {
            format!("OnCalendar=*-*-* {hour:02}:{minute:02}:00{zone}")
        }
        // A POSIX expression becomes one OnCalendar= line per systemd expression it needs (two for
        // the DOM/DOW OR case, which systemd unions); anything else is the historical verbatim
        // OnCalendar pass-through, so pre-existing rows render exactly as they always did.
        ScheduleSpec::Cron(expr) => match parse_posix_cron(expr) {
            Some(fields) => cron_to_on_calendar(&fields)
                .into_iter()
                .map(|line| format!("OnCalendar={line}{zone}"))
                .collect::<Vec<_>>()
                .join("\n"),
            None => format!("OnCalendar={expr}{zone}"),
        },
    };
    // `Persistent=` is the misfire policy: true runs one tick that was missed while the machine
    // was off, false drops it.
    let persistent = policy.misfire != "skip";
    Ok(format!(
        "[Unit]\nDescription=Forge schedule timer {id}\n\n\
         [Timer]\n{body}\nPersistent={persistent}\n\n\
         [Install]\nWantedBy=timers.target\n"
    ))
}

fn quote_unit_arg(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('%', "%%")
            .replace('$', "$$")
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\r', " ")
            .replace('\n', "\\n")
    )
}

#[allow(clippy::too_many_arguments)]
fn install_systemd(
    id: &str,
    spec: &ScheduleSpec,
    cwd: &str,
    forge_exe: &str,
    policy: &SchedulePolicy,
) -> Result<()> {
    let dir = systemd_user_dir()?;
    std::fs::create_dir_all(&dir).context("creating ~/.config/systemd/user")?;
    let service = render_systemd_service(id, cwd, forge_exe);
    let timer = render_systemd_timer(id, spec, policy)?;
    let service_path = dir.join(format!("forge-{id}.service"));
    let timer_path = dir.join(format!("forge-{id}.timer"));
    std::fs::write(&service_path, service).context("writing systemd service unit")?;
    if let Err(error) = std::fs::write(&timer_path, timer).context("writing systemd timer unit") {
        let _ = std::fs::remove_file(&service_path);
        return Err(error);
    }
    if let Err(error) = run_checked("systemctl", &["--user", "daemon-reload"]) {
        let _ = std::fs::remove_file(&service_path);
        let _ = std::fs::remove_file(&timer_path);
        return Err(error);
    }
    if let Err(e) = run_checked(
        "systemctl",
        &["--user", "enable", "--now", &format!("forge-{id}.timer")],
    ) {
        // Leave nothing behind: the caller rolls the DB row back, so the units (and the
        // `timers.target.wants` symlink `enable` writes before `--now` fails) would be orphans.
        let _ = uninstall_systemd(id);
        // The only spec that can render a unit systemd refuses is the OnCalendar pass-through —
        // everything else is generated from input we validated. systemd's own message ("bad unit
        // file setting") does not say which setting, so name it here.
        return Err(match spec {
            ScheduleSpec::Cron(expr) if parse_posix_cron(expr).is_none() => e.context(format!(
                "`{expr}` is not a 5-field cron expression, so it was written to the timer as a \
                 systemd OnCalendar= expression and systemd rejected it — check it with \
                 `systemd-analyze calendar '{expr}'`, or use standard cron (e.g. `0 6 * * 1`)"
            )),
            _ => e,
        });
    }
    Ok(())
}

fn uninstall_systemd(id: &str) -> Result<()> {
    let dir = systemd_user_dir()?;
    let _ = run_checked(
        "systemctl",
        &["--user", "disable", "--now", &format!("forge-{id}.timer")],
    );
    let _ = std::fs::remove_file(dir.join(format!("forge-{id}.service")));
    let _ = std::fs::remove_file(dir.join(format!("forge-{id}.timer")));
    let _ = run_checked("systemctl", &["--user", "daemon-reload"]);
    Ok(())
}

/// launchd and Task Scheduler only know the machine's own zone, so a schedule pinned to another
/// one cannot be expressed there. Saying so beats firing at the wrong hour.
fn reject_foreign_zone(os: &str, policy: &SchedulePolicy) -> Result<()> {
    match policy
        .tz
        .as_deref()
        .map(str::trim)
        .filter(|tz| !tz.is_empty())
    {
        Some(tz) => anyhow::bail!(
            "{os} timers always use this machine's time zone, so a schedule in {tz} can't be \
             installed here - clear the time zone, or run the schedule on a Linux daemon"
        ),
        None => Ok(()),
    }
}

// --- launchd (macOS) ---

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(super) fn render_launchd_plist(
    id: &str,
    cwd: &str,
    forge_exe: &str,
    spec: &ScheduleSpec,
    policy: &SchedulePolicy,
) -> Result<String> {
    reject_foreign_zone("macOS", policy)?;
    let args = [forge_exe, "schedule", "tick", id].map(str::to_string);
    let mut args_xml = String::new();
    for a in &args {
        args_xml.push_str(&format!("        <string>{}</string>\n", xml_escape(a)));
    }

    let schedule_xml = match spec {
        ScheduleSpec::Every(d) => format!(
            "    <key>StartInterval</key>\n    <integer>{}</integer>\n",
            d.as_secs()
        ),
        ScheduleSpec::Daily { hour, minute } => format!(
            "    <key>StartCalendarInterval</key>\n    <dict>\n        \
             <key>Hour</key>\n        <integer>{hour}</integer>\n        \
             <key>Minute</key>\n        <integer>{minute}</integer>\n    </dict>\n"
        ),
        // launchd takes an *array* of calendar dicts and fires when any of them matches, so both a
        // translated cron expression and the DOM/DOW OR case fit. An OnCalendar string does not:
        // launchd has no equivalent grammar, so it is rejected exactly as before.
        ScheduleSpec::Cron(expr) => {
            let parsed = parse_posix_cron(expr).ok_or_else(|| {
                anyhow::anyhow!(
                    "`{expr}` isn't a 5-field cron expression, and macOS has no equivalent of \
                     systemd's OnCalendar syntax — use standard cron (e.g. `0 6 * * 1`), --every, \
                     or --at"
                )
            })?;
            let mut xml = String::from("    <key>StartCalendarInterval</key>\n    <array>\n");
            for entry in cron_to_launchd_intervals(expr, &parsed)? {
                xml.push_str("        <dict>\n");
                for (key, value) in entry {
                    xml.push_str(&format!(
                        "            <key>{key}</key>\n            <integer>{value}</integer>\n"
                    ));
                }
                xml.push_str("        </dict>\n");
            }
            xml.push_str("    </array>\n");
            xml
        }
    };

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
    );
    out.push_str("<plist version=\"1.0\">\n<dict>\n");
    out.push_str(&format!(
        "    <key>Label</key>\n    <string>dev.forge.schedule.{id}</string>\n"
    ));
    out.push_str("    <key>ProgramArguments</key>\n    <array>\n");
    out.push_str(&args_xml);
    out.push_str("    </array>\n");
    out.push_str(&format!(
        "    <key>WorkingDirectory</key>\n    <string>{}</string>\n",
        xml_escape(cwd)
    ));
    out.push_str(&schedule_xml);
    out.push_str("</dict>\n</plist>\n");
    Ok(out)
}

fn install_launchd(
    id: &str,
    spec: &ScheduleSpec,
    cwd: &str,
    forge_exe: &str,
    policy: &SchedulePolicy,
) -> Result<()> {
    let dir = launchd_agents_dir()?;
    std::fs::create_dir_all(&dir).context("creating ~/Library/LaunchAgents")?;
    let plist = render_launchd_plist(id, cwd, forge_exe, spec, policy)?;
    let path = dir.join(format!("dev.forge.schedule.{id}.plist"));
    std::fs::write(&path, plist).context("writing launchd plist")?;
    if let Err(error) = run_checked("launchctl", &["load", &path.to_string_lossy()]) {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(())
}

fn uninstall_launchd(id: &str) -> Result<()> {
    let dir = launchd_agents_dir()?;
    let path = dir.join(format!("dev.forge.schedule.{id}.plist"));
    let _ = run_checked("launchctl", &["unload", &path.to_string_lossy()]);
    let _ = std::fs::remove_file(&path);
    Ok(())
}

// --- Task Scheduler (Windows) ---

/// `Command::Run` has no `--cwd` flag, so the task's `/TR` command line itself `cd`s into the
/// working directory before invoking forge (wrapped in `cmd /C` since `cd` is a shell builtin,
/// not something `schtasks` can exec directly).
pub(super) fn render_schtasks_create_args(
    id: &str,
    cwd: &str,
    forge_exe: &str,
    spec: &ScheduleSpec,
    policy: &SchedulePolicy,
) -> Result<Vec<String>> {
    reject_foreign_zone("Windows", policy)?;
    let quoted_cwd = cwd.replace('"', "\"\"");
    let quoted_exe = forge_exe.replace('"', "\"\"");
    let inner = format!(r#"cd /d ""{quoted_cwd}"" && ""{quoted_exe}"" schedule tick {id}"#);

    let mut args = vec![
        "/Create".to_string(),
        "/TN".to_string(),
        format!("forge-{id}"),
        "/TR".to_string(),
        format!("cmd /S /C \"\"{inner}\"\""),
        "/F".to_string(),
    ];
    match spec {
        ScheduleSpec::Every(d) => {
            let seconds = d.as_secs();
            if seconds % 60 != 0 || !(60..=86_340).contains(&seconds) {
                anyhow::bail!(
                    "Windows Task Scheduler supports --every only from 1 to 1439 whole minutes; got {seconds}s"
                );
            }
            let minutes = seconds / 60;
            args.push("/SC".to_string());
            args.push("MINUTE".to_string());
            args.push("/MO".to_string());
            args.push(minutes.to_string());
        }
        ScheduleSpec::Daily { hour, minute } => {
            args.push("/SC".to_string());
            args.push("DAILY".to_string());
            args.push("/ST".to_string());
            args.push(format!("{hour:02}:{minute:02}"));
        }
        // Task Scheduler expresses much less than cron, so the translator rejects (with the reason)
        // rather than approximating; an OnCalendar string has no Windows meaning at all.
        ScheduleSpec::Cron(expr) => {
            let parsed = parse_posix_cron(expr).ok_or_else(|| {
                anyhow::anyhow!(
                    "`{expr}` isn't a 5-field cron expression, and Windows has no equivalent of \
                     systemd's OnCalendar syntax — use standard cron (e.g. `0 6 * * 1`), --every, \
                     or --at"
                )
            })?;
            args.extend(cron_to_schtasks_trigger(expr, &parsed)?);
        }
    }
    Ok(args)
}

fn install_schtasks(
    id: &str,
    spec: &ScheduleSpec,
    cwd: &str,
    forge_exe: &str,
    policy: &SchedulePolicy,
) -> Result<()> {
    let args = render_schtasks_create_args(id, cwd, forge_exe, spec, policy)?;
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_checked("schtasks", &arg_refs)
}

fn uninstall_schtasks(id: &str) -> Result<()> {
    let _ = run_checked(
        "schtasks",
        &["/Delete", "/TN", &format!("forge-{id}"), "/F"],
    );
    Ok(())
}

#[cfg(test)]
mod quote_tests {
    use super::quote_unit_arg;

    #[test]
    fn quote_unit_arg_escapes_dollar_and_newlines() {
        let q = quote_unit_arg("echo $HOME\nsecond \"x\"");
        assert_eq!(q, "\"echo $$HOME\\nsecond \\\"x\\\"\"");
        assert!(!q.contains('\n'));
    }
}
