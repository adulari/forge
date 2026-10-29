//! Bounded, secret-free operational diagnostics for Forge clients.
//!
//! The endpoint deliberately reports aggregate process/runtime facts only. It never returns
//! daemon tokens, environment values, provider credentials, workspace paths, prompts, or log
//! contents, so the same projection is safe over the end-to-end encrypted Anywhere bridge.

use std::sync::Arc;

use axum::extract::State;
use sysinfo::System;

use crate::serve::DaemonState;

#[derive(serde::Serialize)]
pub(crate) struct DiagnosticsResponse {
    checked_at: i64,
    host: HostDiagnostics,
    resources: ResourceDiagnostics,
    runtime: RuntimeDiagnostics,
    checks: Vec<DiagnosticCheck>,
}

#[derive(serde::Serialize)]
struct HostDiagnostics {
    hostname: String,
    version: &'static str,
    protocol: u32,
    pid: u32,
    process_uptime_secs: u64,
    os: &'static str,
    arch: &'static str,
}

#[derive(Default, serde::Serialize)]
struct ResourceDiagnostics {
    process_memory_bytes: u64,
    process_virtual_memory_bytes: u64,
    system_total_memory_bytes: u64,
    system_available_memory_bytes: u64,
    cpu_count: usize,
    load_average_one: f64,
    load_average_five: f64,
    load_average_fifteen: f64,
}

#[derive(serde::Serialize)]
struct RuntimeDiagnostics {
    sessions: usize,
    busy_sessions: usize,
    waiting_sessions: usize,
    terminals: usize,
    terminal_clients: usize,
    web_push_ready: bool,
    native_push_ready: bool,
}

#[derive(serde::Serialize)]
struct DiagnosticCheck {
    id: &'static str,
    status: &'static str,
    label: &'static str,
    detail: String,
    fix: Option<&'static str>,
}

impl DiagnosticCheck {
    fn ok(id: &'static str, label: &'static str, detail: impl Into<String>) -> Self {
        Self {
            id,
            status: "ok",
            label,
            detail: detail.into(),
            fix: None,
        }
    }

    fn warn(
        id: &'static str,
        label: &'static str,
        detail: impl Into<String>,
        fix: &'static str,
    ) -> Self {
        Self {
            id,
            status: "warn",
            label,
            detail: detail.into(),
            fix: Some(fix),
        }
    }
}

struct HostProbe {
    resources: ResourceDiagnostics,
    process_uptime_secs: u64,
    config_ok: bool,
    git_version: Option<String>,
    shell: String,
}

impl HostProbe {
    fn unavailable() -> Self {
        Self {
            resources: ResourceDiagnostics::default(),
            process_uptime_secs: 0,
            config_ok: false,
            git_version: None,
            shell: "unavailable".to_string(),
        }
    }
}

fn bounded_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(120)
        .collect()
}

fn host_probe() -> HostProbe {
    // Only what the response reports: a full `new_all()` walks every process on the host.
    let mut system = System::new();
    system.refresh_memory();
    system.refresh_cpu_all();
    let load = System::load_average();
    let pid = sysinfo::get_current_pid().ok();
    if let Some(pid) = pid {
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    }
    let process = pid.and_then(|pid| system.process(pid));
    let resources = ResourceDiagnostics {
        process_memory_bytes: process.map_or(0, sysinfo::Process::memory),
        process_virtual_memory_bytes: process.map_or(0, sysinfo::Process::virtual_memory),
        system_total_memory_bytes: system.total_memory(),
        system_available_memory_bytes: system.available_memory(),
        cpu_count: system.cpus().len(),
        load_average_one: load.one,
        load_average_five: load.five,
        load_average_fifteen: load.fifteen,
    };
    let git_version = std::process::Command::new("git")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| bounded_line(&output.stdout))
        .filter(|line| !line.is_empty());
    let shell = std::env::var_os(if cfg!(windows) { "COMSPEC" } else { "SHELL" })
        .and_then(|value| {
            std::path::Path::new(&value)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| if cfg!(windows) { "powershell" } else { "sh" }.to_string());
    HostProbe {
        resources,
        process_uptime_secs: process.map_or(0, sysinfo::Process::run_time),
        config_ok: forge_config::load().is_ok(),
        git_version,
        shell,
    }
}

async fn database_check(state: &Arc<DaemonState>) -> DiagnosticCheck {
    let store = state.store.clone();
    match tokio::task::spawn_blocking(move || store.session_exists("diagnostics-probe")).await {
        Ok(Ok(_)) => {
            DiagnosticCheck::ok("database", "Session database", "open and answering queries")
        }
        Ok(Err(error)) => DiagnosticCheck::warn(
            "database",
            "Session database",
            format!("query failed: {error}"),
            "check disk space and permissions on the forge data directory, then restart the daemon",
        ),
        Err(_) => DiagnosticCheck::warn(
            "database",
            "Session database",
            "the database probe did not complete",
            "restart the daemon",
        ),
    }
}

