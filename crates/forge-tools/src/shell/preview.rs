//! Preview URLs: `shell_job {action:"expose", id, port}` puts a running background job's port on a
//! public `https://` URL through the same cloudflared/ngrok machinery `/remote` uses.
//!
//! The tunnel is a detached process (its own session, output on `<jobs>/<id>.tunnel.log`), so it
//! survives the turn exactly like the dev server it fronts. Detached also means it survives a
//! Forge crash, and a public, unauthenticated tunnel with nothing left to stop it is the one
//! failure this feature must not have. Three things bound its life:
//!
//! - its pid lives in the job record, so stopping the job (or the job exiting) stops the tunnel;
//! - a TTL timer in the process that started it;
//! - a registry of every live tunnel under the data dir, swept by [`reap_orphan_tunnels`] at
//!   daemon start and by `forge doctor`, which kills tunnels whose job is gone or whose TTL is up.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use super::background::{
    detach_into_new_session, get, jobs_dir, now_secs, pid_alive, proc_start_ticks, read_record,
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
    let dir = jobs_dir(workspace, cwd);
    if let Some(existing) = job.tunnel.clone() {
        if existing.is_alive() && existing.port == port {
            return Ok(exposed_text(id, &existing, settings.ttl, true));
        }
        stop_tunnel(&dir, &settings, id, &existing);
        job.tunnel = None;
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
    let record = spawn_detached(kind, port, &config, &dir, id).await?;

    job.tunnel = Some(record.clone());
    write_record(&dir, &job)?;
    if let Some(registry) = &settings.registry_dir {
        register(registry, &dir.join(format!("{id}.json")), id, &record);
    }
    schedule_ttl(dir, settings.clone(), id, record.clone());
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
) -> Result<TunnelRecord, String> {
    let bin =
        tunnel::which(kind.binary()).ok_or_else(|| format!("{} not on PATH", kind.binary()))?;
    spawn_bin(bin, kind, port, config, dir, id).await
}

async fn spawn_bin(
    bin: PathBuf,
    kind: tunnel::TunnelKind,
    port: u16,
    config: &TunnelConfig,
    dir: &Path,
    id: u32,
) -> Result<TunnelRecord, String> {
    let (argv, readiness) = tunnel::tunnel_plan(kind, port, config).map_err(|e| e.to_string())?;
    let log = dir.join(format!("{id}.tunnel.log"));
    let file = std::fs::File::create(&log).map_err(|e| format!("open {}: {e}", log.display()))?;
    let file_err = file
        .try_clone()
        .map_err(|e| format!("clone log handle: {e}"))?;

    let mut cmd = tokio::process::Command::new(bin);
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
            stop_tunnel(&dir, &settings, id, &record);
        }
    });
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
mod tests {
    use super::super::background::{start, stop};
    use super::*;
    use crate::sandbox::SandboxPolicy;

