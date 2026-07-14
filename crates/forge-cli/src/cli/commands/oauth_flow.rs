//! Shared helpers for headless-friendly OAuth login flows.
//!
//! Provider and MCP logins pick between three flows depending on whether a
//! local browser can reach this host's loopback port:
//!
//! - [`LoginFlow::Device`] — RFC 8628 device-authorization grant. Best for a
//!   headless host when the authorization server supports it: we show a URL +
//!   user code and poll the token endpoint.
//! - [`LoginFlow::Loopback`] — the classic redirect caught by a local HTTP
//!   listener. Requires a browser that can reach `127.0.0.1:PORT`.
//! - [`LoginFlow::Paste`] — out-of-band completion: the user authorizes on any
//!   device and pastes the redirect URL (or bare `code`) back into the CLI.

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use serde::Deserialize;
use std::io::IsTerminal;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginFlow {
    Device,
    Loopback,
    Paste,
}

/// True when no local browser is available/reachable: `FORGE_NO_BROWSER` is set,
/// or stdin/stdout is not a TTY (e.g. an SSH pipe or CI).
pub fn is_headless() -> bool {
    std::env::var("FORGE_NO_BROWSER").is_ok()
        || !std::io::stdin().is_terminal()
        || !std::io::stdout().is_terminal()
}

/// Decide the login flow from explicit overrides, provider capability, and env.
///
/// Precedence: explicit `--device` / `--paste` win; otherwise a headless host
/// prefers `device` when the provider supports RFC 8628, else `paste`; a host
/// with a reachable browser uses `loopback`.
pub fn select_login_flow(
    force_device: bool,
    force_paste: bool,
    device_supported: bool,
    headless: bool,
) -> LoginFlow {
    if force_device {
        LoginFlow::Device
    } else if force_paste {
        LoginFlow::Paste
    } else if headless {
        if device_supported {
            LoginFlow::Device
        } else {
            LoginFlow::Paste
        }
    } else {
        LoginFlow::Loopback
    }
}

/// Generate a random URL-safe OAuth `state` value for CSRF protection.
pub fn generate_state() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Extract `code` and `state` from a redirect query string (`a=b&c=d`).
pub fn parse_code_and_state(query: &str) -> (Option<String>, Option<String>) {
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        match (it.next(), it.next()) {
            (Some("code"), Some(v)) => code = Some(url_decode(v)),
            (Some("state"), Some(v)) => state = Some(url_decode(v)),
            _ => {}
        }
    }
    (code, state)
}

/// Parse the authorization `code` from a pasted redirect URL or bare code, and
/// verify the `state` when present.
///
/// Accepts any of:
/// - a full redirect URL: `http://localhost:PORT/cb?code=...&state=...`
/// - a bare query string: `code=...&state=...`
/// - a bare authorization code (no `code=` — nothing to CSRF-check, PKCE still
///   binds the exchange)
///
/// When a `state` parameter is present it MUST equal `expected_state`, else the
/// call fails (possible CSRF). A pasted URL that carries a `code` but no `state`
/// is rejected.
pub fn parse_pasted_redirect(input: &str, expected_state: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        bail!("no redirect URL or authorization code provided");
    }

    if !input.contains("code=") {
        // Bare authorization code. There is no state to verify; the PKCE
        // verifier bound at authorize time still protects the exchange.
        return Ok(input.to_string());
    }

    let query = input.split_once('?').map(|(_, q)| q).unwrap_or(input);
    // Drop any URL fragment.
    let query = query.split('#').next().unwrap_or(query);
    let (code, state) = parse_code_and_state(query);

    let code = code.ok_or_else(|| anyhow!("pasted URL is missing the 'code' parameter"))?;
    match state.as_deref() {
        Some(s) if s == expected_state => Ok(code),
        Some(_) => bail!("OAuth state mismatch — aborting (possible CSRF)"),
        None => bail!("pasted URL is missing the 'state' parameter"),
    }
}

fn url_decode(s: &str) -> String {
    urlencoding::decode(s)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| s.to_string())
}

// --- Generic RFC 8628 device-authorization grant (used when a server supports it) ---

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DeviceTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    error: Option<String>,
}

/// Device-code details to display to the user while polling.
pub struct DeviceCodeInfo {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub interval: u64,
    pub expires_in: u64,
}

/// Tokens returned by a successful device-authorization grant.
pub struct DeviceTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub expires_in: Option<u64>,
}

