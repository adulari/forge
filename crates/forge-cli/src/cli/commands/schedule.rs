//! `forge schedule` — register/list/remove recurring tasks that fire `forge run` via a native OS
//! timer (systemd `--user` on Linux, launchd on macOS, Task Scheduler on Windows). Headless analog
//! to `/loop`/`/goal`: there is no long-lived Forge process — the OS scheduler drives each tick by
//! re-invoking the `forge` binary, exactly like a cron job.

use anyhow::{Context, Result};

use crate::*;

pub(crate) fn schedule_cmd(cmd: Option<ScheduleCmd>) -> Result<()> {
    match cmd {
        None | Some(ScheduleCmd::List) => list_schedules_cmd(),
        Some(ScheduleCmd::Add {
            task,
            every,
            at,
            cron,
            mode,
            model,
        }) => add_schedule_cmd(task.join(" "), every, at, cron, mode, model),
        Some(ScheduleCmd::Remove { id }) => remove_schedule_cmd(&id),
    }
}

// ---------------------------------------------------------------------------
// Schedule spec — a small enum instead of a full cron parser. `Every`/`Daily` cover the common
// cases and render per-OS below; `Cron` is an escape hatch for systemd's own `OnCalendar=` syntax
// (not portable to launchd/schtasks, which reject it).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScheduleSpec {
    Every(std::time::Duration),
    Daily { hour: u32, minute: u32 },
    Cron(String),
}

impl ScheduleSpec {
    /// Serialize to the `schedule.cron` column's TEXT format.
    pub(crate) fn to_stored(&self) -> String {
        match self {
            ScheduleSpec::Every(d) => format!("every:{}", d.as_secs()),
            ScheduleSpec::Daily { hour, minute } => format!("daily:{hour:02}:{minute:02}"),
            ScheduleSpec::Cron(expr) => format!("cron:{expr}"),
        }
    }

    /// Parse the `schedule.cron` column back into a spec (round-trips with [`Self::to_stored`]).
    pub(crate) fn from_stored(s: &str) -> Option<Self> {
        if let Some(rest) = s.strip_prefix("every:") {
            return rest
                .parse::<u64>()
                .ok()
                .map(|secs| ScheduleSpec::Every(std::time::Duration::from_secs(secs)));
        }
        if let Some(rest) = s.strip_prefix("daily:") {
            let (h, m) = rest.split_once(':')?;
            return Some(ScheduleSpec::Daily {
                hour: h.parse().ok()?,
                minute: m.parse().ok()?,
            });
        }
        s.strip_prefix("cron:")
            .map(|expr| ScheduleSpec::Cron(expr.to_string()))
    }

    /// Human summary for `add`'s confirmation line and `list`.
    fn describe(&self) -> String {
        match self {
            ScheduleSpec::Every(d) => format!("every {}", fmt_duration_human(*d)),
            ScheduleSpec::Daily { hour, minute } => format!("daily at {hour:02}:{minute:02}"),
            ScheduleSpec::Cron(expr) => format!("cron `{expr}`"),
        }
    }
}

/// Parse `--every` shorthand: `<N><unit>` with unit s/m/h/d (e.g. `30m`, `1h`, `1d`).
fn parse_every(spec: &str) -> Result<std::time::Duration> {
    let trimmed = spec.trim();
    let bad =
        || anyhow::anyhow!("invalid --every value '{spec}' — expected e.g. `30m`, `1h`, `1d`");
    if trimmed.len() < 2 {
        return Err(bad());
    }
    let (num, unit) = trimmed.split_at(trimmed.len() - 1);
    let n: u64 = num.parse().map_err(|_| bad())?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => return Err(bad()),
    };
    if secs == 0 {
        anyhow::bail!("--every must be greater than zero");
    }
    Ok(std::time::Duration::from_secs(secs))
}

/// Parse `--at "HH:MM"` into a 24h hour/minute pair.
fn parse_at(spec: &str) -> Result<(u32, u32)> {
    let (h, m) = spec
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("--at must be HH:MM, got '{spec}'"))?;
    let hour: u32 = h
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid hour in --at '{spec}'"))?;
    let minute: u32 = m
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid minute in --at '{spec}'"))?;
    if hour > 23 || minute > 59 {
        anyhow::bail!("--at '{spec}' is out of range (00:00–23:59)");
    }
    Ok((hour, minute))
}

