use crate::*;
use anyhow::{Context, Result};

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
fn codex_authorize_url(state: &str, code_challenge: &str) -> String {
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
        if accounts.len() == 1 {
            let (_, tokens, _) = &accounts[0];
            println!("codex-oauth: signed in ({})", describe(tokens));
        } else {
            println!(
                "codex-oauth: {} account(s) · auto-rotation ON (round-robin)",
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

    // PKCE + loopback on the official Codex callback port.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", CODEX_OAUTH_CALLBACK_PORT))
        .await
        .with_context(|| {
            format!(
                "could not bind 127.0.0.1:{CODEX_OAUTH_CALLBACK_PORT} — free the port (another \
                 Codex/Forge auth may be running) and retry"
            )
        })?;

    let pkce = forge_config::Pkce::generate();
    let state = forge_config::random_state();
    let auth_url = codex_authorize_url(&state, &pkce.challenge);

    let no_browser = std::env::var("FORGE_NO_BROWSER").as_deref() == Ok("1") || {
        use std::io::IsTerminal;
        !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal()
    };

    println!("To sign in to ChatGPT (Plus/Pro) with OAuth, open:\n");
    println!("    {auth_url}\n");
    if no_browser {
        println!(
            "(headless / FORGE_NO_BROWSER=1 — open the URL on a machine that can reach this host's \
             port {CODEX_OAUTH_CALLBACK_PORT})"
        );
    } else if let Err(e) = crate::cli::commands::mcp::open_browser(&auth_url) {
        println!("(could not open browser automatically: {e} — open the URL manually)");
    }
    println!(
        "Waiting for approval on 127.0.0.1:{CODEX_OAUTH_CALLBACK_PORT}… press Ctrl-C to cancel."
    );

    let (code, returned_state) = wait_for_oauth_callback(listener)
        .await
        .context("waiting for OAuth callback")?;
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

/// Render a whole number of seconds as the coarsest useful unit (`"54m"`, `"3h"`, `"2d"`).
fn human_secs(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 3600 {
        format!("{}m", (secs / 60).max(1))
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// A human label + free/paid hint for a key-based provider, shown in `forge init`.
pub(crate) fn provider_label(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "Anthropic (Claude API) — paid",
        "openai" => "OpenAI (GPT API) — paid",
        "gemini" => "Google Gemini — free tier + paid",
        "xai" => "xAI (Grok) — paid",
        "xai-oauth" => "xAI (Grok) — SuperGrok/X Premium subscription (OAuth, no API key)",
        "codex-oauth" => "OpenAI ChatGPT — Plus/Pro subscription (OAuth, no API key)",
        "deepseek" => "DeepSeek — paid",
        "openrouter" => "OpenRouter (gateway, many models) — paid + some :free",
        "groq" => "Groq — free tier (fast)",
        "opencode_go" => "OpenCode Zen — paid credit (curated coding models)",
        "github_copilot" => "GitHub Models — free inference",
        "mimo" => "Xiaomi MiMo — free",
        "minimax" => "MiniMax — free tier",
        "cohere" => "Cohere — Command A (218B), free trial tier",
        "bedrock" => "AWS Bedrock — enterprise (Bedrock API key; pin model ids)",
        "vertex" => "Google Vertex AI — enterprise (needs VERTEX_PROJECT_ID; pin model ids)",
        "together" => "Together AI — gateway (OSS frontier, metered)",
        "fireworks" => "Fireworks AI — gateway (fast OSS, metered)",
        "perplexity" => "Perplexity — Sonar (online + reasoning, metered)",
        // Custom OpenAI-compatible providers carry their label in the registry.
        other => forge_config::custom_provider(other)
            .map(|p| p.label)
            .unwrap_or("provider"),
    }
}

/// The subscription plans a CLI bridge can be backed by: `(human label, stored slug)`. Captured
/// by `forge init` so the mesh knows the usage headroom (quota-aware routing, L3). The exact
/// quota numbers aren't asserted here — only which plan the user holds.
pub(crate) fn bridge_plans(
    kind: forge_provider::CliKind,
) -> &'static [(&'static str, &'static str)] {
    match kind {
        forge_provider::CliKind::ClaudeCode => &[
            ("Free", "free"),
            ("Pro", "pro"),
            ("Max 5×", "max-5x"),
            ("Max 20×", "max-20x"),
            ("API credits / unsure", "unknown"),
        ],
        forge_provider::CliKind::Codex => &[
            ("Plus", "plus"),
            ("Pro", "pro"),
            ("Team", "team"),
            ("Enterprise", "enterprise"),
            ("API credits / unsure", "unknown"),
        ],
        forge_provider::CliKind::Antigravity => &[
            ("Free", "free"),
            ("Pro", "pro"),
            ("Ultra", "ultra"),
            ("API credits / unsure", "unknown"),
        ],
    }
}

/// Whether the user looks un-onboarded: no provider key, no installed bridge, and no saved
/// config. Pure so it's testable; the caller adds the tty check before auto-launching `init`.
pub(crate) fn needs_onboarding(has_any_key: bool, any_bridge: bool, config_exists: bool) -> bool {
    !has_any_key && !any_bridge && !config_exists
}

/// Read one trimmed line from stdin with a prompt (no echo suppression — same as `auth`).
/// Opt-in: if `[local] autostart` is set, ensure the configured local model's Ollama server is up
/// before the chat starts. Best-effort and non-fatal — a failure just means the mesh won't have the
/// local model this session.
pub(crate) fn maybe_autostart_local() {
    let cfg = forge_config::load().unwrap_or_default();
    if !cfg.local.autostart || !local::ollama_installed() {
        return;
    }
    if local::ollama_start_serve() {
        if let Some(tag) = &cfg.local.model {
            if !local::ollama_installed_models().iter().any(|m| m == tag) {
                println!("⚒ local: pulling {tag} (first run)…");
                local::ollama_pull(tag);
            }
            println!("⚒ local model ready: ollama::{tag}");
        }
    }
}

/// The animated `forge local` menu (no-arg on a terminal): pick a model to install/start, or view
/// status. Loops until the user closes it; each action prints, then waits for Enter before the
/// menu redraws (it owns its own alternate screen).
pub(crate) async fn local_menu() -> Result<()> {
    enum Act {
        Model(String),
        Status,
        Close,
    }
    let scores = local_bench_scores().await;
    loop {
        let specs = local::detect_specs();
        let cands = local::discover_ranked(&specs, scores.as_ref()).await;
        let installed = if local::ollama_installed() {
            local::ollama_installed_models()
        } else {
            Vec::new()
        };
        let mut items: Vec<forge_tui::SelectItem> = Vec::new();
        let mut acts: Vec<Act> = Vec::new();
        for c in &cands {
            let have = installed.iter().any(|t| t == &c.ollama_tag);
            let bench = if c.benchmarked {
                format!("AA {:.0}", c.score)
            } else {
                "—".to_string()
            };
            items.push(forge_tui::SelectItem {
                label: c.label.clone(),
                hint: format!(
                    "{} · ~{:.0} GB · bench {bench}{}",
                    c.ollama_tag,
                    c.min_memory_gb,
                    if have {
                        " · installed → start"
                    } else {
                        " → install"
                    }
                ),
                preselected: false,
            });
            acts.push(Act::Model(c.ollama_tag.clone()));
        }
        items.push(forge_tui::SelectItem {
            label: "Status".into(),
            hint: "runtime + installed models + autostart".into(),
            preselected: false,
        });
        acts.push(Act::Status);
        items.push(forge_tui::SelectItem {
            label: "Close".into(),
            hint: String::new(),
            preselected: false,
        });
        acts.push(Act::Close);

        let title = format!(
            "forge local — {:.0} GB usable · {} · GPU: {} · ranked by Artificial Analysis",
            specs.model_memory_gb(),
            specs.os,
            specs
                .gpu
                .as_ref()
                .map(|g| g.name.as_str())
                .unwrap_or("none")
        );
        let Some(idx) = forge_tui::select_one(&title, &items)? else {
            return Ok(());
        };
        match &acts[idx] {
            Act::Close => return Ok(()),
            Act::Status => {
                local_status();
                let _ = prompt_line("\n  press Enter to continue…");
            }
            Act::Model(tag) => {
                let have = local::ollama_installed_models().iter().any(|t| t == tag);
                let res = if have {
                    local_start(Some(tag))
                } else {
                    local_install(Some(tag))
                };
                if let Err(e) = res {
                    println!("⚠ {e}");
                }
                let _ = prompt_line("\n  press Enter to continue…");
            }
        }
    }
}

/// Artificial Analysis benchmark scores for ranking local models (cache-first; `None` if disabled
/// or unavailable). Seeds the coverage check with the static catalog's tags.
pub(crate) async fn local_bench_scores() -> Option<forge_mesh::BenchmarkScores> {
    let cfg = forge_config::load().unwrap_or_default();
    let ids: Vec<String> = local::CATALOG
        .iter()
        .map(|m| format!("ollama::{}", m.ollama_tag))
        .collect();
    benchmarks::ensure(&cfg, &ids, false).await
}

/// `forge local [subcommand]`: detect specs, install/run a local model via Ollama, list, status.
/// No subcommand on a terminal → the animated interactive menu; otherwise (piped) → `detect`.
pub(crate) async fn local_cmd(sub: Option<LocalCmd>) -> Result<()> {
    let Some(sub) = sub else {
        use std::io::IsTerminal;
        if std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
            return local_menu().await;
        }
        print_specs_and_recommendation().await;
        return Ok(());
    };
    match sub {
        LocalCmd::Detect => {
            print_specs_and_recommendation().await;
            Ok(())
        }
        LocalCmd::Install { key } => local_install(key.as_deref()),
        LocalCmd::List => {
            if !local::ollama_installed() {
                println!("Ollama is not installed. Run `forge local install` to set it up.");
                return Ok(());
            }
            let models = local::ollama_installed_models();
            if models.is_empty() {
                println!("No local models pulled yet. Run `forge local install`.");
            } else {
                println!("Local models ({}):", models.len());
                for m in models {
                    println!("  • {m}");
                }
            }
            Ok(())
        }
        LocalCmd::Start { key } => local_start(key.as_deref()),
        LocalCmd::Status => {
            local_status();
            Ok(())
        }
    }
}

/// Print the detected specs + the ranked recommendation list.
pub(crate) async fn print_specs_and_recommendation() {
    let specs = local::detect_specs();
    let gpu = match &specs.gpu {
        Some(g) => match g.vram_gb {
            Some(v) => format!("{} ({v:.0} GB VRAM)", g.name),
            None => g.name.clone(),
        },
        None => "none detected".to_string(),
    };
    println!("⚒ This machine");
    println!(
        "  RAM {:.0} GB · {} cores · {} · GPU: {gpu}",
        specs.total_ram_gb, specs.cpu_cores, specs.os
    );
    println!(
        "  model memory budget: ~{:.0} GB\n",
        specs.model_memory_gb()
    );

    let scores = local_bench_scores().await;
    let cands = local::discover_ranked(&specs, scores.as_ref()).await;
    if cands.is_empty() {
        println!("No model fits this machine's memory (the smallest needs ~4 GB).");
        return;
    }
    let benched = cands.iter().filter(|c| c.benchmarked).count();
    println!(
        "Models that fit, ranked by Artificial Analysis benchmark score ({benched}/{} rated):",
        cands.len()
    );
    for (i, c) in cands.iter().enumerate() {
        let rec = if i == 0 { "  ‹recommended›" } else { "" };
        let bench = if c.benchmarked {
            format!("AA {:.0}", c.score)
        } else {
            "unrated".to_string()
        };
        println!(
            "  {} {:<26} [{}]  {} · ~{:.0} GB · {bench}{rec}",
            if i == 0 { "▸" } else { " " },
            c.label,
            c.ollama_tag,
            c.family,
            c.min_memory_gb,
        );
        if !c.blurb.is_empty() {
            println!("      {}", c.blurb);
        }
    }
    println!(
        "\nInstall with `forge local install` (recommended) or `forge local install <tag-or-key>`."
    );
}

/// Ensure Ollama is installed (offering to install it), then pull the chosen (or recommended)
/// model. `name` is a raw Ollama tag (`qwen2.5-coder:14b`), a catalog key (`qwen2.5-coder-14b`),
/// or `None` for the recommended pick.
pub(crate) fn local_install(name: Option<&str>) -> Result<()> {
    let specs = local::detect_specs();
    // Resolve to (display label, ollama tag).
    let (label, tag): (String, String) = match name {
        Some(n) if n.contains(':') => (n.to_string(), n.to_string()), // raw tag
        Some(k) => {
            let m = local::model_by_key(k)
                .with_context(|| format!("unknown model '{k}' — see `forge local detect`"))?;
            (m.label.to_string(), m.ollama_tag.to_string())
        }
        None => {
            let m = *local::recommend(&specs)
                .first()
                .context("no local model fits this machine (needs ≥4 GB)")?;
            (m.label.to_string(), m.ollama_tag.to_string())
        }
    };

    if !local::ollama_installed() {
        println!("Ollama (the local-model runtime) is not installed.");
        match local::ollama_install_command(&specs) {
            Some((cmd, args)) => {
                let shown = std::iter::once(cmd.to_string())
                    .chain(args.iter().cloned())
                    .collect::<Vec<_>>()
                    .join(" ");
                let yes = prompt_line(&format!("Install it now with `{shown}`? [Y/n]: "))?;
                if yes.is_empty()
                    || yes.eq_ignore_ascii_case("y")
                    || yes.eq_ignore_ascii_case("yes")
                {
                    if !local::run_install(cmd, &args) {
                        anyhow::bail!("Ollama install failed — install it manually from https://ollama.com/download, then re-run.");
                    }
                } else {
                    println!("Skipped. Install Ollama from https://ollama.com/download, then re-run `forge local install`.");
                    return Ok(());
                }
            }
            None => {
                println!("Install Ollama from https://ollama.com/download, then re-run `forge local install`.");
                return Ok(());
            }
        }
    }

    println!("Pulling {label} ({tag})…");
    if !local::ollama_pull(&tag) {
        anyhow::bail!(
            "`ollama pull {tag}` failed. The tag may not exist in your Ollama version — check `ollama list` / upgrade Ollama, or pick another model with `forge local detect`."
        );
    }
    println!("✓ {label} is ready. It's available in the mesh as `ollama::{tag}`.");
    println!("  Start it with `forge local start {tag}`, or enable `[local] autostart` in config.");
    Ok(())
}

/// Ensure the Ollama server is up and the chosen model is available.
pub(crate) fn local_start(key: Option<&str>) -> Result<()> {
    if !local::ollama_installed() {
        anyhow::bail!("Ollama is not installed. Run `forge local install` first.");
    }
    let cfg = forge_config::load().unwrap_or_default();
    // Choose the model: raw tag as-is; catalog key → its tag; else configured tag; else recommended.
    let tag: String = match key {
        Some(n) if n.contains(':') => n.to_string(),
        Some(k) => local::model_by_key(k)
            .map(|m| m.ollama_tag.to_string())
            .with_context(|| format!("unknown model '{k}'"))?,
        None => cfg
            .local
            .model
            .clone()
            .or_else(|| {
                let specs = local::detect_specs();
                local::recommend(&specs)
                    .first()
                    .map(|m| m.ollama_tag.to_string())
            })
            .context("no model configured and none fits — run `forge local install`")?,
    };
    print!("Starting Ollama… ");
    std::io::Write::flush(&mut std::io::stdout()).ok();
    if !local::ollama_start_serve() {
        anyhow::bail!("could not start `ollama serve` (is it already running on another port?)");
    }
    println!("up.");
    if !local::ollama_installed_models().iter().any(|m| m == &tag) {
        println!("Model {tag} not pulled yet — pulling…");
        if !local::ollama_pull(&tag) {
            anyhow::bail!("`ollama pull {tag}` failed.");
        }
    }
    println!("✓ Local model ready: `ollama::{tag}` (mesh will route to it).");
    Ok(())
}

/// Print local-runtime status: install, serving, models, and the autostart config.
pub(crate) fn local_status() {
    let cfg = forge_config::load().unwrap_or_default();
    match local::ollama_version() {
        Some(v) => println!("Ollama: installed ({v})"),
        None => {
            println!("Ollama: not installed — run `forge local install`");
            return;
        }
    }
    println!(
        "Server:  {}",
        if local::ollama_serving() {
            "running (localhost:11434)"
        } else {
            "stopped — `forge local start`"
        }
    );
    let models = local::ollama_installed_models();
    println!(
        "Models:  {}",
        if models.is_empty() {
            "none".to_string()
        } else {
            models.join(", ")
        }
    );
    println!(
        "Autostart: {}{}",
        if cfg.local.autostart { "on" } else { "off" },
        cfg.local
            .model
            .as_deref()
            .map(|m| format!(" · model {m}"))
            .unwrap_or_default()
    );
}

pub(crate) fn prompt_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    std::io::Write::flush(&mut std::io::stdout()).ok();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("reading stdin")?;
    Ok(line.trim().to_string())
}

/// `forge init`: interactive first-run setup. Walks the key-based providers (offering to store a
/// key for each), then each installed CLI bridge (asking which subscription plan backs it), and
/// writes the plans to the user config. Keys go to the OS keyring, never the config (ADR-0007).
pub(crate) fn init() -> Result<()> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        anyhow::bail!("`forge init` is interactive — run it in a terminal");
    }
    let cfg = forge_config::load().unwrap_or_default();
    let outcome =
        forge_tui::init_wizard::run(wizard_input(cfg.permission_mode, cfg.mesh.credit_mode))
            .context("running the setup wizard")?;
    if outcome.cancelled {
        println!("Setup cancelled — run `forge init` anytime.");
        return Ok(());
    }
    let path = apply_wizard_outcome(&outcome)?;
    println!("✓ Setup saved to {}", path.display());
    println!(
        "  {} key(s) stored · {} bridge plan(s) recorded.",
        outcome.keys.len(),
        outcome.plans.len()
    );
    println!("  The mesh routes across these by task tier + cost. Try `forge models`.");
    Ok(())
}

