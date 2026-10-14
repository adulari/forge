use super::super::background::{start, stop};
use super::*;
use crate::sandbox::SandboxPolicy;

/// A stand-in tunnel: a detached `sleep` registered exactly the way `expose` registers one.
async fn fake_tunnel(registry: &Path, dir: &Path, job_pid: u32, started_at: u64) -> TunnelRecord {
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
        None,
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
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("authtoken missing"), "{err}");
}

#[tokio::test]
async fn the_ttl_guard_closes_the_tunnel_without_forge() {
    if tunnel::which("timeout").is_none() {
        return;
    }
    let ws = tempfile::tempdir().unwrap();
    let fake = fake_binary(
        ws.path(),
        "fake-cloudflared",
        "#!/bin/sh\necho 'INF |  https://brief-owl.trycloudflare.com  |' >&2\nexec sleep 60\n",
    );
    let record = spawn_bin(
        fake,
        tunnel::TunnelKind::Cloudflared,
        5173,
        &TunnelConfig::default(),
        ws.path(),
        9,
        Some(Duration::from_secs(1)),
    )
    .await
    .unwrap();
    assert!(record.is_alive());
    for _ in 0..100 {
        if !record.is_alive() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    kill_tunnel(record.pid, record.start_ticks);
    panic!("the TTL guard did not stop the tunnel");
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
