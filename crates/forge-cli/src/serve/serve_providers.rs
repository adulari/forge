//! Direct-only provider and account management for the Serve control surface.
//!
//! This module intentionally does not participate in the Forge Anywhere typed transport. Provider
//! metadata and mutations are available only to a client connected directly to the daemon's
//! token-authenticated HTTP origin. Responses expose environment-variable names, booleans, and
//! masked fingerprints; raw API keys and OAuth tokens never cross this boundary.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use axum::extract::{Json, Path, State};
use axum::response::Response;

use super::serve_config::mutation_lock;
use super::{err_response, json_response, DaemonState};

mod validation;
use validation::*;

const MAX_PROVIDER_ID_CHARS: usize = 64;
const MAX_ACCOUNT_ID_CHARS: usize = 256;
const MAX_API_KEY_CHARS: usize = 16 * 1024;
const MAX_ENDPOINT_CHARS: usize = 2 * 1024;
const MAX_LABEL_CHARS: usize = 160;
const MAX_MODELS: usize = 100;
const MAX_MODEL_CHARS: usize = 256;

#[derive(serde::Serialize)]
struct ProvidersResponse {
    direct_only: bool,
    restart_required: bool,
    notice: Option<String>,
    providers: Vec<ProviderRow>,
}

#[derive(serde::Serialize)]
struct ProviderRow {
    id: String,
    label: String,
    kind: &'static str,
    enabled: bool,
    configured: bool,
    auth_status: &'static str,
    keyless: bool,
    env_var: Option<String>,
    environment_key_present: bool,
    stored_key_fingerprints: Vec<String>,
    free: bool,
    endpoint: Option<String>,
    azure_resource: Option<String>,
    azure_api_version: Option<String>,
    models: Vec<String>,
    accounts: Vec<OAuthAccountRow>,
    login_command: Option<&'static str>,
    installed: Option<bool>,
    version: Option<String>,
    serving: Option<bool>,
    restart_required: bool,
}

#[derive(serde::Serialize)]
struct OAuthAccountRow {
    id: String,
    active: bool,
    expires_at: Option<i64>,
    expiry_status: &'static str,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoreProviderKeyRequest {
    key: String,
    mode: StoreKeyMode,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoreKeyMode {
    Append,
    Replace,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetProviderEnabledRequest {
    enabled: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OAuthAccountRequest {
    account_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CustomProviderRequest {
    namespace: String,
    base_url: String,
    api_key_env: Option<String>,
    #[serde(default)]
    free: bool,
    #[serde(default)]
    models: Vec<String>,
    label: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AzureProviderRequest {
    resource: Option<String>,
    endpoint: Option<String>,
    api_version: Option<String>,
    api_key_env: Option<String>,
    #[serde(default)]
    deployments: Vec<String>,
    #[serde(default)]
    free: bool,
    label: Option<String>,
}

pub(super) async fn providers_page() -> Response {
    match load_provider_response(None, false).await {
        Ok(response) => json_response(&response),
        Err(error) => err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &error),
    }
}

pub(super) async fn store_provider_key(
    State(state): State<Arc<DaemonState>>,
    Path(provider): Path<String>,
    Json(request): Json<StoreProviderKeyRequest>,
) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let provider = validate_provider_id(&provider)?;
        if !known_keyed_provider(provider) {
            return Err(format!("'{provider}' is not a keyed provider"));
        }
        let key = validate_api_key(&request.key)?;
        let _mutation = mutation_lock()
            .lock()
            .map_err(|_| "configuration mutation lock is poisoned".to_string())?;
        match request.mode {
            StoreKeyMode::Append => forge_config::add_api_key(provider, key).map(|_| ()),
            StoreKeyMode::Replace => forge_config::store_api_key(provider, key),
        }
        .map_err(|error| error.to_string())?;
        after_credential_change(&store, provider, true);
        Ok(())
    })
    .await;
    mutation_response(
        result,
        Some("Stored securely. Raw key material is never returned by this API.".to_string()),
        false,
        "could not store provider key",
    )
    .await
}

pub(super) async fn remove_provider_keys(
    State(state): State<Arc<DaemonState>>,
    Path(provider): Path<String>,
) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let provider = validate_provider_id(&provider)?;
        if !known_keyed_provider(provider) {
            return Err(format!("'{provider}' is not a keyed provider"));
        }
        let _mutation = mutation_lock()
            .lock()
            .map_err(|_| "configuration mutation lock is poisoned".to_string())?;
        let removed = forge_config::remove_api_key(provider).map_err(|error| error.to_string())?;
        after_credential_change(&store, provider, false);
        Ok(removed)
    })
    .await;

    let notice = match &result {
        Ok(Ok(true)) => Some(
            "Removed Forge-owned stored keys. Environment-provided keys are unchanged.".to_string(),
        ),
        Ok(Ok(false)) => Some(
            "No Forge-owned key was stored. Environment-provided keys are unchanged.".to_string(),
        ),
        _ => None,
    };
    mutation_response(
        result.map(|inner| inner.map(|_| ())),
        notice,
        false,
        "could not remove provider keys",
    )
    .await
}

