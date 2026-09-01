//! `forge service` — opt-in, always-on background daemon for `forge serve`, installed as a
//! user-level OS service (systemd `--user` on Linux, a launchd agent on macOS, a logon
//! scheduled task on Windows — no root/sudo anywhere). Unlike `forge schedule` (which fires
//! one-shot `forge run` ticks on a timer), this supervises ONE long-lived `forge serve` process:
//! the OS restarts it on crash, and (Linux/macOS) at login.
//!
//! There is exactly one service per user (unlike schedules, which are id-keyed and can be
//! plural) — install/uninstall/status/start/stop/restart all target the same fixed unit name.
//! The chosen `forge serve` flags (exposure + port) are baked into the installed unit itself,
//! which is the single source of truth: `status` never parses them back out, it only asks the
//! OS service manager whether the unit exists / is running, and independently probes the port
//! (defaulting to the same `[remote] port` resolution `forge serve` itself uses, or an explicit
//! `--port` override for a service installed on a non-default port).

use anyhow::{Context, Result};

use crate::*;

use super::service_report::{
    activation_report, running_daemon, wait_for_running_daemon, Activation,
};

pub(crate) fn service_cmd(cmd: ServiceCmd) -> Result<()> {
    match cmd {
        ServiceCmd::Install {
            anywhere,
            lan,
            local,
            port,
        } => install_cmd(Exposure::from_flags(anywhere, lan, local), port),
        ServiceCmd::Uninstall => uninstall_cmd(),
        ServiceCmd::Status { port } => status_cmd(port),
        ServiceCmd::Start => control_cmd(ServiceControl::Start),
        ServiceCmd::Stop => control_cmd(ServiceControl::Stop),
        ServiceCmd::Restart => control_cmd(ServiceControl::Restart),
        ServiceCmd::Alert => super::service_alert::alert_cmd(),
    }
}

// ---------------------------------------------------------------------------
// Exposure — mirrors `forge serve`'s own `--local`/`--lan`/`--tunnel` (default: LAN).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exposure {
    Local,
    Lan,
    Anywhere,
}

impl Exposure {
    fn from_flags(anywhere: bool, lan: bool, local: bool) -> Self {
        let _ = lan; // clap already rejects combining flags; `lan` is accepted for symmetry only.
        if anywhere {
            Exposure::Anywhere
        } else if local {
            Exposure::Local
        } else {
            Exposure::Lan
        }
    }

    /// The `forge serve` flag this exposure maps to — always baked in explicitly (even for the
    /// LAN default) so the installed unit is self-documenting.
    fn flag(&self) -> &'static str {
        match self {
            Exposure::Local => "--local",
            Exposure::Lan => "--lan",
            Exposure::Anywhere => "--tunnel",
        }
    }
}

enum ServiceControl {
    Start,
    Stop,
    Restart,
}

// ---------------------------------------------------------------------------
// forge service install / uninstall / status / start / stop / restart
// ---------------------------------------------------------------------------

pub(crate) fn resolved_port(port: Option<u16>) -> u16 {
    port.unwrap_or_else(|| forge_config::load().unwrap_or_default().remote.serve_port())
}

fn token_file_path() -> Option<std::path::PathBuf> {
    forge_config::config_dir().map(|d| d.join("serve-token"))
}

fn install_cmd(exposure: Exposure, port: Option<u16>) -> Result<()> {
    let forge_exe = std::env::current_exe()
        .context("resolving the forge binary path")?
        .to_string_lossy()
        .to_string();
    let port = resolved_port(port);

    // Captured BEFORE the unit is rewritten: an upgrade replaces the binary at the same path, so
    // afterwards there is nothing left to compare the new daemon against.
    let before = running_daemon();
    let was_running = query_service_status()?.running;

    let outcome = install_service(&forge_exe, exposure, port, was_running)?;

    println!("✓ installed forge-serve ({})", outcome.backend_label);
    println!("  unit: {}", outcome.unit_path);
    if let Some(note) = outcome.note {
        println!("  note: {note}");
    }
    let after = if cfg!(target_os = "linux") {
        wait_for_running_daemon()
    } else {
        running_daemon()
    };
    let post_status = query_service_status();
    let failure = match (&outcome.activation, &post_status) {
        (Activation::Failed(err), _) => Some(err.clone()),
        (_, Ok(status)) if !status.running => {
            Some("the service manager reports that the daemon is not running".to_string())
        }
        (_, Err(err)) => Some(format!(
            "could not verify the daemon after activation: {err:#}"
        )),
        _ if cfg!(target_os = "linux") && after.is_none() => {
            Some("the service manager has no identifiable daemon process".to_string())
        }
        _ => None,
    };
    println!(
        "  {}",
        activation_report(outcome.activation, before.as_ref(), after.as_ref())
    );
    if let Some(token_path) = token_file_path() {
        println!(
            "  connect: http://127.0.0.1:{port}/<token> — token is minted on first start at \
             {}",
            token_path.display()
        );
    } else {
        println!("  connect: port {port} once running (could not resolve the config dir for the token file)");
    }
    if let Some(err) = failure {
        anyhow::bail!("the unit was written but the daemon is not running it: {err}");
    }
    Ok(())
}

