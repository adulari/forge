//! Provider-account OAuth (device-code grant) — the pure, offline-testable half for
//! subscription-backed inference providers (first: xAI/Grok, see docs' xai-oauth guide). This is
//! separate from [`crate::oauth`], which is the authorization_code + PKCE + loopback flow for
//! OAuth-*protected MCP servers*: a device-code login has no browser redirect, just a
//! print-a-code / poll-a-token-endpoint exchange. Both reuse [`crate::oauth::OAuthTokens`] and the
//! same keyring-only storage discipline (ADR-0007: tokens live in the keyring, never in
//! config/logs). The networked half (device-code request, token polling, refresh, inference)
//! lands in forge-provider; it builds on these types.

use base64::Engine as _;

use crate::ConfigError;

/// xAI's OAuth issuer (device-code + refresh token endpoint host).
pub const XAI_OAUTH_ISSUER: &str = "https://auth.x.ai";
pub const XAI_DEVICE_CODE_ENDPOINT: &str = "https://auth.x.ai/oauth2/device/code";
pub const XAI_TOKEN_ENDPOINT: &str = "https://auth.x.ai/oauth2/token";
/// The public client id xAI's own OpenClaw integration uses (also used by the Hermes agent's
/// `xai-oauth` provider). Forge has no dedicated client id of its own yet — see the deferred list
/// in docs' xai-oauth guide; this is a known Phase-1 limitation, not a bug.
pub const XAI_OAUTH_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const XAI_OAUTH_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
pub const XAI_DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Keyring provider-key `xai-oauth` tokens are stored under (distinct from the `xai` API-key
/// provider and from any MCP server named `xai`).
pub const XAI_OAUTH_KEYRING_PROVIDER: &str = "xai";

/// RFC 8628 §3.2 device-authorization response.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    #[serde(default)]
    pub interval: Option<u64>,
}

impl DeviceCodeResponse {
    /// How long to sleep between polls. RFC 8628 defaults to 5s when the server omits `interval`.
    pub fn poll_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.interval.unwrap_or(5))
    }
}

/// One poll of the token endpoint, decoded (RFC 8628 §3.5).
#[derive(Debug, Clone, PartialEq)]
pub enum DevicePollOutcome {
    Tokens(crate::oauth::OAuthTokens),
    /// `authorization_pending` — keep polling at the current interval.
    Pending,
    /// `slow_down` — keep polling, but the caller must add 5s to its interval (RFC 8628 §3.5).
    SlowDown,
    /// `access_denied` — the user declined; stop polling.
    Denied(String),
    /// `expired_token` — the device code's `expires_in` window passed; stop polling.
    Expired,
}

/// Parse a token-endpoint poll response (HTTP `status` + raw `body`) into a [`DevicePollOutcome`].
/// `now` is unix seconds, used to turn a relative `expires_in` into an absolute `expires_at`.
pub fn parse_device_token_response(
    status: u16,
    body: &str,
    now: i64,
) -> Result<DevicePollOutcome, ConfigError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| ConfigError::Keyring(format!("invalid xAI token response: {e}")))?;

    if status == 200 {
        let access_token = v
            .get("access_token")
            .and_then(|x| x.as_str())
            .ok_or_else(|| ConfigError::Keyring("xAI token response missing access_token".into()))?
            .to_string();
        let refresh_token = v
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .map(str::to_string);
        let expires_in = v.get("expires_in").and_then(|x| x.as_i64()).unwrap_or(0);
        let scopes = v
            .get("scope")
            .and_then(|x| x.as_str())
            .map(|s| s.split_whitespace().map(str::to_string).collect())
            .unwrap_or_else(|| {
                XAI_OAUTH_SCOPE
                    .split_whitespace()
                    .map(str::to_string)
                    .collect()
            });
        return Ok(DevicePollOutcome::Tokens(crate::oauth::OAuthTokens {
            access_token,
            refresh_token,
            expires_at: if expires_in > 0 { now + expires_in } else { 0 },
            token_endpoint: XAI_TOKEN_ENDPOINT.to_string(),
            client_id: XAI_OAUTH_CLIENT_ID.to_string(),
            scopes,
        }));
    }

    let error = v.get("error").and_then(|x| x.as_str()).unwrap_or("");
    match error {
        "authorization_pending" => Ok(DevicePollOutcome::Pending),
        "slow_down" => Ok(DevicePollOutcome::SlowDown),
        "access_denied" => Ok(DevicePollOutcome::Denied(
            v.get("error_description")
                .and_then(|x| x.as_str())
                .unwrap_or("sign-in was declined")
                .to_string(),
        )),
        "expired_token" => Ok(DevicePollOutcome::Expired),
        _ => Err(ConfigError::Keyring(format!(
            "xAI device-token poll failed (HTTP {status}): {}",
            v.get("error_description")
                .and_then(|x| x.as_str())
                .unwrap_or(body)
        ))),
    }
}

