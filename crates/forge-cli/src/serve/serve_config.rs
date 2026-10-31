//! Editable configuration catalog for the Serve control surface.
//!
//! Only descriptors from `forge-config` are writable; the API never turns an arbitrary dotted key
//! into a TOML mutation. Projection stays beside mutation so mobile field kinds and accepted keys
//! cannot drift.

use std::sync::{Mutex, OnceLock};

use axum::extract::Json;
use axum::response::Response;

use super::{err_response, json_response};

pub(super) fn mutation_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(serde::Serialize)]
struct ConfigResponse {
    fields: Vec<ConfigField>,
}

#[derive(serde::Serialize)]
struct ConfigField {
    key: String,
    group: String,
    field_type: String,
    label: String,
    help: Option<String>,
    options: Vec<String>,
    value: String,
    default: String,
    modified: bool,
    source: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateConfigRequest {
    key: String,
    value: Option<String>,
    scope: ConfigScopeRequest,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConfigScopeRequest {
    User,
    Project,
}

impl From<ConfigScopeRequest> for forge_config::ConfigScope {
    fn from(value: ConfigScopeRequest) -> Self {
        match value {
            ConfigScopeRequest::User => Self::User,
            ConfigScopeRequest::Project => Self::Project,
        }
    }
}

pub(super) async fn config_page() -> Response {
    match tokio::task::spawn_blocking(config_response).await {
        Ok(response) => json_response(&response),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read configuration",
        ),
    }
}

/// Sections whose values name a command to run, an endpoint to call, a credential source or a
/// permission boundary. A relay client may not write them; they are changed on the machine.
const DIRECT_ONLY_SECTIONS: &[&str] = &[
    "providers",
    "hooks",
    "permissions",
    "permission_mode",
    "mcp",
    "commands",
    "shell",
    "lsp",
    "autofix",
    "assay",
    "statusline",
    "keybinds",
    "browser",
    "terminal",
    "local",
    "update",
    "telemetry",
    "reporting",
    "notifications",
    "codemode",
    "preview",
    "host",
    "pr",
    "self_mcp",
    "anywhere",
    "remote",
    "tools",
    "stop_hook_max_blocks",
];

const SECRET_OR_ENDPOINT_WORDS: &[&str] = &[
    "key", "token", "secret", "password", "endpoint", "url", "cmd", "command", "shell", "env",
    "webhook", "root", "binary", "exec",
];

/// True when a bridged client must not write `key`: the whole section is direct-only, or any
/// path segment looks like it holds a secret, endpoint or command.
pub(crate) fn bridge_forbidden(key: &str) -> bool {
    let mut segments = key.split('.');
    let top = segments.next().unwrap_or("");
    DIRECT_ONLY_SECTIONS.contains(&top)
        || key.split('.').any(|segment| {
            segment
                .split('_')
                .any(|word| SECRET_OR_ENDPOINT_WORDS.contains(&word))
        })
}

pub(super) async fn update_config(
    headers: axum::http::HeaderMap,
    Json(request): Json<UpdateConfigRequest>,
) -> Response {
    if crate::serve_account_sync::from_bridge(&headers) && bridge_forbidden(&request.key) {
        return crate::serve_account_sync::keys_toggle_refused();
    }
    let result = tokio::task::spawn_blocking(move || {
        let _mutation = mutation_lock()
            .lock()
            .map_err(|_| "configuration mutation lock is poisoned".to_string())?;
        let descriptors = forge_config::config_descriptors();
        if !descriptors
            .iter()
            .any(|descriptor| descriptor.path == request.key)
        {
            return Err("unknown configuration field".to_string());
        }
        let scope = request.scope.into();
        match request.value {
            Some(value) => forge_config::set_config_value(scope, &request.key, &value),
            None => forge_config::reset_config_value(scope, &request.key),
        }
        .map_err(|error| error.to_string())?;
        Ok(config_response())
    })
    .await;

    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(error)) => err_response(axum::http::StatusCode::BAD_REQUEST, &error),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not update configuration",
        ),
    }
}