fn uninstall_cmd() -> Result<()> {
    uninstall_service()?;
    println!("✓ removed the forge-serve background service");
    Ok(())
}

fn status_cmd(port: Option<u16>) -> Result<()> {
    let status = query_service_status()?;
    let port = resolved_port(port);
    let port_up = probe_port(port);

    println!("installed: {}", if status.installed { "yes" } else { "no" });
    println!("running:   {}", if status.running { "yes" } else { "no" });
    println!(
        "port {port}:  {}",
        if port_up {
            "responding"
        } else {
            "not responding"
        }
    );
    match running_daemon() {
        Some(d) => println!("process:   {d}"),
        None if status.running => {
            println!("process:   unknown (could not resolve the running binary)")
        }
        None => {}
    }
    if !status.detail.is_empty() {
        println!("detail:    {}", status.detail);
    }
    if let Some(failure) = status_failure(&status, port, port_up) {
        anyhow::bail!("{failure}");
    }
    Ok(())
}

/// Return the machine-readable failure for `forge service status` after its human-readable
/// report has been printed. Keeping the report and the exit status separate lets people inspect a
/// stopped service interactively while allowing a watchdog to detect the same outage reliably.
fn status_failure(status: &ServiceStatus, port: u16, port_up: bool) -> Option<String> {
    if !status.installed {
        return Some("forge-serve is not installed".to_string());
    }
    if !status.running {
        return Some(format!(
            "forge-serve is installed but not running ({})",
            if status.detail.is_empty() {
                "unknown state"
            } else {
                status.detail.as_str()
            }
        ));
    }
    if !port_up {
        return Some(format!(
            "forge-serve is active but port {port} is not responding"
        ));
    }
    None
}

fn control_cmd(action: ServiceControl) -> Result<()> {
    let verb = match action {
        ServiceControl::Start => "started",
        ServiceControl::Stop => "stopped",
        ServiceControl::Restart => "restarted",
    };
    control_service(action)?;
    println!("✓ {verb} forge-serve");
    Ok(())
}

/// TCP-connect probe with a short timeout — never blocks the CLI for long on a dead port.
pub(crate) fn probe_port(port: u16) -> bool {
    let addr = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500)).is_ok()
}

// ---------------------------------------------------------------------------
// OS backend install/uninstall/status/control. The unit/plist/schtasks STRING renderers below
// this point are pure and unit tested; only the `*_service` functions (and the per-OS helpers
// they dispatch to) shell out to the real service manager, so `cargo test` never touches this
// machine's systemd/launchd/Task Scheduler. `cfg!(target_os = ..)` runtime branches (not
// `#[cfg]` compile gates) so every branch still typechecks on this (Linux) build host, matching
// `forge schedule`'s pattern.
// ---------------------------------------------------------------------------

struct InstallOutcome {
    backend_label: &'static str,
    unit_path: String,
    note: Option<String>,
    activation: Activation,
}

pub(crate) struct ServiceStatus {
    pub(crate) installed: bool,
    pub(crate) running: bool,
    /// Raw state word from the OS service manager: `active`, `failed`, `activating`, …
    pub(crate) detail: String,
}

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

pub(crate) const SYSTEMD_UNIT_NAME: &str = "forge-serve.service";
const LAUNCHD_LABEL: &str = "dev.forge.serve";
const SCHTASKS_NAME: &str = "ForgeServe";

fn install_service(
    forge_exe: &str,
    exposure: Exposure,
    port: u16,
    was_running: bool,
) -> Result<InstallOutcome> {
    if cfg!(target_os = "linux") {
        install_systemd(forge_exe, exposure, port, was_running)
    } else if cfg!(target_os = "macos") {
        install_launchd(forge_exe, exposure, port, was_running)
    } else if cfg!(target_os = "windows") {
        install_schtasks(forge_exe, exposure, port, was_running)
    } else {
        anyhow::bail!("forge service has no background-daemon backend for this platform")
    }
}