/// Keyring key for a provider's OAuth tokens — `provider-oauth:<provider>`, distinct from API
/// keys (env-var-scheme keyring entries) and MCP-server OAuth tokens (`mcp-oauth:<server>`,
/// [`crate::oauth::oauth_keyring_key`]).
pub fn provider_oauth_keyring_key(provider: &str) -> String {
    format!("provider-oauth:{provider}")
}

/// Persist a provider's OAuth tokens (keyring, encrypted-file fallback; ADR-0007: never in
/// config/logs). Updates the *active* account only — see [`crate::oauth::OAuthAccountStore`].
pub fn store_provider_oauth_tokens(
    provider: &str,
    tokens: &crate::oauth::OAuthTokens,
) -> Result<(), ConfigError> {
    crate::oauth::store_active_tokens(&provider_oauth_keyring_key(provider), tokens)
}

/// Load a provider's *active* OAuth tokens, or `None` if none stored / unreadable.
pub fn load_provider_oauth_tokens(provider: &str) -> Option<crate::oauth::OAuthTokens> {
    crate::oauth::load_active_tokens(&provider_oauth_keyring_key(provider))
}

/// Delete a provider's stored OAuth tokens — every account. Idempotent: `Ok(false)` if none were
/// stored.
pub fn clear_provider_oauth_tokens(provider: &str) -> Result<bool, ConfigError> {
    crate::oauth::clear_account_store(&provider_oauth_keyring_key(provider))
}

/// Add (or overwrite) an OAuth account for `provider` and make it active.
pub fn add_provider_oauth_account(
    provider: &str,
    id: &str,
    tokens: &crate::oauth::OAuthTokens,
) -> Result<(), ConfigError> {
    crate::oauth::add_account(&provider_oauth_keyring_key(provider), id, tokens)
}

/// `(id, tokens, is_active)` for every OAuth account stored for `provider`.
pub fn list_provider_oauth_accounts(
    provider: &str,
) -> Vec<(String, crate::oauth::OAuthTokens, bool)> {
    crate::oauth::list_accounts(&provider_oauth_keyring_key(provider))
}

/// Switch `provider`'s active OAuth account. Errors if `id` isn't stored.
pub fn switch_provider_oauth_account(provider: &str, id: &str) -> Result<(), ConfigError> {
    crate::oauth::switch_account(&provider_oauth_keyring_key(provider), id)
}

/// Remove one OAuth account for `provider`. Promotes a remaining account to active if the removed
/// one was active; deletes the whole entry if none remain. `Ok(false)` if `id` wasn't stored.
pub fn remove_provider_oauth_account(provider: &str, id: &str) -> Result<bool, ConfigError> {
    crate::oauth::remove_account(&provider_oauth_keyring_key(provider), id)
}

/// First free `account-N` id for a fresh `provider` login with no better label available.
pub fn next_provider_oauth_account_id(provider: &str) -> String {
    crate::oauth::next_default_account_id(&provider_oauth_keyring_key(provider))
}

