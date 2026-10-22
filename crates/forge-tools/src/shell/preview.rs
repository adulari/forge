//! Preview URLs: `shell_job {action:"expose", id, port}` puts a running background job's port on a
//! public `https://` URL through the same cloudflared/ngrok machinery `/remote` uses.
//!
//! The tunnel is a detached process (its own session, output on `<jobs>/<id>.tunnel.log`), so it
//! survives the turn exactly like the dev server it fronts. Detached also means it survives a
//! Forge crash, and a public, unauthenticated tunnel with nothing left to stop it is the one
//! failure this feature must not have. Three things bound its life:
//!
//! - its pid lives in the job record, so stopping the job (or the job exiting) stops the tunnel;
//! - the TTL, enforced by wrapping the tunnel in coreutils `timeout` where it exists (so it holds
//!   even when Forge crashes) and by a timer in the process that started it;
//! - a registry of every live tunnel under the data dir, swept by [`reap_orphan_tunnels`] at
//!   daemon start and by `forge doctor`, which kills tunnels whose job is gone or whose TTL is up.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use super::background::{
    detach_into_new_session, get, job_dir, now_secs, pid_alive, proc_start_ticks, read_record,
    signal_group, write_record, SIGKILL, SIGTERM,
};
use crate::tunnel::{self, TunnelConfig};

/// How long a tunnel gets to print its public URL before the expose fails.
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_TTL: Duration = Duration::from_secs(2 * 60 * 60);

/// What `[preview]` config resolves to, installed once at startup by the binary.
#[derive(Debug, Clone)]
pub struct PreviewSettings {
    pub enabled: bool,
    /// `[remote]` fixed-tunnel settings, only when `[preview] use_fixed_hostname = true`.
    pub fixed: Option<TunnelConfig>,
    pub ttl: Duration,
    /// Where the cross-workspace tunnel registry lives. `None` = no registry (tests, no data dir).
    pub registry_dir: Option<PathBuf>,
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            fixed: None,
            ttl: DEFAULT_TTL,
            registry_dir: None,
        }
    }
}

static SETTINGS: std::sync::Mutex<Option<PreviewSettings>> = std::sync::Mutex::new(None);

pub fn set_preview_settings(settings: PreviewSettings) {
    *SETTINGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(settings);
}

fn settings() -> PreviewSettings {
    SETTINGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .unwrap_or_default()
}

/// The tunnel fronting a job, as stored under `"tunnel"` in the job record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelRecord {
    pub pid: u32,
    pub start_ticks: Option<u64>,
    pub port: u16,
    pub url: String,
    pub provider: String,
    pub started_at: u64,
    pub log: PathBuf,
}

impl TunnelRecord {
    pub(super) fn to_json(&self) -> Value {
        json!({
            "pid": self.pid,
            "start_ticks": self.start_ticks,
            "port": self.port,
            "url": self.url,
            "provider": self.provider,
            "started_at": self.started_at,
            "log": self.log.display().to_string(),
        })
    }

    pub(super) fn from_json(value: &Value) -> Option<Self> {
        Some(Self {
            pid: value.get("pid")?.as_u64()? as u32,
            start_ticks: value.get("start_ticks").and_then(Value::as_u64),
            port: value.get("port")?.as_u64()? as u16,
            url: value.get("url")?.as_str()?.to_string(),
            provider: value
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            started_at: value.get("started_at").and_then(Value::as_u64).unwrap_or(0),
            log: PathBuf::from(value.get("log").and_then(Value::as_str).unwrap_or_default()),
        })
    }

    /// Whether the recorded tunnel process is still the one we started (not a recycled pid).
    pub fn is_alive(&self) -> bool {
        same_process(self.pid, self.start_ticks)
    }
}

fn same_process(pid: u32, recorded_ticks: Option<u64>) -> bool {
    if !pid_alive(pid) {
        return false;
    }
    match (recorded_ticks, proc_start_ticks(pid)) {
        (Some(recorded), Some(current)) => recorded == current,
        (Some(_), None) => false,
        _ => true,
    }
}

