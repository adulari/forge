//! Data and side effects behind the TUI `/anywhere` menu. `forge-tui` owns the screen and key
//! handling; everything that reads the machine or the service lives here.
//!
//! Status comes from the same builder `forge anywhere sync` prints (labels and counts, never a
//! value). The enrolled devices need the service and are fetched on a slower beat, so a missing
//! network never blanks the sync section.

use std::time::Duration;

use forge_tui::anywhere_menu::{
    AnywhereSnapshot, CategoryView, Connector, DeviceView, Devices, MissingToolView, SignInView,
};
use serde::Deserialize;

use super::account_sync::{self, AccountSyncStatus};
use super::{
    client, configured_host_name, current_link_health, ensure_access_token, human_duration, now_ms,
    send_json, DeviceList, LinkHealth, StateStore,
};

/// A host is online when it has been heard from within a few relay heartbeats (20 s apart).
const HOST_ONLINE_WITHIN_MS: i64 = 90_000;
const SERVICE_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Deserialize)]
struct HostList {
    hosts: Vec<HostRow>,
}

#[derive(Deserialize)]
struct HostRow {
    #[serde(default)]
    revoked_at: Option<String>,
    #[serde(default)]
    last_seen_at: Option<String>,
}

fn epoch_ms(raw: &str) -> Option<i64> {
    let value: i64 = raw.trim().parse().ok()?;
    Some(if value > 100_000_000_000 {
        value
    } else {
        value * 1000
    })
}

fn date(raw: &str) -> String {
    epoch_ms(raw)
        .and_then(|ms| chrono::DateTime::from_timestamp(ms / 1000, 0))
        .map_or_else(|| raw.to_string(), |t| t.format("%Y-%m-%d").to_string())
}

pub(crate) fn view_of(
    status: &AccountSyncStatus,
    signed_in_as: Option<String>,
    connector: (Connector, String),
    device_label: String,
    sync_enabled: bool,
    worker: Option<Option<String>>,
) -> AnywhereSnapshot {
    AnywhereSnapshot {
        signed_in: status.signed_in,
        login: signed_in_as,
        connector: connector.0,
        connector_text: connector.1,
        device_label,
        sync_enabled,
        worker_running: worker.is_some(),
        last_sync_at: status.last_sync_at,
        last_error: worker.flatten().or_else(|| status.last_error.clone()),
        categories: status
            .categories
            .iter()
            .map(|c| CategoryView {
                id: c.id.into(),
                label: c.label.into(),
                enabled: c.enabled,
                items: c.items,
                pending: c.pending,
                last_sync_at: c.last_sync_at,
                remote_only: c.remote_only.unwrap_or(0),
                conflicts: c
                    .conflicts
                    .iter()
                    .map(|x| (x.item.clone(), x.detail.clone()))
                    .collect(),
                skipped: c
                    .skipped
                    .iter()
                    .map(|x| (x.item.clone(), x.reason.clone()))
                    .collect(),
                requires_direct: c.toggle_requires_direct,
            })
            .collect(),
        sign_ins: status
            .sign_ins
            .iter()
            .map(|s| SignInView {
                provider: s.provider.clone(),
                label: s.label.clone(),
                command: s.command.clone(),
            })
            .collect(),
        missing_tools: status
            .missing_tools
            .iter()
            .map(|t| MissingToolView {
                label: t.label.clone(),
                hint: t.hint.clone(),
            })
            .collect(),
        taken_at_ms: now_ms() as i64,
    }
}

/// The daemon's sync worker answers `GET /api/anywhere/sync`; `Some(last_error)` means it is up.
async fn probe_worker() -> Option<Option<String>> {
    let port = forge_config::load().ok()?.remote.serve_port();
    let token = crate::serve::read_daemon_token().ok()?;
    let response = reqwest::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build()
        .ok()?
        .get(format!("http://127.0.0.1:{port}/{token}/api/anywhere/sync"))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: serde_json::Value = response.json().await.ok()?;
    Some(
        body.get("last_error")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    )
}