pub(super) async fn set_provider_enabled(
    Path(provider): Path<String>,
    Json(request): Json<SetProviderEnabledRequest>,
) -> Response {
    let desired = request.enabled;
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let provider = validate_provider_id(&provider)?;
        if !known_provider(provider) {
            return Err(format!("unknown provider '{provider}'"));
        }
        let _mutation = mutation_lock()
            .lock()
            .map_err(|_| "configuration mutation lock is poisoned".to_string())?;
        let mut disabled = user_disabled_providers()?;
        if desired {
            disabled.retain(|entry| entry != provider);
        } else if !disabled.iter().any(|entry| entry == provider) {
            disabled.push(provider.to_string());
        }
        let raw = serde_json::to_string(&disabled).map_err(|error| error.to_string())?;
        forge_config::set_config_value(forge_config::ConfigScope::User, "mesh.disabled", &raw)
            .map_err(|error| error.to_string())?;
        crate::cli::commands::models::invalidate_catalog_cache();
        let effective = forge_config::load()
            .map_err(|error| error.to_string())?
            .mesh
            .disabled
            .iter()
            .all(|entry| entry != provider);
        Ok(effective)
    })
    .await;

    let notice = match &result {
        Ok(Ok(effective)) if *effective == desired => Some(
            "Saved for new sessions. Running sessions retain their current provider router."
                .to_string(),
        ),
        Ok(Ok(_)) => Some(
            "Saved in user settings, but a higher-precedence project or environment override keeps \
             the effective state unchanged."
                .to_string(),
        ),
        _ => None,
    };
    mutation_response(
        result.map(|inner| inner.map(|_| ())),
        notice,
        false,
        "could not update provider state",
    )
    .await
}

pub(super) async fn switch_oauth_account(
    State(state): State<Arc<DaemonState>>,
    Path(provider): Path<String>,
    Json(request): Json<OAuthAccountRequest>,
) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let (_, keyring_provider) = oauth_provider(&provider)?;
        let account_id = validate_account_id(&request.account_id)?;
        let _mutation = mutation_lock()
            .lock()
            .map_err(|_| "configuration mutation lock is poisoned".to_string())?;
        forge_config::provider_oauth::switch_provider_oauth_account(keyring_provider, account_id)
            .map_err(|error| error.to_string())?;
        after_credential_change(&store, &provider, true);
        Ok(())
    })
    .await;
    mutation_response(
        result,
        Some("Active subscription account changed immediately.".to_string()),
        false,
        "could not switch OAuth account",
    )
    .await
}

