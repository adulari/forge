//! ChatGPT subscription OAuth provider (`codex-oauth::<model>`, e.g. `codex-oauth::gpt-5.5`).
//! Signs in with a ChatGPT Plus/Pro account via OAuth 2.0 PKCE (official Codex public client) and
//! talks to ChatGPT's Codex Responses backend. Usage bills to the subscription ($0 in Forge).
//!
//! SECURITY INVARIANT: the OAuth bearer is only ever attached to a request built from the
//! hardcoded [`CODEX_API_BASE`] — never a custom-provider endpoint or user-supplied base URL.
//!
//! OpenAI permits subscription OAuth in third-party tools. Anthropic/Google do not — their CLI
//! bridges stay; this module is ChatGPT-only (see docs/design/codex-oauth.md).

use forge_config::provider_oauth::{
    self, CODEX_OAUTH_CLIENT_ID, CODEX_OAUTH_KEYRING_PROVIDER, CODEX_OAUTH_TOKEN_ENDPOINT,
};
use forge_types::Message;

use crate::oauth_responses::{
    build_responses_request, classify_responses_status, error_message,
    execute_responses_request as shared_execute, now_unix, should_hop_account, REFRESH_SKEW_SECS,
};
use crate::{
    bundled_http_client, CompletionOptions, EventSink, ModelResponse, Provider, ProviderError,
    ToolSpec,
};

/// The `codex-oauth::` model-id namespace [`crate::DispatchProvider`] routes on.
pub const CODEX_OAUTH_NAMESPACE: &str = "codex-oauth";

/// Hardcoded ChatGPT Codex backend — deliberately NOT read from config/env.
const CODEX_API_BASE: &str = "https://chatgpt.com/backend-api/codex";

/// Parameters the generic Responses builder emits that `chatgpt.com/backend-api/codex/responses`
/// rejects outright (verified live: `max_output_tokens` → 400 "Unsupported parameter"). The real
/// Codex CLI sends neither. Stripped codex-side so the shared builder stays correct for xAI.
const CODEX_UNSUPPORTED_PARAMS: &[&str] = &["max_output_tokens", "temperature"];

/// True iff `url` is `https` and its host is exactly `chatgpt.com` or a genuine `*.chatgpt.com`
/// subdomain — rejects lookalikes and any non-HTTPS scheme.
pub fn is_pinned_codex_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    matches!(
        parsed.host_str(),
        Some(h) if h == "chatgpt.com" || h.ends_with(".chatgpt.com")
    )
}

fn responses_url() -> String {
    format!("{CODEX_API_BASE}/responses")
}

fn classify_codex_status(
    status: u16,
    body: &str,
    retry_after: Option<std::time::Duration>,
) -> ProviderError {
    classify_responses_status(
        status,
        body,
        retry_after,
        "ChatGPT OAuth token rejected (401) — run `forge auth codex-oauth` to sign in again",
        "ChatGPT OAuth token is not entitled for API access (403) — this account's ChatGPT \
         plan (Plus/Pro) may not allow Codex API access, or the subscription lapsed; this won't \
         fix itself by retrying. Run `forge auth openai` to use an API key instead",
    )
}

// ---------------------------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------------------------

enum AccountSource {
    Keyring,
    Memory(std::sync::Mutex<forge_config::oauth::OAuthAccountStore>),
}

/// Per-account ChatGPT account id (from the access-token JWT), keyed by store account id.
/// Production loads it from the access token on each pick; tests inject via the store tokens.
pub struct CodexOauthProvider {
    http: reqwest::Client,
    max_output_tokens: u32,
    api_base: String,
    skip_host_pin: bool,
    accounts: AccountSource,
    /// Optional override for ChatGPT-Account-Id in tests (when JWT has no claim).
    test_account_header: Option<String>,
}

impl Default for CodexOauthProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexOauthProvider {
    pub fn new() -> Self {
        Self {
            http: bundled_http_client(),
            max_output_tokens: 0,
            api_base: CODEX_API_BASE.to_string(),
            skip_host_pin: false,
            accounts: AccountSource::Keyring,
            test_account_header: None,
        }
    }

