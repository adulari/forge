//! Which models a ChatGPT account can run through `codex-oauth::`.
//!
//! The Codex backend publishes the answer itself: `GET /backend-api/codex/models` is the catalog the
//! Codex CLI refreshes into `~/.codex/models_cache.json`, and it is per account, so a model the
//! plan cannot use (`gpt-5.4-mini` is refused outright for ChatGPT logins) is simply absent. Asking
//! it is the only way a fresh install, with no Codex CLI beside it, sees models shipped after this
//! release was cut. The bundled list is a last resort for when the backend cannot be reached, not
//! the source of truth.

use std::path::Path;
use std::time::Duration;

use super::*;
use crate::cli_provider::{
    bridge_model_cache_path, codex_cli_uses_chatgpt_login, parse_codex_catalog_models,
    read_bridge_model_cache, remember_bridge_models_at,
};

/// What the backend itself listed for a ChatGPT account when this release was cut, in the order it
/// ranks them. Used only when neither the backend nor a previous answer from it is available, so it
/// must never carry an id the backend refuses for ChatGPT logins: `gpt-5.4-mini` answers
/// `400 "not supported when using Codex with a ChatGPT account"`, which is what the sign-in probe
/// reported to a user whose login had in fact succeeded.
pub(super) const SEED_MODELS: &[&str] = &[
    "gpt-6.1-sol",
    "gpt-6-astra",
    "gpt-6-sol",
    "gpt-6-luna",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
];

/// `bridge-models.json` key for the last list the backend returned. The file is keyed by provider
/// prefix; `codex-oauth` is not a bridge, but the cache, its atomic merge and its reader are exactly
/// what a "last known good list" needs.
const CACHE_KEY: &str = "codex-oauth";

/// The `client_version` the catalog is requested for. The backend gates models by the client's
/// version: verified live, `0.1.0` is answered with `{"models":[]}` and `0.163.0` with every model
/// this account can run. A model that needs a newer client stays hidden until this is raised, which
/// is the safe direction: Forge is never offered a model whose protocol it was not built against.
/// An empty answer is treated as a failed lookup and falls back to the last known list.
const CATALOG_CLIENT_VERSION: &str = "0.163.0";

/// Long enough for a slow link, short enough that the 8 s discovery budget still has room to fall
/// back to the cached list.
const CATALOG_TIMEOUT: Duration = Duration::from_secs(5);

/// One answer from the catalog endpoint, kept raw so the caller decides what a status means.
pub(super) struct CatalogReply {
    pub(super) status: u16,
    pub(super) models: Vec<String>,
    pub(super) body: String,
}

fn models_url(base: &str) -> String {
    format!("{}/models", base.trim_end_matches('/'))
}

pub(super) async fn get_catalog(
    http: &reqwest::Client,
    base: &str,
    access_token: &str,
    chatgpt_account_id: &str,
) -> Result<CatalogReply, reqwest::Error> {
    let resp = http
        .get(models_url(base))
        .query(&[("client_version", CATALOG_CLIENT_VERSION)])
        .bearer_auth(access_token)
        .header(reqwest::header::ACCEPT, "application/json")
        .header("ChatGPT-Account-Id", chatgpt_account_id)
        .timeout(CATALOG_TIMEOUT)
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body = resp.text().await?;
    let models = if (200..300).contains(&status) {
        parse_codex_catalog_models(&body)
    } else {
        Vec::new()
    };
    Ok(CatalogReply {
        status,
        models,
        body,
    })
}

impl CodexOauthProvider {
    /// The live catalog for the active session. A 401 force-refreshes that account's token and asks
    /// once more, like every other call on this provider.
    pub(super) async fn fetch_catalog(&self) -> Result<Vec<String>, ProviderError> {
        self.ensure_pinned(&models_url(&self.api_base));
        let (token, chatgpt_id) = self.access_token_for(None).await?;
        let mut reply = request_catalog(self, &token, &chatgpt_id).await?;
        if reply.status == 401 {
            let (token, chatgpt_id) = self.force_refresh_account(None, &token).await?;
            reply = request_catalog(self, &token, &chatgpt_id).await?;
        }
        if !(200..300).contains(&reply.status) {
            return Err(classify_codex_status(reply.status, &reply.body, None));
        }
        if reply.models.is_empty() {
            return Err(ProviderError::Unavailable(
                "the Codex backend listed no models".to_string(),
            ));
        }
        Ok(reply.models)
    }
}

async fn request_catalog(
    provider: &CodexOauthProvider,
    token: &str,
    chatgpt_id: &str,
) -> Result<CatalogReply, ProviderError> {
    get_catalog(&provider.http, &provider.api_base, token, chatgpt_id)
        .await
        .map_err(|e| ProviderError::Unavailable(format!("Codex model catalog request failed: {e}")))
}

/// Persist a list the backend just returned so a later failed lookup replays it.
pub(super) fn remember_listed(models: &[String]) {
    if let Some(path) = bridge_model_cache_path() {
        remember_bridge_models_at(
            &path,
            CACHE_KEY,
            models,
            u64::try_from(now_unix()).unwrap_or(0),
        );
    }
}

/// What to offer when the backend cannot be asked: the last list it gave, else the Codex CLI's
/// own list when that CLI is signed in with the same ChatGPT account, else the bundled seeds.
pub(super) fn fallback_models() -> Vec<String> {
    fallback_from(
        bridge_model_cache_path().as_deref(),
        codex_cli_uses_chatgpt_login(),
    )
}

/// An API-key Codex CLI advertises API-only models (`gpt-5.4-mini`) that a ChatGPT login refuses,
/// so its list only stands in for ours when it is signed in with a ChatGPT account.
fn fallback_from(cache: Option<&Path>, cli_uses_chatgpt: bool) -> Vec<String> {
    let cached = cache.map(read_bridge_model_cache).unwrap_or_default();
    let listed = |key: &str| {
        cached
            .get(key)
            .map(|entry| entry.models.clone())
            .filter(|models| !models.is_empty())
    };
    listed(CACHE_KEY)
        .or_else(|| cli_uses_chatgpt.then(|| listed("codex-cli")).flatten())
        .unwrap_or_else(|| SEED_MODELS.iter().map(|m| (*m).to_string()).collect())
}

/// `codex-oauth::<id>` for each distinct non-empty id, in the given order.
pub(super) fn namespaced(models: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for model in models {
        let model = model.trim();
        if model.is_empty() {
            continue;
        }
        let id = format!("{CODEX_OAUTH_NAMESPACE}::{model}");
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// The model the sign-in probe spends its one tiny completion on. It must be one the account can
/// run over plain HTTPS (the WebSocket-only ids 404 there, which would read as a dead login), and a
/// small tier is plenty for a one-word reply. Taken from what the account can actually use, so no
/// model name has to stay valid for the probe to keep working.
pub(super) fn probe_model(listed: &[String]) -> String {
    pick_probe_model(listed)
        .or_else(|| pick_probe_model(SEED_MODELS))
        .unwrap_or_else(|| SEED_MODELS[0].to_string())
}

fn pick_probe_model<S: AsRef<str>>(names: &[S]) -> Option<String> {
    let usable: Vec<&str> = names
        .iter()
        .map(AsRef::as_ref)
        .filter(|name| !codex_websocket::CODEX_WEBSOCKET_MODELS.contains(name))
        .collect();
    usable
        .iter()
        .find(|name| name.contains("luna") || name.contains("mini"))
        .or_else(|| usable.first())
        .map(|name| (*name).to_string())
}

#[cfg(test)]
mod tests;