pub(super) async fn remove_oauth_account(
    State(state): State<Arc<DaemonState>>,
    Path((provider, account_id)): Path<(String, String)>,
) -> Response {
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let (_, keyring_provider) = oauth_provider(&provider)?;
        let account_id = validate_account_id(&account_id)?;
        let _mutation = mutation_lock()
            .lock()
            .map_err(|_| "configuration mutation lock is poisoned".to_string())?;
        let removed = forge_config::provider_oauth::remove_provider_oauth_account(
            keyring_provider,
            account_id,
        )
        .map_err(|error| error.to_string())?;
        if !removed {
            return Err("OAuth account was not found".to_string());
        }
        after_credential_change(&store, &provider, false);
        Ok(())
    })
    .await;
    mutation_response(
        result,
        Some("Removed the selected subscription account from secure storage.".to_string()),
        false,
        "could not remove OAuth account",
    )
    .await
}

pub(super) async fn save_custom_provider(Json(request): Json<CustomProviderRequest>) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let config = validated_custom_provider(request)?;
        let _mutation = mutation_lock()
            .lock()
            .map_err(|_| "configuration mutation lock is poisoned".to_string())?;
        forge_config::add_custom_provider(&config).map_err(|error| error.to_string())?;
        crate::cli::commands::models::invalidate_catalog_cache();
        Ok(())
    })
    .await;
    mutation_response(
        result,
        Some(
            "Custom endpoint saved. Restart the Forge daemon before routing new sessions through it."
                .to_string(),
        ),
        true,
        "could not save custom provider",
    )
    .await
}

pub(super) async fn remove_custom_provider(Path(namespace): Path<String>) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let namespace = validate_provider_id(&namespace)?;
        let exists = forge_config::user_custom_providers()
            .iter()
            .any(|provider| provider.namespace == namespace);
        if !exists {
            return Err(format!("custom provider '{namespace}' was not found"));
        }
        let _mutation = mutation_lock()
            .lock()
            .map_err(|_| "configuration mutation lock is poisoned".to_string())?;
        forge_config::remove_api_key(namespace).map_err(|error| error.to_string())?;
        let removed =
            forge_config::remove_custom_provider(namespace).map_err(|error| error.to_string())?;
        if !removed {
            return Err(format!("custom provider '{namespace}' was not found"));
        }
        crate::cli::commands::models::invalidate_catalog_cache();
        forge_provider::invalidate_plan_cache();
        Ok(())
    })
    .await;
    mutation_response(
        result,
        Some(
            "Custom endpoint and its Forge-owned keys were removed. Environment variables are \
             unchanged; restart the daemon to unload the endpoint."
                .to_string(),
        ),
        true,
        "could not remove custom provider",
    )
    .await
}

pub(super) async fn save_azure_provider(Json(request): Json<AzureProviderRequest>) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let config = validated_azure_provider(request)?;
        let _mutation = mutation_lock()
            .lock()
            .map_err(|_| "configuration mutation lock is poisoned".to_string())?;
        forge_config::add_azure_provider(&config).map_err(|error| error.to_string())?;
        crate::cli::commands::models::invalidate_catalog_cache();
        Ok(())
    })
    .await;
    mutation_response(
        result,
        Some(
            "Azure provider saved. Restart the Forge daemon before routing new sessions through it."
                .to_string(),
        ),
        true,
        "could not save Azure provider",
    )
    .await
}

pub(super) async fn remove_azure_provider() -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        if forge_config::user_azure_config().is_none() {
            return Err("Azure provider was not configured".to_string());
        }
        let _mutation = mutation_lock()
            .lock()
            .map_err(|_| "configuration mutation lock is poisoned".to_string())?;
        forge_config::remove_api_key(forge_config::AZURE_NS).map_err(|error| error.to_string())?;
        let removed = forge_config::remove_azure_provider().map_err(|error| error.to_string())?;
        if !removed {
            return Err("Azure provider was not configured".to_string());
        }
        crate::cli::commands::models::invalidate_catalog_cache();
        forge_provider::invalidate_plan_cache();
        Ok(())
    })
    .await;
    mutation_response(
        result,
        Some(
            "Azure configuration and Forge-owned keys were removed. Environment variables are \
             unchanged; restart the daemon to unload the provider."
                .to_string(),
        ),
        true,
        "could not remove Azure provider",
    )
    .await
}