fn uninstall_service() -> Result<()> {
    if cfg!(target_os = "linux") {
        uninstall_systemd()
    } else if cfg!(target_os = "macos") {
        uninstall_launchd()
    } else if cfg!(target_os = "windows") {
        uninstall_schtasks()
    } else {
        Ok(())
    }
}

pub(crate) fn query_service_status() -> Result<ServiceStatus> {
    if cfg!(target_os = "linux") {
        status_systemd()
    } else if cfg!(target_os = "macos") {
        status_launchd()
    } else if cfg!(target_os = "windows") {
        status_schtasks()
    } else {
        Ok(ServiceStatus {
            installed: false,
            running: false,
            detail: "unsupported platform".to_string(),
        })
    }
}

fn control_service(action: ServiceControl) -> Result<()> {
    if cfg!(target_os = "linux") {
        control_systemd(action)
    } else if cfg!(target_os = "macos") {
        control_launchd(action)
    } else if cfg!(target_os = "windows") {
        control_schtasks(action)
    } else {
        anyhow::bail!("forge service has no background-daemon backend for this platform")
    }
}

/// Run `cmd args…` directly (no shell), surfacing a non-zero exit or spawn failure with stderr
/// attached and an actionable hint — the one place that actually touches the host's service
/// manager for a mutating call (install/uninstall/start/stop/restart).
fn run_checked(cmd: &str, args: &[&str], hint: &str) -> Result<()> {
    let output = std::process::Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("spawning `{cmd}` — {hint}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "`{cmd} {}` failed: {} — {hint}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Run `cmd args…` and return (success, trimmed stdout, trimmed stderr) without treating a
/// non-zero exit as an error — status queries use exit codes/stdout as signal, not failure.
pub(crate) fn run_capture(cmd: &str, args: &[&str]) -> Result<(bool, String, String)> {
    let output = std::process::Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("spawning `{cmd}`"))?;
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        String::from_utf8_lossy(&output.stderr).trim().to_string(),
    ))
}

// --- systemd (Linux) ---

/// Marker recording which Forge rendered a unit. `install` writes the unit once and nothing
/// rewrites it, so without this there is no way to tell a current unit from one written years ago
/// except by grepping for individual directives — which only finds drift someone already knew to
/// look for. Doctor compares this to the running version and reports the gap generically.
pub(crate) const UNIT_VERSION_PREFIX: &str = "# forge-unit-version: ";

fn render_systemd_service(forge_exe: &str, exposure: Exposure, port: u16) -> String {
    format!(
        "[Unit]\nDescription=Forge serve — headless multi-session daemon\n\
         {UNIT_VERSION_PREFIX}{}\n\
         # Stopping is not the same as telling anyone: the observed outages sat in `failed` for\n\
         # days and were found only because someone went looking. systemd delivers this, so the\n\
         # alarm does not share fate with the daemon it watches.\n\
         # MUST live in [Unit]: OnFailure= is a unit-section directive and systemd SILENTLY\n\
         # ignores it under [Service], which is where it was first written — the alarm looked\n\
         # installed and never fired.\n\
         OnFailure={}\n\n\
         [Service]\nExecStart={forge_exe} serve {} --port {port}\nRestart=on-failure\n\
         # A permanent failure (exit 78 = EX_CONFIG, e.g. a store this build cannot open) cannot be\n\
         # fixed by retrying. Without this the daemon restarts forever — 632 times in one observed\n\
         # case, with Anywhere down throughout and the unit still reporting `activating`. Stopping\n\
         # makes it `failed`, which is at least visible. Transient failures still retry forever,\n\
         # which is what the network-resilience drop-in wants.\nRestartPreventExitStatus=78\n\
         # An agent-owned compiler or language server may be the kernel's OOM victim. Keep the\n\
         # daemon and its recoverable session metadata alive in that case.\nOOMPolicy=continue\n\n\
         [Install]\nWantedBy=default.target\n",
        env!("CARGO_PKG_VERSION"),
        super::service_alert::ALERT_UNIT_NAME,
        exposure.flag()
    )
}

/// Run `cmd` for a whole activation sequence, folding a failed step into `Activation::Failed`
/// rather than aborting the install — the unit on disk is already correct and worth reporting.
fn activation_result(inner: Result<Activation>) -> Activation {
    inner.unwrap_or_else(|e| Activation::Failed(format!("{e:#}")))
}

type Runner<'a> = &'a mut dyn FnMut(&[&str]) -> Result<()>;