/// Whether the store was recovered from damage recently, and how many pre-migration copies exist.
fn store_recovery_check() -> Option<DiagnosticCheck> {
    const NOTICE_SHOWN_SECS: i64 = 7 * 24 * 60 * 60;
    let db = crate::store_path();
    let notice = forge_store::db_safety::read_notice(&db)
        .filter(|n| chrono::Utc::now().timestamp() - n.at < NOTICE_SHOWN_SECS);
    let Some(notice) = notice else {
        let copies = forge_store::db_safety::list_backups(&db).len();
        return (copies > 0).then(|| {
            DiagnosticCheck::ok(
                "store_backups",
                "Store backups",
                format!("{copies} pre-migration copy(ies) kept next to the store"),
            )
        });
    };
    let outcome = match &notice.restored_from {
        Some(from) => format!("Restored from {from}."),
        None => "No backup could replace it, so the store started empty.".to_string(),
    };
    let kept = if notice.quarantined.is_empty() {
        String::new()
    } else {
        format!(" The previous file is kept at {}.", notice.quarantined)
    };
    let detail = format!("{}. {outcome}{kept}", notice.reason);
    Some(DiagnosticCheck::warn(
        "store_recovery",
        "Session store recovery",
        detail,
        "recent sessions may be missing; `forge backup list` shows the copies available",
    ))
}

fn power_check() -> DiagnosticCheck {
    let power = crate::power::status();
    let mode = power.mode.wire_name();
    if let Some(problem) = power.problem {
        return DiagnosticCheck::warn(
            "power",
            "Keep awake",
            format!("{mode}: could not hold a sleep inhibitor ({problem})"),
            "install systemd/logind (Linux) or set [power] keep_awake = \"off\"",
        );
    }
    let state = if power.held { "holding" } else { "idle" };
    DiagnosticCheck::ok(
        "power",
        "Keep awake",
        format!("{mode} ({state}, {})", power.backend),
    )
}

pub(crate) async fn diagnostics(
    State(state): State<Arc<DaemonState>>,
) -> axum::Json<DiagnosticsResponse> {
    // Never repeat the blocking probe on the async runtime when the worker fails.
    let probe = tokio::task::spawn_blocking(host_probe)
        .await
        .unwrap_or_else(|_| HostProbe::unavailable());
    let sessions = state.registry.all().await;
    let mut busy_sessions = 0;
    let mut waiting_sessions = 0;
    for session in &sessions {
        let snapshot = session.snapshot_rx.borrow().snapshot.clone();
        if snapshot.busy {
            busy_sessions += 1;
        }
        if snapshot.permission_prompt.is_some() || snapshot.question.is_some() {
            waiting_sessions += 1;
        }
    }
    let (terminals, terminal_clients) = state.terminals.diagnostic_counts().await;

    let mut checks = vec![
        database_check(&state).await,
        DiagnosticCheck::ok(
            "terminal",
            "Terminal runtime",
            format!("login shell: {}", probe.shell),
        ),
    ];
    checks.push(if probe.config_ok {
        DiagnosticCheck::ok("config", "Layered configuration", "loaded successfully")
    } else {
        DiagnosticCheck::warn(
            "config",
            "Layered configuration",
            "one or more configuration layers could not be loaded",
            "run `forge doctor` on the host for the failing layer and an actionable fix",
        )
    });
    checks.push(power_check());
    if let Some(check) = tokio::task::spawn_blocking(store_recovery_check)
        .await
        .ok()
        .flatten()
    {
        checks.push(check);
    }
    checks.push(match probe.git_version {
        Some(version) => DiagnosticCheck::ok("git", "Git", version),
        None => DiagnosticCheck::warn(
            "git",
            "Git",
            "git could not be executed",
            "install Git and make it available on the host PATH",
        ),
    });

    axum::Json(DiagnosticsResponse {
        checked_at: chrono::Utc::now().timestamp(),
        host: HostDiagnostics {
            hostname: crate::anywhere::default_host_name(),
            version: env!("CARGO_PKG_VERSION"),
            protocol: crate::remote::PROTOCOL_VERSION,
            pid: std::process::id(),
            process_uptime_secs: probe.process_uptime_secs,
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        },
        resources: probe.resources,
        runtime: RuntimeDiagnostics {
            sessions: sessions.len(),
            busy_sessions,
            waiting_sessions,
            terminals,
            terminal_clients,
            web_push_ready: state.push.is_some(),
            native_push_ready: state.apns.is_some(),
        },
        checks,
    })
}