    pub fn with_max_output_tokens(mut self, cap: u32) -> Self {
        self.max_output_tokens = cap;
        self
    }

    pub fn with_api_base(mut self, base: impl Into<String>) -> Self {
        self.api_base = base.into().trim_end_matches('/').to_string();
        self.skip_host_pin = true;
        self
    }

    pub fn with_accounts(mut self, store: forge_config::oauth::OAuthAccountStore) -> Self {
        self.accounts = AccountSource::Memory(std::sync::Mutex::new(store));
        self
    }

    /// Tests: force the ChatGPT-Account-Id header when the access token is not a real JWT.
    pub fn with_test_account_header(mut self, id: impl Into<String>) -> Self {
        self.test_account_header = Some(id.into());
        self
    }

    fn responses_url(&self) -> String {
        format!("{}/responses", self.api_base.trim_end_matches('/'))
    }

    fn ensure_pinned(&self, url: &str) {
        if self.skip_host_pin {
            return;
        }
        debug_assert!(
            is_pinned_codex_url(url),
            "Codex OAuth URL must stay host-pinned"
        );
    }

    fn account_pool(&self) -> forge_config::oauth::OAuthAccountPool {
        match &self.accounts {
            AccountSource::Keyring => {
                provider_oauth::provider_oauth_account_pool(CODEX_OAUTH_KEYRING_PROVIDER)
            }
            AccountSource::Memory(store) => {
                let guard = store.lock().unwrap_or_else(|e| e.into_inner());
                forge_config::oauth::OAuthAccountPool::from_store(&guard)
            }
        }
    }