async fn mutation_response<T>(
    result: Result<Result<T, String>, tokio::task::JoinError>,
    notice: Option<String>,
    restart_required: bool,
    join_error: &'static str,
) -> Response {
    match result {
        Ok(Ok(_)) => match load_provider_response(notice, restart_required).await {
            Ok(response) => json_response(&response),
            Err(error) => err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &error),
        },
        Ok(Err(error)) => err_response(axum::http::StatusCode::BAD_REQUEST, &error),
        Err(_) => err_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, join_error),
    }
}

async fn load_provider_response(
    notice: Option<String>,
    restart_required: bool,
) -> Result<ProvidersResponse, String> {
    let response =
        tokio::task::spawn_blocking(move || build_provider_response(notice, restart_required))
            .await
            .map_err(|_| "could not read provider configuration".to_string())?;

    let (claude, codex, antigravity) = tokio::join!(
        forge_provider::CliKind::ClaudeCode.cli_version(),
        forge_provider::CliKind::Codex.cli_version(),
        forge_provider::CliKind::Antigravity.cli_version(),
    );
    let mut response = response;
    for (id, version) in [
        ("claude-cli", claude),
        ("codex-cli", codex),
        ("agy-cli", antigravity),
    ] {
        if let Some(row) = response.providers.iter_mut().find(|row| row.id == id) {
            row.version = version;
        }
    }
    Ok(response)
}

fn build_provider_response(notice: Option<String>, restart_required: bool) -> ProvidersResponse {
    let config = forge_config::load().unwrap_or_default();
    let disabled = &config.mesh.disabled;
    let mut providers = Vec::new();

    providers.push(oauth_row(
        "codex-oauth",
        "OpenAI ChatGPT — subscription OAuth",
        forge_config::provider_oauth::CODEX_OAUTH_KEYRING_PROVIDER,
        "forge auth codex-oauth",
        disabled,
    ));
    providers.push(oauth_row(
        "xai-oauth",
        "xAI Grok — subscription OAuth",
        forge_config::provider_oauth::XAI_OAUTH_KEYRING_PROVIDER,
        "forge auth xai-oauth",
        disabled,
    ));

    for (kind, label, login_command) in [
        (
            forge_provider::CliKind::ClaudeCode,
            "Claude Code — Pro/Max CLI bridge",
            "claude",
        ),
        (
            forge_provider::CliKind::Codex,
            "OpenAI Codex — ChatGPT CLI bridge",
            "codex login",
        ),
        (
            forge_provider::CliKind::Antigravity,
            "Google Antigravity — CLI bridge",
            "agy",
        ),
    ] {
        let id = kind.prefix();
        let installed = kind.available();
        let models = config
            .mesh
            .bridge_models
            .get(id)
            .cloned()
            .unwrap_or_else(|| {
                kind.default_models()
                    .iter()
                    .map(|model| (*model).to_string())
                    .collect()
            });
        providers.push(ProviderRow {
            id: id.to_string(),
            label: label.to_string(),
            kind: "cli",
            enabled: provider_enabled(id, disabled),
            configured: installed,
            auth_status: if installed { "unverified" } else { "missing" },
            keyless: true,
            env_var: None,
            environment_key_present: false,
            stored_key_fingerprints: Vec::new(),
            free: false,
            endpoint: None,
            azure_resource: None,
            azure_api_version: None,
            models,
            accounts: Vec::new(),
            login_command: Some(login_command),
            installed: Some(installed),
            version: None,
            serving: None,
            restart_required: false,
        });
    }

    let ollama_installed = crate::local::ollama_installed();
    let ollama_serving = crate::local::ollama_serving();
    providers.push(ProviderRow {
        id: "ollama".to_string(),
        label: "Ollama — local models".to_string(),
        kind: "local",
        enabled: provider_enabled("ollama", disabled),
        configured: ollama_serving,
        auth_status: if ollama_serving {
            "ready"
        } else if ollama_installed {
            "stopped"
        } else {
            "missing"
        },
        keyless: true,
        env_var: None,
        environment_key_present: false,
        stored_key_fingerprints: Vec::new(),
        free: true,
        endpoint: Some(config.local.endpoint.clone()),
        azure_resource: None,
        azure_api_version: None,
        models: config.local.model.clone().into_iter().collect(),
        accounts: Vec::new(),
        login_command: Some("forge local status"),
        installed: Some(ollama_installed),
        version: crate::local::ollama_version(),
        serving: Some(ollama_serving),
        restart_required: false,
    });

    let user_custom: HashMap<String, forge_config::CustomProviderConfig> =
        forge_config::user_custom_providers()
            .into_iter()
            .map(|provider| (provider.namespace.clone(), provider))
            .collect();
    let mut keyed: BTreeSet<String> = forge_config::known_key_providers()
        .filter(|provider| {
            forge_config::custom_provider(provider).is_none()
                || forge_config::CUSTOM_OPENAI_PROVIDERS
                    .iter()
                    .any(|builtin| builtin.namespace == *provider)
                || user_custom.contains_key(*provider)
        })
        .map(str::to_string)
        .collect();
    keyed.extend(user_custom.keys().cloned());
    keyed.remove(forge_config::AZURE_NS);

    for provider in keyed {
        providers.push(api_key_row(&provider, user_custom.get(&provider), disabled));
    }

    if let Some(azure) = forge_config::user_azure_config() {
        providers.push(azure_row(azure, disabled));
    }

    ProvidersResponse {
        direct_only: true,
        restart_required,
        notice,
        providers,
    }
}