/// Expose `port` of background job `id` and return the tool result text.
pub(super) async fn expose(
    workspace: Option<&Path>,
    cwd: &str,
    id: u32,
    port: u16,
) -> Result<String, String> {
    let settings = settings();
    if !settings.enabled {
        return Err(
            "preview URLs are disabled by config ([preview] enabled = false). Ask the user to \
             enable it if they want the dev server reachable from outside this machine."
                .to_string(),
        );
    }
    if port == 0 {
        return Err("expose needs a port between 1 and 65535".to_string());
    }
    let Some(mut job) = get(workspace, cwd, id) else {
        return Err(format!("no background job {id} in this workspace"));
    };
    if !job.is_running() {
        return Err(format!(
            "job {id} is {}; start the dev server with shell background:true first",
            job.status()
        ));
    }
    let dir = job_dir(workspace, cwd, id);
    if let Some(existing) = job.tunnel.take() {
        if existing.is_alive() && existing.port == port {
            return Ok(exposed_text(id, &existing, settings.ttl, true));
        }
        stop_tunnel_blocking(dir.clone(), settings.clone(), id, existing).await;
    }

    let config = settings.fixed.clone().unwrap_or_default();
    let kind = if settings.fixed.is_some() {
        tunnel::resolve_tunnel_kind(&config).map_err(|e| e.to_string())?
    } else {
        tunnel::detect_tunnel().ok_or_else(|| {
            "no tunnel tool found on PATH — install cloudflared (free, no account) or ngrok"
                .to_string()
        })?
    };
    let record = spawn_detached(kind, port, &config, &dir, id, settings.ttl).await?;

    // The tunnel took seconds to come up: re-read the record so an exit recorded meanwhile is
    // neither overwritten nor left with a public tunnel in front of nothing.
    let job = match get(workspace, cwd, id) {
        Some(mut fresh) if fresh.is_running() => {
            fresh.tunnel = Some(record.clone());
            fresh
        }
        _ => {
            tokio::task::spawn_blocking(move || kill_tunnel(record.pid, record.start_ticks))
                .await
                .ok();
            return Err(format!(
                "job {id} exited while its tunnel was starting; the tunnel was closed"
            ));
        }
    };
    write_record(&dir, &job)?;
    if let Some(registry) = &settings.registry_dir {
        register(registry, &dir.join(format!("{id}.json")), id, &record);
    }
    schedule_ttl(dir, settings.clone(), id, record.clone());
    super::preview_links::record_evidence(id, &record);
    Ok(exposed_text(id, &record, settings.ttl, false))
}

fn exposed_text(id: u32, record: &TunnelRecord, ttl: Duration, reused: bool) -> String {
    format!(
        "preview: job {id} port {} is {} at {}\n  WARNING: this URL is public and \
         unauthenticated — anyone who has it reaches the dev server. Do not expose anything \
         holding real credentials or data.\n  tunnel pid {} ({}), log {}. It stops with the \
         job, on shell_job stop, or after {} min.",
        record.port,
        if reused { "already public" } else { "public" },
        record.url,
        record.pid,
        record.provider,
        record.log.display(),
        ttl.as_secs() / 60
    )
}

async fn spawn_detached(
    kind: tunnel::TunnelKind,
    port: u16,
    config: &TunnelConfig,
    dir: &Path,
    id: u32,
    ttl: Duration,
) -> Result<TunnelRecord, String> {
    let bin =
        tunnel::which(kind.binary()).ok_or_else(|| format!("{} not on PATH", kind.binary()))?;
    spawn_bin(bin, kind, port, config, dir, id, Some(ttl)).await
}