/// Best-effort account label from a device-code response's `id_token` (a JWT): decode the
/// payload (base64url — **no signature verification**, this is only a display label, the
/// device-code grant itself already authenticated the account) and return its `email` claim.
/// `None` if `id_token` isn't a 3-part JWT, the payload doesn't decode, or there's no `email`.
pub fn extract_email_from_id_token(id_token: &str) -> Option<String> {
    let payload_b64 = id_token.split('.').nth(1)?;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload_b64))
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    claims
        .get("email")
        .and_then(|e| e.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyring_key_is_namespaced() {
        assert_eq!(provider_oauth_keyring_key("xai"), "provider-oauth:xai");
    }

    #[test]
    fn device_code_response_parses_with_and_without_optionals() {
        let full: DeviceCodeResponse = serde_json::from_str(
            r#"{"device_code":"d","user_code":"ABCD-EFGH","verification_uri":"https://auth.x.ai/activate",
                "verification_uri_complete":"https://auth.x.ai/activate?user_code=ABCD-EFGH",
                "expires_in":900,"interval":5}"#,
        )
        .unwrap();
        assert_eq!(full.poll_interval(), std::time::Duration::from_secs(5));
        assert_eq!(
            full.verification_uri_complete.as_deref(),
            Some("https://auth.x.ai/activate?user_code=ABCD-EFGH")
        );

        let minimal: DeviceCodeResponse = serde_json::from_str(
            r#"{"device_code":"d","user_code":"ABCD-EFGH","verification_uri":"https://auth.x.ai/activate","expires_in":900}"#,
        )
        .unwrap();
        assert!(minimal.verification_uri_complete.is_none());
        // RFC 8628 default interval is 5s when the server omits it.
        assert_eq!(minimal.poll_interval(), std::time::Duration::from_secs(5));
    }

    #[test]
    fn poll_outcome_pending_and_slow_down() {
        let pending =
            parse_device_token_response(400, r#"{"error":"authorization_pending"}"#, 1000).unwrap();
        assert_eq!(pending, DevicePollOutcome::Pending);
        let slow = parse_device_token_response(400, r#"{"error":"slow_down"}"#, 1000).unwrap();
        assert_eq!(slow, DevicePollOutcome::SlowDown);
    }

    #[test]
    fn poll_outcome_terminal_denied_and_expired() {
        let denied = parse_device_token_response(
            400,
            r#"{"error":"access_denied","error_description":"user declined"}"#,
            1000,
        )
        .unwrap();
        assert_eq!(
            denied,
            DevicePollOutcome::Denied("user declined".to_string())
        );
        let expired =
            parse_device_token_response(400, r#"{"error":"expired_token"}"#, 1000).unwrap();
        assert_eq!(expired, DevicePollOutcome::Expired);
    }

    #[test]
    fn poll_success_builds_tokens_with_absolute_expiry() {
        let outcome = parse_device_token_response(
            200,
            r#"{"access_token":"at","refresh_token":"rt","expires_in":3600,"scope":"openid api:access"}"#,
            1000,
        )
        .unwrap();
        match outcome {
            DevicePollOutcome::Tokens(t) => {
                assert_eq!(t.access_token, "at");
                assert_eq!(t.refresh_token.as_deref(), Some("rt"));
                assert_eq!(t.expires_at, 4600);
                assert_eq!(t.token_endpoint, XAI_TOKEN_ENDPOINT);
                assert_eq!(t.client_id, XAI_OAUTH_CLIENT_ID);
                assert_eq!(
                    t.scopes,
                    vec!["openid".to_string(), "api:access".to_string()]
                );
            }
            other => panic!("expected Tokens, got {other:?}"),
        }
    }

    #[test]
    fn poll_failure_with_unknown_error_is_an_error() {
        assert!(parse_device_token_response(500, r#"{"error":"server_error"}"#, 1000).is_err());
    }

    #[test]
    fn tokens_round_trip_and_expiry_reuses_oauth_shape() {
        let t = crate::oauth::OAuthTokens {
            access_token: "at".into(),
            refresh_token: Some("rt".into()),
            expires_at: 1000,
            token_endpoint: XAI_TOKEN_ENDPOINT.into(),
            client_id: XAI_OAUTH_CLIENT_ID.into(),
            scopes: vec!["api:access".into()],
        };
        assert!(t.is_expired(950, 60));
        assert!(!t.is_expired(800, 60));
    }

    /// Hand-build an unsigned JWT (`header.payload.signature`, base64url no-pad) carrying the
    /// given claims — `extract_email_from_id_token` never checks the signature, so an empty one
    /// is fine for this test.
    fn fake_id_token(claims_json: &str) -> String {
        let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!(
            "{}.{}.{}",
            b64(r#"{"alg":"none"}"#),
            b64(claims_json),
            "sig"
        )
    }

    #[test]
    fn extracts_email_claim_from_id_token_payload() {
        let jwt = fake_id_token(r#"{"sub":"123","email":"trader@x.ai","name":"Trader"}"#);
        assert_eq!(
            extract_email_from_id_token(&jwt),
            Some("trader@x.ai".to_string())
        );
    }

    #[test]
    fn id_token_without_email_claim_returns_none() {
        let jwt = fake_id_token(r#"{"sub":"123"}"#);
        assert_eq!(extract_email_from_id_token(&jwt), None);
    }

    #[test]
    fn malformed_id_token_returns_none_not_a_panic() {
        assert_eq!(extract_email_from_id_token("not-a-jwt"), None);
        assert_eq!(extract_email_from_id_token("a.b"), None);
        assert_eq!(extract_email_from_id_token("a.!!!notb64!!!.c"), None);
    }

    #[test]
    fn provider_account_helpers_add_list_switch_remove() {
        // Exercises the provider_oauth-flavored wrappers end to end using the same
        // OAuthAccountStore the mcp-oauth flavor uses — construct a store directly (offline,
        // no keyring I/O) and drive it through the same methods the wrappers delegate to.
        let mut store = crate::oauth::OAuthAccountStore::new_single(
            "personal",
            crate::oauth::OAuthTokens {
                access_token: "at1".into(),
                refresh_token: None,
                expires_at: 0,
                token_endpoint: XAI_TOKEN_ENDPOINT.into(),
                client_id: XAI_OAUTH_CLIENT_ID.into(),
                scopes: vec![],
            },
        );
        store.add(
            "supergrok@x.ai",
            crate::oauth::OAuthTokens {
                access_token: "at2".into(),
                refresh_token: None,
                expires_at: 0,
                token_endpoint: XAI_TOKEN_ENDPOINT.into(),
                client_id: XAI_OAUTH_CLIENT_ID.into(),
                scopes: vec![],
            },
        );
        assert_eq!(store.active, "supergrok@x.ai");
        assert_eq!(store.list().len(), 2);
        store.switch("personal").unwrap();
        assert_eq!(store.active_tokens().unwrap().access_token, "at1");
        assert!(store.remove("personal"));
        assert_eq!(store.active, "supergrok@x.ai", "promoted the last account");
    }
}
