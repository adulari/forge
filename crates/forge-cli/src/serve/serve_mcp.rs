//! MCP catalog and mutation API for the Serve control surface.
//!
//! The daemon reads the same layered catalog as the CLI while mutating only explicit project/user
//! `mcp.toml` owners. Imported servers remain visible but read-only instead of being silently copied
//! into a second source of truth.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use axum::extract::Json;
use axum::response::Response;

use super::{err_response, json_response};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateMcpServerRequest {
    name: String,
    transport: McpTransportRequest,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    url: Option<String>,
    token_env: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum McpTransportRequest {
    Stdio,
    Http,
    Sse,
}

#[derive(serde::Serialize)]
struct McpServerRow {
    name: String,
    transport: String,
    enabled: bool,
    auth_configured: bool,
    secret_env_count: usize,
    editable: bool,
}

#[derive(serde::Serialize)]
struct McpResponse {
    servers: Vec<McpServerRow>,
    allowed_servers: Vec<String>,
    allowed_tools: Vec<String>,
    call_timeout_secs: u64,
    connect_timeout_secs: u64,
}

fn valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

pub(super) async fn create_mcp_server(Json(request): Json<CreateMcpServerRequest>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let name = request.name.trim();
        if !valid_server_name(name) {
            return Err(
                "server name must use only letters, numbers, hyphens, or underscores".to_string(),
            );
        }
        let path = PathBuf::from(".forge/mcp.toml");
        let mut config = load_mutable_mcp_toml(&path)?;
        if config.servers.iter().any(|server| server.name == name) {
            return Err("a server with that name already exists".to_string());
        }
        let transport = match request.transport {
            McpTransportRequest::Stdio => {
                let command = request
                    .command
                    .filter(|command| !command.trim().is_empty())
                    .ok_or_else(|| "stdio servers need a command".to_string())?;
                forge_config::McpTransport::Stdio {
                    command,
                    args: request.args,
                    env: HashMap::new(),
                }
            }
            McpTransportRequest::Http => forge_config::McpTransport::Http {
                url: valid_mcp_url(request.url)?,
                headers: HashMap::new(),
            },
            McpTransportRequest::Sse => forge_config::McpTransport::Sse {
                url: valid_mcp_url(request.url)?,
                headers: HashMap::new(),
            },
        };
        let auth = request
            .token_env
            .filter(|name| !name.trim().is_empty())
            .map(|token_env| forge_config::McpAuth {
                token_env: Some(token_env),
                token_keyring: None,
                header: None,
                oauth: None,
            });
        config.servers.push(forge_config::McpServerConfig {
            name: name.to_string(),
            transport,
            auth,
            secret_env: Vec::new(),
            enabled: true,
        });
        forge_config::write_mcp_toml(&path, &config).map_err(|error| error.to_string())
    })
    .await;
    match result {
        Ok(Ok(())) => mcp_page().await,
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not save MCP server",
        ),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateMcpServerRequest {
    name: String,
    enabled: bool,
}

fn mcp_toml_scopes() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from(".forge/mcp.toml")];
    if let Some(dir) = forge_config::config_dir() {
        paths.push(dir.join("mcp.toml"));
    }
    paths
}

pub(super) async fn update_mcp_server(Json(request): Json<UpdateMcpServerRequest>) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        for path in mcp_toml_scopes() {
            let mut config = load_mutable_mcp_toml(&path)?;
            let Some(server) = config
                .servers
                .iter_mut()
                .find(|server| server.name == request.name)
            else {
                continue;
            };
            if server.enabled == request.enabled {
                return Ok(());
            }
            server.enabled = request.enabled;
            return forge_config::write_mcp_toml(&path, &config).map_err(|error| error.to_string());
        }
        Err(format!(
            "no server '{}' in .forge/mcp.toml or the user mcp.toml — edit it where it is defined",
            request.name
        ))
    })
    .await;
    match result {
        Ok(Ok(())) => mcp_page().await,
        Ok(Err(message)) => err_response(axum::http::StatusCode::NOT_FOUND, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not update MCP server",
        ),
    }
}

fn load_mutable_mcp_toml(path: &std::path::Path) -> Result<forge_config::McpConfig, String> {
    match std::fs::read_to_string(path) {
        Ok(body) => toml::from_str(&body)
            .map_err(|error| format!("refusing to overwrite invalid {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    }
}

fn valid_mcp_url(url: Option<String>) -> Result<String, String> {
    let url = url.ok_or_else(|| "HTTP and SSE servers need an http(s) URL".to_string())?;
    let parsed = reqwest::Url::parse(&url)
        .map_err(|_| "HTTP and SSE servers need a valid http(s) URL".to_string())?;
    if matches!(parsed.scheme(), "http" | "https") && parsed.host().is_some() {
        Ok(url)
    } else {
        Err("HTTP and SSE servers need a valid http(s) URL".to_string())
    }
}

pub(super) async fn mcp_page() -> Response {
    match tokio::task::spawn_blocking(|| {
        let config = forge_config::load().unwrap_or_default();
        let editable: HashSet<String> = mcp_toml_scopes()
            .iter()
            .flat_map(|path| forge_config::load_mcp_toml(path).servers)
            .map(|server| server.name)
            .collect();
        McpResponse {
            servers: config
                .mcp
                .servers
                .iter()
                .map(|server| McpServerRow {
                    name: server.name.clone(),
                    transport: server.transport_label().to_string(),
                    enabled: server.enabled,
                    auth_configured: server.auth.is_some(),
                    secret_env_count: server.secret_env.len(),
                    editable: editable.contains(&server.name),
                })
                .collect(),
            allowed_servers: config.mcp.allow.servers,
            allowed_tools: config.mcp.allow.tools,
            call_timeout_secs: config.mcp.call_timeout_secs,
            connect_timeout_secs: config.mcp.connect_timeout_secs,
        }
    })
    .await
    {
        Ok(response) => json_response(&response),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read MCP configuration",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{load_mutable_mcp_toml, valid_mcp_url, valid_server_name};

    #[test]
    fn server_names_match_the_persisted_catalog_contract() {
        for valid in ["git", "github-enterprise", "local_tools", "MCP2"] {
            assert!(valid_server_name(valid), "{valid}");
        }
        for invalid in ["", "two words", "slash/name", "dot.name", "é"] {
            assert!(!valid_server_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn remote_transports_require_http_or_https() {
        assert_eq!(
            valid_mcp_url(Some("https://mcp.example.test/sse".into())).unwrap(),
            "https://mcp.example.test/sse"
        );
        assert!(valid_mcp_url(Some("http://127.0.0.1:3000".into())).is_ok());
        assert!(valid_mcp_url(Some("https://".into())).is_err());
        assert!(valid_mcp_url(Some("file:///tmp/socket".into())).is_err());
        assert!(valid_mcp_url(None).is_err());
    }

    #[test]
    fn mutations_refuse_to_replace_malformed_catalogs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("mcp.toml");
        std::fs::write(&path, "not = [valid").unwrap();
        let error = load_mutable_mcp_toml(&path).unwrap_err();
        assert!(error.contains("refusing to overwrite invalid"), "{error}");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "not = [valid");
    }

    #[test]
    fn missing_mutable_catalog_starts_empty() {
        let temp = tempfile::tempdir().unwrap();
        let config = load_mutable_mcp_toml(&temp.path().join("missing.toml")).unwrap();
        assert!(config.servers.is_empty());
    }
}