fn api_key_row(
    provider: &str,
    user_custom: Option<&forge_config::CustomProviderConfig>,
    disabled: &[String],
) -> ProviderRow {
    let stored_key_fingerprints = forge_config::stored_api_key_fingerprints(provider);
    let (label, env_var, environment_key_present, free, endpoint, models, pending_restart) =
        if let Some(custom) = user_custom {
            let env_var = custom
                .api_key_env
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            (
                custom
                    .label
                    .clone()
                    .filter(|label| !label.trim().is_empty())
                    .unwrap_or_else(|| format!("{provider} — custom OpenAI endpoint")),
                env_var.clone(),
                env_var.as_deref().is_some_and(env_family_present),
                custom.free,
                Some(custom.base_url.clone()),
                custom.models.clone(),
                custom_provider_pending_restart(custom),
            )
        } else if let Some(custom) = forge_config::custom_provider(provider) {
            (
                custom.label.to_string(),
                (!custom.env_var.is_empty()).then(|| custom.env_var.to_string()),
                forge_config::provider_environment_key_present(provider),
                custom.free,
                Some(custom.endpoint.to_string()),
                custom
                    .seed_models
                    .iter()
                    .map(|model| (*model).to_string())
                    .collect(),
                false,
            )
        } else {
            (
                crate::cli::commands::local::provider_label(provider).to_string(),
                forge_config::provider_key_env_var(provider).map(str::to_string),
                forge_config::provider_environment_key_present(provider),
                false,
                None,
                Vec::new(),
                false,
            )
        };
    let keyless = env_var.is_none();
    let configured = keyless || environment_key_present || !stored_key_fingerprints.is_empty();
    ProviderRow {
        id: provider.to_string(),
        label,
        kind: if user_custom.is_some() {
            "custom"
        } else {
            "api_key"
        },
        enabled: provider_enabled(provider, disabled),
        configured,
        auth_status: if configured { "configured" } else { "missing" },
        keyless,
        env_var,
        environment_key_present,
        stored_key_fingerprints,
        free,
        endpoint,
        azure_resource: None,
        azure_api_version: None,
        models,
        accounts: Vec::new(),
        login_command: None,
        installed: None,
        version: None,
        serving: None,
        restart_required: pending_restart,
    }
}

