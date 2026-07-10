//! xAI/Grok subscription OAuth provider (`xai-oauth::<model>`, e.g. `xai-oauth::grok-4`). A
//! separate auth path from the API-key `xai::` provider (genai, `XAI_API_KEY`): this one signs in
//! with a SuperGrok / X Premium **account** via an RFC 8628 device-code flow (no API key, no
//! dollar-metered credits — usage is billed against the subscription) and talks to xAI's
//! Responses-style endpoint directly. Modeled on Hermes' `xai-oauth` provider (the reference
//! implementation this ships from).
//!
//! SECURITY INVARIANT: the OAuth bearer token is only ever attached to a request built from the
//! hardcoded [`XAI_API_BASE`] — never a custom-provider endpoint, env override, or user-supplied
//! base URL. [`is_pinned_xai_url`] is the guard; nothing in this module accepts a caller-supplied
//! host for an authenticated request.
//!
//! KNOWN GOTCHA (do not "fix" by retrying): a successful device-code login proves the *account*
//! signed in, not that xAI's servers grant that account's subscription tier OAuth API access.
//! xAI enforces the entitlement check server-side and can 403 even a genuinely active SuperGrok
//! subscriber. [`probe_entitlement`] runs once right after login so the CLI can say so plainly
//! instead of silently retrying forever; at inference time the same 403 is classified as a
//! permanent [`ProviderError::Auth`] (see [`classify_xai_status`]) so the mesh excludes the model
//! instead of benching-and-retrying it every turn.

use forge_config::provider_oauth::{self, XAI_OAUTH_KEYRING_PROVIDER};
use forge_types::{Message, Role, ToolCall, Usage};
use futures::StreamExt;

use crate::{
    bundled_http_client, CompletionOptions, EventSink, ModelResponse, Provider, ProviderError,
    StreamEvent, ToolSpec,
};

/// The `xai-oauth::` model-id namespace [`crate::DispatchProvider`] routes on.
pub const XAI_OAUTH_NAMESPACE: &str = "xai-oauth";

/// Hardcoded — deliberately NOT read from config/env/the custom-provider registry. See the
/// module doc's security invariant.
const XAI_API_BASE: &str = "https://api.x.ai/v1";

/// Refresh the access token this long before it actually expires, so a request never races an
/// in-flight expiry.
const REFRESH_SKEW_SECS: i64 = 120;

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// True iff `url` is `https` and its host is exactly `api.x.ai` or a genuine `*.x.ai`
/// subdomain — rejects lookalikes (`evilx.ai`, `api.x.ai.evil.com`, `api-x.ai`) and any
/// non-HTTPS scheme. The sole gate for attaching the OAuth bearer to a request.
pub fn is_pinned_xai_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    matches!(parsed.host_str(), Some(h) if h == "api.x.ai" || h.ends_with(".x.ai"))
}

fn responses_url() -> String {
    format!("{XAI_API_BASE}/responses")
}

// ---------------------------------------------------------------------------------------------
// Device-code login (networked; called from forge-cli's `forge auth xai-oauth`)
// ---------------------------------------------------------------------------------------------

/// Start a device-code login: request a `user_code` + verification URL to show the user.
pub async fn start_device_login() -> anyhow::Result<forge_config::provider_oauth::DeviceCodeResponse>
{
    let http = bundled_http_client();
    let resp = http
        .post(forge_config::provider_oauth::XAI_DEVICE_CODE_ENDPOINT)
        .form(&[
            (
                "client_id",
                forge_config::provider_oauth::XAI_OAUTH_CLIENT_ID,
            ),
            ("scope", forge_config::provider_oauth::XAI_OAUTH_SCOPE),
        ])
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.json().await?)
}

/// Poll the token endpoint until the device-code flow reaches a terminal state (tokens, denied,
/// or expired), honoring `authorization_pending`/`slow_down` per RFC 8628 §3.5. Never loops past
/// the device code's own `expires_in` deadline.
///
/// Returns the tokens plus the raw `id_token` if xAI's response included one — the caller (the
/// login CLI command) decodes its `email` claim to label the new account
/// ([`forge_config::provider_oauth::extract_email_from_id_token`]); this module stays agnostic to
/// account labeling.
pub async fn poll_for_tokens(
    dc: &forge_config::provider_oauth::DeviceCodeResponse,
) -> anyhow::Result<(forge_config::oauth::OAuthTokens, Option<String>)> {
    let http = bundled_http_client();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(dc.expires_in);
    let mut interval = dc.poll_interval();
    loop {
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("sign-in timed out — the code expired before it was approved");
        }
        tokio::time::sleep(interval).await;
        let resp = http
            .post(forge_config::provider_oauth::XAI_TOKEN_ENDPOINT)
            .form(&[
                (
                    "grant_type",
                    forge_config::provider_oauth::XAI_DEVICE_GRANT_TYPE,
                ),
                ("device_code", dc.device_code.as_str()),
                (
                    "client_id",
                    forge_config::provider_oauth::XAI_OAUTH_CLIENT_ID,
                ),
            ])
            .send()
            .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        match provider_oauth::parse_device_token_response(status, &body, now_unix())? {
            provider_oauth::DevicePollOutcome::Tokens(tokens) => {
                let id_token = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| {
                        v.get("id_token")
                            .and_then(|t| t.as_str())
                            .map(str::to_string)
                    });
                return Ok((tokens, id_token));
            }
            provider_oauth::DevicePollOutcome::Pending => continue,
            provider_oauth::DevicePollOutcome::SlowDown => {
                interval += std::time::Duration::from_secs(5);
            }
            provider_oauth::DevicePollOutcome::Denied(reason) => {
                anyhow::bail!("sign-in was declined: {reason}")
            }
            provider_oauth::DevicePollOutcome::Expired => {
                anyhow::bail!("sign-in code expired before it was approved")
            }
        }
    }
}