/// `forge setup`: the full guided flow — the provider/plan wizard ([`init`]), then an optional
/// local-LLM step. Used by `forge setup`, `forge init`, and the first-run prompt.
pub(crate) fn setup() -> Result<()> {
    init()?;
    offer_local_setup();
    Ok(())
}

/// Interactive local-LLM step of `forge setup`: detect the machine, recommend a Gemma model that
/// fits, and offer to install it (and auto-start it). Best-effort — any failure prints and the
/// flow continues. Skipped on a machine too small for the smallest model.
pub(crate) fn offer_local_setup() {
    let specs = local::detect_specs();
    let picks = local::recommend(&specs);
    let Some(&rec) = picks.first() else {
        return; // nothing fits — don't pester the user
    };
    println!("\n⚒ Local LLM (optional)");
    println!(
        "  This machine (~{:.0} GB usable) can run {} [{}].",
        specs.model_memory_gb(),
        rec.label,
        rec.ollama_tag
    );
    let ans = match prompt_line("  Install it now via Ollama? [Y/n]: ") {
        Ok(a) => a,
        Err(_) => return,
    };
    if !(ans.is_empty() || ans.eq_ignore_ascii_case("y") || ans.eq_ignore_ascii_case("yes")) {
        println!("  Skipped. Run `forge local install` anytime.");
        return;
    }
    if let Err(e) = local_install(Some(rec.key)) {
        println!("  ⚠ {e}");
        return;
    }
    // Offer auto-start so the model is ready whenever Forge runs.
    if let Ok(a) = prompt_line("  Auto-start this model when Forge runs? [y/N]: ") {
        if a.eq_ignore_ascii_case("y") || a.eq_ignore_ascii_case("yes") {
            let _ = forge_config::set_config_value(
                forge_config::ConfigScope::User,
                "local.autostart",
                "true",
            );
            let _ = forge_config::set_config_value(
                forge_config::ConfigScope::User,
                "local.model",
                rec.ollama_tag,
            );
            println!("  ✓ Auto-start enabled ({}).", rec.ollama_tag);
        }
    }
}