    /// A stand-in tunnel: a detached `sleep` registered exactly the way `expose` registers one.
    async fn fake_tunnel(
        registry: &Path,
        dir: &Path,
        job_pid: u32,
        started_at: u64,
    ) -> TunnelRecord {
        let mut cmd = tokio::process::Command::new("sleep");
        cmd.arg("60").kill_on_drop(false);
        detach_into_new_session(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let pid = child.id().unwrap();
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
        let record = TunnelRecord {
            pid,
            start_ticks: proc_start_ticks(pid),
            port: 5173,
            url: format!("https://fake-{pid}.trycloudflare.com"),
            provider: "fake".into(),
            started_at,
            log: dir.join(format!("{job_pid}.tunnel.log")),
        };
        let record_path = dir.join(format!("{job_pid}.json"));
        let mut job = super::read_record(&record_path).unwrap();
        job.tunnel = Some(record.clone());
        write_record(dir, &job).unwrap();
        register(registry, &record_path, job_pid, &record);
        record
    }

    async fn exited_pid() -> u32 {
        let mut child = tokio::process::Command::new("true").spawn().unwrap();
        let pid = child.id().unwrap();
        child.wait().await.unwrap();
        pid
    }

    #[tokio::test]
    async fn a_job_that_exits_on_its_own_takes_its_tunnel_with_it() {
        let ws = tempfile::tempdir().unwrap();
        let registry = tempfile::tempdir().unwrap();
        let cwd = ws.path().display().to_string();
        let dir = jobs_dir(Some(ws.path()), &cwd);
        let job = start("sleep 60", &cwd, Some(ws.path()), &SandboxPolicy::default())
            .await
            .unwrap();
        let tunnel = fake_tunnel(registry.path(), &dir, job.pid, now_secs()).await;
        signal_group(job.pid, SIGKILL);
        assert!(
            wait_dead(&tunnel).await,
            "the job's exit must stop its tunnel"
        );
        for _ in 0..60 {
            if get(Some(ws.path()), &cwd, job.pid)
                .unwrap()
                .tunnel
                .is_none()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the stopped tunnel was never cleared from the job record");
    }

    async fn wait_dead(record: &TunnelRecord) -> bool {
        for _ in 0..60 {
            if !record.is_alive() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    #[tokio::test]
    async fn the_reaper_kills_a_tunnel_whose_job_is_gone_and_spares_a_live_one() {
        let ws = tempfile::tempdir().unwrap();
        let registry = tempfile::tempdir().unwrap();
        let cwd = ws.path().display().to_string();
        let dir = jobs_dir(Some(ws.path()), &cwd);
        let policy = SandboxPolicy::default();
        let live = start("sleep 60", &cwd, Some(ws.path()), &policy)
            .await
            .unwrap();
        // A job a crashed Forge left behind: its process is gone and nobody was alive to stop
        // the tunnel in front of it.
        let dead_pid = exited_pid().await;
        std::fs::write(
            dir.join(format!("{dead_pid}.json")),
            json!({"pid": dead_pid, "command": "npm run dev", "log": "/dev/null", "started_at": 1})
                .to_string(),
        )
        .unwrap();
        let dead = get(Some(ws.path()), &cwd, dead_pid).unwrap();
        let live_tunnel = fake_tunnel(registry.path(), &dir, live.pid, now_secs()).await;
        let orphan = fake_tunnel(registry.path(), &dir, dead.pid, now_secs()).await;

        let report = reap_orphan_tunnels(registry.path(), DEFAULT_TTL);

        assert_eq!(
            report.reaped,
            vec![ReapedTunnel {
                pid: orphan.pid,
                url: orphan.url.clone(),
                reason: ReapReason::Orphaned
            }]
        );
        assert_eq!(report.active, vec![live_tunnel.url.clone()]);
        assert!(
            wait_dead(&orphan).await,
            "the orphaned tunnel must be killed"
        );
        assert!(
            live_tunnel.is_alive(),
            "a tunnel whose job runs is left alone"
        );
        assert!(!registry
            .path()
            .join(format!("{}.json", orphan.pid))
            .exists());
        assert!(get(Some(ws.path()), &cwd, dead.pid)
            .unwrap()
            .tunnel
            .is_none());

        stop(Some(ws.path()), &cwd, live.pid).await.unwrap();
        assert!(
            wait_dead(&live_tunnel).await,
            "stopping the job stops its tunnel"
        );
    }

    #[tokio::test]
    async fn the_reaper_kills_a_tunnel_past_its_ttl_even_while_the_job_runs() {
        let ws = tempfile::tempdir().unwrap();
        let registry = tempfile::tempdir().unwrap();
        let cwd = ws.path().display().to_string();
        let dir = jobs_dir(Some(ws.path()), &cwd);
        let job = start("sleep 60", &cwd, Some(ws.path()), &SandboxPolicy::default())
            .await
            .unwrap();
        let three_hours_ago = now_secs() - 3 * 60 * 60;
        let old = fake_tunnel(registry.path(), &dir, job.pid, three_hours_ago).await;

        let report = reap_orphan_tunnels(registry.path(), DEFAULT_TTL);

        assert_eq!(report.reaped.len(), 1);
        assert_eq!(report.reaped[0].reason, ReapReason::Expired);
        assert!(wait_dead(&old).await);
        assert!(get(Some(ws.path()), &cwd, job.pid).unwrap().is_running());
        stop(Some(ws.path()), &cwd, job.pid).await.unwrap();
    }

    #[test]
    fn the_reaper_never_kills_a_recycled_pid_and_drops_dead_entries() {
        let ws = tempfile::tempdir().unwrap();
        let registry = tempfile::tempdir().unwrap();
        let ours = std::process::id();
        let record = TunnelRecord {
            pid: ours,
            // A start time that is not ours: the pid was "recycled" into this test process.
            start_ticks: proc_start_ticks(ours).map(|t| t + 1).or(Some(1)),
            port: 3000,
            url: "https://stale.trycloudflare.com".into(),
            provider: "fake".into(),
            started_at: 0,
            log: ws.path().join("x.log"),
        };
        register(registry.path(), &ws.path().join("1.json"), 1, &record);
        std::fs::write(registry.path().join("garbage.json"), "not json").unwrap();

        let report = reap_at(registry.path(), DEFAULT_TTL, now_secs());

        assert!(report.reaped.is_empty() && report.active.is_empty());
        assert_eq!(std::fs::read_dir(registry.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_missing_registry_is_an_empty_sweep() {
        let report = reap_orphan_tunnels(
            Path::new("/nonexistent/forge-preview-registry"),
            DEFAULT_TTL,
        );
        assert_eq!(report, ReapReport::default());
    }

    #[tokio::test]
    async fn expose_refuses_when_the_kill_switch_is_off_or_the_job_is_not_running() {
        let ws = tempfile::tempdir().unwrap();
        let cwd = ws.path().display().to_string();
        set_preview_settings(PreviewSettings {
            enabled: false,
            ..PreviewSettings::default()
        });
        let off = expose(Some(ws.path()), &cwd, 1, 3000).await.unwrap_err();
        set_preview_settings(PreviewSettings::default());
        assert!(off.contains("[preview] enabled = false"), "{off}");

        let missing = expose(Some(ws.path()), &cwd, 999_999, 3000)
            .await
            .unwrap_err();
        assert!(missing.contains("no background job"), "{missing}");
        let job = start("exit 0", &cwd, Some(ws.path()), &SandboxPolicy::default())
            .await
            .unwrap();
        for _ in 0..60 {
            if !get(Some(ws.path()), &cwd, job.pid).unwrap().is_running() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let exited = expose(Some(ws.path()), &cwd, job.pid, 3000)
            .await
            .unwrap_err();
        assert!(exited.contains("start the dev server"), "{exited}");
    }

    fn fake_binary(dir: &Path, name: &str, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn a_detached_tunnel_is_ready_once_its_log_shows_the_url_and_outlives_the_call() {
        let ws = tempfile::tempdir().unwrap();
        let fake = fake_binary(
            ws.path(),
            "fake-cloudflared",
            "#!/bin/sh\nsleep 0.2\necho 'INF |  https://quiet-fox.trycloudflare.com  |' >&2\nexec sleep 60\n",
        );

        let record = spawn_bin(
            fake,
            tunnel::TunnelKind::Cloudflared,
            5173,
            &TunnelConfig::default(),
            ws.path(),
            7,
        )
        .await
        .unwrap();

        assert_eq!(record.url, "https://quiet-fox.trycloudflare.com");
        assert_eq!(record.port, 5173);
        assert!(record.is_alive());
        let ours = unsafe { libc::getpgid(0) };
        assert_ne!(
            unsafe { libc::getpgid(record.pid as i32) },
            ours,
            "the tunnel must sit in its own session so a turn's group kill cannot reach it"
        );
        kill_tunnel(record.pid, record.start_ticks);
        assert!(wait_dead(&record).await);
    }

    #[tokio::test]
    async fn a_tunnel_that_dies_before_printing_a_url_fails_with_its_output() {
        let ws = tempfile::tempdir().unwrap();
        let fake = fake_binary(
            ws.path(),
            "fake-ngrok",
            "#!/bin/sh\necho 'ERR authtoken missing'\nexit 1\n",
        );
        let err = spawn_bin(
            fake,
            tunnel::TunnelKind::Ngrok,
            3000,
            &TunnelConfig::default(),
            ws.path(),
            8,
        )
        .await
        .unwrap_err();
        assert!(err.contains("authtoken missing"), "{err}");
    }

    #[test]
    fn a_tunnel_record_round_trips_through_the_job_json() {
        let record = TunnelRecord {
            pid: 42,
            start_ticks: Some(7),
            port: 5173,
            url: "https://a.trycloudflare.com".into(),
            provider: "cloudflared".into(),
            started_at: 100,
            log: PathBuf::from("/tmp/42.tunnel.log"),
        };
        assert_eq!(TunnelRecord::from_json(&record.to_json()), Some(record));
    }
}