/// Request a device + user code from an RFC 8628 device-authorization endpoint.
pub async fn request_device_code(
    device_url: &str,
    client_id: &str,
    scope: &str,
) -> Result<DeviceCodeInfo> {
    let client = reqwest::Client::new();
    let resp = client
        .post(device_url)
        .form(&[("client_id", client_id), ("scope", scope)])
        .send()
        .await
        .map_err(|e| anyhow!("failed to request device code: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("device code request failed ({status}): {body}");
    }

    let d: DeviceCodeResponse = resp
        .json()
        .await
        .map_err(|e| anyhow!("failed to parse device code response: {e}"))?;
    Ok(DeviceCodeInfo {
        device_code: d.device_code,
        user_code: d.user_code,
        verification_uri: d.verification_uri,
        verification_uri_complete: d.verification_uri_complete,
        interval: d.interval.unwrap_or(5).max(1),
        expires_in: d.expires_in.unwrap_or(900),
    })
}

/// Poll a token endpoint until the user authorizes or the code expires.
pub async fn poll_device_token(
    token_url: &str,
    client_id: &str,
    device: &DeviceCodeInfo,
) -> Result<DeviceTokens> {
    let client = reqwest::Client::new();
    let mut poll_interval = Duration::from_secs(device.interval);
    let deadline = Instant::now() + Duration::from_secs(device.expires_in);

    loop {
        if Instant::now() >= deadline {
            bail!("device code expired before authorization");
        }
        tokio::time::sleep(poll_interval).await;

        let resp = client
            .post(token_url)
            .form(&[
                ("client_id", client_id),
                ("device_code", &device.device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .map_err(|e| anyhow!("failed to poll token endpoint: {e}"))?;

        let token: DeviceTokenResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("failed to parse device token response: {e}"))?;

        if let Some(err) = token.error.as_deref() {
            match err {
                "authorization_pending" => continue,
                "slow_down" => {
                    poll_interval += Duration::from_secs(5);
                    continue;
                }
                "expired_token" => bail!("device code expired before authorization"),
                "access_denied" => bail!("authorization was denied"),
                other => bail!("device authorization failed: {other}"),
            }
        }

        if let Some(access_token) = token.access_token {
            return Ok(DeviceTokens {
                access_token,
                refresh_token: token.refresh_token,
                id_token: token.id_token,
                expires_in: token.expires_in,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_explicit_device_wins() {
        assert_eq!(
            select_login_flow(true, false, false, false),
            LoginFlow::Device
        );
        assert_eq!(
            select_login_flow(true, true, false, true),
            LoginFlow::Device
        );
    }

    #[test]
    fn flow_explicit_paste_wins() {
        assert_eq!(
            select_login_flow(false, true, true, false),
            LoginFlow::Paste
        );
    }

    #[test]
    fn flow_headless_prefers_device_when_supported() {
        assert_eq!(
            select_login_flow(false, false, true, true),
            LoginFlow::Device
        );
    }

    #[test]
    fn flow_headless_falls_back_to_paste() {
        assert_eq!(
            select_login_flow(false, false, false, true),
            LoginFlow::Paste
        );
    }

    #[test]
    fn flow_browser_uses_loopback() {
        assert_eq!(
            select_login_flow(false, false, true, false),
            LoginFlow::Loopback
        );
        assert_eq!(
            select_login_flow(false, false, false, false),
            LoginFlow::Loopback
        );
    }

    #[test]
    fn paste_extracts_code_from_full_url() {
        let code = parse_pasted_redirect(
            "http://localhost:1455/auth/callback?code=abc123&state=xyz789",
            "xyz789",
        )
        .unwrap();
        assert_eq!(code, "abc123");
    }

    #[test]
    fn paste_extracts_code_from_bare_query() {
        let code = parse_pasted_redirect("code=a%2Bb&state=st", "st").unwrap();
        assert_eq!(code, "a+b");
    }

    #[test]
    fn paste_rejects_state_mismatch() {
        let err = parse_pasted_redirect(
            "http://localhost:1455/cb?code=abc&state=WRONG",
            "expected",
        )
        .unwrap_err();
        assert!(err.to_string().contains("state mismatch"));
    }

    #[test]
    fn paste_rejects_missing_state_when_code_present() {
        let err = parse_pasted_redirect("http://localhost/cb?code=abc", "expected").unwrap_err();
        assert!(err.to_string().contains("missing the 'state'"));
    }

    #[test]
    fn paste_accepts_bare_code() {
        let code = parse_pasted_redirect("just-a-code", "unused-state").unwrap();
        assert_eq!(code, "just-a-code");
    }

    #[test]
    fn paste_rejects_empty() {
        assert!(parse_pasted_redirect("   ", "st").is_err());
    }

    #[test]
    fn paste_ignores_url_fragment() {
        let code =
            parse_pasted_redirect("http://localhost/cb?code=c1&state=s1#frag", "s1").unwrap();
        assert_eq!(code, "c1");
    }
}