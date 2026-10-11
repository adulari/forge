//! Other machines for session-to-session messaging: the account's other hosts, their live
//! sessions, and delivering a message into one of them. Plugs into `crate::fleet_peers` as its
//! [`RemoteHosts`] and rides the encrypted relay through `connector::controller`; the host being
//! asked answers from its own daemon (`GET /api/sessions`, `POST /api/sessions/{id}/message`
//! behind typed bridge routes), so nothing here widens what a device of the account could
//! already do.
//!
//! Everything degrades to "offline": a host that does not answer is listed with its last-seen
//! time and never fails the tool. With Anywhere off or not set up this returns nothing at all and
//! no network request is made.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use forge_anywhere_protocol::bridge::{BridgeRequest, RouteId};
use forge_core::fleet::{FleetHostStatus, FleetSessionInfo};
use serde::Deserialize;

use super::connector::controller::HostCaller;
use crate::fleet_peers::{RemoteHostSessions, RemoteHosts, RemoteMessage};

/// How long one host gets to answer before it is reported offline.
const HOST_TIMEOUT: Duration = Duration::from_secs(8);
/// A host that did not answer is not asked again for this long, so a powered-off machine does not
/// add its timeout to every listing and every send that misses locally.
const OFFLINE_BACKOFF: Duration = Duration::from_secs(30);

static OFFLINE: Mutex<Vec<(String, Instant, String)>> = Mutex::new(Vec::new());

#[derive(Debug, Deserialize)]
struct HostList {
    hosts: Vec<HostRow>,
}

#[derive(Debug, Clone, Deserialize)]
struct HostRow {
    id: String,
    #[serde(default)]
    device_id: String,
    name: String,
    #[serde(default)]
    revoked_at: Option<String>,
    #[serde(default)]
    last_seen_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceList {
    devices: Vec<DeviceRow>,
}

#[derive(Debug, Deserialize)]
struct DeviceRow {
    id: String,
    #[serde(default)]
    last_seen_at: Option<String>,
}

/// One row of a host's `GET /api/sessions`, only the fields session messaging needs.
#[derive(Debug, Deserialize)]
struct RemoteRow {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    busy: bool,
    #[serde(default)]
    waiting: bool,
    #[serde(default)]
    terminal: bool,
    #[serde(default)]
    read_only: bool,
}

pub(crate) struct AnywhereHosts;

fn enabled() -> bool {
    forge_config::load().is_ok_and(|c| c.anywhere.enabled)
}

fn recently_offline(name: &str) -> Option<String> {
    let mut offline = OFFLINE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    offline.retain(|(_, at, _)| at.elapsed() < OFFLINE_BACKOFF);
    offline
        .iter()
        .find(|(n, _, _)| n == name)
        .map(|(_, _, why)| why.clone())
}

fn mark_offline(name: &str, why: &str) {
    let mut offline = OFFLINE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    offline.retain(|(n, _, _)| n != name);
    offline.push((name.to_string(), Instant::now(), why.to_string()));
}

fn clear_offline(name: &str) {
    OFFLINE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|(n, _, _)| n != name);
}

fn bridge_request(
    route: RouteId,
    method: &str,
    parameters: Vec<String>,
    body: Vec<u8>,
) -> BridgeRequest {
    BridgeRequest {
        request_id: rand::random::<[u8; 16]>(),
        route,
        method: method.to_string(),
        parameters,
        headers: if body.is_empty() {
            Vec::new()
        } else {
            vec![("content-type".into(), "application/json".into())]
        },
        body,
        body_blob: None,
    }
}

fn sessions_of(host: &str, body: &[u8]) -> Result<Vec<FleetSessionInfo>, String> {
    let rows: Vec<RemoteRow> = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| FleetSessionInfo {
            id: r.id,
            title: r.title,
            cwd: r.cwd,
            model: r.model,
            state: if r.waiting {
                "waiting"
            } else if r.busy {
                "busy"
            } else {
                "idle"
            }
            .into(),
            host: Some(host.to_string()),
            terminal: r.terminal,
            can_receive: !r.read_only,
            is_me: false,
        })
        .collect())
}

async fn hosts_of(caller: &HostCaller) -> anyhow::Result<Vec<HostRow>> {
    let list: HostList = caller.service_get("/v1/hosts").await?;
    let own = caller.own_host_id.clone();
    Ok(list
        .hosts
        .into_iter()
        .filter(|h| h.revoked_at.is_none() && h.id != own)
        .collect())
}