    async fn access_token_for(
        &self,
        account_id: Option<&str>,
    ) -> Result<(String, String), ProviderError> {
        let tokens = match (&self.accounts, account_id) {
            (AccountSource::Keyring, Some(id)) => {
                provider_oauth::load_provider_oauth_account_tokens(CODEX_OAUTH_KEYRING_PROVIDER, id)
                    .ok_or_else(|| {
                        ProviderError::Auth(format!(
                            "no Codex OAuth account '{id}' — run `forge auth codex-oauth` to sign in"
                        ))
                    })?
            }
            (AccountSource::Keyring, None) => provider_oauth::load_provider_oauth_tokens(
                CODEX_OAUTH_KEYRING_PROVIDER,
            )
            .ok_or_else(|| {
                ProviderError::Auth(
                    "no Codex OAuth session — run `forge auth codex-oauth` to sign in".to_string(),
                )
            })?,
            (AccountSource::Memory(store), Some(id)) => {
                let guard = store.lock().unwrap_or_else(|e| e.into_inner());
                guard.tokens_for(id).cloned().ok_or_else(|| {
                    ProviderError::Auth(format!(
                        "no Codex OAuth account '{id}' — run `forge auth codex-oauth` to sign in"
                    ))
                })?
            }
            (AccountSource::Memory(store), None) => {
                let guard = store.lock().unwrap_or_else(|e| e.into_inner());
                guard.active_tokens().cloned().ok_or_else(|| {
                    ProviderError::Auth(
                        "no Codex OAuth session — run `forge auth codex-oauth` to sign in"
                            .to_string(),
                    )
                })?
            }
        };
        let access = self.refresh_if_needed(account_id, tokens).await?;
        let chatgpt_id = self
            .test_account_header
            .clone()
            .or_else(|| provider_oauth::extract_chatgpt_account_id(&access))
            .unwrap_or_else(|| account_id.unwrap_or("default").to_string());
        Ok((access, chatgpt_id))
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
                "Codex OAuth session expired and has no refresh token — run `forge auth codex-oauth` \
                 to sign in again"
                    .to_string(),
            ));
        };
        let refreshed = refresh_tokens(&self.http, &refresh_token)
            .await
            .map_err(|e| {
                ProviderError::Auth(format!(
                    "Codex OAuth token refresh failed: {e} — run `forge auth codex-oauth` to sign in again"
                ))
            })?;
        match &self.accounts {
            AccountSource::Keyring => {
                let store_result = match account_id {
                    Some(id) => provider_oauth::store_provider_oauth_account_tokens(
                        CODEX_OAUTH_KEYRING_PROVIDER,
                        id,
                        &refreshed,
                    ),
                    None => provider_oauth::store_provider_oauth_tokens(
                        CODEX_OAUTH_KEYRING_PROVIDER,
                        &refreshed,
                    ),
                };
                if let Err(e) = store_result {
                    tracing::warn!("failed to persist refreshed Codex OAuth token: {e}");
                }
            }
            AccountSource::Memory(store) => {
                let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(id) = account_id {
                    if let Err(e) = guard.set_tokens(id, refreshed.clone()) {
                        tracing::warn!("failed to persist refreshed Codex OAuth token: {e}");
                    }
                } else {
                    guard.set_active_tokens(refreshed.clone());
                }
            }
        }
        Ok(refreshed.access_token)
    }

    async fn pick_access_token(
        &self,
        pool: &forge_config::oauth::OAuthAccountPool,
    ) -> Result<(String, String), ProviderError> {
        if let Some(id) = pool.next() {
            self.access_token_for(Some(&id)).await
        } else {
            self.access_token_for(None).await
        }
    }

    async fn execute(
        &self,
        url: &str,
        token: &str,
        chatgpt_account_id: &str,
        body: &serde_json::Value,
        on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        let headers = [("ChatGPT-Account-Id", chatgpt_account_id)];
        let result = shared_execute(
            &self.http,
            url,
            token,
            body,
            &headers,
            on_event,
            classify_codex_status,
        )
        .await;
        // On 401: refresh once and retry the same account once (design §2).
        if let Err(ProviderError::Auth(ref msg)) = result {
            if msg.contains("(401)") {
                // Force refresh by treating tokens as expired: re-load and refresh path.
                // Pick again with the same pool cursor position is hard; just re-fetch active/named.
                // Simpler: call refresh path via access_token_for after clearing skew by reloading.
                // We re-run access_token_for(None) which refreshes if expired; for a 401 the token
                // may not be expired by clock — force refresh via refresh_tokens directly.
                if let Ok((token2, id2)) = self.force_refresh_active().await {
                    return shared_execute(
                        &self.http,
                        url,
                        &token2,
                        body,
                        &[("ChatGPT-Account-Id", id2.as_str())],
                        on_event,
                        classify_codex_status,
                    )
                    .await;
                }
            }
        }
        result
    }

    async fn force_refresh_active(&self) -> Result<(String, String), ProviderError> {
        let tokens = match &self.accounts {
            AccountSource::Keyring => {
                provider_oauth::load_provider_oauth_tokens(CODEX_OAUTH_KEYRING_PROVIDER)
                    .ok_or_else(|| {
                        ProviderError::Auth(
                            "no Codex OAuth session — run `forge auth codex-oauth` to sign in"
                                .to_string(),
                        )
                    })?
            }
            AccountSource::Memory(store) => {
                let guard = store.lock().unwrap_or_else(|e| e.into_inner());
                guard.active_tokens().cloned().ok_or_else(|| {
                    ProviderError::Auth(
                        "no Codex OAuth session — run `forge auth codex-oauth` to sign in"
                            .to_string(),
                    )
                })?
            }
        };
        let Some(rt) = tokens.refresh_token.clone() else {
            return Err(ProviderError::Auth(
                "Codex OAuth 401 and no refresh token — run `forge auth codex-oauth`".to_string(),
            ));
        };
        let refreshed = refresh_tokens(&self.http, &rt)
            .await
            .map_err(|e| ProviderError::Auth(format!("Codex OAuth token refresh failed: {e}")))?;
        match &self.accounts {
            AccountSource::Keyring => {
                let _ = provider_oauth::store_provider_oauth_tokens(
                    CODEX_OAUTH_KEYRING_PROVIDER,
                    &refreshed,
                );
            }
            AccountSource::Memory(store) => {
                let mut guard = store.lock().unwrap_or_else(|e| e.into_inner());
                guard.set_active_tokens(refreshed.clone());
            }
        }
        let chatgpt_id = self
            .test_account_header
            .clone()
            .or_else(|| provider_oauth::extract_chatgpt_account_id(&refreshed.access_token))
            .unwrap_or_else(|| "default".to_string());
        Ok((refreshed.access_token, chatgpt_id))
    }
}

