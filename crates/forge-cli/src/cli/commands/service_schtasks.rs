//! `forge service`'s Windows backend: a logon task in Task Scheduler.

use anyhow::Result;

use super::super::service_report::Activation;
use super::{
    activation_result, run_capture, run_checked, Exposure, InstallOutcome, Runner, ServiceControl,
    ServiceStatus, SCHTASKS_NAME,
};

// --- Task Scheduler (Windows) ---
//
// Not a real Windows Service (SCM): `forge serve` doesn't speak the Service Control Manager
// protocol (SERVICE_STATUS reporting, control-code handling), and teaching it to would mean
// either rewriting it as a Windows service (a large, Windows-only surface) or wrapping it with
// an SCM shim like NSSM — an external dependency we won't require. A logon scheduled task gets
// the same practical result (start automatically, run in the background, restart on failure via
// `/RI` is not supported by schtasks — restart-on-crash is systemd/launchd-only) with zero extra
// tooling.

pub(super) fn render_schtasks_create_args(
    forge_exe: &str,
    exposure: Exposure,
    port: u16,
) -> Vec<String> {
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
pub(super) fn activate_schtasks(
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

pub(super) fn install_schtasks(
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

pub(super) fn uninstall_schtasks() -> Result<()> {
    let _ = run_checked("schtasks", &["/End", "/TN", SCHTASKS_NAME], "");
    let _ = run_checked("schtasks", &["/Delete", "/TN", SCHTASKS_NAME, "/F"], "");
    Ok(())
}

pub(super) fn status_schtasks() -> Result<ServiceStatus> {
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

pub(super) fn control_schtasks(action: ServiceControl) -> Result<()> {
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