fn azure_row(config: forge_config::AzureConfig, disabled: &[String]) -> ProviderRow {
    let resolved = config.clone().into_provider().ok();
    let env_var = config
        .api_key_env
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| forge_config::AZURE_DEFAULT_KEY_ENV.to_string());
    let environment_key_present = env_family_present(&env_var);
    let stored_key_fingerprints = forge_config::stored_api_key_fingerprints(forge_config::AZURE_NS);
    let configured = environment_key_present || !stored_key_fingerprints.is_empty();
    ProviderRow {
        id: forge_config::AZURE_NS.to_string(),
        label: config
            .label
            .clone()
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| "Azure OpenAI".to_string()),
        kind: "azure",
        enabled: provider_enabled(forge_config::AZURE_NS, disabled),
        configured,
        auth_status: if configured { "configured" } else { "missing" },
        keyless: false,
        env_var: Some(env_var),
        environment_key_present,
        stored_key_fingerprints,
        free: config.free,
        endpoint: resolved.as_ref().map(|provider| provider.endpoint.clone()),
        azure_resource: config.resource.clone(),
        azure_api_version: config.api_version.clone(),
        models: config.deployments.clone(),
        accounts: Vec::new(),
        login_command: None,
        installed: None,
        version: None,
        serving: None,
        restart_required: azure_provider_pending_restart(&config),
    }
}