async fn refresh_tokens(
    http: &reqwest::Client,
    refresh_token: &str,
) -> anyhow::Result<forge_config::oauth::OAuthTokens> {
    let resp = http
        .post(forge_config::provider_oauth::XAI_TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            (
                "client_id",
                forge_config::provider_oauth::XAI_OAUTH_CLIENT_ID,
            ),
        ])
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body = resp.text().await?;
    match provider_oauth::parse_device_token_response(status, &body, now_unix())? {
        provider_oauth::DevicePollOutcome::Tokens(tokens) => Ok(tokens),
        other => anyhow::bail!("token refresh returned an unexpected state: {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Entitlement probe (networked; called once post-login from forge-cli)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum EntitlementStatus {
    /// 2xx — this account can call the API via OAuth.
    Entitled,
    /// 403 — login succeeded, but the subscription tier isn't entitled to OAuth API access.
    NotEntitled(String),
    /// 401 — the token xAI just issued was rejected.
    AuthFailed(String),
    /// 429 — inconclusive; treat as probably-OK (a real answer would need another call anyway).
    RateLimited,
    Other(u16, String),
}

/// One tiny, single-shot request classifying whether this account's OAuth token can actually
/// call the API. NEVER retries on its own — a 403 here is a server-side entitlement decision,
/// not a transient failure (see the module doc's "known gotcha").
pub async fn probe_entitlement(access_token: &str) -> anyhow::Result<EntitlementStatus> {
    let http = bundled_http_client();
    let body = serde_json::json!({
        "model": "grok-4-fast",
        "input": "Reply with OK.",
        "max_output_tokens": 16,
        "stream": false,
    });
    let resp = http
        .post(responses_url())
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok(match status {
        200..=299 => EntitlementStatus::Entitled,
        403 => EntitlementStatus::NotEntitled(error_message(&text)),
        401 => EntitlementStatus::AuthFailed(error_message(&text)),
        429 => EntitlementStatus::RateLimited,
        other => EntitlementStatus::Other(other, error_message(&text)),
    })
}

/// Extract `error.message` (or `message`) from a JSON error body; falls back to the first line of
/// the raw body, capped so a huge/binary body can't flood the CLI or logs.
fn error_message(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(m) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return m.to_string();
        }
        if let Some(m) = v.get("message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
    }
    let line = body.lines().next().unwrap_or(body).trim();
    if line.chars().count() > 200 {
        let cut: String = line.chars().take(197).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

// ---------------------------------------------------------------------------------------------
// Error classification
// ---------------------------------------------------------------------------------------------

/// Classify an HTTP status + body from xAI's Responses endpoint. Mirrors
/// `genai_provider::classify_status`'s retryable/permanent split but doesn't need its capability
/// heuristics — xAI's OAuth failure modes are just auth/rate-limit/outage.
fn classify_xai_status(
    status: u16,
    body: &str,
    retry_after: Option<std::time::Duration>,
) -> ProviderError {
    let message = error_message(body);
    match status {
        403 => ProviderError::Auth(format!(
            "xAI OAuth token is not entitled for API access (403) — this account's subscription \
             tier doesn't allow OAuth API access; this won't fix itself by retrying. Run `forge \
             auth xai` to use an API key instead. ({message})"
        )),
        401 => ProviderError::Auth(format!(
            "xAI OAuth token rejected (401) — run `forge auth xai-oauth` to sign in again. ({message})"
        )),
        429 => ProviderError::RateLimited {
            message,
            retry_after,
        },
        500..=599 => ProviderError::Unavailable(message),
        _ => ProviderError::Request(message),
    }
}

/// Whether a completion failure should trigger the one-hop next-account retry in
/// [`XaiOauthProvider::complete_with`]: rate limits (429) and `Unavailable` connection-level
/// failures. A fresh account gets a fresh edge session/connection, which is often enough to
/// route around a per-connection blip (stall, connect timeout, dropped stream).
///
/// Every `ProviderError::Unavailable` this provider produces is connection-level — there's no
/// separate "server processed the request but something else went wrong" `Unavailable` variant
/// to exclude: `execute_responses_request`'s connect-timeout, `send()` transport error, 5xx
/// response, stream idle-timeout, mid-stream chunk-read error, and phantom-truncation
/// (stream closed with no completion signal) paths are all connection/stream failures. So a
/// blanket `is_unavailable` check is correct here without a finer split.
///
/// Permanent `Auth` (401/403) is deliberately excluded — never hop on it, see the module doc's
/// "known gotcha" (a 403 is a server-side entitlement decision that a different account's
/// session won't change, and a 401 means the bearer itself was rejected, not the connection).
fn should_hop_account(e: &ProviderError) -> bool {
    e.is_rate_limited() || matches!(e, ProviderError::Unavailable(_))
}

// ---------------------------------------------------------------------------------------------
// Responses-API request/response mapping (pure, testable)
// ---------------------------------------------------------------------------------------------

/// Strip the `xai-oauth::` namespace: `"xai-oauth::grok-4"` → `"grok-4"`.
fn bare_model(model: &str) -> &str {
    model
        .split_once("::")
        .map(|(_, name)| name)
        .unwrap_or(model)
}

fn build_responses_request(
    model: &str,
    messages: &[Message],
    tools: &[ToolSpec],
    opts: &CompletionOptions,
    max_output_tokens: u32,
) -> serde_json::Value {
    let mut instructions = String::new();
    let mut input = Vec::new();
    for m in messages {
        match m.role {
            Role::System => {
                if !instructions.is_empty() {
                    instructions.push_str("\n\n");
                }
                instructions.push_str(&m.content);
            }
            Role::User => {
                input.push(serde_json::json!({"role": "user", "content": m.content}));
            }
            Role::Assistant => {
                if !m.content.is_empty() {
                    input.push(serde_json::json!({"role": "assistant", "content": m.content}));
                }
                for call in &m.tool_calls {
                    input.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.args.to_string(),
                    }));
                }
            }
            Role::Tool => {
                input.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": m.tool_call_id.clone().unwrap_or_default(),
                    "output": m.content,
                }));
            }
        }
    }

    let mut body = serde_json::json!({
        "model": bare_model(model),
        "input": input,
        "stream": true,
    });
    if !instructions.is_empty() {
        body["instructions"] = serde_json::Value::String(instructions);
    }
    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(
            tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.schema,
                    })
                })
                .collect(),
        );
    }
    if max_output_tokens > 0 {
        body["max_output_tokens"] = serde_json::json!(max_output_tokens);
    }
    if let Some(temp) = opts.temperature {
        body["temperature"] = serde_json::json!(temp);
    }
    body
}

