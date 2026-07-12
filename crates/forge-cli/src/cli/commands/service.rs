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
    }
}

// ---------------------------------------------------------------------------
// Exposure — mirrors `forge serve`'s own `--local`/`--lan`/`--anywhere` (default: LAN).
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
            Exposure::Anywhere => "--anywhere",
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

fn resolved_port(port: Option<u16>) -> u16 {
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

    let outcome = install_service(&forge_exe, exposure, port)?;

    println!("✓ installed forge-serve ({})", outcome.backend_label);
    println!("  unit: {}", outcome.unit_path);
    println!(
        "  runs: {forge_exe} serve {} --port {port}",
        exposure.flag()
    );
    if let Some(note) = outcome.note {
        println!("  note: {note}");
    }
    if let Some(token_path) = token_file_path() {
        println!(
            "  connect: http://127.0.0.1:{port}/<token> — token is minted on first start at \
             {}",
            token_path.display()
        );
    } else {
        println!("  connect: port {port} once running (could not resolve the config dir for the token file)");
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
    if !status.detail.is_empty() {
        println!("detail:    {}", status.detail);
    }
    Ok(())
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
fn probe_port(port: u16) -> bool {
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
}

struct ServiceStatus {
    installed: bool,
    running: bool,
    detail: String,
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

const SYSTEMD_UNIT_NAME: &str = "forge-serve.service";
const LAUNCHD_LABEL: &str = "dev.forge.serve";
const SCHTASKS_NAME: &str = "ForgeServe";

fn install_service(forge_exe: &str, exposure: Exposure, port: u16) -> Result<InstallOutcome> {
    if cfg!(target_os = "linux") {
        install_systemd(forge_exe, exposure, port)
    } else if cfg!(target_os = "macos") {
        install_launchd(forge_exe, exposure, port)
    } else if cfg!(target_os = "windows") {
        install_schtasks(forge_exe, exposure, port)
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

fn query_service_status() -> Result<ServiceStatus> {
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
fn run_capture(cmd: &str, args: &[&str]) -> Result<(bool, String, String)> {
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

fn render_systemd_service(forge_exe: &str, exposure: Exposure, port: u16) -> String {
    format!(
        "[Unit]\nDescription=Forge serve — headless multi-session daemon\n\n\
         [Service]\nExecStart={forge_exe} serve {} --port {port}\nRestart=on-failure\n\n\
         [Install]\nWantedBy=default.target\n",
        exposure.flag()
    )
}

fn install_systemd(forge_exe: &str, exposure: Exposure, port: u16) -> Result<InstallOutcome> {
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

    run_checked("systemctl", &["--user", "daemon-reload"], hint)?;
    run_checked(
        "systemctl",
        &["--user", "enable", "--now", SYSTEMD_UNIT_NAME],
        hint,
    )?;
    Ok(InstallOutcome {
        backend_label: "systemd --user",
        unit_path: unit_path.display().to_string(),
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
    let _ = run_checked("systemctl", &["--user", "daemon-reload"], hint);
    Ok(())
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

fn install_launchd(forge_exe: &str, exposure: Exposure, port: u16) -> Result<InstallOutcome> {
    let dir = launchd_agents_dir()?;
    std::fs::create_dir_all(&dir).context("creating ~/Library/LaunchAgents")?;
    let plist = render_launchd_plist(forge_exe, exposure, port);
    let path = dir.join(format!("{LAUNCHD_LABEL}.plist"));
    std::fs::write(&path, plist).context("writing the launchd agent plist")?;

    let target = macos_gui_target()?;
    let hint = "is launchd reachable? (are you in a GUI login session?)";
    // `bootstrap` is the modern (10.10+) API; fall back to the legacy `load -w` for older macOS
    // or launchd builds that reject bootstrap for user agents.
    let bootstrap = std::process::Command::new("launchctl")
        .args(["bootstrap", &target, &path.to_string_lossy()])
        .output()
        .with_context(|| format!("spawning `launchctl bootstrap` — {hint}"))?;
    if !bootstrap.status.success() {
        run_checked("launchctl", &["load", "-w", &path.to_string_lossy()], hint)?;
    }
    Ok(InstallOutcome {
        backend_label: "launchd agent",
        unit_path: path.display().to_string(),
        note: None,
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

fn install_schtasks(forge_exe: &str, exposure: Exposure, port: u16) -> Result<InstallOutcome> {
    let hint = "is Task Scheduler reachable? (`schtasks` requires an interactive logon session)";
    let args = render_schtasks_create_args(forge_exe, exposure, port);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_checked("schtasks", &arg_refs, hint)?;
    // Start it now too, rather than waiting for the next logon.
    run_checked("schtasks", &["/Run", "/TN", SCHTASKS_NAME], hint)?;
    Ok(InstallOutcome {
        backend_label: "Task Scheduler logon task",
        unit_path: format!("Task Scheduler: \\{SCHTASKS_NAME}"),
        note: None,
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
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn systemd_service_unit_encodes_local_and_anywhere() {
        let local = render_systemd_service("/bin/forge", Exposure::Local, 1234);
        assert!(local.contains("serve --local --port 1234"));
        let anywhere = render_systemd_service("/bin/forge", Exposure::Anywhere, 1234);
        assert!(anywhere.contains("serve --anywhere --port 1234"));
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
}
