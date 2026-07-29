//! Credential and OAuth login flows.

use super::human_secs;
use crate::*;
use anyhow::{Context, Result};

/// Undo the provider-wide auth bench a credential failure left behind, so a repaired credential
/// takes effect on the very next turn.
///
/// The first auth error writes a 24h `__forge_provider__::<provider>` exclusion row
/// ([`Store::exclude_provider`]) and `ModelHealth::is_benched` then drops every alias of that
/// provider from routing AND from the failover chain. `ProviderError::Auth`'s own doc promises the
/// exclusion "recovers automatically once the user fixes the key" via a periodic re-probe — but no
/// such re-probe exists in the runtime, so the only thing that ever cleared it was the
/// undiscoverable `forge models --probe`. Every successful credential write is therefore the place
/// to clear it: the user's next action after `forge auth` is a turn, not a probe.
///
/// Best-effort by design: a store that will not open must not fail a sign-in that already succeeded.
fn clear_auth_exclusion(provider: &str) {
    if let Ok(store) = open_store() {
        clear_auth_exclusion_in(&store, provider);
    }
}

/// Store half of [`clear_auth_exclusion`], separated so it is testable against a scratch store
/// without a keyring write.
///
/// Per-model rows are cleared too, but ONLY the ones benched for an auth reason: a rate-limit or
/// outage bench says nothing about the credential and must survive a re-login, otherwise signing in
/// would resurrect a model that is genuinely still 429ing.
pub(crate) fn clear_auth_exclusion_in(store: &Store, provider: &str) {
    let _ = store.clear_provider_health(provider);
    let prefix = format!("{provider}::");
    for (model, _, reason) in store.current_benched_report().unwrap_or_default() {
        if model.starts_with(&prefix) && reason.contains("auth failed") {
            let _ = store.clear_model_health(&model);
        }
    }
}

pub(crate) fn auth(provider: &str, remove: bool, list: bool, replace: bool) -> Result<()> {
    let known_provider = forge_config::known_key_providers().any(|p| p == provider);
    let known_search = forge_config::known_search_providers().any(|p| p == provider);
    // `artificialanalysis` is the benchmark Data API key (ADR-0011), not a model/search provider,
    // but it stores/resolves via the same keyring entry name.
    let known_data = provider == "artificialanalysis";
    if !known_provider && !known_search && !known_data {
        let mut known: Vec<_> = forge_config::known_key_providers().collect();
        known.extend(forge_config::known_search_providers());
        known.push("artificialanalysis");
        known.push("xai-oauth");
        known.push("codex-oauth");
        anyhow::bail!(
            "unknown provider '{provider}' — known providers are: {}",
            known.join(", ")
        );
    }
    if list {
        let fps = forge_config::api_key_fingerprints(provider);
        if fps.is_empty() {
            println!("no {provider} keys configured");
        } else {
            println!(
                "{provider}: {} key(s) configured — {}",
                fps.len(),
                fps.join(", ")
            );
        }
        return Ok(());
    }
    if remove {
        let removed = forge_config::remove_api_key(provider)
            .with_context(|| format!("removing {provider} key(s) from the OS keyring"))?;
        if removed {
            println!("removed all stored {provider} key(s) from the OS keyring");
        } else {
            println!("no {provider} key was stored — nothing to remove");
        }
        return Ok(());
    }
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        print!("paste {provider} API key (input hidden is not supported; press enter): ");
        std::io::Write::flush(&mut std::io::stdout()).ok();
    }
    let mut key = String::new();
    std::io::stdin()
        .read_line(&mut key)
        .context("reading key from stdin")?;
    let key = key.trim();
    if key.is_empty() {
        anyhow::bail!("no key provided");
    }
    if replace {
        forge_config::store_api_key(provider, key)
            .with_context(|| format!("storing {provider} key"))?;
        println!(
            "stored {provider} key, replacing any previous key(s) (OS keyring / encrypted file)"
        );
    } else {
        let n = forge_config::add_api_key(provider, key)
            .with_context(|| format!("storing {provider} key"))?;
        let note = if n > 1 {
            format!(" — {n} keys now stored; Forge rotates across them")
        } else {
            String::new()
        };
        println!("stored {provider} key (OS keyring, or encrypted file if no keyring is available){note}");
    }
    // Search/data keys have no mesh health record; only model providers can be excluded.
    if known_provider {
        clear_auth_exclusion(provider);
    }
    Ok(())
}