fn fmt_duration_human(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs > 0 && secs % 86_400 == 0 {
        format!("{}d", secs / 86_400)
    } else if secs > 0 && secs % 3600 == 0 {
        format!("{}h", secs / 3600)
    } else if secs > 0 && secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

// ---------------------------------------------------------------------------
// forge schedule add / list / remove
// ---------------------------------------------------------------------------

fn add_schedule_cmd(
    task: String,
    every: Option<String>,
    at: Option<String>,
    cron: Option<String>,
    mode: Option<String>,
    model: Option<String>,
) -> Result<()> {
    if task.trim().is_empty() {
        anyhow::bail!("empty task — usage: forge schedule add \"<task>\" --every 30m");
    }
    let spec = match (every, at, cron) {
        (Some(e), None, None) => ScheduleSpec::Every(parse_every(&e)?),
        (None, Some(a), None) => {
            let (hour, minute) = parse_at(&a)?;
            ScheduleSpec::Daily { hour, minute }
        }
        (None, None, Some(c)) => ScheduleSpec::Cron(c),
        (None, None, None) => {
            anyhow::bail!("pass one of --every / --at / --cron, e.g. `--every 30m`")
        }
        _ => unreachable!("clap's conflicts_with_all rules out combining --every/--at/--cron"),
    };

    let cwd = std::env::current_dir().context("resolving current directory")?;
    let cwd = cwd.to_string_lossy().to_string();
    let forge_exe = std::env::current_exe()
        .context("resolving the forge binary path")?
        .to_string_lossy()
        .to_string();

    let store = open_store()?;
    let id = forge_types::new_id();
    store
        .add_schedule(
            &id,
            &task,
            &cwd,
            mode.as_deref(),
            model.as_deref(),
            &spec.to_stored(),
        )
        .context("persisting schedule")?;

    // Install the OS timer only after the row lands — on failure, roll the row back so a failed
    // `add` never leaves an orphaned schedule with no matching timer.
    if let Err(e) = install_timer(
        &id,
        &spec,
        &task,
        &cwd,
        mode.as_deref(),
        model.as_deref(),
        &forge_exe,
    ) {
        let _ = store.remove_schedule(&id);
        return Err(e).context("installing the OS timer");
    }

    println!(
        "✓ scheduled ({}) in {cwd}\n  task: {task}\n  id: {}",
        spec.describe(),
        &id[..id.len().min(8)]
    );
    Ok(())
}

fn list_schedules_cmd() -> Result<()> {
    let store = open_store()?;
    let rows = store.list_schedules().context("listing schedules")?;
    if rows.is_empty() {
        println!("no schedules registered — `forge schedule add \"<task>\" --every 30m`");
        return Ok(());
    }
    println!(
        "{:<10} {:<7} {:<16} {:<9} {:<30} TASK",
        "ID", "ENABLED", "SCHEDULE", "LAST RUN", "CWD"
    );
    for s in &rows {
        let id: String = s.id.chars().take(8).collect();
        let spec = ScheduleSpec::from_stored(&s.cron)
            .map(|sp| sp.describe())
            .unwrap_or_else(|| s.cron.clone());
        let last_run = s
            .last_run
            .map(fmt_age)
            .unwrap_or_else(|| "never".to_string());
        println!(
            "{:<10} {:<7} {:<16} {:<9} {:<30} {}",
            id,
            if s.enabled { "yes" } else { "no" },
            spec,
            last_run,
            s.cwd,
            s.task
        );
    }
    Ok(())
}

fn remove_schedule_cmd(id_prefix: &str) -> Result<()> {
    let store = open_store()?;
    let id = resolve_schedule_id(&store, id_prefix)?;
    uninstall_timer(&id)?;
    store.remove_schedule(&id).context("deleting schedule")?;
    println!("✓ removed schedule {}", &id[..id.len().min(8)]);
    Ok(())
}

fn resolve_schedule_id(store: &Store, prefix: &str) -> Result<String> {
    let mut matches = store
        .matching_schedule_ids(prefix)
        .context("looking up schedule")?;
    match matches.len() {
        0 => anyhow::bail!("no schedule matching '{prefix}' — see `forge schedule list`"),
        1 => Ok(matches.remove(0)),
        n => anyhow::bail!("'{prefix}' is ambiguous ({n} schedules match) — use more characters"),
    }
}

// ---------------------------------------------------------------------------
// OS timer install/uninstall. The unit/plist/schtasks STRING renderers below this point are pure
// and unit tested; only `install_timer`/`uninstall_timer` (and the per-OS `install_*`/`uninstall_*`
// they dispatch to) shell out to the real scheduler, so `cargo test` never touches this machine's
// systemd/launchd/registry. `cfg!(target_os = ..)` runtime branches (not `#[cfg]` compile gates) so
// every branch still typechecks on this (Linux) build host.
// ---------------------------------------------------------------------------

fn systemd_user_dir() -> Result<std::path::PathBuf> {
    let home =
        forge_config::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    Ok(home.join(".config/systemd/user"))
}

fn launchd_agents_dir() -> Result<std::path::PathBuf> {
    let home =
        forge_config::home_dir().ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
    Ok(home.join("Library/LaunchAgents"))
}

fn install_timer(
    id: &str,
    spec: &ScheduleSpec,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
) -> Result<()> {
    if cfg!(target_os = "linux") {
        install_systemd(id, spec, task, cwd, mode, model, forge_exe)
    } else if cfg!(target_os = "macos") {
        install_launchd(id, spec, task, cwd, mode, model, forge_exe)
    } else if cfg!(target_os = "windows") {
        install_schtasks(id, spec, task, cwd, mode, model, forge_exe)
    } else {
        anyhow::bail!("forge schedule has no OS-timer backend for this platform")
    }
}

fn uninstall_timer(id: &str) -> Result<()> {
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

fn render_systemd_service(
    id: &str,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
) -> String {
    let mut exec = format!("{forge_exe} run {}", quote_unit_arg(task));
    if let Some(m) = mode {
        exec.push_str(&format!(" --mode {m}"));
    }
    if let Some(m) = model {
        exec.push_str(&format!(" --model {}", quote_unit_arg(m)));
    }
    format!(
        "[Unit]\nDescription=Forge scheduled task {id}\n\n\
         [Service]\nType=oneshot\nWorkingDirectory={cwd}\nExecStart={exec}\n"
    )
}

fn render_systemd_timer(id: &str, spec: &ScheduleSpec) -> Result<String> {
    let body = match spec {
        ScheduleSpec::Every(d) => {
            let secs = d.as_secs();
            format!("OnActiveSec={secs}s\nOnUnitActiveSec={secs}s")
        }
        ScheduleSpec::Daily { hour, minute } => {
            format!("OnCalendar=*-*-* {hour:02}:{minute:02}:00")
        }
        ScheduleSpec::Cron(expr) => format!("OnCalendar={expr}"),
    };
    Ok(format!(
        "[Unit]\nDescription=Forge schedule timer {id}\n\n\
         [Timer]\n{body}\nPersistent=true\n\n\
         [Install]\nWantedBy=timers.target\n"
    ))
}

fn quote_unit_arg(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[allow(clippy::too_many_arguments)]
fn install_systemd(
    id: &str,
    spec: &ScheduleSpec,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
) -> Result<()> {
    let dir = systemd_user_dir()?;
    std::fs::create_dir_all(&dir).context("creating ~/.config/systemd/user")?;
    let service = render_systemd_service(id, task, cwd, mode, model, forge_exe);
    let timer = render_systemd_timer(id, spec)?;
    std::fs::write(dir.join(format!("forge-{id}.service")), service)
        .context("writing systemd service unit")?;
    std::fs::write(dir.join(format!("forge-{id}.timer")), timer)
        .context("writing systemd timer unit")?;

    run_checked("systemctl", &["--user", "daemon-reload"])?;
    run_checked(
        "systemctl",
        &["--user", "enable", "--now", &format!("forge-{id}.timer")],
    )?;
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

// --- launchd (macOS) ---

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render_launchd_plist(
    id: &str,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
    spec: &ScheduleSpec,
) -> Result<String> {
    let mut args = vec![forge_exe.to_string(), "run".to_string(), task.to_string()];
    if let Some(m) = mode {
        args.push("--mode".to_string());
        args.push(m.to_string());
    }
    if let Some(m) = model {
        args.push("--model".to_string());
        args.push(m.to_string());
    }
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
        ScheduleSpec::Cron(_) => anyhow::bail!(
            "--cron isn't supported on macOS (launchd has no cron grammar) — use --every or --at"
        ),
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

#[allow(clippy::too_many_arguments)]
fn install_launchd(
    id: &str,
    spec: &ScheduleSpec,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
) -> Result<()> {
    let dir = launchd_agents_dir()?;
    std::fs::create_dir_all(&dir).context("creating ~/Library/LaunchAgents")?;
    let plist = render_launchd_plist(id, task, cwd, mode, model, forge_exe, spec)?;
    let path = dir.join(format!("dev.forge.schedule.{id}.plist"));
    std::fs::write(&path, plist).context("writing launchd plist")?;
    run_checked("launchctl", &["load", &path.to_string_lossy()])?;
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
fn render_schtasks_create_args(
    id: &str,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
    spec: &ScheduleSpec,
) -> Result<Vec<String>> {
    let mut inner = format!(
        "cd /d {cwd} && \"{forge_exe}\" run \"{}\"",
        task.replace('"', "\\\"")
    );
    if let Some(m) = mode {
        inner.push_str(&format!(" --mode {m}"));
    }
    if let Some(m) = model {
        inner.push_str(&format!(" --model \"{}\"", m.replace('"', "\\\"")));
    }

    let mut args = vec![
        "/Create".to_string(),
        "/TN".to_string(),
        format!("forge-{id}"),
        "/TR".to_string(),
        format!("cmd /C \"{inner}\""),
        "/F".to_string(),
    ];
    match spec {
        ScheduleSpec::Every(d) => {
            let minutes = (d.as_secs() / 60).max(1);
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
        ScheduleSpec::Cron(_) => anyhow::bail!(
            "--cron isn't supported on Windows (schtasks has no cron grammar) — use --every or --at"
        ),
    }
    Ok(args)
}

#[allow(clippy::too_many_arguments)]
fn install_schtasks(
    id: &str,
    spec: &ScheduleSpec,
    task: &str,
    cwd: &str,
    mode: Option<&str>,
    model: Option<&str>,
    forge_exe: &str,
) -> Result<()> {
    let args = render_schtasks_create_args(id, task, cwd, mode, model, forge_exe, spec)?;
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
mod tests {
    use super::*;

    #[test]
    fn parse_every_handles_units_and_rejects_garbage() {
        assert_eq!(
            parse_every("30m").unwrap(),
            std::time::Duration::from_secs(1800)
        );
        assert_eq!(
            parse_every("1h").unwrap(),
            std::time::Duration::from_secs(3600)
        );
        assert_eq!(
            parse_every("2d").unwrap(),
            std::time::Duration::from_secs(172_800)
        );
        assert_eq!(
            parse_every("45s").unwrap(),
            std::time::Duration::from_secs(45)
        );
        assert!(parse_every("abc").is_err());
        assert!(parse_every("0m").is_err());
        assert!(parse_every("10x").is_err());
    }

    #[test]
    fn parse_at_handles_valid_and_invalid_times() {
        assert_eq!(parse_at("09:30").unwrap(), (9, 30));
        assert_eq!(parse_at("23:59").unwrap(), (23, 59));
        assert!(parse_at("24:00").is_err());
        assert!(parse_at("9").is_err());
        assert!(parse_at("09:60").is_err());
    }

    #[test]
    fn schedule_spec_stored_round_trips() {
        let every = ScheduleSpec::Every(std::time::Duration::from_secs(1800));
        assert_eq!(every.to_stored(), "every:1800");
        assert_eq!(ScheduleSpec::from_stored("every:1800"), Some(every));

        let daily = ScheduleSpec::Daily { hour: 9, minute: 5 };
        assert_eq!(daily.to_stored(), "daily:09:05");
        assert_eq!(ScheduleSpec::from_stored("daily:09:05"), Some(daily));

        let cron = ScheduleSpec::Cron("Mon *-*-* 09:00:00".to_string());
        assert_eq!(cron.to_stored(), "cron:Mon *-*-* 09:00:00");
        assert_eq!(
            ScheduleSpec::from_stored("cron:Mon *-*-* 09:00:00"),
            Some(cron)
        );

        assert_eq!(ScheduleSpec::from_stored("garbage"), None);
    }

    #[test]
    fn systemd_service_unit_contains_task_cwd_and_exec() {
        let unit = render_systemd_service(
            "abc123",
            "check the deploy",
            "/home/user/proj",
            Some("bypass"),
            Some("openai::gpt-4o"),
            "/usr/local/bin/forge",
        );
        assert!(unit.contains("WorkingDirectory=/home/user/proj"));
        assert!(unit.contains("\"check the deploy\""));
        assert!(unit.contains("/usr/local/bin/forge run"));
        assert!(unit.contains("--mode bypass"));
        assert!(unit.contains("--model \"openai::gpt-4o\""));
    }

    #[test]
    fn systemd_timer_unit_encodes_every_daily_and_cron() {
        let every = render_systemd_timer(
            "abc123",
            &ScheduleSpec::Every(std::time::Duration::from_secs(1800)),
        )
        .unwrap();
        assert!(every.contains("OnUnitActiveSec=1800s"));

        let daily = render_systemd_timer(
            "abc123",
            &ScheduleSpec::Daily {
                hour: 9,
                minute: 30,
            },
        )
        .unwrap();
        assert!(daily.contains("OnCalendar=*-*-* 09:30:00"));

        let cron =
            render_systemd_timer("abc123", &ScheduleSpec::Cron("*-*-* 06:00:00".into())).unwrap();
        assert!(cron.contains("OnCalendar=*-*-* 06:00:00"));
    }

    #[test]
    fn launchd_plist_contains_task_cwd_and_interval() {
        let plist = render_launchd_plist(
            "abc123",
            "check the deploy",
            "/Users/me/proj",
            None,
            None,
            "/usr/local/bin/forge",
            &ScheduleSpec::Every(std::time::Duration::from_secs(3600)),
        )
        .unwrap();
        assert!(plist.contains("<string>check the deploy</string>"));
        assert!(plist.contains("<string>/Users/me/proj</string>"));
        assert!(plist.contains("<key>StartInterval</key>\n    <integer>3600</integer>"));
        assert!(plist.contains("dev.forge.schedule.abc123"));
    }

    #[test]
    fn launchd_plist_rejects_cron() {
        let err = render_launchd_plist(
            "abc123",
            "task",
            "/Users/me",
            None,
            None,
            "/usr/local/bin/forge",
            &ScheduleSpec::Cron("* * * * *".into()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("launchd"));
    }

    #[test]
    fn schtasks_args_contain_task_cwd_and_interval() {
        let args = render_schtasks_create_args(
            "abc123",
            "check the deploy",
            "C:\\Users\\me\\proj",
            Some("bypass"),
            None,
            "C:\\forge\\forge.exe",
            &ScheduleSpec::Every(std::time::Duration::from_secs(1800)),
        )
        .unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("check the deploy"));
        assert!(joined.contains("C:\\Users\\me\\proj"));
        assert!(joined.contains("/SC MINUTE"));
        assert!(joined.contains("/MO 30"));
        assert!(joined.contains("forge-abc123"));
    }

    #[test]
    fn schtasks_rejects_cron() {
        let err = render_schtasks_create_args(
            "abc123",
            "task",
            "C:\\x",
            None,
            None,
            "forge.exe",
            &ScheduleSpec::Cron("* * * * *".into()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Windows"));
    }
}
