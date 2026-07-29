//! MCP OAuth login flow.

use anyhow::{Context, Result};

use super::{open_browser, read_callback_params};

fn paste_redirect_uri(port: Option<u16>) -> Result<String> {
    let port = port.ok_or_else(|| {
        anyhow::anyhow!(
            "paste login requires oauth.redirect_port so the authorization server can redirect to a registered loopback URI"
        )
    })?;
    Ok(format!("http://127.0.0.1:{port}/callback"))
}

#[cfg(test)]
fn registered_client_matches_redirect(
    client: &forge_mcp::oauth::RegisteredClient,
    redirect_uri: &str,
) -> bool {
    client.redirect_uri.as_deref() == Some(redirect_uri)
}

pub(crate) async fn mcp_login(
    server: &str,
    force_device: bool,
    paste: Option<String>,
) -> Result<()> {
    forge_config::inject_provider_keys();
    let config = forge_config::load().unwrap_or_default();

    // Find the server by name.
    let srv = config
        .mcp
        .servers
        .iter()
        .find(|s| s.name == server)
        .ok_or_else(|| anyhow::anyhow!("no server '{server}' in .forge/mcp.toml"))?;

    // Must have an oauth config entry.
    let oauth_cfg = srv
        .auth
        .as_ref()
        .and_then(|a| a.oauth.as_ref())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "server '{server}' has no [auth.oauth] config — add it to .forge/mcp.toml"
            )
        })?;

    let http = forge_provider::bundled_http_client();

    // Discover the authorization server issuer.
    let issuer = if let Some(i) = &oauth_cfg.issuer {
        i.clone()
    } else {
        // Probe the server's well-known resource-metadata endpoint (RFC 9728).
        let url = match &srv.transport {
            forge_config::McpTransport::Http { url, .. } => {
                let base = url.trim_end_matches('/');
                format!("{base}/.well-known/oauth-protected-resource/mcp")
            }
            _ => anyhow::bail!("OAuth login only supported for HTTP transports"),
        };
        println!("Discovering auth server from {url} …");
        let meta = forge_mcp::oauth::fetch_resource_metadata(&http, &url)
            .await
            .map_err(|e| anyhow::anyhow!("fetching resource metadata from {url}: {e}"))?;
        meta.authorization_servers
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("resource metadata has no authorization_servers"))?
    };

    println!("Auth server: {issuer}");

    // Fetch auth server metadata (RFC 8414).
    let as_meta = forge_mcp::oauth::fetch_auth_server_metadata(&http, &issuer)
        .await
        .map_err(|e| anyhow::anyhow!("fetching auth server metadata from {issuer}: {e}"))?;

    let flow = crate::cli::commands::oauth_flow::select_login_flow(
        force_device,
        paste.is_some(),
        as_meta.device_authorization_endpoint.is_some(),
        crate::cli::commands::oauth_flow::is_headless(),
    );

    if matches!(flow, crate::cli::commands::oauth_flow::LoginFlow::Device) {
        let device_url = as_meta
            .device_authorization_endpoint
            .as_deref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "this MCP authorization server does not advertise an RFC 8628 \
                 device_authorization_endpoint"
                )
            })?;
        let registered = forge_mcp::oauth::load_registered_client(server);
        let client_id = oauth_cfg
            .client_id
            .clone()
            .or_else(|| registered.as_ref().map(|client| client.client_id.clone()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "device login needs an OAuth client_id in .forge/mcp.toml or a prior \
                     browser login that registered one"
                )
            })?;
        let scope = if oauth_cfg.scopes.is_empty() {
            "mcp offline_access".to_string()
        } else {
            oauth_cfg.scopes.join(" ")
        };
        let device =
            crate::cli::commands::oauth_flow::request_device_code(device_url, &client_id, &scope)
                .await?;
        println!(
            "Open {} and enter the displayed user code.",
            device.verification_uri
        );
        println!("User code: {}", device.user_code);
        if let Some(uri) = device.verification_uri_complete.as_deref() {
            println!("Or open the complete verification URL: {uri}");
        }
        let device_tokens = crate::cli::commands::oauth_flow::poll_device_token(
            &as_meta.token_endpoint,
            &client_id,
            &device,
        )
        .await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        let tokens = forge_config::OAuthTokens {
            access_token: device_tokens.access_token,
            refresh_token: device_tokens.refresh_token,
            expires_at: device_tokens
                .expires_in
                .map(|expires| now + expires as i64)
                .unwrap_or(0),
            token_endpoint: as_meta.token_endpoint.clone(),
            client_id,
            scopes: oauth_cfg.scopes.clone(),
        };
        let account_id = forge_config::next_oauth_account_id(server);
        forge_config::add_oauth_account(server, &account_id, &tokens)
            .context("storing OAuth tokens in keyring")?;
        println!("✓ OAuth tokens stored for '{server}' (account '{account_id}').");
        return Ok(());
    }

    let (listener, redirect_uri) =
        if matches!(flow, crate::cli::commands::oauth_flow::LoginFlow::Paste) {
            (None, paste_redirect_uri(oauth_cfg.redirect_port)?)
        } else {
            let redirect_port = oauth_cfg.redirect_port.unwrap_or(0);
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", redirect_port))
                .await
                .context("binding loopback redirect listener")?;
            let bound_port = listener.local_addr()?.port();
            (
                Some(listener),
                format!("http://127.0.0.1:{bound_port}/callback"),
            )
        };

    // PKCE + state.
    let pkce = forge_config::Pkce::generate();
    let state = forge_config::random_state();
    let scopes = if oauth_cfg.scopes.is_empty() {
        vec!["mcp".to_string(), "offline_access".to_string()]
    } else {
        oauth_cfg.scopes.clone()
    };

    // Resolve the OAuth client. Precedence: an explicitly-pinned config `client_id` → a previously
    // registered client (reused across logins) → RFC 7591 dynamic client registration against the
    // auth server's `registration_endpoint`. Hosted servers (GitHub/Linear/Notion) reject the old
    // hardcoded public client, so registration is what actually makes them work. The `redirect_uri`
    // is known now (listener bound), which DCR needs.
    let (client_id, client_secret) = if let Some(id) = oauth_cfg.client_id.clone() {
        (id, None)
    } else if let Some(rc) =
        forge_mcp::oauth::load_registered_client_for(server, &issuer, &redirect_uri)
    {
        println!("Using previously registered client '{}'.", rc.client_id);
        (rc.client_id, rc.client_secret)
    } else if let Some(reg_ep) = as_meta.registration_endpoint.clone() {
        println!("Registering OAuth client (RFC 7591) at {reg_ep} …");
        let mut registered = forge_mcp::oauth::register_client(
            &http,
            &reg_ep,
            std::slice::from_ref(&redirect_uri),
            &scopes,
            "Forge",
        )
        .await
        .map_err(|e| anyhow::anyhow!("dynamic client registration: {e}"))?;
        registered.redirect_uri = Some(redirect_uri.clone());
        registered.issuer = Some(issuer.clone());
        forge_mcp::oauth::store_registered_client(server, &registered)
            .map_err(|e| anyhow::anyhow!("persisting registered client: {e}"))?;
        println!("✓ Registered client '{}'.", registered.client_id);
        (registered.client_id, registered.client_secret)
    } else {
        // No registration endpoint advertised and no client configured — fall back to the public
        // client id (works for auth servers that accept an unregistered public client).
        ("forge-mcp-client".to_string(), None)
    };

    let auth_url = forge_config::authorize_url(
        &as_meta.authorization_endpoint,
        &client_id,
        &redirect_uri,
        &scopes,
        &state,
        &pkce.challenge,
    );

    println!("Authorization URL:\n  {auth_url}");
    if !matches!(flow, crate::cli::commands::oauth_flow::LoginFlow::Paste) {
        if let Err(e) = open_browser(&auth_url) {
            println!("(could not open browser automatically: {e})");
            println!("Please open the URL above manually.");
        }
    }

    if !matches!(flow, crate::cli::commands::oauth_flow::LoginFlow::Paste) {
        println!("Waiting for authorization callback on {redirect_uri} …");
    }
    let (code, returned_state) =
        if matches!(flow, crate::cli::commands::oauth_flow::LoginFlow::Paste) {
            println!(
                "Paste the complete redirect URL (including the state parameter) here; a bare \
             authorization code is also accepted."
            );
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
        } else {
            let listener = listener.expect("loopback login binds a listener");
            let (mut stream, _) =
                tokio::time::timeout(std::time::Duration::from_secs(120), listener.accept())
                    .await
                    .context("timed out waiting for OAuth callback (120 s)")?
                    .context("accepting callback connection")?;
            let result = read_callback_params(&mut stream).await?;
            let _ = tokio::io::AsyncWriteExt::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
              <html><body><h2>Authorization complete. You can close this tab.</h2></body></html>",
            )
            .await;
            drop(stream);
            result
        };

    // CSRF check.
    if returned_state != state {
        anyhow::bail!("OAuth state mismatch — possible CSRF. Login aborted.");
    }

    // Exchange the code for tokens.
    println!("Exchanging authorization code …");
    let tokens = forge_mcp::oauth::exchange_code(
        &http,
        &as_meta.token_endpoint,
        &code,
        &redirect_uri,
        &client_id,
        &pkce.verifier,
        client_secret.as_deref(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("token exchange: {e}"))?;

    // Store in keyring, as a new account (the authorization_code flow has no generic way to get
    // an account label like device-code's id_token email, so this falls back to `account-N`) —
    // and make it active, so a re-login always lands on the freshest tokens.
    let account_id = forge_config::next_oauth_account_id(server);
    forge_config::add_oauth_account(server, &account_id, &tokens)
        .context("storing OAuth tokens in keyring")?;

    println!(
        "✓ OAuth tokens stored for '{server}' (account '{account_id}'). Forge will refresh them \
         automatically. Multiple accounts? `forge mcp login {server}` again to add another, or \
         `forge mcp logout {server} --account <id>` to remove one."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{paste_redirect_uri, registered_client_matches_redirect};

    #[test]
    fn registered_clients_are_reused_only_for_their_registered_redirect() {
        let client = forge_mcp::oauth::RegisteredClient {
            client_id: "client".into(),
            client_secret: None,
            redirect_uri: Some("http://127.0.0.1:8787/callback".into()),
            issuer: Some("https://issuer.test".into()),
        };
        assert!(registered_client_matches_redirect(
            &client,
            "http://127.0.0.1:8787/callback"
        ));
        assert!(!registered_client_matches_redirect(
            &client,
            "http://127.0.0.1:8788/callback"
        ));
    }

    #[test]
    fn paste_login_uses_configured_redirect_without_binding_a_listener() {
        assert_eq!(
            paste_redirect_uri(Some(8787)).unwrap(),
            "http://127.0.0.1:8787/callback"
        );
        assert!(paste_redirect_uri(None).is_err());
    }
}
