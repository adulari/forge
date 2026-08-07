//! `forge send <target> <message>` — message-inject a running fleet session from the CLI without
//! attaching. Uses the same daemon discovery/auth as `forge attach` (`GET /api/sessions` to
//! resolve the target against the live fleet), then `POST .../message` — persisted first
//! (forge-store's `fleet_message` table, migration 25, survives a daemon restart before
//! delivery) and handed straight to the target's input queue when it's already live.

use anyhow::{bail, Context, Result};

use crate::attach::{fetch_sessions, resolve_base_url, resolve_token, SessionInfo};

pub(crate) async fn send_cmd(
    target: String,
    message: String,
    steer: bool,
    url: Option<String>,
    token: Option<String>,
) -> Result<()> {
    let text = message.trim();
    if text.is_empty() {
        bail!("message must not be empty");
    }
    if text.len() > 16 * 1024 {
        bail!("message is {} bytes, exceeds the 16KB limit", text.len());
    }
    let base = resolve_base_url(url);
    let token = resolve_token(token)?;
    let http = reqwest::Client::new();
    let sessions = fetch_sessions(&http, &base, &token).await?;
    let peer = resolve_send_target(&sessions, &target)?;

    let mode = if steer { "steer" } else { "follow_up" };
    let send_url = format!("{base}/{token}/api/sessions/{}/message", peer.id);
    let resp = http
        .post(&send_url)
        .json(&serde_json::json!({
            "text": text,
            "mode": mode,
            "sender_kind": "cli",
        }))
        .send()
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "could not reach the forge serve daemon at {base} — is it running? \
                 (start it with `forge serve --local`)  [{e}]"
            )
        })?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("daemon rejected the message ({status}): {body}");
    }
    let label = if peer.title.is_empty() {
        "(untitled)"
    } else {
        peer.title.as_str()
    };
    println!(
        "✓ sent to {label} ({}) [{mode}]",
        &peer.id[..peer.id.len().min(8)]
    );
    Ok(())
}

/// Resolve `address` against the live fleet fetched from `GET /api/sessions`: exact id match wins
/// outright, else an exact (and unique) title match, else a unique id prefix. Shares
/// [`forge_core::fleet::resolve_target`] with the `message_session` virtual tool and the CLI-
/// bridge's `mcp_serve::fleet` mirror — one resolution rule for every fleet-messaging transport.
fn resolve_send_target<'a>(sessions: &'a [SessionInfo], address: &str) -> Result<&'a SessionInfo> {
    let peers: Vec<forge_core::fleet::FleetPeer> = sessions
        .iter()
        .map(|s| forge_core::fleet::FleetPeer {
            id: s.id.clone(),
            title: (!s.title.is_empty()).then(|| s.title.clone()),
        })
        .collect();
    let resolved = forge_core::fleet::resolve_target(&peers, address).map_err(|e| {
        let known = forge_core::fleet::describe_peers(&peers);
        if sessions.is_empty() {
            anyhow::anyhow!("no sessions are running on this daemon")
        } else {
            anyhow::anyhow!("{e} for {address:?}. live sessions: [{known}]")
        }
    })?;
    sessions
        .iter()
        .find(|s| s.id == resolved.id)
        .context("resolved peer vanished from the fetched session list")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, title: &str) -> SessionInfo {
        SessionInfo {
            id: id.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn resolves_exact_id_unique_prefix_and_name() {
        let sessions = vec![info("aaa111", "worker"), info("aab222", "other")];
        assert_eq!(
            resolve_send_target(&sessions, "aaa111").unwrap().id,
            "aaa111"
        );
        assert_eq!(resolve_send_target(&sessions, "aab").unwrap().id, "aab222");
        assert_eq!(
            resolve_send_target(&sessions, "worker").unwrap().id,
            "aaa111"
        );
    }

    #[test]
    fn ambiguous_prefix_and_unknown_address_error() {
        let sessions = vec![info("aaa111", "w1"), info("aaa222", "w2")];
        assert!(resolve_send_target(&sessions, "aaa").is_err());
        assert!(resolve_send_target(&sessions, "zzz").is_err());
    }
}