/// `enable --now` cannot move an already-running unit onto a changed `ExecStart`; only `restart`
/// can. `enable` (without `--now`) still runs so boot-time activation is repaired either way.
fn activate_systemd(was_running: bool, run: Runner<'_>) -> Result<Activation> {
    run(&["--user", "daemon-reload"])?;
    if was_running {
        run(&["--user", "enable", SYSTEMD_UNIT_NAME])?;
        run(&["--user", "restart", SYSTEMD_UNIT_NAME])?;
        Ok(Activation::Restarted)
    } else {
        run(&["--user", "enable", "--now", SYSTEMD_UNIT_NAME])?;
        Ok(Activation::Started)
    }
}

fn install_systemd(
    forge_exe: &str,
    exposure: Exposure,
    port: u16,
    was_running: bool,
) -> Result<InstallOutcome> {
    let hint = "is a systemd user manager available? (are you in a systemd session / logged in \
                via a graphical or SSH session with `XDG_RUNTIME_DIR` set?)";
    let dir = systemd_user_dir()?;
    std::fs::create_dir_all(&dir).context("creating ~/.config/systemd/user")?;
    let unit_path = dir.join(SYSTEMD_UNIT_NAME);
    std::fs::write(
        &unit_path,
        render_systemd_service(forge_exe, exposure, port),
    )
    .context("writing the systemd user unit")?;

    // Written before the reload so the OnFailure= target exists the moment the unit is enabled.
    std::fs::write(
        dir.join(super::service_alert::ALERT_UNIT_NAME),
        super::service_alert::render_systemd_alert_unit(forge_exe),
    )
    .context("writing the systemd failure-notification unit")?;

    let activation = activation_result(activate_systemd(was_running, &mut |args| {
        run_checked("systemctl", args, hint)
    }));
    Ok(InstallOutcome {
        backend_label: "systemd --user",
        unit_path: unit_path.display().to_string(),
        activation,
        note: Some(
            "surviving reboot BEFORE you log in requires `loginctl enable-linger $USER` \
             (not run automatically — it may need auth)"
                .to_string(),
        ),
    })
}

fn uninstall_systemd() -> Result<()> {
    let dir = systemd_user_dir()?;
    let hint = "is a systemd user manager available?";
    let _ = run_checked(
        "systemctl",
        &["--user", "disable", "--now", SYSTEMD_UNIT_NAME],
        hint,
    );
    let _ = std::fs::remove_file(dir.join(SYSTEMD_UNIT_NAME));
    let _ = std::fs::remove_file(dir.join(super::service_alert::ALERT_UNIT_NAME));
    let _ = run_checked("systemctl", &["--user", "daemon-reload"], hint);
    Ok(())
}

/// The installed systemd unit's text, if there is one.
///
/// `forge service install` writes this file ONCE and nothing rewrites it afterwards, so an upgraded
/// binary can be paired with a unit rendered by a much older version. Doctor reads it to report that
/// drift; see `doctor::unit_drift_checks`.
#[cfg(target_os = "linux")]
pub(crate) fn installed_unit_text() -> Option<String> {
    std::fs::read_to_string(systemd_user_dir().ok()?.join(SYSTEMD_UNIT_NAME)).ok()
}

fn status_systemd() -> Result<ServiceStatus> {
    let installed = systemd_user_dir()
        .map(|d| d.join(SYSTEMD_UNIT_NAME).is_file())
        .unwrap_or(false);
    let (_, stdout, _) = run_capture("systemctl", &["--user", "is-active", SYSTEMD_UNIT_NAME])?;
    let running = stdout == "active";
    Ok(ServiceStatus {
        installed,
        running,
        detail: stdout,
    })
}

fn control_systemd(action: ServiceControl) -> Result<()> {
    let verb = match action {
        ServiceControl::Start => "start",
        ServiceControl::Stop => "stop",
        ServiceControl::Restart => "restart",
    };
    run_checked(
        "systemctl",
        &["--user", verb, SYSTEMD_UNIT_NAME],
        "is the service installed? (`forge service install`) and is a systemd user manager \
         available?",
    )
}

