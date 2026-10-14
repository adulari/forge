//! Binary-side wiring for preview URLs (`shell_job` expose): installs `[preview]` into the tool
//! crate and runs the orphan-tunnel reaper at daemon start and from `forge doctor`.

use std::path::PathBuf;
use std::time::Duration;

use crate::doctor::{check, Check, Status};

/// Cross-workspace registry of live preview tunnels: one `<tunnel pid>.json` each.
fn registry_dir() -> Option<PathBuf> {
    forge_config::data_dir().map(|dir| dir.join("preview-tunnels"))
}

fn ttl(config: &forge_config::Config) -> Duration {
    Duration::from_secs(config.preview.ttl_minutes.max(1) * 60)
}

pub(crate) fn install(config: &forge_config::Config) {
    let fixed = config
        .preview
        .use_fixed_hostname
        .then(|| crate::remote::tunnel_config(&config.remote))
        .filter(|cfg| cfg.tunnel_name.is_some() || cfg.tunnel_hostname.is_some());
    forge_tools::set_preview_settings(forge_tools::PreviewSettings {
        enabled: config.preview.enabled,
        fixed,
        ttl: ttl(config),
        registry_dir: registry_dir(),
    });
}

fn reap(config: &forge_config::Config) -> Option<forge_tools::ReapReport> {
    let registry = registry_dir()?;
    Some(forge_tools::reap_orphan_tunnels(&registry, ttl(config)))
}

/// A tunnel left behind by a crashed Forge is a public door to a dev server; close it before the
/// daemon starts serving.
pub(crate) fn reap_at_daemon_start(config: &forge_config::Config) {
    let Some(report) = reap(config) else {
        return;
    };
    for tunnel in &report.reaped {
        println!(
            "⚒ closed preview tunnel {} ({}, {})",
            tunnel.url,
            tunnel.pid,
            reason_label(&tunnel.reason)
        );
    }
}

pub(crate) fn doctor_check(config: &forge_config::Config) -> Check {
    let Some(report) = reap(config) else {
        return check(Status::Info, "Preview tunnels", "no data directory", None);
    };
    let enabled = if config.preview.enabled {
        "enabled (asks before each expose)"
    } else {
        "disabled ([preview] enabled = false)"
    };
    let mut detail = format!(
        "{enabled}; {} live, TTL {} min",
        report.active.len(),
        config.preview.ttl_minutes
    );
    if !report.reaped.is_empty() {
        detail.push_str(&format!(
            "; closed {} orphaned/expired: {}",
            report.reaped.len(),
            report
                .reaped
                .iter()
                .map(|t| format!("{} ({})", t.url, reason_label(&t.reason)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let status = if report.active.is_empty() {
        Status::Ok
    } else {
        Status::Info
    };
    check(status, "Preview tunnels", detail, None)
}

fn reason_label(reason: &forge_tools::ReapReason) -> &'static str {
    match reason {
        forge_tools::ReapReason::Orphaned => "its job is gone",
        forge_tools::ReapReason::Expired => "past its TTL",
    }
}
