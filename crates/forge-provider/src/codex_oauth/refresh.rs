//! Token refresh for the ChatGPT OAuth provider: single-flight (in-process latch plus a
//! cross-process file lock), adoption of tokens a peer already rotated, and the token endpoint call.

use super::*;

impl CodexOauthProvider {
    pub(super) async fn refresh_if_needed(
        &self,
        account_id: Option<&str>,
        tokens: forge_config::oauth::OAuthTokens,
    ) -> Result<String, ProviderError> {
        if !tokens.is_expired(now_unix(), REFRESH_SKEW_SECS) {
            return Ok(tokens.access_token);
        }
        // Single-flight: only one concurrent completion per account performs the network refresh;
        // the rest await here, then re-load below and reuse the token the winner just stored.
        let lock = self.refresh_lock(account_id);
        let _guard = lock.lock().await;
        let _file_lock = self.cross_process_refresh_lock().await;
        // Re-check under the latches — a peer (this process or another) may have refreshed while we
        // waited.
        let current = self.load_tokens(account_id)?;
        if !current.is_expired(now_unix(), REFRESH_SKEW_SECS) {
            return Ok(current.access_token);
        }
        if current.refresh_token.is_none() {
            return Err(ProviderError::Auth(
                "Codex OAuth session expired and has no refresh token — run `forge auth codex-oauth` \
                 to sign in again"
                    .to_string(),
            ));
        }
        Ok(self
            .refresh_or_adopt(account_id, current)
            .await?
            .access_token)
    }

    /// Wait for the cross-process refresh lock. Proceeding without it (timeout, unsupported
    /// platform) is safe: [`Self::refresh_or_adopt`] recovers from a lost rotation race.
    async fn cross_process_refresh_lock(&self) -> Option<crate::refresh_lock::RefreshFileLock> {
        let path = self.refresh_lock_path.as_deref()?;
        let lock = crate::refresh_lock::acquire(path, std::time::Duration::from_secs(30)).await;
        if lock.is_none() {
            tracing::warn!(
                path = %path.display(),
                "could not take the Codex OAuth refresh file lock; refreshing without it"
            );
        }
        lock
    }

    /// Spend `current`'s refresh token and store the result. The refresh token rotates, so when the
    /// server rejects it a peer process may simply have spent it first: reload the stored tokens
    /// and, if they differ from the ones we used, take them instead of declaring the credential
    /// dead (which would exclude a healthy provider although the keyring is already fresh).
    async fn refresh_or_adopt(
        &self,
        account_id: Option<&str>,
        current: forge_config::oauth::OAuthTokens,
    ) -> Result<forge_config::oauth::OAuthTokens, ProviderError> {
        let mut used = current;
        let mut adopted_once = false;
        loop {
            let Some(refresh_token) = used.refresh_token.clone() else {
                return Err(ProviderError::Auth(
                    "Codex OAuth session has no refresh token — run `forge auth codex-oauth`"
                        .to_string(),
                ));
            };
            match refresh_tokens(&self.http, &self.token_endpoint, &refresh_token).await {
                Ok(refreshed) => {
                    let refreshed = carry_refresh_token(refreshed, &used);
                    self.store_refreshed(account_id, &refreshed);
                    return Ok(refreshed);
                }
                Err(rejected @ provider_oauth::TokenRefreshFailure::Rejected { .. })
                    if !adopted_once =>
                {
                    adopted_once = true;
                    let reloaded = self.load_tokens(account_id)?;
                    if reloaded.refresh_token == used.refresh_token
                        && reloaded.access_token == used.access_token
                    {
                        return Err(refresh_failure_to_provider_error(rejected));
                    }
                    tracing::info!(
                        "Codex OAuth refresh token was rotated by a peer process; using stored tokens"
                    );
                    if !reloaded.is_expired(now_unix(), REFRESH_SKEW_SECS) {
                        return Ok(reloaded);
                    }
                    used = reloaded;
                }
                Err(failure) => return Err(refresh_failure_to_provider_error(failure)),
            }
        }
    }