// --- launchd (macOS) ---

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render_launchd_plist(forge_exe: &str, exposure: Exposure, port: u16) -> String {
    let args = [
        forge_exe.to_string(),
        "serve".to_string(),
        exposure.flag().to_string(),
        "--port".to_string(),
        port.to_string(),
    ];
    let mut args_xml = String::new();
    for a in &args {
        args_xml.push_str(&format!("        <string>{}</string>\n", xml_escape(a)));
    }

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
    );
    out.push_str("<plist version=\"1.0\">\n<dict>\n");
    out.push_str(&format!(
        "    <key>Label</key>\n    <string>{LAUNCHD_LABEL}</string>\n"
    ));
    out.push_str("    <key>ProgramArguments</key>\n    <array>\n");
    out.push_str(&args_xml);
    out.push_str("    </array>\n");
    out.push_str("    <key>RunAtLoad</key>\n    <true/>\n");
    // Restart on crash only (not on a clean/successful exit) — the daemon-supervision analog of
    // systemd's Restart=on-failure.
    out.push_str(
        "    <key>KeepAlive</key>\n    <dict>\n        <key>SuccessfulExit</key>\n        \
         <false/>\n    </dict>\n",
    );
    out.push_str("</dict>\n</plist>\n");
    out
}

fn macos_gui_target() -> Result<String> {
    let (ok, uid, stderr) = run_capture("id", &["-u"])?;
    if !ok || uid.is_empty() {
        anyhow::bail!("could not resolve the current UID via `id -u`: {stderr}");
    }
    Ok(format!("gui/{uid}"))
}

/// A loaded agent keeps serving its ORIGINAL plist, so a changed `ProgramArguments` (or a
/// replaced binary) needs a bootout/bootstrap cycle; `bootstrap` alone fails on an already
/// bootstrapped label.
fn activate_launchd(
    was_running: bool,
    target: &str,
    plist_path: &str,
    run: Runner<'_>,
) -> Result<Activation> {
    let service = format!("{target}/{LAUNCHD_LABEL}");
    if was_running {
        run(&["bootout", &service])?;
    }
    // `bootstrap` is the modern (10.10+) API; fall back to the legacy `load -w` for older macOS
    // or launchd builds that reject bootstrap for user agents.
    if run(&["bootstrap", target, plist_path]).is_err() {
        run(&["load", "-w", plist_path])?;
    }
    if was_running {
        Ok(Activation::Restarted)
    } else {
        Ok(Activation::Started)
    }
}

fn install_launchd(
    forge_exe: &str,
    exposure: Exposure,
    port: u16,
    was_running: bool,
) -> Result<InstallOutcome> {
    let dir = launchd_agents_dir()?;
    std::fs::create_dir_all(&dir).context("creating ~/Library/LaunchAgents")?;
    let plist = render_launchd_plist(forge_exe, exposure, port);
    let path = dir.join(format!("{LAUNCHD_LABEL}.plist"));
    std::fs::write(&path, plist).context("writing the launchd agent plist")?;

    let target = macos_gui_target()?;
    let hint = "is launchd reachable? (are you in a GUI login session?)";
    let activation = activation_result(activate_launchd(
        was_running,
        &target,
        &path.to_string_lossy(),
        &mut |args| run_checked("launchctl", args, hint),
    ));
    Ok(InstallOutcome {
        backend_label: "launchd agent",
        unit_path: path.display().to_string(),
        note: None,
        activation,
    })
}

fn uninstall_launchd() -> Result<()> {
    let dir = launchd_agents_dir()?;
    let path = dir.join(format!("{LAUNCHD_LABEL}.plist"));
    if let Ok(target) = macos_gui_target() {
        let _ = run_checked(
            "launchctl",
            &["bootout", &format!("{target}/{LAUNCHD_LABEL}")],
            "is launchd reachable?",
        );
    }
    let _ = run_checked("launchctl", &["unload", "-w", &path.to_string_lossy()], "");
    let _ = std::fs::remove_file(&path);
    Ok(())
}

fn status_launchd() -> Result<ServiceStatus> {
    let installed = launchd_agents_dir()
        .map(|d| d.join(format!("{LAUNCHD_LABEL}.plist")).is_file())
        .unwrap_or(false);
    let target = macos_gui_target()?;
    let (running, stdout, _) = run_capture(
        "launchctl",
        &["print", &format!("{target}/{LAUNCHD_LABEL}")],
    )?;
    Ok(ServiceStatus {
        installed,
        running,
        detail: if running {
            "loaded".to_string()
        } else {
            stdout
        },
    })
}

fn control_launchd(action: ServiceControl) -> Result<()> {
    let target = macos_gui_target()?;
    let service = format!("{target}/{LAUNCHD_LABEL}");
    let hint = "is the service installed? (`forge service install`)";
    match action {
        ServiceControl::Start => run_checked("launchctl", &["kickstart", &service], hint),
        ServiceControl::Stop => run_checked("launchctl", &["kill", "SIGTERM", &service], hint),
        ServiceControl::Restart => run_checked("launchctl", &["kickstart", "-k", &service], hint),
    }
}