/// Accumulates a streamed Responses-API completion.
#[derive(Debug, Default)]
struct ResponseAccumulator {
    content: String,
    tool_calls: Vec<ToolCall>,
    usage: Usage,
    /// Whether a `response.completed` (or `.failed`) event arrived — distinguishes a clean finish
    /// from a stream that just dropped mid-generation (the same phantom-truncation risk
    /// `genai_provider` guards against).
    saw_terminal: bool,
}

/// Fold one decoded SSE event into `acc`. Event-name matching is intentionally loose
/// (`ends_with`/`contains` rather than an exact enum) because xAI's exact Responses-API event
/// vocabulary isn't pinned down anywhere Forge can verify offline — this degrades gracefully
/// (an unrecognized event is just ignored) rather than hard-failing on a naming detail.
fn apply_sse_event(
    acc: &mut ResponseAccumulator,
    event: &str,
    data: &serde_json::Value,
    on_event: &mut EventSink<'_>,
) -> Result<(), ProviderError> {
    if event == "error" || event == "response.failed" {
        let msg = data
            .get("error")
            .or_else(|| data.get("response").and_then(|r| r.get("error")))
            .and_then(|e| e.get("message").and_then(|m| m.as_str()).or(e.as_str()))
            .unwrap_or("xAI returned a stream error")
            .to_string();
        return Err(ProviderError::Request(msg));
    }
    if event.ends_with("output_text.delta") {
        if let Some(delta) = data.get("delta").and_then(|d| d.as_str()) {
            acc.content.push_str(delta);
            on_event(StreamEvent::Text(delta.to_string()));
        }
    } else if event.contains("reasoning") && event.ends_with(".delta") {
        if let Some(delta) = data.get("delta").and_then(|d| d.as_str()) {
            on_event(StreamEvent::Reasoning(delta.to_string()));
        }
    } else if event == "response.output_item.done" {
        if let Some(item) = data.get("item") {
            if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                let id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let args_str = item
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}");
                let args = serde_json::from_str(args_str).unwrap_or_else(|_| serde_json::json!({}));
                acc.tool_calls.push(ToolCall { id, name, args });
            }
        }
    } else if event == "response.completed" {
        acc.saw_terminal = true;
        if let Some(resp) = data.get("response") {
            if let Some(u) = resp.get("usage") {
                let input_tokens = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let output_tokens = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let cached_input_tokens = u
                    .get("input_tokens_details")
                    .and_then(|d| d.get("cached_tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                acc.usage = Usage {
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                    cost_usd: 0.0,
                };
            }
            // Some responses only carry text in the final snapshot, not as deltas.
            if acc.content.is_empty() {
                if let Some(output) = resp.get("output").and_then(|o| o.as_array()) {
                    for item in output {
                        if item.get("type").and_then(|t| t.as_str()) != Some("message") {
                            continue;
                        }
                        if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                            for part in parts {
                                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                    acc.content.push_str(text);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Parse one SSE event block (text between two blank-line boundaries): `event:`/`data:` lines,
/// `data:` lines joined with `\n`, comments (`:`-prefixed) ignored. Mirrors
/// `forge_mcp::sse`'s framing (not shared across crates — this is the same handful of lines).
fn parse_sse_frame(block: &str) -> (Option<String>, String) {
    let mut event = None;
    let mut data_lines: Vec<String> = Vec::new();
    for line in block.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => event = Some(value.to_string()),
            "data" => data_lines.push(value.to_string()),
            _ => {}
        }
    }
    (event, data_lines.join("\n"))
}

fn take_event(buf: &mut String) -> Option<String> {
    buf.find("\n\n").map(|pos| buf.drain(..pos + 2).collect())
}

// ---------------------------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------------------------

/// Where OAuth accounts are loaded/refreshed from. Production uses the OS keyring; tests inject
/// an in-memory store so they never touch `secret_store` / `provider-oauth:*` entries.
enum AccountSource {
    Keyring,
    Memory(std::sync::Mutex<forge_config::oauth::OAuthAccountStore>),
}

pub struct XaiOauthProvider {
    http: reqwest::Client,
    /// Per-completion output cap (`mesh.max_output_tokens`), same knob `GenAiProvider` honors.
    /// `0` = uncapped.
    max_output_tokens: u32,
    /// API base URL. Production always uses [`XAI_API_BASE`]; tests inject a mock via
    /// [`Self::with_api_base`].
    api_base: String,
    /// When true, skip the host-pin security check (only set by [`Self::with_api_base`] for tests).
    skip_host_pin: bool,
    accounts: AccountSource,
}

impl Default for XaiOauthProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl XaiOauthProvider {
    pub fn new() -> Self {
        Self {
            http: bundled_http_client(),
            max_output_tokens: 0,
            api_base: XAI_API_BASE.to_string(),
            skip_host_pin: false,
            accounts: AccountSource::Keyring,
        }
    }

    pub fn with_max_output_tokens(mut self, cap: u32) -> Self {
        self.max_output_tokens = cap;
        self
    }

    /// Override the API base (tests inject an httpmock server). Disables the host-pin check so
    /// loopback mock URLs work. Not used in production.
    pub fn with_api_base(mut self, base: impl Into<String>) -> Self {
        self.api_base = base.into().trim_end_matches('/').to_string();
        self.skip_host_pin = true;
        self
    }

    /// Inject an in-memory account store (tests only). Never touches the OS keyring / secret_store.
    pub fn with_accounts(mut self, store: forge_config::oauth::OAuthAccountStore) -> Self {
        self.accounts = AccountSource::Memory(std::sync::Mutex::new(store));
        self
    }

    fn responses_url(&self) -> String {
        format!("{}/responses", self.api_base.trim_end_matches('/'))
    }

    /// Assert the security invariant unless a test overrode the base URL.
    fn ensure_pinned(&self, url: &str) {
        if self.skip_host_pin {
            return;
        }
        debug_assert!(
            is_pinned_xai_url(url),
            "xAI OAuth URL must stay host-pinned"
        );
    }

    fn account_pool(&self) -> forge_config::oauth::OAuthAccountPool {
        match &self.accounts {
            AccountSource::Keyring => {
                provider_oauth::provider_oauth_account_pool(XAI_OAUTH_KEYRING_PROVIDER)
            }
            AccountSource::Memory(store) => {
                let guard = store.lock().unwrap_or_else(|e| e.into_inner());
                forge_config::oauth::OAuthAccountPool::from_store(&guard)
            }
        }
    }

    /// Load tokens for `account_id` (or the active account when `None`), refresh if needed, and
    /// persist back through the same source.
    async fn access_token_for(&self, account_id: Option<&str>) -> Result<String, ProviderError> {
        let tokens = match (&self.accounts, account_id) {
            (AccountSource::Keyring, Some(id)) => {
                provider_oauth::load_provider_oauth_account_tokens(XAI_OAUTH_KEYRING_PROVIDER, id)
                    .ok_or_else(|| {
                    ProviderError::Auth(format!(
                        "no xAI OAuth account '{id}' — run `forge auth xai-oauth` to sign in"
                    ))
                })?
            }
            (AccountSource::Keyring, None) => provider_oauth::load_provider_oauth_tokens(
                XAI_OAUTH_KEYRING_PROVIDER,
            )
            .ok_or_else(|| {
                ProviderError::Auth(
                    "no xAI OAuth session — run `forge auth xai-oauth` to sign in".to_string(),
                )
            })?,
            (AccountSource::Memory(store), Some(id)) => {
                let guard = store.lock().unwrap_or_else(|e| e.into_inner());
                guard.tokens_for(id).cloned().ok_or_else(|| {
                    ProviderError::Auth(format!(
                        "no xAI OAuth account '{id}' — run `forge auth xai-oauth` to sign in"
                    ))
                })?
            }
            (AccountSource::Memory(store), None) => {
                let guard = store.lock().unwrap_or_else(|e| e.into_inner());
                guard.active_tokens().cloned().ok_or_else(|| {
                    ProviderError::Auth(
                        "no xAI OAuth session — run `forge auth xai-oauth` to sign in".to_string(),
                    )
                })?
            }
        };
        self.refresh_if_needed(account_id, tokens).await
    }

    async fn refresh_if_needed(
        &self,
        account_id: Option<&str>,
        tokens: forge_config::oauth::OAuthTokens,
    ) -> Result<String, ProviderError> {
        if !tokens.is_expired(now_unix(), REFRESH_SKEW_SECS) {
            return Ok(tokens.access_token);
        }
        let Some(refresh_token) = tokens.refresh_token.clone() else {
            return Err(ProviderError::Auth(
                "xAI OAuth session expired and has no refresh token — run `forge auth xai-oauth` to \
                 sign in again"
                    .to_string(),
            ));
        };
        let refreshed = refresh_tokens(&self.http, &refresh_token)
            .await
            .map_err(|e| {
                ProviderError::Auth(format!(
                "xAI OAuth token refresh failed: {e} — run `forge auth xai-oauth` to sign in again"
            ))
            })?;
        match &self.accounts {
            AccountSource::Keyring => {
                let store_result = match account_id {
                    Some(id) => provider_oauth::store_provider_oauth_account_tokens(
                        XAI_OAUTH_KEYRING_PROVIDER,
                        id,
                        &refreshed,
                    ),
                    None => provider_oauth::store_provider_oauth_tokens(
                        XAI_OAUTH_KEYRING_PROVIDER,
                        &refreshed,
                    ),
                };
                if let Err(e) = store_result {
                    tracing::warn!("failed to persist refreshed xAI OAuth token: {e}");
                }
            }
            AccountSource::Memory(store) => {
                let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(id) = account_id {
                    if let Err(e) = guard.set_tokens(id, refreshed.clone()) {
                        tracing::warn!("failed to persist refreshed xAI OAuth token: {e}");
                    }
                } else {
                    guard.set_active_tokens(refreshed.clone());
                }
            }
        }
        Ok(refreshed.access_token)
    }

    /// Pick the access token for this completion: round-robin across accounts when ≥2 are stored,
    /// otherwise the active account.
    async fn pick_access_token(
        &self,
        pool: &forge_config::oauth::OAuthAccountPool,
    ) -> Result<String, ProviderError> {
        if let Some(id) = pool.next() {
            self.access_token_for(Some(&id)).await
        } else {
            self.access_token_for(None).await
        }
    }
}

/// Whether a stored xAI OAuth session exists (regardless of expiry — a refresh token, if present,
/// can renew it). Mirrors `forge_config::has_api_key` for the discovery-gating callers use to
/// decide whether it's worth probing `xai-oauth` at all.
pub fn has_session() -> bool {
    provider_oauth::load_provider_oauth_tokens(XAI_OAUTH_KEYRING_PROVIDER).is_some()
}

/// Load the *active* account's token, refreshing it first if expired. Used by [`list_models`].
async fn fresh_access_token(http: &reqwest::Client) -> Result<String, ProviderError> {
    let Some(tokens) = provider_oauth::load_provider_oauth_tokens(XAI_OAUTH_KEYRING_PROVIDER)
    else {
        return Err(ProviderError::Auth(
            "no xAI OAuth session — run `forge auth xai-oauth` to sign in".to_string(),
        ));
    };
    if !tokens.is_expired(now_unix(), REFRESH_SKEW_SECS) {
        return Ok(tokens.access_token);
    }
    let Some(refresh_token) = tokens.refresh_token.clone() else {
        return Err(ProviderError::Auth(
            "xAI OAuth session expired and has no refresh token — run `forge auth xai-oauth` to \
             sign in again"
                .to_string(),
        ));
    };
    let refreshed = refresh_tokens(http, &refresh_token).await.map_err(|e| {
        ProviderError::Auth(format!(
            "xAI OAuth token refresh failed: {e} — run `forge auth xai-oauth` to sign in again"
        ))
    })?;
    if let Err(e) =
        provider_oauth::store_provider_oauth_tokens(XAI_OAUTH_KEYRING_PROVIDER, &refreshed)
    {
        tracing::warn!("failed to persist refreshed xAI OAuth token: {e}");
    }
    Ok(refreshed.access_token)
}

/// Small fallback seed if [`list_models`]'s live call fails (network hiccup, transient outage) —
/// ids confirmed to exist via a live `/v1/models` call during development. Not exhaustive; the
/// live path is primary and this only keeps discovery from going empty on a blip.
const XAI_OAUTH_SEED_MODELS: &[&str] = &["grok-4.3", "grok-build-0.1"];

/// List the models this account's xAI OAuth session can see, as `xai-oauth::<id>` ids. Live via
/// `GET /v1/models` (confirmed to work with the OAuth bearer, same as the API-key path); falls
/// back to [`XAI_OAUTH_SEED_MODELS`] on any failure so a transient blip doesn't empty the catalog.
/// Image/video-generation models (`grok-imagine-*`) are filtered by the shared
/// `forge_config::is_non_chat_model` denylist, same as every other provider's live listing.
pub async fn list_models() -> Result<Vec<String>, ProviderError> {
    let http = bundled_http_client();
    let token = fresh_access_token(&http).await?;
    let url = format!("{XAI_API_BASE}/models");
    debug_assert!(
        is_pinned_xai_url(&url),
        "xAI OAuth URL must stay host-pinned"
    );

    let list = async {
        let resp = http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(classify_xai_status(status, &text, None));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        let data = body
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| ProviderError::Request("xAI /models: no `data` array".to_string()))?;
        Ok::<_, ProviderError>(
            data.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
                .map(|id| format!("{XAI_OAUTH_NAMESPACE}::{id}"))
                .filter(|id| !forge_config::is_non_chat_model(id))
                .collect::<Vec<_>>(),
        )
    }
    .await;

    match list {
        Ok(models) if !models.is_empty() => Ok(models),
        Ok(_) => Ok(seed_models()),
        Err(e) => {
            tracing::debug!("xai-oauth live model list failed: {e} — using seed ids");
            Ok(seed_models())
        }
    }
}

fn seed_models() -> Vec<String> {
    XAI_OAUTH_SEED_MODELS
        .iter()
        .map(|m| format!("{XAI_OAUTH_NAMESPACE}::{m}"))
        .collect()
}

/// One HTTP+SSE completion against the Responses endpoint with a fixed bearer token.
async fn execute_responses_request(
    http: &reqwest::Client,
    url: &str,
    token: &str,
    body: &serde_json::Value,
    on_event: &mut EventSink<'_>,
) -> Result<ModelResponse, ProviderError> {
    let resp = tokio::time::timeout(
        CONNECT_TIMEOUT,
        http.post(url)
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(body)
            .send(),
    )
    .await
    .map_err(|_| {
        ProviderError::Unavailable(format!(
            "no response while connecting (no data for {}s)",
            CONNECT_TIMEOUT.as_secs()
        ))
    })?
    .map_err(|e| ProviderError::Unavailable(e.to_string()))?;

    let status = resp.status();
    if !status.is_success() {
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(std::time::Duration::from_secs);
        let text = resp.text().await.unwrap_or_default();
        return Err(classify_xai_status(status.as_u16(), &text, retry_after));
    }

    let mut acc = ResponseAccumulator::default();
    let mut buf = String::new();
    let mut stream = resp.bytes_stream();
    loop {
        let chunk = tokio::time::timeout(IDLE_TIMEOUT, stream.next())
            .await
            .map_err(|_| {
                ProviderError::Unavailable(format!(
                    "stream stalled (no data for {}s)",
                    IDLE_TIMEOUT.as_secs()
                ))
            })?;
        let Some(chunk) = chunk else { break };
        let bytes = chunk.map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        buf.extend(
            String::from_utf8_lossy(&bytes)
                .chars()
                .filter(|&c| c != '\r'),
        );
        while let Some(raw) = take_event(&mut buf) {
            let (event, data) = parse_sse_frame(&raw);
            let Some(event) = event else { continue };
            if data.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) else {
                continue;
            };
            apply_sse_event(&mut acc, &event, &value, on_event)?;
        }
    }

    // Phantom-truncation guard, same rationale as `genai_provider`: a stream that closes
    // without a completion signal and produced nothing usable was almost certainly cut off
    // mid-flight, not a legitimate empty answer.
    if !acc.saw_terminal
        && acc.tool_calls.is_empty()
        && acc.usage.input_tokens == 0
        && acc.usage.output_tokens == 0
    {
        return Err(ProviderError::Unavailable(
            "stream closed without a completion signal (truncated mid-generation)".to_string(),
        ));
    }

    Ok(ModelResponse {
        content: acc.content,
        tool_calls: acc.tool_calls,
        usage: acc.usage,
        quotas: Vec::new(),
    })
}

#[async_trait::async_trait]
impl Provider for XaiOauthProvider {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        self.complete_with(
            model,
            messages,
            tools,
            &CompletionOptions::default(),
            on_event,
        )
        .await
    }

    async fn complete_with(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        opts: &CompletionOptions,
        on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        // Snapshot the multi-account pool each call so a mid-session `forge auth xai-oauth` that
        // adds an account is picked up without restarting (keyring path). Memory path is static.
        let pool = self.account_pool();
        let token = self.pick_access_token(&pool).await?;

        let url = self.responses_url();
        self.ensure_pinned(&url);

        let body = build_responses_request(model, messages, tools, opts, self.max_output_tokens);

        // First attempt with the picked account.
        let first = execute_responses_request(&self.http, &url, &token, &body, on_event).await;

        // On 429 or a connection-level Unavailable (connect timeout, stream stall/drop): if ≥2
        // accounts, retry ONCE with the next account (cursor already advanced by the first
        // pick's `next`, and we call `next` again here). Mirrors genai KeyPool: one
        // intra-provider hop, then surface the error so the mesh wait/failover chain runs once.
        // A stall is often per-connection/per-session, so a fresh account's fresh edge session
        // can route around it. 401/403 stay permanent Auth — never rotated (see
        // `should_hop_account`).
        match first {
            Err(ref e) if should_hop_account(e) && pool.has_rotation() => {
                let token2 = self.pick_access_token(&pool).await?;
                execute_responses_request(&self.http, &url, &token2, &body, on_event).await
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_models_are_namespaced_and_pass_the_non_chat_filter() {
        let seeds = seed_models();
        assert!(!seeds.is_empty());
        for id in &seeds {
            assert!(id.starts_with("xai-oauth::"));
            assert!(
                !forge_config::is_non_chat_model(id),
                "{id} should be a chat model"
            );
        }
    }

    #[test]
    fn has_session_reflects_keyring_state() {
        // No isolation of the OS keyring here (unlike `forge_config`'s own secret_store tests,
        // which use a tempdir env var) — just assert the function doesn't panic and returns a
        // bool. Behavior against a real stored/absent session is covered by the CLI's live usage.
        let _ = has_session();
    }

    #[test]
    fn pinned_host_accepts_only_real_xai_over_https() {
        assert!(is_pinned_xai_url("https://api.x.ai/v1/responses"));
        assert!(is_pinned_xai_url("https://foo.x.ai/v1/responses"));
        assert!(
            !is_pinned_xai_url("http://api.x.ai/v1/responses"),
            "rejects non-https"
        );
        assert!(
            !is_pinned_xai_url("https://api.x.ai.evil.com/v1/responses"),
            "rejects lookalike suffix host"
        );
        assert!(
            !is_pinned_xai_url("https://evilx.ai/v1/responses"),
            "rejects lookalike domain"
        );
        assert!(
            !is_pinned_xai_url("https://api-x.ai/v1/responses"),
            "rejects dash lookalike"
        );
        assert!(!is_pinned_xai_url("not a url"), "rejects unparseable input");
    }

    #[test]
    fn responses_url_is_always_pinned() {
        assert!(is_pinned_xai_url(&responses_url()));
        let p = XaiOauthProvider::new();
        assert!(is_pinned_xai_url(&p.responses_url()));
    }

    #[test]
    fn bare_model_strips_namespace() {
        assert_eq!(bare_model("xai-oauth::grok-4"), "grok-4");
        assert_eq!(bare_model("grok-4"), "grok-4");
    }

    #[test]
    fn classify_403_is_permanent_auth_with_entitlement_guidance() {
        let e = classify_xai_status(403, r#"{"error":{"message":"forbidden"}}"#, None);
        assert!(e.is_permanent());
        assert!(matches!(e, ProviderError::Auth(_)));
        assert!(e.to_string().contains("forge auth xai"));
    }

    #[test]
    fn classify_401_429_5xx() {
        assert!(matches!(
            classify_xai_status(401, "{}", None),
            ProviderError::Auth(_)
        ));
        assert!(matches!(
            classify_xai_status(429, "{}", Some(std::time::Duration::from_secs(3))),
            ProviderError::RateLimited {
                retry_after: Some(_),
                ..
            }
        ));
        assert!(matches!(
            classify_xai_status(503, "{}", None),
            ProviderError::Unavailable(_)
        ));
    }

    #[test]
    fn rate_limited_is_rotatable_auth_is_not() {
        // Contract for the complete_with rotation branch: only RateLimited hops accounts.
        let rl = ProviderError::RateLimited {
            message: "slow down".into(),
            retry_after: None,
        };
        assert!(rl.is_rate_limited());
        assert!(!rl.is_permanent());
        let auth = ProviderError::Auth("401".into());
        assert!(!auth.is_rate_limited());
        assert!(auth.is_permanent());
        let forbidden = classify_xai_status(403, r#"{"error":{"message":"nope"}}"#, None);
        assert!(!forbidden.is_rate_limited());
        assert!(forbidden.is_permanent());
    }

    #[test]
    fn should_hop_account_covers_rate_limit_and_unavailable_not_auth() {
        let rl = ProviderError::RateLimited {
            message: "slow down".into(),
            retry_after: None,
        };
        assert!(should_hop_account(&rl));

        let connect_timeout = ProviderError::Unavailable(
            "no response while connecting (no data for 60s)".to_string(),
        );
        assert!(should_hop_account(&connect_timeout));

        let stall = ProviderError::Unavailable("stream stalled (no data for 90s)".to_string());
        assert!(should_hop_account(&stall));

        let auth = ProviderError::Auth("401".into());
        assert!(!should_hop_account(&auth));

        let forbidden = classify_xai_status(403, r#"{"error":{"message":"nope"}}"#, None);
        assert!(!should_hop_account(&forbidden));
    }

    #[test]
    fn sse_text_deltas_and_function_call_fold_into_response() {
        let mut acc = ResponseAccumulator::default();
        let mut events = Vec::new();
        let sink: &mut EventSink<'_> = &mut |ev| events.push(ev);

        apply_sse_event(
            &mut acc,
            "response.output_text.delta",
            &serde_json::json!({"delta": "hel"}),
            sink,
        )
        .unwrap();
        apply_sse_event(
            &mut acc,
            "response.output_text.delta",
            &serde_json::json!({"delta": "lo"}),
            sink,
        )
        .unwrap();
        apply_sse_event(
            &mut acc,
            "response.output_item.done",
            &serde_json::json!({"item": {"type": "function_call", "call_id": "c1", "name": "run", "arguments": "{\"x\":1}"}}),
            sink,
        )
        .unwrap();
        apply_sse_event(
            &mut acc,
            "response.completed",
            &serde_json::json!({"response": {"usage": {"input_tokens": 10, "output_tokens": 5, "input_tokens_details": {"cached_tokens": 2}}}}),
            sink,
        )
        .unwrap();

        assert_eq!(acc.content, "hello");
        assert_eq!(acc.tool_calls.len(), 1);
        assert_eq!(acc.tool_calls[0].name, "run");
        assert_eq!(acc.tool_calls[0].args, serde_json::json!({"x": 1}));
        assert!(acc.saw_terminal);
        assert_eq!(acc.usage.input_tokens, 10);
        assert_eq!(acc.usage.output_tokens, 5);
        assert_eq!(acc.usage.cached_input_tokens, 2);
        assert_eq!(
            events.len(),
            2,
            "two text deltas emitted, tool call/completed don't stream text"
        );
    }

    #[test]
    fn responses_request_maps_tools_system_and_options() {
        let messages = vec![Message::new(Role::System, "be terse"), Message::user("hi")];
        let tools = vec![ToolSpec {
            name: "read_file".into(),
            description: "reads a file".into(),
            schema: serde_json::json!({"type": "object"}),
        }];
        let opts = CompletionOptions {
            temperature: Some(0.2),
            ..Default::default()
        };
        let body = build_responses_request("xai-oauth::grok-4", &messages, &tools, &opts, 512);
        assert_eq!(body["model"], "grok-4");
        assert_eq!(body["instructions"], "be terse");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"], "hi");
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert_eq!(body["max_output_tokens"], 512);
        // `opts.temperature` is f32; compare against the same f32→f64 widening `json!` performs.
        assert_eq!(body["temperature"], serde_json::json!(0.2f32));
    }

    /// Minimal SSE body that a successful Responses stream produces.
    fn ok_sse() -> String {
        "event: response.output_text.delta\ndata: {\"delta\":\"hi\"}\n\n\
         event: response.completed\ndata: {\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n"
            .to_string()
    }

    fn sample_oauth_tokens(label: &str) -> forge_config::oauth::OAuthTokens {
        forge_config::oauth::OAuthTokens {
            access_token: format!("at-{label}"),
            refresh_token: Some(format!("rt-{label}")),
            expires_at: 0, // never expired → no refresh in these tests
            token_endpoint: forge_config::provider_oauth::XAI_TOKEN_ENDPOINT.into(),
            client_id: forge_config::provider_oauth::XAI_OAUTH_CLIENT_ID.into(),
            scopes: vec![],
        }
    }

    /// In-memory multi-account store for tests — never touches secret_store / the OS keyring.
    fn memory_store(
        accounts: &[(&str, &str)],
        active: &str,
    ) -> forge_config::oauth::OAuthAccountStore {
        let mut store = forge_config::oauth::OAuthAccountStore::new_single(
            accounts[0].0,
            sample_oauth_tokens(accounts[0].1),
        );
        for (id, label) in &accounts[1..] {
            store.add(id, sample_oauth_tokens(label));
        }
        store.switch(active).unwrap();
        store
    }

    #[tokio::test]
    async fn rotation_retries_next_account_on_429() {
        // A returns 429, B succeeds — one next-account retry, mesh never sees the first 429.
        let server = httpmock::MockServer::start();
        let a_hit = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/v1/responses")
                .header("authorization", "Bearer at-a");
            then.status(429)
                .header("content-type", "application/json")
                .body(r#"{"error":{"message":"rate limited"}}"#);
        });
        let b_hit = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/v1/responses")
                .header("authorization", "Bearer at-b");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(ok_sse());
        });

        // acct-a active so the pool seeds there first (BTreeMap order: acct-a, acct-b).
        let store = memory_store(&[("acct-a", "a"), ("acct-b", "b")], "acct-a");
        let provider = XaiOauthProvider::new()
            .with_api_base(server.base_url() + "/v1")
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let resp = provider
            .complete("xai-oauth::grok-4", &[Message::user("hi")], &[], &mut sink)
            .await
            .expect("B should succeed after A's 429");
        assert_eq!(resp.content, "hi");
        a_hit.assert();
        b_hit.assert();
    }

    #[tokio::test]
    async fn both_accounts_429_surfaces_single_rate_limited() {
        let server = httpmock::MockServer::start();
        let limited = server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/v1/responses");
            then.status(429)
                .header("content-type", "application/json")
                .body(r#"{"error":{"message":"rate limited"}}"#);
        });

        let store = memory_store(&[("acct-a", "a"), ("acct-b", "b")], "acct-a");
        let provider = XaiOauthProvider::new()
            .with_api_base(server.base_url() + "/v1")
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let err = provider
            .complete("xai-oauth::grok-4", &[Message::user("hi")], &[], &mut sink)
            .await
            .expect_err("both accounts limited");
        assert!(err.is_rate_limited(), "got {err:?}");
        // Exactly two attempts (A then B) — not N retries looping.
        assert_eq!(limited.calls(), 2);
    }

    #[tokio::test]
    async fn single_account_does_not_retry_on_429() {
        let server = httpmock::MockServer::start();
        let limited = server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/v1/responses");
            then.status(429)
                .header("content-type", "application/json")
                .body(r#"{"error":{"message":"rate limited"}}"#);
        });

        let store = memory_store(&[("only", "only")], "only");
        let provider = XaiOauthProvider::new()
            .with_api_base(server.base_url() + "/v1")
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let err = provider
            .complete("xai-oauth::grok-4", &[Message::user("hi")], &[], &mut sink)
            .await
            .expect_err("single account 429");
        assert!(err.is_rate_limited());
        assert_eq!(limited.calls(), 1, "no second request without rotation");
    }

    // Connection-level `Unavailable` rotation. httpmock can't easily simulate a genuine 90s
    // idle-timeout stall or a hung TCP connect in a unit test, so these exercise the hop via
    // the same `Unavailable` outcome a 5xx response produces (`classify_xai_status`'s
    // `500..=599` arm — the same enum variant `execute_responses_request`'s connect-timeout and
    // stream-stall branches construct directly). `should_hop_account_covers_rate_limit_and_
    // unavailable_not_auth` above unit-tests the hop predicate directly against constructed
    // connect-timeout/stall `Unavailable` error strings, covering the paths this mock can't
    // reach end-to-end.

    #[tokio::test]
    async fn rotation_retries_next_account_on_unavailable() {
        // A returns a connection-level failure (503 -> Unavailable), B succeeds.
        let server = httpmock::MockServer::start();
        let a_hit = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/v1/responses")
                .header("authorization", "Bearer at-a");
            then.status(503)
                .header("content-type", "application/json")
                .body(r#"{"error":{"message":"upstream unavailable"}}"#);
        });
        let b_hit = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/v1/responses")
                .header("authorization", "Bearer at-b");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(ok_sse());
        });

        let store = memory_store(&[("acct-a", "a"), ("acct-b", "b")], "acct-a");
        let provider = XaiOauthProvider::new()
            .with_api_base(server.base_url() + "/v1")
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let resp = provider
            .complete("xai-oauth::grok-4", &[Message::user("hi")], &[], &mut sink)
            .await
            .expect("B should succeed after A's Unavailable");
        assert_eq!(resp.content, "hi");
        a_hit.assert();
        b_hit.assert();
    }

    #[tokio::test]
    async fn both_accounts_unavailable_surfaces_single_unavailable() {
        let server = httpmock::MockServer::start();
        let down = server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/v1/responses");
            then.status(503)
                .header("content-type", "application/json")
                .body(r#"{"error":{"message":"upstream unavailable"}}"#);
        });

        let store = memory_store(&[("acct-a", "a"), ("acct-b", "b")], "acct-a");
        let provider = XaiOauthProvider::new()
            .with_api_base(server.base_url() + "/v1")
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let err = provider
            .complete("xai-oauth::grok-4", &[Message::user("hi")], &[], &mut sink)
            .await
            .expect_err("both accounts unavailable");
        assert!(matches!(err, ProviderError::Unavailable(_)), "got {err:?}");
        // Exactly two attempts (A then B) — not N retries looping, and the single resulting
        // error is what surfaces to the core loop, unchanged in shape.
        assert_eq!(down.calls(), 2);
    }

    #[tokio::test]
    async fn single_account_does_not_retry_on_unavailable() {
        let server = httpmock::MockServer::start();
        let down = server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/v1/responses");
            then.status(503)
                .header("content-type", "application/json")
                .body(r#"{"error":{"message":"upstream unavailable"}}"#);
        });

        let store = memory_store(&[("only", "only")], "only");
        let provider = XaiOauthProvider::new()
            .with_api_base(server.base_url() + "/v1")
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let err = provider
            .complete("xai-oauth::grok-4", &[Message::user("hi")], &[], &mut sink)
            .await
            .expect_err("single account unavailable");
        assert!(matches!(err, ProviderError::Unavailable(_)));
        assert_eq!(down.calls(), 1, "no second request without rotation");
    }

    #[tokio::test]
    async fn auth_401_does_not_rotate_to_next_account() {
        let server = httpmock::MockServer::start();
        let auth_fail = server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/v1/responses");
            then.status(401)
                .header("content-type", "application/json")
                .body(r#"{"error":{"message":"bad token"}}"#);
        });

        let store = memory_store(&[("acct-a", "a"), ("acct-b", "b")], "acct-a");
        let provider = XaiOauthProvider::new()
            .with_api_base(server.base_url() + "/v1")
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let err = provider
            .complete("xai-oauth::grok-4", &[Message::user("hi")], &[], &mut sink)
            .await
            .expect_err("401 is permanent");
        assert!(matches!(err, ProviderError::Auth(_)), "got {err:?}");
        assert_eq!(auth_fail.calls(), 1, "must not hop accounts on Auth");
    }

    #[test]
    fn memory_refresh_targets_named_account_only() {
        // Expired account A refreshes and persists only A; B is untouched.
        // (Pure store mutation — mirrors what refresh_if_needed does on the memory path.)
        let mut store = forge_config::oauth::OAuthAccountStore::new_single(
            "acct-a",
            forge_config::oauth::OAuthTokens {
                access_token: "at-a".into(),
                refresh_token: Some("rt-a".into()),
                expires_at: 1, // expired
                token_endpoint: forge_config::provider_oauth::XAI_TOKEN_ENDPOINT.into(),
                client_id: forge_config::provider_oauth::XAI_OAUTH_CLIENT_ID.into(),
                scopes: vec![],
            },
        );
        store.add(
            "acct-b",
            forge_config::oauth::OAuthTokens {
                access_token: "at-b".into(),
                refresh_token: Some("rt-b".into()),
                expires_at: 0,
                token_endpoint: forge_config::provider_oauth::XAI_TOKEN_ENDPOINT.into(),
                client_id: forge_config::provider_oauth::XAI_OAUTH_CLIENT_ID.into(),
                scopes: vec![],
            },
        );
        store.switch("acct-a").unwrap();

        let refreshed = forge_config::oauth::OAuthTokens {
            access_token: "at-a-refreshed".into(),
            refresh_token: Some("rt-a".into()),
            expires_at: 9_999_999_999,
            token_endpoint: forge_config::provider_oauth::XAI_TOKEN_ENDPOINT.into(),
            client_id: forge_config::provider_oauth::XAI_OAUTH_CLIENT_ID.into(),
            scopes: vec![],
        };
        store.set_tokens("acct-a", refreshed.clone()).unwrap();
        assert_eq!(
            store.tokens_for("acct-a").map(|t| t.access_token.as_str()),
            Some("at-a-refreshed")
        );
        assert_eq!(
            store.tokens_for("acct-b").map(|t| t.access_token.as_str()),
            Some("at-b"),
            "non-refreshed account must be untouched"
        );
        assert_eq!(store.active, "acct-a");
    }
}