    /// Force-refresh the token for `account_id` (named account or the active session) after a 401 —
    /// the token was rejected even though it may not be clock-expired. Serialized by the same
    /// per-account single-flight latch as [`Self::refresh_if_needed`]: if a peer already refreshed
    /// while we waited (the stored token no longer equals `stale_token` and is unexpired), reuse it
    /// instead of spending the rotating refresh_token again.
    pub(super) async fn force_refresh_account(
        &self,
        account_id: Option<&str>,
        stale_token: &str,
    ) -> Result<(String, String), ProviderError> {
        let lock = self.refresh_lock(account_id);
        let _guard = lock.lock().await;
        let _file_lock = self.cross_process_refresh_lock().await;
        let current = self.load_tokens(account_id)?;
        if current.access_token != stale_token && !current.is_expired(now_unix(), REFRESH_SKEW_SECS)
        {
            let chatgpt_id = self.chatgpt_id_for(&current.access_token, account_id);
            return Ok((current.access_token, chatgpt_id));
        }
        if current.refresh_token.is_none() {
            return Err(ProviderError::Auth(
                "Codex OAuth 401 and no refresh token — run `forge auth codex-oauth`".to_string(),
            ));
        }
        let refreshed = self.refresh_or_adopt(account_id, current).await?;
        let chatgpt_id = self.chatgpt_id_for(&refreshed.access_token, account_id);
        Ok((refreshed.access_token, chatgpt_id))
    }
}

/// Only a token the authorization server REJECTED is an auth failure; a rate-limited, down, or
/// unreachable token endpoint leaves the credential unproven, and reporting that as `Auth` would
/// exclude a healthy subscription provider-wide. Either way the upstream status and message are
/// carried through verbatim so the user reads the real cause.
pub(super) fn refresh_failure_to_provider_error(
    failure: provider_oauth::TokenRefreshFailure,
) -> ProviderError {
    match failure {
        provider_oauth::TokenRefreshFailure::Rejected { status, message } => {
            ProviderError::Auth(format!(
                "Codex OAuth refresh token rejected (HTTP {status}: {message}) — \
                 re-authentication required: run `forge auth codex-oauth`"
            ))
        }
        transient => ProviderError::Unavailable(format!(
            "Codex OAuth token refresh could not complete ({transient}) — credential not disabled, \
             will retry"
        )),
    }
}

/// A refresh response may omit `refresh_token` (non-rotating server). Keep the previous one so the
/// stored credential stays refreshable instead of degrading to a single expiring access token.
pub(super) fn carry_refresh_token(
    mut refreshed: forge_config::oauth::OAuthTokens,
    previous: &forge_config::oauth::OAuthTokens,
) -> forge_config::oauth::OAuthTokens {
    if refreshed.refresh_token.is_none() {
        refreshed.refresh_token = previous.refresh_token.clone();
    }
    refreshed
}

pub(super) async fn refresh_tokens(
    http: &reqwest::Client,
    token_endpoint: &str,
    refresh_token: &str,
) -> Result<forge_config::oauth::OAuthTokens, provider_oauth::TokenRefreshFailure> {
    let transient = |message: String| provider_oauth::TokenRefreshFailure::Transient {
        status: None,
        message,
    };
    let resp = http
        .post(token_endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CODEX_OAUTH_CLIENT_ID),
        ])
        .send()
        .await
        .map_err(|e| transient(e.to_string()))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| transient(e.to_string()))?;
    if status != 200 {
        return Err(provider_oauth::classify_token_refresh_failure(
            status, &body,
        ));
    }
    provider_oauth::parse_codex_token_response(status, &body, now_unix()).map_err(|e| {
        provider_oauth::TokenRefreshFailure::Transient {
            status: Some(status),
            message: e.to_string(),
        }
    })
}