pub(crate) async fn load_snapshot(mock: bool) -> Result<AnywhereSnapshot, String> {
    if mock {
        return Ok(mock_snapshot());
    }
    let status =
        tokio::task::spawn_blocking(|| account_sync::status_snapshot(&crate::open_store()?))
            .await
            .map_err(|_| "status task failed".to_string())?
            .map_err(|e| format!("{e:#}"))?;
    let config = forge_config::load().map_err(|e| e.to_string())?;
    let state = StateStore::platform()
        .and_then(|s| s.load())
        .map_err(|e| format!("{e:#}"))?;
    let connector = if !config.anywhere.enabled {
        (Connector::Off, "not enabled on this machine".to_string())
    } else {
        match current_link_health().unwrap_or(LinkHealth::Unknown) {
            LinkHealth::Healthy { age } => (
                Connector::Live,
                format!("live · relay {} ago", human_duration(age)),
            ),
            LinkHealth::Stale { age } => (
                Connector::Unhealthy,
                format!("stale · relay {} ago", human_duration(age)),
            ),
            LinkHealth::Disconnected => (Connector::Unhealthy, "relay disconnected".to_string()),
            LinkHealth::Unknown => (Connector::Unhealthy, "no relay exchange yet".to_string()),
        }
    };
    let device_label = match state.device_id.as_deref() {
        Some(id) => format!("{} · {}", &id[..id.len().min(8)], configured_host_name()),
        None => configured_host_name(),
    };
    let worker = probe_worker().await;
    Ok(view_of(
        &status,
        state.github_login.clone().filter(|_| status.signed_in),
        connector,
        device_label,
        config.anywhere.sync,
        worker,
    ))
}

async fn fetch_devices() -> anyhow::Result<Devices> {
    let store = StateStore::platform()?;
    let mut state = store.load()?;
    if !state.is_logged_in() {
        anyhow::bail!("not signed in");
    }
    let token = ensure_access_token(&store, &mut state).await?;
    let service_url = forge_config::load()?.anywhere.service_url().to_owned();
    let devices: DeviceList = send_json(
        client()?
            .get(format!("{service_url}/v1/devices"))
            .bearer_auth(&token),
    )
    .await?;
    let hosts: HostList = send_json(
        client()?
            .get(format!("{service_url}/v1/hosts"))
            .bearer_auth(&token),
    )
    .await?;
    let now = now_ms() as i64;
    let live: Vec<&HostRow> = hosts
        .hosts
        .iter()
        .filter(|h| h.revoked_at.is_none())
        .collect();
    let online = live
        .iter()
        .filter(|h| {
            h.last_seen_at
                .as_deref()
                .and_then(epoch_ms)
                .is_some_and(|seen| now - seen <= HOST_ONLINE_WITHIN_MS)
        })
        .count();
    let this = state.device_id.clone();
    Ok(Devices::Loaded {
        hosts_online: online,
        hosts_total: live.len(),
        devices: devices
            .devices
            .into_iter()
            .map(|d| DeviceView {
                this_device: this.as_deref() == Some(d.id.as_str()),
                last_seen_at: d.last_seen_at.as_deref().and_then(epoch_ms),
                enrolled: date(&d.created_at),
                id: d.id,
                name: d.name,
            })
            .collect(),
    })
}

pub(crate) async fn load_devices(mock: bool) -> Devices {
    if mock {
        return mock_devices();
    }
    match tokio::time::timeout(SERVICE_TIMEOUT, fetch_devices()).await {
        Ok(Ok(devices)) => devices,
        Ok(Err(error)) => Devices::Unavailable(format!("{error:#}")),
        Err(_) => Devices::Unavailable("the service did not answer in time".into()),
    }
}

/// Switch one category on this device and nudge a running daemon to act on it.
pub(crate) async fn set_category(mock: bool, category: &str, enabled: bool) -> Result<(), String> {
    if mock {
        MOCK_FLAGS
            .lock()
            .map_err(|_| "mock state poisoned".to_string())?
            .insert(category.to_string(), enabled);
        return Ok(());
    }
    if enabled && !super::signed_in() {
        return Err(account_sync::not_signed_in_message().to_string());
    }
    let category = category.to_string();
    tokio::task::spawn_blocking(move || {
        let _lock = crate::serve::config_mutation_lock()
            .lock()
            .map_err(|_| "config lock poisoned".to_string())?;
        forge_config::account_sync::set_account_sync_flag(&category, enabled)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "could not update sync settings".to_string())??;
    let _ = sync_now(false).await;
    Ok(())
}

/// Ask the running daemon for a cycle. Errors say what to do about it.
pub(crate) async fn sync_now(mock: bool) -> Result<(), String> {
    if mock {
        return Ok(());
    }
    let port = forge_config::load()
        .map_err(|e| e.to_string())?
        .remote
        .serve_port();
    let token = crate::serve::read_daemon_token()
        .map_err(|_| "No sync worker is running. Start `forge serve`.".to_string())?;
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!(
            "http://127.0.0.1:{port}/{token}/api/anywhere/sync/now"
        ))
        .send()
        .await
        .map_err(|_| "No sync worker is running. Start `forge serve`.".to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "The sync worker refused (HTTP {})",
            response.status()
        ))
    }
}