/// Build the config-wizard inputs from what Forge knows: key-based model providers, search-API
/// providers (for `web_search`), and every INSTALLED CLI bridge (with its subscription plans).
/// Shared by `forge init` and the in-chat `/config` command.
pub(crate) fn wizard_input(
    current_permission: forge_types::PermissionMode,
    current_credit_mode: forge_types::CreditMode,
) -> forge_tui::WizardInput {
    let providers = forge_config::known_key_providers()
        .map(|p| forge_tui::ProviderItem {
            id: p.to_string(),
            label: provider_label(p).to_string(),
            had_key: forge_config::has_api_key(p),
        })
        .collect();
    let search = forge_config::known_search_providers()
        .map(|p| forge_tui::ProviderItem {
            id: p.to_string(),
            label: forge_config::search_provider_label(p).to_string(),
            had_key: forge_config::has_search_key(p),
        })
        .collect();
    let bridges = forge_provider::CliKind::all()
        .into_iter()
        .filter(|k| k.available())
        .map(|k| forge_tui::BridgeItem {
            prefix: k.prefix().to_string(),
            plans: bridge_plans(k)
                .iter()
                .map(|(l, s)| (l.to_string(), s.to_string()))
                .collect(),
        })
        .collect();
    forge_tui::WizardInput {
        providers,
        search,
        bridges,
        current_permission,
        current_credit_mode,
    }
}