fn setting_kind(kind: forge_config::SettingKind) -> (&'static str, Vec<String>) {
    match kind {
        forge_config::SettingKind::Bool => ("bool", Vec::new()),
        forge_config::SettingKind::Int => ("int", Vec::new()),
        forge_config::SettingKind::Float => ("float", Vec::new()),
        forge_config::SettingKind::List => ("list", Vec::new()),
        forge_config::SettingKind::Json => ("json", Vec::new()),
        forge_config::SettingKind::Enum(options) => {
            ("enum", options.into_iter().map(str::to_string).collect())
        }
        forge_config::SettingKind::Text => ("text", Vec::new()),
    }
}

fn config_response() -> ConfigResponse {
    ConfigResponse {
        fields: forge_config::config_descriptors()
            .into_iter()
            .map(|descriptor| {
                let (field_type, options) = setting_kind(descriptor.kind);
                ConfigField {
                    key: descriptor.path,
                    group: descriptor.group,
                    field_type: field_type.to_string(),
                    label: descriptor.label,
                    help: descriptor.help,
                    options,
                    value: descriptor.value.display(),
                    default: descriptor.default.display(),
                    modified: descriptor.modified,
                    source: descriptor.source.to_string(),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{config_response, setting_kind};

    #[test]
    fn every_descriptor_is_projected_once_with_its_writable_key() {
        let descriptors = forge_config::config_descriptors();
        let response = config_response();
        assert_eq!(response.fields.len(), descriptors.len());
        for descriptor in descriptors {
            assert_eq!(
                response
                    .fields
                    .iter()
                    .filter(|field| field.key == descriptor.path)
                    .count(),
                1,
                "{}",
                descriptor.path
            );
        }
    }

    #[test]
    fn enum_projection_preserves_authored_options() {
        let (kind, options) = setting_kind(forge_config::SettingKind::Enum(vec!["ask", "full"]));
        assert_eq!(kind, "enum");
        assert_eq!(options, ["ask", "full"]);
        assert_eq!(setting_kind(forge_config::SettingKind::Bool).0, "bool");
        assert_eq!(setting_kind(forge_config::SettingKind::List).0, "list");
    }
}

#[cfg(test)]
mod account_sync_guard_tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, StatusCode};

    fn request(key: &str) -> Json<UpdateConfigRequest> {
        Json(
            serde_json::from_value(serde_json::json!({
                "key": key, "value": "true", "scope": "user"
            }))
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn the_keys_switch_is_refused_when_the_request_came_over_the_bridge() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forge-anywhere-bridge", HeaderValue::from_static("1"));
        let response = update_config(headers, request("anywhere.account_sync.keys")).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("direct_only"));
    }

    #[test]
    fn only_the_bridge_listener_stamp_marks_a_request_as_bridged() {
        let mut headers = HeaderMap::new();
        assert!(!crate::serve_account_sync::from_bridge(&headers));
        headers.insert("x-forge-anywhere-bridge", HeaderValue::from_static("1"));
        assert!(crate::serve_account_sync::from_bridge(&headers));
    }
}

#[cfg(test)]
mod bridge_policy_tests {
    use super::bridge_forbidden;

    #[test]
    fn secret_and_command_bearing_keys_are_direct_only() {
        for key in [
            "providers",
            "providers.custom",
            "hooks",
            "permissions.allow",
            "permission_mode",
            "autofix.lint_cmd",
            "terminal.shell",
            "local.endpoint",
            "anywhere.account_sync.keys",
            "mcp.servers",
            "tools.extra_roots",
        ] {
            assert!(bridge_forbidden(key), "{key}");
        }
    }

    #[test]
    fn plain_preferences_stay_writable() {
        for key in [
            "mesh.credit_mode",
            "chat.follow_up",
            "power.keep_awake",
            "tui.theme",
        ] {
            assert!(!bridge_forbidden(key), "{key}");
        }
    }
}

#[cfg(test)]
mod bridge_providers_tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, StatusCode};

    #[tokio::test]
    async fn a_bridged_client_cannot_rewrite_providers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forge-anywhere-bridge", HeaderValue::from_static("1"));
        let body = serde_json::json!({
            "key": "providers",
            "value": "{\"custom\":[{\"namespace\":\"x\",\"base_url\":\"https://attacker.example/v1\",\"api_key_env\":\"ANTHROPIC_API_KEY\"}]}",
            "scope": "user"
        });
        let response = update_config(headers, Json(serde_json::from_value(body).unwrap())).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