static MOCK_FLAGS: std::sync::Mutex<std::collections::BTreeMap<String, bool>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// A signed-in account with a mix of states, for `--mock` runs and screenshots. It never
/// touches the real account or the service.
fn mock_snapshot() -> AnywhereSnapshot {
    let now = now_ms() as i64;
    let flags = MOCK_FLAGS.lock().map(|f| f.clone()).unwrap_or_default();
    let on = |id: &str, default: bool| flags.get(id).copied().unwrap_or(default);
    let cat = |id: &str, label: &str, enabled: bool, items: usize| CategoryView {
        id: id.into(),
        label: label.into(),
        enabled,
        items: if enabled { items } else { 0 },
        last_sync_at: enabled.then_some(now - 130_000),
        requires_direct: id == "keys",
        ..CategoryView::default()
    };
    let mut categories = vec![
        CategoryView {
            remote_only: 3,
            ..cat("keys", "Keys & tokens", on("keys", false), 12)
        },
        CategoryView {
            pending: 1,
            ..cat("skills", "Skills & agents", on("skills", true), 6)
        },
        CategoryView {
            conflicts: vec![(
                "setting: tui.theme".into(),
                "changed here and on another device; kept the other device's".into(),
            )],
            ..cat("settings", "Settings", on("settings", true), 9)
        },
        CategoryView {
            remote_only: 1,
            skipped: vec![(
                "mcp: github".into(),
                "needs `npx`, not installed here".into(),
            )],
            ..cat("mcp", "MCP servers", on("mcp", true), 2)
        },
        cat("instructions", "Instructions", on("instructions", false), 1),
    ];
    for c in &mut categories {
        if !c.enabled {
            c.pending = 0;
            c.conflicts.clear();
            c.skipped.clear();
        }
    }
    AnywhereSnapshot {
        signed_in: true,
        login: Some("mock-user".into()),
        connector: Connector::Live,
        connector_text: "live · relay 4s ago".into(),
        device_label: "a1b2c3d4 · workstation".into(),
        sync_enabled: true,
        worker_running: true,
        last_sync_at: Some(now - 130_000),
        last_error: None,
        categories,
        sign_ins: vec![SignInView {
            provider: "codex".into(),
            label: "Codex (ChatGPT)".into(),
            command: "forge auth codex-oauth".into(),
        }],
        missing_tools: vec![MissingToolView {
            label: "Claude Code".into(),
            hint: "npm install -g @anthropic-ai/claude-code".into(),
        }],
        taken_at_ms: now,
    }
}

fn mock_devices() -> Devices {
    let now = now_ms() as i64;
    Devices::Loaded {
        hosts_online: 2,
        hosts_total: 3,
        devices: vec![
            DeviceView {
                id: "a1b2c3d4e5f60718".into(),
                name: "workstation".into(),
                enrolled: "2026-09-12".into(),
                last_seen_at: Some(now),
                this_device: true,
            },
            DeviceView {
                id: "99aa88bb77cc6655".into(),
                name: "laptop".into(),
                enrolled: "2026-10-02".into(),
                last_seen_at: Some(now - 3 * 60_000),
                this_device: false,
            },
            DeviceView {
                id: "0f1e2d3c4b5a6978".into(),
                name: "iPhone".into(),
                enrolled: "2026-10-21".into(),
                last_seen_at: Some(now - 5 * 3_600_000),
                this_device: false,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anywhere::account_sync::status_fixture;

    #[test]
    fn the_view_carries_counts_and_labels_from_the_status() {
        let status = status_fixture();
        let view = view_of(
            &status,
            Some("octo".into()),
            (Connector::Live, "live".into()),
            "abcd1234 · box".into(),
            true,
            Some(Some("boom".into())),
        );
        assert!(view.signed_in && view.worker_running);
        assert_eq!(view.last_error.as_deref(), Some("boom"));
        let keys = &view.categories[0];
        assert!(keys.requires_direct && !keys.enabled);
        assert_eq!(keys.remote_only, 3);
        let mcp = view.categories.iter().find(|c| c.id == "mcp").unwrap();
        assert_eq!(mcp.skipped[0].0, "mcp: github");
        assert_eq!(view.sign_ins[0].command, "forge auth codex-oauth");
        let none = view_of(
            &status,
            None,
            (Connector::Off, String::new()),
            String::new(),
            false,
            None,
        );
        assert!(!none.worker_running);
    }

    #[test]
    fn timestamps_accept_seconds_and_milliseconds() {
        assert_eq!(epoch_ms("1793390000"), Some(1_793_390_000_000));
        assert_eq!(epoch_ms("1793390000123"), Some(1_793_390_000_123));
        assert_eq!(epoch_ms("not a time"), None);
        assert_eq!(date("1793390000"), "2026-10-30");
    }

    #[tokio::test]
    async fn mock_toggles_stick_and_never_touch_config() {
        set_category(true, "instructions", true).await.unwrap();
        let view = load_snapshot(true).await.unwrap();
        assert!(
            view.categories
                .iter()
                .find(|c| c.id == "instructions")
                .unwrap()
                .enabled
        );
        set_category(true, "instructions", false).await.unwrap();
        assert!(matches!(load_devices(true).await, Devices::Loaded { .. }));
    }
}