/// Persist a wizard outcome: keys → OS keyring (ADR-0007), plans + settings → user config; then
/// inject keys into this process's env so a running session picks them up immediately.
/// Returns the config path. Shared by `forge init` and `/config`.
pub(crate) fn apply_wizard_outcome(
    outcome: &forge_tui::WizardOutcome,
) -> Result<std::path::PathBuf> {
    for (provider, key) in &outcome.keys {
        forge_config::store_api_key(provider, key)
            .with_context(|| format!("storing {provider} key"))?;
    }
    let path = forge_config::write_subscriptions(&outcome.plans).context("writing config")?;
    forge_config::write_settings(outcome.permission, outcome.credit_mode)
        .context("writing settings")?;
    forge_config::inject_provider_keys();
    forge_config::inject_search_keys();
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Codex authorize request must match OpenAI's registered public client
    /// (`app_EMoamEEZ73f0CkXaXp7hrann`) byte-exact, or Hydra rejects it with
    /// `authorize_hydra_invalid_request` before consent even renders: the `localhost` (not
    /// `127.0.0.1`) redirect_uri, plus the two Codex-CLI-specific params.
    #[test]
    fn codex_authorize_url_matches_registered_client() {
        let url = codex_authorize_url("test-state", "test-challenge");
        assert!(
            url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"),
            "expected percent-encoded localhost redirect_uri in {url}"
        );
        assert!(
            url.contains("id_token_add_organizations=true"),
            "missing id_token_add_organizations param in {url}"
        );
        assert!(
            url.contains("codex_cli_simplified_flow=true"),
            "missing codex_cli_simplified_flow param in {url}"
        );
    }
}