fn oauth_row(
    id: &str,
    label: &str,
    keyring_provider: &str,
    login_command: &'static str,
    disabled: &[String],
) -> ProviderRow {
    let now = unix_now();
    let accounts: Vec<OAuthAccountRow> =
        forge_config::provider_oauth::list_provider_oauth_accounts(keyring_provider)
            .into_iter()
            .map(|(id, tokens, active)| OAuthAccountRow {
                id,
                active,
                expires_at: (tokens.expires_at > 0).then_some(tokens.expires_at),
                expiry_status: if tokens.expires_at == 0 {
                    "unknown"
                } else if tokens.expires_at <= now {
                    "expired"
                } else {
                    "valid"
                },
            })
            .collect();
    let configured = !accounts.is_empty();
    let auth_status = if accounts
        .iter()
        .any(|account| account.active && account.expiry_status == "expired")
    {
        "expired"
    } else if configured {
        "configured"
    } else {
        "missing"
    };
    ProviderRow {
        id: id.to_string(),
        label: label.to_string(),
        kind: "oauth",
        enabled: provider_enabled(id, disabled),
        configured,
        auth_status,
        keyless: true,
        env_var: None,
        environment_key_present: false,
        stored_key_fingerprints: Vec::new(),
        free: false,
        endpoint: None,
        azure_resource: None,
        azure_api_version: None,
        models: Vec::new(),
        accounts,
        login_command: Some(login_command),
        installed: None,
        version: None,
        serving: None,
        restart_required: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_and_account_identifiers_are_closed_and_bounded() {
        for valid in ["openai", "codex-oauth", "local_provider_2"] {
            assert_eq!(validate_provider_id(valid).unwrap(), valid);
        }
        for invalid in ["", "two words", "provider/name", "provider.name", "é"] {
            assert!(validate_provider_id(invalid).is_err(), "{invalid}");
        }
        assert!(validate_provider_id(&"a".repeat(MAX_PROVIDER_ID_CHARS + 1)).is_err());
        assert!(validate_account_id("person@example.test").is_ok());
        assert!(validate_account_id("line\nbreak").is_err());
    }

    #[test]
    fn api_keys_reject_empty_multiline_control_and_oversized_values() {
        assert_eq!(validate_api_key("  sk-valid  ").unwrap(), "sk-valid");
        assert!(validate_api_key(" ").is_err());
        assert!(validate_api_key("one\ntwo").is_err());
        assert!(validate_api_key("one\u{7f}two").is_err());
        assert!(validate_api_key(&"x".repeat(MAX_API_KEY_CHARS + 1)).is_err());
    }

    #[test]
    fn oauth_routes_cannot_select_arbitrary_keyring_names() {
        assert_eq!(oauth_provider("codex-oauth").unwrap().1, "codex");
        assert_eq!(oauth_provider("xai-oauth").unwrap().1, "xai");
        for invalid in ["codex", "xai", "mcp-oauth", "provider-oauth:codex"] {
            assert!(oauth_provider(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn custom_and_azure_requests_are_validated_before_persistence() {
        let custom = validated_custom_provider(CustomProviderRequest {
            namespace: "lmstudio".into(),
            base_url: "http://127.0.0.1:1234/v1".into(),
            api_key_env: Some("LMSTUDIO_API_KEY".into()),
            free: true,
            models: vec!["qwen-coder".into()],
            label: Some("LM Studio".into()),
        })
        .unwrap();
        assert_eq!(custom.namespace, "lmstudio");
        assert!(validated_custom_provider(CustomProviderRequest {
            namespace: "bad/name".into(),
            base_url: "file:///tmp/model".into(),
            api_key_env: Some("lowercase".into()),
            free: true,
            models: Vec::new(),
            label: None,
        })
        .is_err());

        let azure = validated_azure_provider(AzureProviderRequest {
            resource: Some("acme".into()),
            endpoint: None,
            api_version: None,
            api_key_env: None,
            deployments: vec!["gpt-4o".into()],
            free: false,
            label: None,
        })
        .unwrap();
        assert_eq!(azure.resource.as_deref(), Some("acme"));
    }

    #[test]
    fn disabled_provider_parser_refuses_malformed_or_mistyped_config() {
        assert_eq!(
            parse_disabled_providers("[mesh]\ndisabled = [\"openai\", \"ollama\"]").unwrap(),
            ["openai", "ollama"]
        );
        assert!(parse_disabled_providers("[mesh]\ndisabled = \"openai\"").is_err());
        assert!(parse_disabled_providers("[mesh]\ndisabled = [\"openai\", 2]").is_err());
        assert!(parse_disabled_providers("[mesh\ndisabled = []").is_err());
        assert!(parse_disabled_providers("[mesh]\nself_review = true")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn serialized_provider_rows_never_include_stored_key_material() {
        forge_config::store_api_key("openai", "raw-secret-NEVER-SERIALIZE").unwrap();
        let row = api_key_row("openai", None, &[]);
        let json = serde_json::to_string(&row).unwrap();
        forge_config::remove_api_key("openai").unwrap();

        assert!(!json.contains("raw-secret-NEVER-SERIALIZE"));
        assert!(json.contains("…LIZE"));
        assert!(json.contains("\"environment_key_present\""));
    }

    #[tokio::test]
    async fn daemon_routes_store_and_remove_keys_without_echoing_them() {
        use tower::ServiceExt;

        let state = Arc::new(DaemonState {
            registry: Arc::new(super::super::SessionRegistry::new()),
            terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: std::env::temp_dir().display().to_string(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = super::super::daemon_router(state);
        let key = "route-secret-NEVER-ECHO-BED1";
        let request = axum::http::Request::post("/tok/api/providers/bedrock/keys")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({ "key": key, "mode": "replace" }).to_string(),
            ))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains(key));
        assert!(text.contains("…BED1"));

        let request = axum::http::Request::delete("/tok/api/providers/bedrock/keys")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert!(forge_config::stored_api_key_fingerprints("bedrock").is_empty());
    }
}