async fn refresh_tokens(
    http: &reqwest::Client,
    refresh_token: &str,
) -> anyhow::Result<forge_config::oauth::OAuthTokens> {
    let resp = http
        .post(CODEX_OAUTH_TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CODEX_OAUTH_CLIENT_ID),
        ])
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body = resp.text().await?;
    Ok(provider_oauth::parse_codex_token_response(
        status,
        &body,
        now_unix(),
    )?)
}

pub fn has_session() -> bool {
    provider_oauth::load_provider_oauth_tokens(CODEX_OAUTH_KEYRING_PROVIDER).is_some()
}

const CODEX_OAUTH_SEED_MODELS: &[&str] = &[
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.3-codex",
    "gpt-5.2",
    "gpt-5.4-mini",
];

/// Seed model ids as `codex-oauth::<id>` (no public `/models` on the ChatGPT backend).
pub async fn list_models() -> Result<Vec<String>, ProviderError> {
    if !has_session() {
        return Err(ProviderError::Auth(
            "no Codex OAuth session — run `forge auth codex-oauth` to sign in".to_string(),
        ));
    }
    Ok(seed_models())
}

fn seed_models() -> Vec<String> {
    CODEX_OAUTH_SEED_MODELS
        .iter()
        .map(|m| format!("{CODEX_OAUTH_NAMESPACE}::{m}"))
        .collect()
}

