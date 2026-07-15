//! Shared OAuth flow selection and headless completion helpers.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::io::{self, IsTerminal};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginFlow {
    Device,
    Loopback,
    Paste,
}

/// Whether this process is running without a browser-capable interactive terminal.
pub fn is_headless() -> bool {
    std::env::var_os("FORGE_NO_BROWSER").is_some()
        || !std::io::stdin().is_terminal()
        || !std::io::stdout().is_terminal()
}

/// Resolve explicit flags and server capability into one OAuth flow.
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
    } else if headless && device_supported {
        LoginFlow::Device
    } else if headless {
        LoginFlow::Paste
    } else {
        LoginFlow::Loopback
    }
}

/// Read a pasted redirect or authorization code without echoing an extra prompt.
pub fn read_pasted_redirect_from_stdin() -> Result<String> {
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("reading pasted OAuth redirect")?;
    if input.trim().is_empty() {
        bail!("no redirect URL or authorization code provided")
    }
    Ok(input)
}

/// Extract an authorization code from a pasted redirect URL, query, or bare code.
/// URLs carrying a `code` must also carry the expected CSRF state.
pub fn parse_pasted_redirect(input: &str, expected_state: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        bail!("no redirect URL or authorization code provided")
    }
    if !input.contains("code=") {
        return Ok(input.to_string());
    }
    let query = input
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or(input)
        .split('#')
        .next()
        .unwrap_or_default();
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "code" => code = Some(url_decode(value)),
            "state" => state = Some(url_decode(value)),
            _ => {}
        }
    }
    let code = code.ok_or_else(|| anyhow!("pasted URL is missing the 'code' parameter"))?;
    match state.as_deref() {
        Some(value) if value == expected_state => Ok(code),
        Some(_) => bail!("OAuth state mismatch — aborting (possible CSRF)"),
        None => bail!("pasted URL is missing the 'state' parameter"),
    }
}

fn url_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[index + 1..index + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    output.push(byte as char);
                    index += 3;
                    continue;
                }
            }
        }
        output.push(if bytes[index] == b'+' {
            ' '
        } else {
            bytes[index] as char
        });
        index += 1;
    }
    output
}

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
    expires_in: Option<u64>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

pub struct DeviceCodeInfo {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub interval: u64,
    pub expires_in: u64,
}

pub struct DeviceTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
}

/// Request an RFC 8628 device code from an advertised endpoint.
pub async fn request_device_code(
    device_url: &str,
    client_id: &str,
    scope: &str,
) -> Result<DeviceCodeInfo> {
    let response = reqwest::Client::new()
        .post(device_url)
        .form(&[("client_id", client_id), ("scope", scope)])
        .send()
        .await
        .with_context(|| format!("requesting device code from {device_url}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("device code request failed ({status}): {body}");
    }
    let body: DeviceCodeResponse = response
        .json()
        .await
        .context("parsing device code response")?;
    Ok(DeviceCodeInfo {
        device_code: body.device_code,
        user_code: body.user_code,
        verification_uri: body.verification_uri,
        verification_uri_complete: body.verification_uri_complete,
        interval: body.interval.unwrap_or(5).max(1),
        expires_in: body.expires_in.unwrap_or(900),
    })
}

/// Poll an RFC 8628 token endpoint until approval, denial, or expiry.
pub async fn poll_device_token(
    token_url: &str,
    client_id: &str,
    device: &DeviceCodeInfo,
) -> Result<DeviceTokens> {
    let client = reqwest::Client::new();
    let mut interval = Duration::from_secs(device.interval);
    let deadline = Instant::now() + Duration::from_secs(device.expires_in);
    loop {
        if Instant::now() >= deadline {
            bail!("device code expired before authorization");
        }
        tokio::time::sleep(interval).await;
        let response = client
            .post(token_url)
            .form(&[
                ("client_id", client_id),
                ("device_code", device.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .with_context(|| format!("polling token endpoint {token_url}"))?;
        let status = response.status();
        let body: DeviceTokenResponse = response
            .json()
            .await
            .context("parsing device token response")?;
        if let Some(error) = body.error.as_deref() {
            let detail = body
                .error_description
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            match error {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval += Duration::from_secs(5);
                    continue;
                }
                "expired_token" => bail!("device code expired before authorization"),
                "access_denied" => bail!("authorization was denied{detail}"),
                other => bail!("device authorization failed ({status}): {other}{detail}"),
            }
        }
        if let Some(access_token) = body.access_token {
            return Ok(DeviceTokens {
                access_token,
                refresh_token: body.refresh_token,
                expires_in: body.expires_in,
            });
        }
        bail!("device token response did not include an access token ({status})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_precedence_is_explicit_then_headless_capability() {
        assert_eq!(
            select_login_flow(true, true, false, true),
            LoginFlow::Device
        );
        assert_eq!(
            select_login_flow(false, true, true, false),
            LoginFlow::Paste
        );
        assert_eq!(
            select_login_flow(false, false, true, true),
            LoginFlow::Device
        );
        assert_eq!(
            select_login_flow(false, false, false, true),
            LoginFlow::Paste
        );
        assert_eq!(
            select_login_flow(false, false, false, false),
            LoginFlow::Loopback
        );
    }

    #[test]
    fn pasted_redirect_requires_matching_state() {
        assert_eq!(
            parse_pasted_redirect("https://localhost/cb?code=a%2Bb&state=ok", "ok").unwrap(),
            "a+b"
        );
        assert!(parse_pasted_redirect("https://localhost/cb?code=a&state=no", "ok").is_err());
        assert!(parse_pasted_redirect("https://localhost/cb?code=a", "ok").is_err());
        assert_eq!(
            parse_pasted_redirect("bare-code", "unused").unwrap(),
            "bare-code"
        );
    }
}