async fn one_host(
    caller: &HostCaller,
    host: &HostRow,
    device_seen: Option<String>,
) -> RemoteHostSessions {
    let last_seen = host.last_seen_at.clone().or(device_seen);
    let offline = |why: String| {
        mark_offline(&host.name, &why);
        RemoteHostSessions {
            status: FleetHostStatus {
                name: host.name.clone(),
                online: false,
                last_seen: last_seen.clone(),
                error: Some(why),
            },
            sessions: Vec::new(),
        }
    };
    if let Some(why) = recently_offline(&host.name) {
        return offline(why);
    }
    let request = bridge_request(
        RouteId::ListSessions,
        "GET",
        vec![String::new()],
        Vec::new(),
    );
    match caller
        .call(&host.id, Some(&host.device_id), &request, HOST_TIMEOUT)
        .await
    {
        Ok(resp) if resp.status < 300 => match sessions_of(&host.name, &resp.body) {
            Ok(sessions) => {
                clear_offline(&host.name);
                RemoteHostSessions {
                    status: FleetHostStatus {
                        name: host.name.clone(),
                        online: true,
                        last_seen: None,
                        error: None,
                    },
                    sessions,
                }
            }
            Err(e) => offline(format!("unreadable session list: {e}")),
        },
        Ok(resp) => offline(format!("host answered HTTP {}", resp.status)),
        Err(e) => offline(format!("{e:#}")),
    }
}

#[async_trait::async_trait]
impl RemoteHosts for AnywhereHosts {
    fn local_name(&self) -> String {
        forge_config::load()
            .ok()
            .and_then(|c| c.anywhere.host_name)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(super::default_host_name)
    }

    async fn list(&self) -> Vec<RemoteHostSessions> {
        if !enabled() {
            return Vec::new();
        }
        let Ok(caller) = HostCaller::open().await else {
            return Vec::new();
        };
        let Ok(hosts) = hosts_of(&caller).await else {
            return Vec::new();
        };
        let seen: HashMap<String, String> = caller
            .service_get::<DeviceList>("/v1/devices")
            .await
            .map(|l| {
                l.devices
                    .into_iter()
                    .filter_map(|d| d.last_seen_at.map(|s| (d.id, s)))
                    .collect()
            })
            .unwrap_or_default();
        let caller = &caller;
        futures::future::join_all(hosts.iter().map(|h| {
            let device_seen = seen.get(&h.device_id).cloned();
            async move { one_host(caller, h, device_seen).await }
        }))
        .await
    }

    async fn send(
        &self,
        host: &str,
        session_id: &str,
        message: &RemoteMessage,
    ) -> Result<(), String> {
        if !enabled() {
            return Err("Forge Anywhere is not enabled".into());
        }
        let caller = HostCaller::open().await.map_err(|e| format!("{e:#}"))?;
        let row = hosts_of(&caller)
            .await
            .map_err(|e| format!("{e:#}"))?
            .into_iter()
            .find(|h| h.name.eq_ignore_ascii_case(host))
            .ok_or_else(|| format!("no Anywhere host named '{host}'"))?;
        if let Some(why) = recently_offline(&row.name) {
            return Err(format!("host {} is offline ({why})", row.name));
        }
        let body = serde_json::to_vec(&serde_json::json!({
            "text": message.text,
            "mode": message.mode.as_str(),
            "sender_kind": "session",
            "sender_id": message.sender_id,
            "sender_label": message.sender_label,
        }))
        .map_err(|e| e.to_string())?;
        let request = bridge_request(
            RouteId::SessionMessage,
            "POST",
            vec![session_id.to_string()],
            body,
        );
        match caller
            .call(&row.id, Some(&row.device_id), &request, HOST_TIMEOUT)
            .await
        {
            Ok(resp) if resp.status < 300 => {
                clear_offline(&row.name);
                Ok(())
            }
            Ok(resp) => {
                let detail = serde_json::from_slice::<serde_json::Value>(&resp.body)
                    .ok()
                    .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                    .unwrap_or_else(|| format!("HTTP {}", resp.status));
                Err(format!("host {} refused the message: {detail}", row.name))
            }
            Err(e) => {
                mark_offline(&row.name, &format!("{e:#}"));
                Err(format!("host {} is not reachable: {e:#}", row.name))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_session_list_maps_state_terminal_and_watch_only() {
        let body = br#"[
            {"id":"a1","title":"w","cwd":"/p","model":"m","busy":true,"waiting":false},
            {"id":"b2","title":"","cwd":"/q","busy":false,"waiting":true,"terminal":true,"read_only":true},
            {"id":"c3","busy":false,"waiting":false}
        ]"#;
        let rows = sessions_of("ssu-dev", body).unwrap();
        assert_eq!(rows[0].state, "busy");
        assert_eq!(rows[0].host.as_deref(), Some("ssu-dev"));
        assert!(rows[0].can_receive);
        assert_eq!(rows[1].state, "waiting");
        assert!(rows[1].terminal && !rows[1].can_receive);
        assert_eq!(rows[2].state, "idle");
        assert!(sessions_of("h", b"not json").is_err());
    }

    #[test]
    fn an_offline_host_is_remembered_briefly() {
        mark_offline("peers-test-host", "no answer");
        assert_eq!(
            recently_offline("peers-test-host").as_deref(),
            Some("no answer")
        );
        clear_offline("peers-test-host");
        assert_eq!(recently_offline("peers-test-host"), None);
    }
}