// --- Task Scheduler (Windows) ---
//
// Not a real Windows Service (SCM): `forge serve` doesn't speak the Service Control Manager
// protocol (SERVICE_STATUS reporting, control-code handling), and teaching it to would mean
// either rewriting it as a Windows service (a large, Windows-only surface) or wrapping it with
// an SCM shim like NSSM — an external dependency we won't require. A logon scheduled task gets
// the same practical result (start automatically, run in the background, restart on failure via
// `/RI` is not supported by schtasks — restart-on-crash is systemd/launchd-only) with zero extra
// tooling.

fn render_schtasks_create_args(forge_exe: &str, exposure: Exposure, port: u16) -> Vec<String> {
    vec![
        "/Create".to_string(),
        "/TN".to_string(),
        SCHTASKS_NAME.to_string(),
        "/SC".to_string(),
        "ONLOGON".to_string(),
        "/TR".to_string(),
        format!("\"{forge_exe}\" serve {} --port {port}", exposure.flag()),
        "/F".to_string(),
    ]
}

/// `/Create /F` replaces the task definition but never touches the instance already running it,
/// and `/Run` on a running task is refused — so an upgrade needs an explicit `/End` first.
fn activate_schtasks(
    was_running: bool,
    create_args: &[&str],
    run: Runner<'_>,
) -> Result<Activation> {
    run(create_args)?;
    if was_running {
        run(&["/End", "/TN", SCHTASKS_NAME])?;
    }
    // Start it now too, rather than waiting for the next logon.
    run(&["/Run", "/TN", SCHTASKS_NAME])?;
    Ok(if was_running {
        Activation::Restarted
    } else {
        Activation::Started
    })
}

fn install_schtasks(
    forge_exe: &str,
    exposure: Exposure,
    port: u16,
    was_running: bool,
) -> Result<InstallOutcome> {
    let hint = "is Task Scheduler reachable? (`schtasks` requires an interactive logon session)";
    let args = render_schtasks_create_args(forge_exe, exposure, port);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let activation = activation_result(activate_schtasks(was_running, &arg_refs, &mut |a| {
        run_checked("schtasks", a, hint)
    }));
    Ok(InstallOutcome {
        backend_label: "Task Scheduler logon task",
        unit_path: format!("Task Scheduler: \\{SCHTASKS_NAME}"),
        note: None,
        activation,
    })
}

fn uninstall_schtasks() -> Result<()> {
    let _ = run_checked("schtasks", &["/End", "/TN", SCHTASKS_NAME], "");
    let _ = run_checked("schtasks", &["/Delete", "/TN", SCHTASKS_NAME, "/F"], "");
    Ok(())
}

fn status_schtasks() -> Result<ServiceStatus> {
    let (ok, stdout, _) = run_capture(
        "schtasks",
        &["/Query", "/TN", SCHTASKS_NAME, "/FO", "LIST", "/V"],
    )?;
    let running = stdout
        .lines()
        .find(|l| l.trim_start().starts_with("Status:"))
        .map(|l| l.contains("Running"))
        .unwrap_or(false);
    Ok(ServiceStatus {
        installed: ok,
        running,
        detail: stdout,
    })
}

