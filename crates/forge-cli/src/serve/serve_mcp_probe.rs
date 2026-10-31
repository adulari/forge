//! MCP probe, reload, edit, delete and built-in toggles for the Serve control surface.
//!
//! A probe connects to one server on a throwaway manager (with its own `disabled_tools` cleared so
//! the full tool list is visible), caches the outcome by server name, and shuts the child down.
//! `GET /api/mcp` only ever reads the cache, so listing never spawns a process.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use axum::extract::Json;
use axum::response::Response;
use axum::routing::{get, patch, post, MethodRouter};
use axum::Router;
use std::sync::Arc;

use super::serve_mcp::{
    load_mutable_mcp_toml, mcp_page, mcp_toml_scopes, valid_mcp_url, valid_server_name,
    McpTransportRequest,
};
use super::{err_response, json_response};

pub(super) fn mcp_route() -> MethodRouter<Arc<super::DaemonState>> {
    get(mcp_page)
        .post(super::serve_mcp::create_mcp_server)
        .patch(super::serve_mcp::update_mcp_server)
        .put(edit_mcp_server)
        .delete(delete_mcp_server)
}

pub(super) fn mcp_subroutes(base: &str) -> Router<Arc<super::DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/mcp/probe"), post(probe_mcp_server))
        .route(&format!("{base}/api/mcp/reload"), post(reload_mcp))
        .route(
            &format!("{base}/api/mcp/builtin"),
            patch(update_mcp_builtin),
        )
}

#[derive(Clone, serde::Serialize)]
pub(super) struct ProbeTool {
    name: String,
    description: String,
}

#[derive(Clone, serde::Serialize)]
pub(super) struct ProbeResource {
    uri: String,
    name: String,
}

#[derive(Clone, serde::Serialize)]
pub(super) struct ProbeResult {
    pub(super) status: String,
    pub(super) last_error: Option<String>,
    pub(super) tools: Vec<ProbeTool>,
    pub(super) resources: Vec<ProbeResource>,
}

fn cache() -> &'static Mutex<HashMap<String, ProbeResult>> {
    static CACHE: OnceLock<Mutex<HashMap<String, ProbeResult>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

pub(super) fn cached(name: &str) -> Option<ProbeResult> {
    cache().lock().ok()?.get(name).cloned()
}

fn forget(name: &str) {
    if let Ok(mut cache) = cache().lock() {
        cache.remove(name);
    }
}

async fn probe(config: &forge_config::McpConfig, name: &str) -> ProbeResult {
    let failed = |message: String| ProbeResult {
        status: "failed".into(),
        last_error: Some(message),
        tools: Vec::new(),
        resources: Vec::new(),
    };
    if !config.server_allowed(name) {
        return ProbeResult {
            status: "blocked".into(),
            last_error: Some("not in the [mcp.allow] servers list".into()),
            tools: Vec::new(),
            resources: Vec::new(),
        };
    }
    let mut one = config.clone();
    one.servers.retain(|server| server.name == name);
    one.allow = Default::default();
    for server in &mut one.servers {
        server.enabled = true;
        server.disabled_tools.clear();
    }
    if one.servers.is_empty() {
        return failed("server is not configured".into());
    }
    let manager = forge_mcp::McpManager::connect_all(&one).await;
    let line = manager.status_lines().into_iter().find(|l| l.name == name);
    let prefix = format!("{name}__");
    let result = match line {
        Some(line) => ProbeResult {
            last_error: (line.status == "failed" || line.status == "unauthorized")
                .then(|| line.detail.clone().unwrap_or_else(|| line.status.clone())),
            status: line.status,
            tools: manager
                .tool_lines(name)
                .into_iter()
                .map(|(qualified, description)| ProbeTool {
                    name: qualified
                        .strip_prefix(&prefix)
                        .unwrap_or(&qualified)
                        .to_string(),
                    description,
                })
                .collect(),
            resources: manager
                .resource_lines(name)
                .into_iter()
                .map(|(uri, name)| ProbeResource { uri, name })
                .collect(),
        },
        None => failed("server did not report a status".into()),
    };
    manager.shutdown().await;
    if let Ok(mut cache) = cache().lock() {
        cache.insert(name.to_string(), result.clone());
    }
    result
}

fn load_config() -> forge_config::McpConfig {
    forge_config::load().unwrap_or_default().mcp
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NameRequest {
    name: String,
}

#[derive(serde::Serialize)]
struct ProbeResponse {
    name: String,
    #[serde(flatten)]
    result: ProbeResult,
    disabled_tools: Vec<String>,
}

pub(super) async fn probe_mcp_server(Json(request): Json<NameRequest>) -> Response {
    let Ok(config) = tokio::task::spawn_blocking(load_config).await else {
        return err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read MCP configuration",
        );
    };
    let Some(server) = config.servers.iter().find(|s| s.name == request.name) else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "no such MCP server");
    };
    let disabled_tools = server.disabled_tools.clone();
    let result = probe(&config, &request.name).await;
    json_response(&ProbeResponse {
        name: request.name,
        result,
        disabled_tools,
    })
}