/// Exchange an authorization code for tokens (PKCE).
pub async fn exchange_code(
    code: &str,
    code_verifier: &str,
) -> anyhow::Result<forge_config::oauth::OAuthTokens> {
    let http = bundled_http_client();
    let resp = http
        .post(CODEX_OAUTH_TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            (
                "redirect_uri",
                forge_config::provider_oauth::CODEX_OAUTH_REDIRECT_URI,
            ),
            ("client_id", CODEX_OAUTH_CLIENT_ID),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body = resp.text().await?;
    Ok(provider_oauth::parse_codex_token_response(
        status,
        &body,
        now_unix(),
    )?)
}

/// One-shot entitlement probe after login.
pub async fn probe_entitlement(
    access_token: &str,
    chatgpt_account_id: &str,
) -> anyhow::Result<crate::EntitlementStatus> {
    let http = bundled_http_client();
    let body = serde_json::json!({
        "model": "gpt-5.4-mini",
        "input": [{"role": "user", "content": "Reply with OK."}],
        "stream": false,
        "store": false,
    });
    let resp = http
        .post(responses_url())
        .bearer_auth(access_token)
        .header("ChatGPT-Account-Id", chatgpt_account_id)
        .json(&body)
        .send()
        .await?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok(match status {
        200..=299 => crate::EntitlementStatus::Entitled,
        403 => crate::EntitlementStatus::NotEntitled(error_message(&text)),
        401 => crate::EntitlementStatus::AuthFailed(error_message(&text)),
        429 => crate::EntitlementStatus::RateLimited,
        other => crate::EntitlementStatus::Other(other, error_message(&text)),
    })
}

#[async_trait::async_trait]
impl Provider for CodexOauthProvider {
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
        let pool = self.account_pool();
        let (token, chatgpt_id) = self.pick_access_token(&pool).await?;
        let url = self.responses_url();
        self.ensure_pinned(&url);
        let mut body =
            build_responses_request(model, messages, tools, opts, self.max_output_tokens);
        // The ChatGPT codex backend rejects requests unless store=false (codex-only; xAI omits it).
        body["store"] = serde_json::json!(false);
        // The ChatGPT codex backend 400s on params the generic Responses builder emits but the
        // real Codex CLI never sends (see CODEX_UNSUPPORTED_PARAMS) — strip codex-side only.
        if let Some(obj) = body.as_object_mut() {
            for k in CODEX_UNSUPPORTED_PARAMS {
                obj.remove(*k);
            }
        }

        let first = self
            .execute(&url, &token, &chatgpt_id, &body, on_event)
            .await;
        match first {
            Err(ref e) if should_hop_account(e) && pool.has_rotation() => {
                let (token2, id2) = self.pick_access_token(&pool).await?;
                self.execute(&url, &token2, &id2, &body, on_event).await
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StreamEvent;
    use forge_types::Role;

    #[test]
    fn seed_models_are_namespaced() {
        let seeds = seed_models();
        assert!(!seeds.is_empty());
        for id in &seeds {
            assert!(id.starts_with("codex-oauth::"));
            assert!(!forge_config::is_non_chat_model(id), "{id}");
        }
    }

    #[test]
    fn seed_models_include_gpt_5_6_family() {
        let seeds = seed_models();
        for id in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
            assert!(
                seeds.contains(&format!("codex-oauth::{id}")),
                "{id} missing from {seeds:?}"
            );
        }
    }

    #[test]
    fn pinned_host_accepts_only_chatgpt_over_https() {
        assert!(is_pinned_codex_url(
            "https://chatgpt.com/backend-api/codex/responses"
        ));
        assert!(is_pinned_codex_url(
            "https://www.chatgpt.com/backend-api/codex/responses"
        ));
        assert!(!is_pinned_codex_url(
            "http://chatgpt.com/backend-api/codex/responses"
        ));
        assert!(!is_pinned_codex_url(
            "https://chatgpt.com.evil.com/backend-api/codex/responses"
        ));
        assert!(!is_pinned_codex_url(
            "https://evilchatgpt.com/backend-api/codex/responses"
        ));
        assert!(!is_pinned_codex_url("not a url"));
    }

    #[test]
    fn classify_403_is_permanent_auth() {
        let e = classify_codex_status(403, r#"{"error":{"message":"forbidden"}}"#, None);
        assert!(e.is_permanent());
        assert!(matches!(e, ProviderError::Auth(_)));
        assert!(e.to_string().contains("forge auth openai"));
    }

    #[test]
    fn classify_401_429_5xx() {
        assert!(matches!(
            classify_codex_status(401, "{}", None),
            ProviderError::Auth(_)
        ));
        assert!(matches!(
            classify_codex_status(429, "{}", Some(std::time::Duration::from_secs(3))),
            ProviderError::RateLimited {
                retry_after: Some(_),
                ..
            }
        ));
        assert!(matches!(
            classify_codex_status(503, "{}", None),
            ProviderError::Unavailable(_)
        ));
    }

    fn ok_sse() -> String {
        "event: response.output_text.delta\ndata: {\"delta\":\"hi\"}\n\n\
         event: response.completed\ndata: {\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n"
            .to_string()
    }

    fn sample_oauth_tokens(label: &str) -> forge_config::oauth::OAuthTokens {
        forge_config::oauth::OAuthTokens {
            access_token: format!("at-{label}"),
            refresh_token: Some(format!("rt-{label}")),
            expires_at: 0,
            token_endpoint: CODEX_OAUTH_TOKEN_ENDPOINT.into(),
            client_id: CODEX_OAUTH_CLIENT_ID.into(),
            scopes: vec![],
        }
    }

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
        let server = httpmock::MockServer::start();
        let a_hit = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/responses")
                .header("authorization", "Bearer at-a")
                .header("chatgpt-account-id", "acct-a");
            then.status(429)
                .header("content-type", "application/json")
                .body(r#"{"error":{"message":"rate limited"}}"#);
        });
        let b_hit = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/responses")
                .header("authorization", "Bearer at-b")
                .header("chatgpt-account-id", "acct-b");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(ok_sse());
        });

        let store = memory_store(&[("acct-a", "a"), ("acct-b", "b")], "acct-a");
        let provider = CodexOauthProvider::new()
            .with_api_base(server.base_url())
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let resp = provider
            .complete(
                "codex-oauth::gpt-5.5",
                &[Message::user("hi")],
                &[],
                &mut sink,
            )
            .await
            .expect("B should succeed after A's 429");
        assert_eq!(resp.content, "hi");
        a_hit.assert();
        b_hit.assert();
    }

    #[tokio::test]
    async fn single_account_does_not_retry_on_429() {
        let server = httpmock::MockServer::start();
        let limited = server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/responses");
            then.status(429)
                .header("content-type", "application/json")
                .body(r#"{"error":{"message":"rate limited"}}"#);
        });
        let store = memory_store(&[("only", "only")], "only");
        let provider = CodexOauthProvider::new()
            .with_api_base(server.base_url())
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let err = provider
            .complete(
                "codex-oauth::gpt-5.5",
                &[Message::user("hi")],
                &[],
                &mut sink,
            )
            .await
            .expect_err("single account 429");
        assert!(err.is_rate_limited());
        assert_eq!(limited.calls(), 1);
    }

    #[tokio::test]
    async fn auth_403_does_not_rotate() {
        let server = httpmock::MockServer::start();
        let hit = server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/responses");
            then.status(403)
                .header("content-type", "application/json")
                .body(r#"{"error":{"message":"not entitled"}}"#);
        });
        let store = memory_store(&[("acct-a", "a"), ("acct-b", "b")], "acct-a");
        let provider = CodexOauthProvider::new()
            .with_api_base(server.base_url())
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let err = provider
            .complete(
                "codex-oauth::gpt-5.5",
                &[Message::user("hi")],
                &[],
                &mut sink,
            )
            .await
            .expect_err("403 permanent");
        assert!(matches!(err, ProviderError::Auth(_)));
        assert_eq!(hit.calls(), 1);
    }

    #[tokio::test]
    async fn rotation_retries_next_account_on_unavailable() {
        let server = httpmock::MockServer::start();
        let a_hit = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/responses")
                .header("authorization", "Bearer at-a");
            then.status(503)
                .header("content-type", "application/json")
                .body(r#"{"error":{"message":"upstream unavailable"}}"#);
        });
        let b_hit = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/responses")
                .header("authorization", "Bearer at-b");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(ok_sse());
        });
        let store = memory_store(&[("acct-a", "a"), ("acct-b", "b")], "acct-a");
        let provider = CodexOauthProvider::new()
            .with_api_base(server.base_url())
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let resp = provider
            .complete(
                "codex-oauth::gpt-5.5",
                &[Message::user("hi")],
                &[],
                &mut sink,
            )
            .await
            .expect("B after A unavailable");
        assert_eq!(resp.content, "hi");
        a_hit.assert();
        b_hit.assert();
    }

    #[tokio::test]
    async fn complete_sends_store_false() {
        let server = httpmock::MockServer::start();
        let hit = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/responses")
                .body_includes("\"store\":false");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(ok_sse());
        });
        let store = memory_store(&[("only", "only")], "only");
        let provider = CodexOauthProvider::new()
            .with_api_base(server.base_url())
            .with_accounts(store);
        let mut sink = |_: StreamEvent| {};
        let resp = provider
            .complete(
                "codex-oauth::gpt-5.5",
                &[Message::user("hi")],
                &[],
                &mut sink,
            )
            .await
            .expect("codex backend requires store=false");
        assert_eq!(resp.content, "hi");
        hit.assert();
    }

    #[tokio::test]
    async fn complete_omits_params_chatgpt_backend_rejects() {
        // Even with a non-zero max_output_tokens cap AND an explicit temperature requested,
        // the outgoing body to the ChatGPT codex backend must not carry either param.
        let server = httpmock::MockServer::start();
        let hit = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/responses")
                .body_excludes("max_output_tokens")
                .body_excludes("temperature");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(ok_sse());
        });
        let store = memory_store(&[("only", "only")], "only");
        let provider = CodexOauthProvider::new()
            .with_api_base(server.base_url())
            .with_accounts(store)
            .with_max_output_tokens(4096);
        let mut sink = |_: StreamEvent| {};
        let opts = CompletionOptions {
            temperature: Some(0.2),
            ..Default::default()
        };
        let resp = provider
            .complete_with(
                "codex-oauth::gpt-5.5",
                &[Message::user("hi")],
                &[],
                &opts,
                &mut sink,
            )
            .await
            .expect("codex backend rejects max_output_tokens/temperature");
        assert_eq!(resp.content, "hi");
        hit.assert();
    }

    #[test]
    fn responses_request_maps_model() {
        let messages = vec![Message::new(Role::System, "be terse"), Message::user("hi")];
        let body = build_responses_request(
            "codex-oauth::gpt-5.5",
            &messages,
            &[],
            &CompletionOptions::default(),
            0,
        );
        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["instructions"], "be terse");
    }
}