/// `timeout -s TERM <secs> <bin> <argv…>` when coreutils/busybox `timeout` is on PATH: the TTL
/// then holds even if Forge dies before its timer fires. Unix only, since Windows' `timeout` is a
/// sleep command.
fn with_ttl_guard(
    bin: PathBuf,
    argv: Vec<String>,
    ttl: Option<Duration>,
) -> (PathBuf, Vec<String>) {
    let guard = ttl
        .filter(|_| cfg!(unix))
        .and_then(|ttl| tunnel::which("timeout").map(|timeout| (timeout, ttl)));
    match guard {
        Some((timeout, ttl)) => {
            let mut wrapped = vec![
                "-s".to_string(),
                "TERM".to_string(),
                ttl.as_secs().max(1).to_string(),
                bin.display().to_string(),
            ];
            wrapped.extend(argv);
            (timeout, wrapped)
        }
        None => (bin, argv),
    }
}

async fn spawn_bin(
    bin: PathBuf,
    kind: tunnel::TunnelKind,
    port: u16,
    config: &TunnelConfig,
    dir: &Path,
    id: u32,
    ttl: Option<Duration>,
) -> Result<TunnelRecord, String> {
    let (argv, readiness) = tunnel::tunnel_plan(kind, port, config).map_err(|e| e.to_string())?;
    let (program, argv) = with_ttl_guard(bin, argv, ttl);
    let log = dir.join(format!("{id}.tunnel.log"));
    let file = std::fs::File::create(&log).map_err(|e| format!("open {}: {e}", log.display()))?;
    let file_err = file
        .try_clone()
        .map_err(|e| format!("clone log handle: {e}"))?;

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(argv)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::from(file_err))
        .kill_on_drop(false);
    detach_into_new_session(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to start {}: {e}", kind.binary()))?;
    let Some(pid) = child.id() else {
        return Err(format!(
            "{} exited before its pid could be read",
            kind.binary()
        ));
    };
    let start_ticks = proc_start_ticks(pid);

    let deadline = std::time::Instant::now() + READY_TIMEOUT;
    let url = loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if let Some(url) = text
            .lines()
            .find_map(|line| readiness.ready_url(kind, line))
        {
            break url;
        }
        let exited = child.try_wait().ok().flatten();
        if exited.is_some() || std::time::Instant::now() >= deadline {
            signal_group(pid, SIGKILL);
            let _ = child.wait().await;
            let tail = super::background::tail(&log, 12).unwrap_or_default();
            let why = match exited {
                Some(status) => format!("exited ({status}) before it was ready"),
                None => format!("was not ready within {}s", READY_TIMEOUT.as_secs()),
            };
            return Err(format!(
                "{} {why}\n--- last output ---\n{tail}",
                kind.binary()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    // Reap it when it ends so a stopped tunnel never lingers as a zombie that reads as alive.
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(TunnelRecord {
        pid,
        start_ticks,
        port,
        url,
        provider: kind.label().to_string(),
        started_at: now_secs(),
        log,
    })
}

fn schedule_ttl(dir: PathBuf, settings: PreviewSettings, id: u32, record: TunnelRecord) {
    tokio::spawn(async move {
        tokio::time::sleep(settings.ttl).await;
        let current = read_record(&dir.join(format!("{id}.json"))).and_then(|job| job.tunnel);
        if current.as_ref() == Some(&record) {
            stop_tunnel_blocking(dir, settings, id, record).await;
        }
    });
}

async fn stop_tunnel_blocking(
    dir: PathBuf,
    settings: PreviewSettings,
    id: u32,
    record: TunnelRecord,
) {
    tokio::task::spawn_blocking(move || stop_tunnel(&dir, &settings, id, &record))
        .await
        .ok();
}

/// Stop the tunnel fronting job `id` (if it has one) and clear it from the record. Called when the
/// job is stopped or exits on its own.
pub(super) fn stop_job_tunnel(dir: &Path, id: u32) {
    let Some(job) = read_record(&dir.join(format!("{id}.json"))) else {
        return;
    };
    if let Some(record) = job.tunnel {
        stop_tunnel(dir, &settings(), id, &record);
    }
}

fn stop_tunnel(dir: &Path, settings: &PreviewSettings, id: u32, record: &TunnelRecord) {
    kill_tunnel(record.pid, record.start_ticks);
    clear_from_job(&dir.join(format!("{id}.json")), record.pid);
    if let Some(registry) = &settings.registry_dir {
        let _ = std::fs::remove_file(registry.join(format!("{}.json", record.pid)));
    }
}

/// SIGTERM the tunnel's group, then SIGKILL if it is still the same process after a short grace.
fn kill_tunnel(pid: u32, ticks: Option<u64>) {
    if !same_process(pid, ticks) {
        return;
    }
    signal_group(pid, SIGTERM);
    for _ in 0..20 {
        if !same_process(pid, ticks) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if same_process(pid, ticks) {
        signal_group(pid, SIGKILL);
    }
}

fn clear_from_job(record_path: &Path, tunnel_pid: u32) {
    let Some(dir) = record_path.parent() else {
        return;
    };
    let Some(mut job) = read_record(record_path) else {
        return;
    };
    if job.tunnel.as_ref().is_some_and(|t| t.pid == tunnel_pid) {
        job.tunnel = None;
        let _ = write_record(dir, &job);
    }
}

fn register(registry: &Path, job_record: &Path, job_pid: u32, record: &TunnelRecord) {
    if std::fs::create_dir_all(registry).is_err() {
        return;
    }
    let entry = json!({
        "tunnel": record.to_json(),
        "job_pid": job_pid,
        "job_record": job_record.display().to_string(),
    });
    let _ = std::fs::write(
        registry.join(format!("{}.json", record.pid)),
        format!("{entry:#}\n"),
    );
}

/// Why the reaper killed a tunnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReapReason {
    /// The job it fronted is gone (stopped, exited, or its record deleted).
    Orphaned,
    /// It outlived the TTL.
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReapedTunnel {
    pub pid: u32,
    pub url: String,
    pub reason: ReapReason,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReapReport {
    pub reaped: Vec<ReapedTunnel>,
    /// Live tunnels left alone: their job is running and they are inside the TTL.
    pub active: Vec<String>,
}

/// Sweep the tunnel registry: kill every tunnel whose job is no longer running or whose age
/// exceeds `ttl`, and drop entries for tunnels that already died. Never touches a pid whose
/// start time no longer matches the recorded one.
pub fn reap_orphan_tunnels(registry: &Path, ttl: Duration) -> ReapReport {
    reap_at(registry, ttl, now_secs())
}

fn reap_at(registry: &Path, ttl: Duration, now: u64) -> ReapReport {
    let mut report = ReapReport::default();
    let Ok(entries) = std::fs::read_dir(registry) else {
        return report;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some((record, job_record)) = read_entry(&path) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if !record.is_alive() {
            clear_from_job(&job_record, record.pid);
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let job_running = read_record(&job_record)
            .is_some_and(|job| job.is_running() && job.tunnel.is_some_and(|t| t.pid == record.pid));
        let reason = if !job_running {
            Some(ReapReason::Orphaned)
        } else if now.saturating_sub(record.started_at) >= ttl.as_secs() {
            Some(ReapReason::Expired)
        } else {
            None
        };
        match reason {
            Some(reason) => {
                kill_tunnel(record.pid, record.start_ticks);
                clear_from_job(&job_record, record.pid);
                let _ = std::fs::remove_file(&path);
                report.reaped.push(ReapedTunnel {
                    pid: record.pid,
                    url: record.url,
                    reason,
                });
            }
            None => report.active.push(record.url),
        }
    }
    report
}

fn read_entry(path: &Path) -> Option<(TunnelRecord, PathBuf)> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let record = TunnelRecord::from_json(value.get("tunnel")?)?;
    let job_record = PathBuf::from(value.get("job_record")?.as_str()?);
    Some((record, job_record))
}

#[cfg(all(test, unix))]
mod tests;