fn control_schtasks(action: ServiceControl) -> Result<()> {
    let hint = "is the service installed? (`forge service install`)";
    match action {
        ServiceControl::Start => run_checked("schtasks", &["/Run", "/TN", SCHTASKS_NAME], hint),
        ServiceControl::Stop => run_checked("schtasks", &["/End", "/TN", SCHTASKS_NAME], hint),
        ServiceControl::Restart => {
            let _ = run_checked("schtasks", &["/End", "/TN", SCHTASKS_NAME], hint);
            run_checked("schtasks", &["/Run", "/TN", SCHTASKS_NAME], hint)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_from_flags_defaults_to_lan() {
        assert_eq!(Exposure::from_flags(false, false, false), Exposure::Lan);
        assert_eq!(Exposure::from_flags(false, true, false), Exposure::Lan);
        assert_eq!(Exposure::from_flags(false, false, true), Exposure::Local);
        assert_eq!(Exposure::from_flags(true, false, false), Exposure::Anywhere);
    }

    #[test]
    fn systemd_service_unit_contains_exec_and_restart_policy() {
        let unit = render_systemd_service("/usr/local/bin/forge", Exposure::Lan, 7420);
        assert!(unit.contains("ExecStart=/usr/local/bin/forge serve --lan --port 7420"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("OOMPolicy=continue"));
        assert!(
            !unit.contains("OOMPolicy=stop"),
            "an OOM-selected agent child must not stop the daemon"
        );
        assert!(unit.contains("WantedBy=default.target"));
    }

    /// `OnFailure=` is a [Unit] directive. systemd SILENTLY ignores it under [Service] — no
    /// warning, no parse error, `systemctl show -p OnFailure` simply returns empty. It was first
    /// rendered under [Service], so the deadman alarm from #1134 looked installed on every machine
    /// and never fired once. Verified on a real host: the unit file contained
    /// `OnFailure=forge-serve-alert.service` while `systemctl --user show forge-serve -p OnFailure`
    /// printed `OnFailure=`.
    ///
    /// Asserting the string is present is NOT enough — that assertion passed while the alarm was
    /// dead. The section it lands in is the whole bug, so that is what this pins.
    #[test]
    fn systemd_on_failure_is_in_the_unit_section_not_the_service_section() {
        let unit = render_systemd_service("/usr/local/bin/forge", Exposure::Lan, 7420);
        let on_failure = unit
            .find("OnFailure=")
            .expect("unit must wire the deadman alarm");
        let service_section = unit
            .find("[Service]")
            .expect("unit must have a [Service] section");
        let unit_section = unit
            .find("[Unit]")
            .expect("unit must have a [Unit] section");

        assert!(
            on_failure > unit_section && on_failure < service_section,
            "OnFailure= must sit in [Unit]; systemd ignores it in [Service]:\n{unit}"
        );
        assert!(
            unit.contains(&format!(
                "OnFailure={}",
                super::super::service_alert::ALERT_UNIT_NAME
            )),
            "OnFailure must name the alert unit:\n{unit}"
        );
    }

    /// The unit records which Forge rendered it. Without the stamp there is no way to tell a
    /// current unit from one written by a much older version, because installing a new Forge never
    /// rewrites it — which is how #996's `RestartPreventExitStatus` silently failed to reach
    /// existing installs.
    #[test]
    fn systemd_service_unit_records_the_version_that_rendered_it() {
        let unit = render_systemd_service("/usr/local/bin/forge", Exposure::Lan, 7420);
        assert!(
            unit.contains(&format!(
                "{UNIT_VERSION_PREFIX}{}",
                env!("CARGO_PKG_VERSION")
            )),
            "unit must carry the rendering version:\n{unit}"
        );
    }

    #[test]
    fn systemd_service_unit_encodes_local_and_anywhere() {
        let local = render_systemd_service("/bin/forge", Exposure::Local, 1234);
        assert!(local.contains("serve --local --port 1234"));
        let anywhere = render_systemd_service("/bin/forge", Exposure::Anywhere, 1234);
        assert!(anywhere.contains("serve --tunnel --port 1234"));
    }

    #[test]
    fn launchd_plist_contains_label_args_and_keepalive() {
        let plist = render_launchd_plist("/usr/local/bin/forge", Exposure::Local, 7451);
        assert!(plist.contains("<string>dev.forge.serve</string>"));
        assert!(plist.contains("<string>/usr/local/bin/forge</string>"));
        assert!(plist.contains("<string>serve</string>"));
        assert!(plist.contains("<string>--local</string>"));
        assert!(plist.contains("<string>7451</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(plist.contains("<key>SuccessfulExit</key>\n        <false/>"));
    }

    #[test]
    fn schtasks_args_contain_task_name_trigger_and_command() {
        let args = render_schtasks_create_args("C:\\forge\\forge.exe", Exposure::Lan, 7420);
        let joined = args.join(" ");
        assert!(joined.contains("/TN ForgeServe"));
        assert!(joined.contains("/SC ONLOGON"));
        assert!(joined.contains("C:\\forge\\forge.exe"));
        assert!(joined.contains("serve --lan --port 7420"));
        assert!(joined.contains("/F"));
    }

    #[test]
    fn probe_port_detects_a_bound_listener_and_a_closed_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(probe_port(port));
        drop(listener);
        // Best-effort: the OS may not release the port instantly, but on a freshly bound
        // ephemeral port this is reliably free in practice for this fast a re-check window.
        // Use an unlikely-to-be-bound low port instead of relying on immediate release.
        assert!(!probe_port(1));
    }

    /// Records every command an activation issues, so the systemctl/launchctl/schtasks calls can
    /// be asserted without touching this machine's service manager.
    type CommandLog = std::rc::Rc<std::cell::RefCell<Vec<String>>>;

    fn recorder() -> (CommandLog, impl FnMut(&[&str]) -> Result<()>) {
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = log.clone();
        (log, move |args: &[&str]| {
            sink.borrow_mut().push(args.join(" "));
            Ok(())
        })
    }

    /// The whole bug: `enable --now` is a no-op for a unit that is ALREADY running, so a
    /// re-rendered ExecStart (or a binary replaced at the same path by an upgrade) never took
    /// effect, while install printed the new command line as if it had.
    #[test]
    fn install_restarts_an_already_running_systemd_unit() {
        let (log, mut run) = recorder();
        let activation = activate_systemd(true, &mut run).unwrap();
        assert_eq!(activation, Activation::Restarted);
        assert_eq!(
            *log.borrow(),
            vec![
                "--user daemon-reload",
                "--user enable forge-serve.service",
                "--user restart forge-serve.service",
            ]
        );
    }

    #[test]
    fn install_only_enables_now_when_the_systemd_unit_is_not_running() {
        let (log, mut run) = recorder();
        let activation = activate_systemd(false, &mut run).unwrap();
        assert_eq!(activation, Activation::Started);
        assert_eq!(
            *log.borrow(),
            vec![
                "--user daemon-reload",
                "--user enable --now forge-serve.service",
            ]
        );
        assert!(!log.borrow().iter().any(|c| c.contains("restart")));
    }

    /// A loaded launchd agent keeps its original ProgramArguments; `bootstrap` alone also fails
    /// on an already-bootstrapped label, so re-installing over a running agent used to error out.
    #[test]
    fn install_boots_out_and_back_in_for_a_running_launchd_agent() {
        let (log, mut run) = recorder();
        let activation = activate_launchd(true, "gui/501", "/tmp/a.plist", &mut run).unwrap();
        assert_eq!(activation, Activation::Restarted);
        assert_eq!(
            *log.borrow(),
            vec![
                "bootout gui/501/dev.forge.serve",
                "bootstrap gui/501 /tmp/a.plist",
            ]
        );

        let (log, mut run) = recorder();
        assert_eq!(
            activate_launchd(false, "gui/501", "/tmp/a.plist", &mut run).unwrap(),
            Activation::Started
        );
        assert_eq!(*log.borrow(), vec!["bootstrap gui/501 /tmp/a.plist"]);
    }

    #[test]
    fn install_ends_a_running_scheduled_task_before_running_the_new_definition() {
        let create = ["/Create", "/TN", "ForgeServe", "/F"];
        let (log, mut run) = recorder();
        assert_eq!(
            activate_schtasks(true, &create, &mut run).unwrap(),
            Activation::Restarted
        );
        assert_eq!(
            *log.borrow(),
            vec![
                "/Create /TN ForgeServe /F",
                "/End /TN ForgeServe",
                "/Run /TN ForgeServe",
            ]
        );

        let (log, mut run) = recorder();
        assert_eq!(
            activate_schtasks(false, &create, &mut run).unwrap(),
            Activation::Started
        );
        assert_eq!(
            *log.borrow(),
            vec!["/Create /TN ForgeServe /F", "/Run /TN ForgeServe"]
        );
    }

    #[test]
    fn a_failed_activation_step_is_reported_instead_of_aborting_the_install() {
        let activation = activation_result(activate_systemd(true, &mut |args| {
            if args.contains(&"restart") {
                anyhow::bail!("Job for forge-serve.service failed");
            }
            Ok(())
        }));
        let Activation::Failed(err) = &activation else {
            panic!("a failed restart must not report success: {activation:?}");
        };
        assert!(err.contains("failed"), "{err}");
        assert!(activation_report(activation, None, None).starts_with("! daemon NOT running"));
    }

    #[test]
    fn status_failure_distinguishes_missing_stopped_and_unresponsive_service() {
        let missing = ServiceStatus {
            installed: false,
            running: false,
            detail: "inactive".into(),
        };
        assert_eq!(
            status_failure(&missing, 7420, false).as_deref(),
            Some("forge-serve is not installed")
        );

        let stopped = ServiceStatus {
            installed: true,
            running: false,
            detail: "failed".into(),
        };
        assert_eq!(
            status_failure(&stopped, 7420, false).as_deref(),
            Some("forge-serve is installed but not running (failed)")
        );

        let unresponsive = ServiceStatus {
            installed: true,
            running: true,
            detail: "active".into(),
        };
        assert_eq!(
            status_failure(&unresponsive, 7420, false).as_deref(),
            Some("forge-serve is active but port 7420 is not responding")
        );
        assert!(status_failure(&unresponsive, 7420, true).is_none());
    }
}
