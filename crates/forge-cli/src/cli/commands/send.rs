//! `forge send <target> <message>` — message-inject a running fleet session from the CLI without
//! attaching. Uses the same daemon discovery/auth as `forge attach`, then `POST /api/peers/message`
//! so the daemon resolves the target (a local session, a `forge chat` terminal, or `host/name` on
//! another machine over Forge Anywhere) and applies the same guards as the agent tools. Delivery
//! is persisted first (forge-store's `fleet_message` table, survives a daemon restart before
//! delivery) and handed straight to the target's input queue when it's already live.

use anyhow::{bail, Result};

use crate::attach::{resolve_base_url, resolve_token};

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
    if text.len() > forge_core::fleet::MAX_MESSAGE_BYTES {
        bail!("message is {} bytes, exceeds the 16KB limit", text.len());
    }
    let base = resolve_base_url(url);
    let token = resolve_token(token)?;
    let http = reqwest::Client::new();
    let mode = if steer { "steer" } else { "follow_up" };
    let resp = http
        .post(format!("{base}/{token}/api/peers/message"))
        .json(&serde_json::json!({ "target": target, "text": text, "mode": mode }))
        .send()
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "could not reach the forge serve daemon at {base} — is it running? \
                 (start it with `forge serve --local`)  [{e}]"
            )
        })?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::NOT_FOUND {
        bail!("the running daemon predates session messaging — restart `forge serve`");
    }
    if !status.is_success() {
        let detail = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or(body);
        bail!("daemon rejected the message ({status}): {detail}");
    }
    let receipt: forge_core::fleet::SendReceipt = serde_json::from_str(&body)?;
    println!("✓ {receipt}");
    Ok(())
}