/// Sign in to xAI/Grok via device-code OAuth (SuperGrok / X Premium subscription — no API key,
/// billed against the subscription instead of metered credits). Multiple accounts can be signed
/// in at once (e.g. a personal account and a SuperGrok trial); one is "active" at a time.
/// `--list` shows every signed-in account, `--switch --account <id>` changes which is active,
/// `--remove` (bare) signs every account out, `--remove --account <id>` signs out just one; the
/// default (and `--replace`, kept only for CLI-shape symmetry with the key-based `auth` command)
/// starts a fresh login and adds it as a new account. Experimental (Phase 1): xAI enforces OAuth
/// API entitlement server-side per account/tier, so a successful login does NOT guarantee
/// inference works — the post-login probe below says so plainly instead of silently retrying.
pub(crate) async fn auth_xai_oauth(
    remove: bool,
    list: bool,
    _replace: bool,
    account: Option<String>,
    switch: bool,
) -> Result<()> {
    use forge_config::provider_oauth::{self, XAI_OAUTH_KEYRING_PROVIDER};

    if switch {
        let id = account
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--switch requires --account <id> (see `--list`)"))?;
        provider_oauth::switch_provider_oauth_account(XAI_OAUTH_KEYRING_PROVIDER, id)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("✓ switched active xai-oauth account to '{id}'");
        return Ok(());
    }

    if list {
        let accounts = provider_oauth::list_provider_oauth_accounts(XAI_OAUTH_KEYRING_PROVIDER);
        if accounts.is_empty() {
            println!("xai-oauth: not signed in — run `forge auth xai-oauth`");
            return Ok(());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let describe = |tokens: &forge_config::OAuthTokens| -> String {
            let expiry = if tokens.expires_at == 0 {
                "no expiry reported".to_string()
            } else {
                let remaining = tokens.expires_at - now;
                if remaining > 0 {
                    format!("access token expires in {}", human_secs(remaining))
                } else {
                    "access token expired".to_string()
                }
            };
            format!(
                "{expiry}, refresh token {}, scopes: {}",
                if tokens.refresh_token.is_some() {
                    "present"
                } else {
                    "absent"
                },
                tokens.scopes.join(" ")
            )
        };
        if accounts.len() == 1 {
            // Keep the single-account case readable — unchanged from before multi-account support.
            let (_, tokens, _) = &accounts[0];
            println!("xai-oauth: signed in ({})", describe(tokens));
        } else {
            println!(
                "xai-oauth: {} account(s) · auto-rotation ON (round-robin)",
                accounts.len()
            );
            for (id, tokens, is_active) in &accounts {
                println!(
                    "  {} {id} — {}",
                    if *is_active { "*" } else { " " },
                    describe(tokens)
                );
            }
            println!(
                "  (* = manual active / rotation seed; requests rotate across all)\n  \
                 switch: `forge auth xai-oauth --switch --account <id>`"
            );
        }
        return Ok(());
    }

    if remove {
        match account.as_deref() {
            Some(id) => {
                let removed =
                    provider_oauth::remove_provider_oauth_account(XAI_OAUTH_KEYRING_PROVIDER, id)
                        .context("removing xAI OAuth account from the OS keyring")?;
                println!(
                    "{}",
                    if removed {
                        format!("removed xai-oauth account '{id}' from the OS keyring")
                    } else {
                        format!("no xai-oauth account '{id}' stored — nothing to remove")
                    }
                );
            }
            None => {
                let removed =
                    provider_oauth::clear_provider_oauth_tokens(XAI_OAUTH_KEYRING_PROVIDER)
                        .context("removing xAI OAuth tokens from the OS keyring")?;
                println!(
                    "{}",
                    if removed {
                        "removed stored xAI OAuth tokens from the OS keyring"
                    } else {
                        "no xAI OAuth tokens stored — nothing to remove"
                    }
                );
            }
        }
        return Ok(());
    }

    println!("To sign in to xAI (Grok) with your SuperGrok / X Premium account, open:\n");
    let dc = forge_provider::start_device_login()
        .await
        .context("starting xAI device-code login")?;
    match &dc.verification_uri_complete {
        Some(url) => println!("    {url}\n"),
        None => println!(
            "    {}\n\nand enter code: {}\n",
            dc.verification_uri, dc.user_code
        ),
    }
    println!("Waiting for approval… press Ctrl-C to cancel.");

    let (tokens, id_token) = forge_provider::poll_for_tokens(&dc)
        .await
        .context("waiting for xAI sign-in")?;
    // Label the account from the id_token's `email` claim when present; otherwise fall back to
    // account-1/account-2/… . Either way this ADDS an account and makes it active — re-running
    // the same account's login overwrites just that one (matched by the same derived id).
    let account_id = id_token
        .as_deref()
        .and_then(provider_oauth::extract_email_from_id_token)
        .unwrap_or_else(|| {
            provider_oauth::next_provider_oauth_account_id(XAI_OAUTH_KEYRING_PROVIDER)
        });
    provider_oauth::add_provider_oauth_account(XAI_OAUTH_KEYRING_PROVIDER, &account_id, &tokens)
        .context("storing xAI OAuth tokens")?;
    // A new subscription login must surface its models without waiting for the 24h cache to age out.
    crate::cli::commands::models::invalidate_catalog_cache();
    // Same for the detected-plan cache: don't serve a stale/absent plan for up to 60s.
    forge_provider::invalidate_plan_cache();
    // …and the same for a provider-wide auth exclusion an earlier expired session left behind.
    clear_auth_exclusion("xai-oauth");

    match forge_provider::probe_entitlement(&tokens.access_token).await {
        Ok(forge_provider::EntitlementStatus::Entitled) => println!(
            "signed in to xAI via OAuth as '{account_id}' — API access confirmed (tokens stored in the OS keyring)\n\
             use models with the xai-oauth:: prefix, e.g.:  forge --model xai-oauth::grok-4\n\
             note: costs show as $0 — usage is billed to your xAI subscription, not metered API credits\n\
             multiple accounts: `forge auth xai-oauth --list` · switch with `--switch --account <id>`"
        ),
        Ok(forge_provider::EntitlementStatus::NotEntitled(msg)) => anyhow::bail!(
            "OAuth sign-in succeeded, but xAI returned 403 for API access: this account's \
             subscription tier is not entitled to use the API via OAuth. This is enforced \
             server-side by xAI — signing in again will not fix it. ({msg})\n\n\
             Tokens are stored (the account may gain entitlement later). To use Grok with Forge \
             now, create an API key at https://console.x.ai and run:\n\n    forge auth xai"
        ),
        Ok(forge_provider::EntitlementStatus::AuthFailed(msg)) => anyhow::bail!(
            "sign-in produced a token xAI rejected (401) — try `forge auth xai-oauth` again, or \
             use `forge auth xai` with an API key. ({msg})"
        ),
        Ok(forge_provider::EntitlementStatus::RateLimited) => println!(
            "signed in as '{account_id}'; the entitlement check was rate-limited (429) — assuming access is OK. \
             If inference fails with 403, run `forge auth xai` instead."
        ),
        Ok(forge_provider::EntitlementStatus::Other(status, msg)) => println!(
            "signed in as '{account_id}'; the entitlement check returned an unexpected status ({status}: {msg}) — \
             tokens are stored, try using xai-oauth:: models directly."
        ),
        Err(e) => println!(
            "signed in as '{account_id}', but the entitlement check itself failed ({e}) — tokens are stored, try \
             using xai-oauth:: models directly."
        ),
    }
    Ok(())
}

/// Build the Codex authorize URL (`forge_config::authorize_url` plus the two params OpenAI's
/// Hydra authorize server requires from the registered Codex CLI client that aren't part of the
/// generic RFC 6749 shape shared with the MCP OAuth path — see `forge_config::authorize_url`).
pub(super) fn codex_authorize_url(state: &str, code_challenge: &str) -> String {
    use forge_config::provider_oauth::{
        CODEX_OAUTH_AUTHORIZE_URL, CODEX_OAUTH_CLIENT_ID, CODEX_OAUTH_REDIRECT_URI,
        CODEX_OAUTH_SCOPE,
    };
    let auth_url = forge_config::authorize_url(
        CODEX_OAUTH_AUTHORIZE_URL,
        CODEX_OAUTH_CLIENT_ID,
        CODEX_OAUTH_REDIRECT_URI,
        &CODEX_OAUTH_SCOPE
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        state,
        code_challenge,
    );
    format!("{auth_url}&id_token_add_organizations=true&codex_cli_simplified_flow=true")
}

/// Sign in to ChatGPT via OAuth 2.0 PKCE (Plus/Pro subscription — no API key, billed against the
/// subscription). Loopback callback on port 1455 (official Codex public client). Multiple accounts
/// supported with the same `--list` / `--switch` / `--remove` surface as `xai-oauth`.
pub(crate) async fn auth_codex_oauth(
    remove: bool,
    list: bool,
    _replace: bool,
    account: Option<String>,
    switch: bool,
    force_device: bool,
    paste: Option<String>,
) -> Result<()> {
    use forge_config::provider_oauth::{
        self, CODEX_OAUTH_CALLBACK_PORT, CODEX_OAUTH_KEYRING_PROVIDER,
    };

    if switch {
        let id = account
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--switch requires --account <id> (see `--list`)"))?;
        provider_oauth::switch_provider_oauth_account(CODEX_OAUTH_KEYRING_PROVIDER, id)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("✓ switched active codex-oauth account to '{id}'");
        return Ok(());
    }

    if list {
        let accounts = provider_oauth::list_provider_oauth_accounts(CODEX_OAUTH_KEYRING_PROVIDER);
        if accounts.is_empty() {
            println!("codex-oauth: not signed in — run `forge auth codex-oauth`");
            return Ok(());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let describe = |tokens: &forge_config::OAuthTokens| -> String {
            let expiry = if tokens.expires_at == 0 {
                "no expiry reported".to_string()
            } else {
                let remaining = tokens.expires_at - now;
                if remaining > 0 {
                    format!("access token expires in {}", human_secs(remaining))
                } else {
                    "access token expired".to_string()
                }
            };
            format!(
                "{expiry}, refresh token {}",
                if tokens.refresh_token.is_some() {
                    "present"
                } else {
                    "absent"
                }
            )
        };
        // Detected live from the ACTIVE account's access-token JWT (Fix 4,
        // docs/design/subscription-efficiency-routing.md) — never the token itself.
        let plan_suffix = forge_provider::codex_oauth_detected_plan()
            .map(|plan| format!(" — plan: {plan}"))
            .unwrap_or_default();
        if accounts.len() == 1 {
            let (_, tokens, _) = &accounts[0];
            println!("codex-oauth: signed in ({}){plan_suffix}", describe(tokens));
        } else {
            println!(
                "codex-oauth: {} account(s) · auto-rotation ON (round-robin)",
                accounts.len()
            );
            for (id, tokens, is_active) in &accounts {
                // The plan claim is only meaningful for the currently ACTIVE account — a
                // rotation sibling's own plan isn't detected here (would need per-account JWT
                // decoding, out of scope for Fix 4).
                let suffix = if *is_active { plan_suffix.as_str() } else { "" };
                println!(
                    "  {} {id} — {}{suffix}",
                    if *is_active { "*" } else { " " },
                    describe(tokens)
                );
            }
            println!(
                "  (* = manual active / rotation seed; requests rotate across all)\n  \
                 switch: `forge auth codex-oauth --switch --account <id>`"
            );
        }
        return Ok(());
    }

    if remove {
        match account.as_deref() {
            Some(id) => {
                let removed =
                    provider_oauth::remove_provider_oauth_account(CODEX_OAUTH_KEYRING_PROVIDER, id)
                        .context("removing Codex OAuth account from the OS keyring")?;
                println!(
                    "{}",
                    if removed {
                        format!("removed codex-oauth account '{id}' from the OS keyring")
                    } else {
                        format!("no codex-oauth account '{id}' stored — nothing to remove")
                    }
                );
            }
            None => {
                let removed =
                    provider_oauth::clear_provider_oauth_tokens(CODEX_OAUTH_KEYRING_PROVIDER)
                        .context("removing Codex OAuth tokens from the OS keyring")?;
                println!(
                    "{}",
                    if removed {
                        "removed stored Codex OAuth tokens from the OS keyring"
                    } else {
                        "no Codex OAuth tokens stored — nothing to remove"
                    }
                );
            }
        }
        return Ok(());
    }

    let flow = crate::cli::commands::oauth_flow::select_login_flow(
        force_device,
        paste.is_some(),
        false,
        crate::cli::commands::oauth_flow::is_headless(),
    );
    if matches!(flow, crate::cli::commands::oauth_flow::LoginFlow::Device) {
        anyhow::bail!(
            "Codex OAuth does not advertise an RFC 8628 device-authorization endpoint; use \
             `--paste` (or run from a browser-capable terminal) instead"
        );
    }

    // PKCE is used by both loopback and pasted redirect completion. The listener is only bound
    // for the loopback flow, so headless hosts do not need an SSH-forwarded callback port.
    let pkce = forge_config::Pkce::generate();
    let state = forge_config::random_state();
    let auth_url = codex_authorize_url(&state, &pkce.challenge);
    let listener = if matches!(flow, crate::cli::commands::oauth_flow::LoginFlow::Loopback) {
        Some(
            tokio::net::TcpListener::bind(("127.0.0.1", CODEX_OAUTH_CALLBACK_PORT))
                .await
                .with_context(|| {
                    format!(
                        "could not bind 127.0.0.1:{CODEX_OAUTH_CALLBACK_PORT} — free the port \
                         (another Codex/Forge auth may be running) and retry"
                    )
                })?,
        )
    } else {
        None
    };

    let no_browser = std::env::var("FORGE_NO_BROWSER").as_deref() == Ok("1") || {
        use std::io::IsTerminal;
        !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal()
    };

    println!("To sign in to ChatGPT (Plus/Pro) with OAuth, open:\n");
    println!("    {auth_url}\n");
    if matches!(flow, crate::cli::commands::oauth_flow::LoginFlow::Paste) {
        println!(
            "Headless paste flow: authorize on any device, then paste the complete redirect URL \
             (including the state parameter) here."
        );
    } else if no_browser {
        println!(
            "(headless / FORGE_NO_BROWSER=1 — open the URL on a machine that can reach this host's \
             port {CODEX_OAUTH_CALLBACK_PORT})"
        );
    } else if let Err(e) = crate::cli::commands::mcp::open_browser(&auth_url) {
        println!("(could not open browser automatically: {e} — open the URL manually)");
    }
    let (code, returned_state) = if let Some(listener) = listener {
        println!(
            "Waiting for approval on 127.0.0.1:{CODEX_OAUTH_CALLBACK_PORT}… press Ctrl-C to cancel."
        );
        wait_for_oauth_callback(listener)
            .await
            .context("waiting for OAuth callback")?
    } else {
        let input = if let Some(value) = paste {
            if value.is_empty() {
                tokio::task::spawn_blocking(
                    crate::cli::commands::oauth_flow::read_pasted_redirect_from_stdin,
                )
                .await
                .context("reading pasted OAuth redirect")??
            } else {
                value
            }
        } else {
            tokio::task::spawn_blocking(
                crate::cli::commands::oauth_flow::read_pasted_redirect_from_stdin,
            )
            .await
            .context("reading pasted OAuth redirect")??
        };
        (
            crate::cli::commands::oauth_flow::parse_pasted_redirect(&input, &state)?,
            state.clone(),
        )
    };
    if returned_state != state {
        anyhow::bail!("OAuth state mismatch — possible CSRF; try again");
    }

    let tokens = forge_provider::exchange_codex_oauth_code(&code, &pkce.verifier)
        .await
        .context("exchanging authorization code")?;

    let chatgpt_id = provider_oauth::extract_chatgpt_account_id(&tokens.access_token);
    let account_id = chatgpt_id
        .clone()
        .or_else(|| provider_oauth::extract_email_from_id_token(&tokens.access_token))
        .unwrap_or_else(|| {
            provider_oauth::next_provider_oauth_account_id(CODEX_OAUTH_KEYRING_PROVIDER)
        });
    provider_oauth::add_provider_oauth_account(CODEX_OAUTH_KEYRING_PROVIDER, &account_id, &tokens)
        .context("storing Codex OAuth tokens")?;
    // A new subscription login must surface its models without waiting for the 24h cache to age out.
    crate::cli::commands::models::invalidate_catalog_cache();
    // Same for the detected-plan cache: don't serve a stale/absent plan for up to 60s.
    forge_provider::invalidate_plan_cache();
    // …and the same for a provider-wide auth exclusion an earlier expired session left behind.
    clear_auth_exclusion("codex-oauth");

    let probe_id = chatgpt_id.as_deref().unwrap_or(&account_id);
    match forge_provider::probe_codex_entitlement(&tokens.access_token, probe_id).await {
        Ok(forge_provider::EntitlementStatus::Entitled) => println!(
            "signed in to ChatGPT via OAuth as '{account_id}' — API access confirmed (tokens stored in the OS keyring)\n\
             use models with the codex-oauth:: prefix, e.g.:  forge --model codex-oauth::gpt-5.5\n\
             note: costs show as $0 — usage is billed to your ChatGPT subscription, not metered API credits\n\
             multiple accounts: `forge auth codex-oauth --list` · switch with `--switch --account <id>`"
        ),
        Ok(forge_provider::EntitlementStatus::NotEntitled(msg)) => anyhow::bail!(
            "OAuth sign-in succeeded, but ChatGPT returned 403 for API access: this account's \
             plan may not allow Codex API access. ({msg})\n\n\
             Tokens are stored. To use OpenAI with Forge now, create an API key and run:\n\n    forge auth openai"
        ),
        Ok(forge_provider::EntitlementStatus::AuthFailed(msg)) => anyhow::bail!(
            "sign-in produced a token ChatGPT rejected (401) — try `forge auth codex-oauth` again, or \
             use `forge auth openai` with an API key. ({msg})"
        ),
        Ok(forge_provider::EntitlementStatus::RateLimited) => println!(
            "signed in as '{account_id}'; the entitlement check was rate-limited (429) — assuming access is OK."
        ),
        Ok(forge_provider::EntitlementStatus::Other(status, msg)) => println!(
            "signed in as '{account_id}'; entitlement check returned unexpected status ({status}: {msg}) — \
             tokens are stored, try using codex-oauth:: models directly."
        ),
        Err(e) => println!(
            "signed in as '{account_id}', but the entitlement check itself failed ({e}) — tokens are stored, try \
             using codex-oauth:: models directly."
        ),
    }
    Ok(())
}

/// Accept one HTTP request on the OAuth loopback listener and extract `code` + `state` query params.
async fn wait_for_oauth_callback(listener: tokio::net::TcpListener) -> Result<(String, String)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut stream, _) = listener
        .accept()
        .await
        .context("accepting OAuth callback connection")?;
    let mut buf = vec![0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .context("reading OAuth callback request")?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let first_line = req.lines().next().unwrap_or("");
    // GET /auth/callback?code=...&state=... HTTP/1.1
    let path = first_line.split_whitespace().nth(1).unwrap_or("");
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            match k {
                "code" => code = Some(urlencoding_decode(v)),
                "state" => state = Some(urlencoding_decode(v)),
                _ => {}
            }
        }
    }
    let body = if code.is_some() {
        "signed in — you can close this tab and return to Forge."
    } else {
        "sign-in failed — no authorization code received."
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let code = code.ok_or_else(|| anyhow::anyhow!("callback missing code parameter"))?;
    let state = state.ok_or_else(|| anyhow::anyhow!("callback missing state parameter"))?;
    Ok((code, state))
}

fn urlencoding_decode(s: &str) -> String {
    // Minimal percent-decode for OAuth query values (code/state are URL-safe).
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push(char::from_u32(h * 16 + l).unwrap_or('?'));
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(' ');
        } else {
            out.push(bytes[i] as char);
        }
        i += 1;
    }
    out
}