pub(super) async fn reload_mcp() -> Response {
    let Ok(config) = tokio::task::spawn_blocking(load_config).await else {
        return err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read MCP configuration",
        );
    };
    let mut set = tokio::task::JoinSet::new();
    for server in config.servers.iter().filter(|s| s.enabled) {
        let (config, name) = (config.clone(), server.name.clone());
        set.spawn(async move { probe(&config, &name).await });
    }
    while set.join_next().await.is_some() {}
    for server in config.servers.iter().filter(|s| !s.enabled) {
        forget(&server.name);
    }
    super::serve_mcp::mcp_listing(true).await
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EditMcpServerRequest {
    name: String,
    transport: McpTransportRequest,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    url: Option<String>,
    token_env: Option<String>,
}

pub(super) async fn edit_mcp_server(Json(request): Json<EditMcpServerRequest>) -> Response {
    let name = request.name.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        if !valid_server_name(&request.name) {
            return Err("invalid server name".into());
        }
        let transport = match request.transport {
            McpTransportRequest::Stdio => forge_config::McpTransport::Stdio {
                command: request
                    .command
                    .filter(|c| !c.trim().is_empty())
                    .ok_or_else(|| "stdio servers need a command".to_string())?,
                args: request.args,
                env: Default::default(),
            },
            McpTransportRequest::Http => forge_config::McpTransport::Http {
                url: valid_mcp_url(request.url)?,
                headers: Default::default(),
            },
            McpTransportRequest::Sse => forge_config::McpTransport::Sse {
                url: valid_mcp_url(request.url)?,
                headers: Default::default(),
            },
        };
        for path in mcp_toml_scopes() {
            let mut config = load_mutable_mcp_toml(&path)?;
            let Some(server) = config.servers.iter_mut().find(|s| s.name == request.name) else {
                continue;
            };
            let previous_env = match &server.transport {
                forge_config::McpTransport::Stdio { env, .. } => env.clone(),
                _ => Default::default(),
            };
            let previous_headers = match &server.transport {
                forge_config::McpTransport::Http { headers, .. }
                | forge_config::McpTransport::Sse { headers, .. } => headers.clone(),
                _ => Default::default(),
            };
            server.transport = match transport {
                forge_config::McpTransport::Stdio { command, args, .. } => {
                    forge_config::McpTransport::Stdio {
                        command,
                        args,
                        env: previous_env,
                    }
                }
                forge_config::McpTransport::Http { url, .. } => forge_config::McpTransport::Http {
                    url,
                    headers: previous_headers,
                },
                forge_config::McpTransport::Sse { url, .. } => forge_config::McpTransport::Sse {
                    url,
                    headers: previous_headers,
                },
            };
            match request.token_env.filter(|v| !v.trim().is_empty()) {
                Some(token_env) => {
                    let auth = server.auth.get_or_insert(forge_config::McpAuth {
                        token_env: None,
                        token_keyring: None,
                        header: None,
                        oauth: None,
                    });
                    auth.token_env = Some(token_env);
                }
                None => {
                    if let Some(auth) = server.auth.as_mut() {
                        auth.token_env = None;
                    }
                    if server
                        .auth
                        .as_ref()
                        .is_some_and(|a| a.token_keyring.is_none() && a.oauth.is_none())
                    {
                        server.auth = None;
                    }
                }
            }
            return forge_config::write_mcp_toml(&path, &config).map_err(|e| e.to_string());
        }
        Err(format!(
            "no server '{}' in .forge/mcp.toml or the user mcp.toml — edit it where it is defined",
            request.name
        ))
    })
    .await;
    forget(&name);
    match result {
        Ok(Ok(())) => super::serve_mcp::mcp_listing(true).await,
        Ok(Err(message)) => err_response(axum::http::StatusCode::NOT_FOUND, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not edit MCP server",
        ),
    }
}

pub(super) async fn delete_mcp_server(Json(request): Json<NameRequest>) -> Response {
    let name = request.name.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        for path in mcp_toml_scopes() {
            let mut config = load_mutable_mcp_toml(&path)?;
            let before = config.servers.len();
            config.servers.retain(|s| s.name != request.name);
            if config.servers.len() != before {
                return forge_config::write_mcp_toml(&path, &config).map_err(|e| e.to_string());
            }
        }
        Err(format!(
            "no server '{}' in .forge/mcp.toml or the user mcp.toml — remove it where it is defined",
            request.name
        ))
    })
    .await;
    forget(&name);
    match result {
        Ok(Ok(())) => super::serve_mcp::mcp_listing(true).await,
        Ok(Err(message)) => err_response(axum::http::StatusCode::NOT_FOUND, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not remove MCP server",
        ),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BuiltinRequest {
    id: String,
    enabled: bool,
}

pub(super) async fn update_mcp_builtin(Json(request): Json<BuiltinRequest>) -> Response {
    if request.id != "self" {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "only the Forge self-MCP server can be toggled",
        );
    }
    let result = tokio::task::spawn_blocking(move || {
        let _lock = super::serve_config::mutation_lock()
            .lock()
            .map_err(|_| "configuration mutation lock is poisoned".to_string())?;
        forge_config::write_self_mcp(request.enabled)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await;
    match result {
        Ok(Ok(())) => super::serve_mcp::mcp_listing(true).await,
        Ok(Err(message)) => err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not update built-in",
        ),
    }
}
