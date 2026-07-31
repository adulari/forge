use super::*;

pub(super) fn validate_provider_id(provider: &str) -> Result<&str, String> {
    let provider = provider.trim();
    if provider.is_empty()
        || provider.chars().count() > MAX_PROVIDER_ID_CHARS
        || !provider
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(
            "provider id must use at most 64 ASCII letters, numbers, hyphens, or underscores"
                .to_string(),
        );
    }
    Ok(provider)
}

pub(super) fn validate_account_id(account_id: &str) -> Result<&str, String> {
    let account_id = account_id.trim();
    if account_id.is_empty()
        || account_id.chars().count() > MAX_ACCOUNT_ID_CHARS
        || account_id.chars().any(char::is_control)
    {
        return Err("account id is empty, too long, or contains control characters".to_string());
    }
    Ok(account_id)
}

pub(super) fn validate_api_key(key: &str) -> Result<&str, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("API key cannot be empty".to_string());
    }
    if key.chars().count() > MAX_API_KEY_CHARS {
        return Err("API key is too large".to_string());
    }
    if key.chars().any(char::is_control) {
        return Err("API key cannot contain newlines or control characters".to_string());
    }
    Ok(key)
}

fn validate_optional_text(
    value: Option<String>,
    field: &str,
    max_chars: usize,
) -> Result<Option<String>, String> {
    value
        .map(|value| {
            let value = value.trim().to_string();
            if value.chars().count() > max_chars || value.chars().any(char::is_control) {
                return Err(format!(
                    "{field} is too long or contains control characters"
                ));
            }
            Ok((!value.is_empty()).then_some(value))
        })
        .transpose()
        .map(Option::flatten)
}

fn validate_env_var(value: Option<String>) -> Result<Option<String>, String> {
    let value = validate_optional_text(value, "environment variable", 128)?;
    if value.as_deref().is_some_and(|value| {
        let mut chars = value.chars();
        !chars
            .next()
            .is_some_and(|character| character.is_ascii_uppercase() || character == '_')
            || !chars.all(|character| {
                character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
            })
    }) {
        return Err(
            "environment variable must use uppercase ASCII letters, digits, and underscores"
                .to_string(),
        );
    }
    Ok(value)
}

fn validate_models(models: Vec<String>, field: &str) -> Result<Vec<String>, String> {
    if models.len() > MAX_MODELS {
        return Err(format!("{field} accepts at most {MAX_MODELS} entries"));
    }
    models
        .into_iter()
        .map(|model| {
            let model = model.trim().to_string();
            if model.is_empty()
                || model.chars().count() > MAX_MODEL_CHARS
                || model.chars().any(char::is_control)
            {
                return Err(format!(
                    "{field} entries must be non-empty, at most {MAX_MODEL_CHARS} characters, and \
                     contain no control characters"
                ));
            }
            Ok(model)
        })
        .collect()
}

pub(super) fn validated_custom_provider(
    request: CustomProviderRequest,
) -> Result<forge_config::CustomProviderConfig, String> {
    let namespace = validate_provider_id(&request.namespace)?.to_string();
    if request.base_url.chars().count() > MAX_ENDPOINT_CHARS
        || request.base_url.chars().any(char::is_control)
    {
        return Err("base URL is too long or contains control characters".to_string());
    }
    let config = forge_config::CustomProviderConfig {
        namespace,
        base_url: request.base_url.trim().to_string(),
        api_key_env: validate_env_var(request.api_key_env)?,
        free: request.free,
        models: validate_models(request.models, "models")?,
        label: validate_optional_text(request.label, "label", MAX_LABEL_CHARS)?,
    };
    config.clone().into_runtime()?;
    Ok(config)
}

pub(super) fn validated_azure_provider(
    request: AzureProviderRequest,
) -> Result<forge_config::AzureConfig, String> {
    let deployments = validate_models(request.deployments, "deployments")?;
    let config = forge_config::AzureConfig {
        resource: validate_optional_text(request.resource, "resource", 256)?,
        endpoint: validate_optional_text(request.endpoint, "endpoint", MAX_ENDPOINT_CHARS)?,
        api_version: validate_optional_text(request.api_version, "API version", 128)?,
        api_key_env: validate_env_var(request.api_key_env)?,
        deployments,
        free: request.free,
        label: validate_optional_text(request.label, "label", MAX_LABEL_CHARS)?,
    };
    config.clone().into_provider()?;
    Ok(config)
}

pub(super) fn oauth_provider(provider: &str) -> Result<(&'static str, &'static str), String> {
    match provider {
        "codex-oauth" => Ok((
            "codex-oauth",
            forge_config::provider_oauth::CODEX_OAUTH_KEYRING_PROVIDER,
        )),
        "xai-oauth" => Ok((
            "xai-oauth",
            forge_config::provider_oauth::XAI_OAUTH_KEYRING_PROVIDER,
        )),
        _ => Err("OAuth provider must be 'codex-oauth' or 'xai-oauth'".to_string()),
    }
}

pub(super) fn known_keyed_provider(provider: &str) -> bool {
    forge_config::user_custom_providers()
        .iter()
        .find(|custom| custom.namespace == provider)
        .is_some_and(|custom| {
            custom
                .api_key_env
                .as_deref()
                .is_some_and(|env| !env.trim().is_empty())
        })
        || (provider == forge_config::AZURE_NS && forge_config::user_azure_config().is_some())
        || (forge_config::provider_key_env_var(provider).is_some()
            && forge_config::known_key_providers().any(|known| known == provider))
}

pub(super) fn known_provider(provider: &str) -> bool {
    known_keyed_provider(provider)
        || matches!(
            provider,
            "codex-oauth" | "xai-oauth" | "claude-cli" | "codex-cli" | "agy-cli" | "ollama"
        )
        || forge_config::user_custom_providers()
            .iter()
            .any(|custom| custom.namespace == provider)
}

pub(super) fn provider_enabled(provider: &str, disabled: &[String]) -> bool {
    !disabled.iter().any(|entry| entry == provider)
}

pub(super) fn env_family_present(env_var: &str) -> bool {
    std::env::var(env_var).is_ok_and(|value| !value.trim().is_empty())
        || (2..=16).any(|index| {
            std::env::var(format!("{env_var}_{index}")).is_ok_and(|value| !value.trim().is_empty())
        })
}

pub(super) fn user_disabled_providers() -> Result<Vec<String>, String> {
    let path = forge_config::scope_path(forge_config::ConfigScope::User)
        .map_err(|error| error.to_string())?;
    let body = match std::fs::read_to_string(&path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    parse_disabled_providers(&body)
        .map_err(|error| format!("refusing to overwrite invalid {}: {error}", path.display()))
}

pub(super) fn parse_disabled_providers(body: &str) -> Result<Vec<String>, String> {
    let root: toml::Table = body.parse().map_err(|error| format!("{error}"))?;
    let Some(value) = root
        .get("mesh")
        .and_then(toml::Value::as_table)
        .and_then(|mesh| mesh.get("disabled"))
    else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| "mesh.disabled must be an array".to_string())?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "mesh.disabled entries must be strings".to_string())
        })
        .collect()
}

pub(super) fn custom_provider_pending_restart(config: &forge_config::CustomProviderConfig) -> bool {
    let Ok(runtime) = config.clone().into_runtime() else {
        return true;
    };
    let Some(active) = forge_config::custom_provider(&runtime.namespace) else {
        return true;
    };
    let expected_label = if runtime.label.is_empty() {
        format!("{} — custom OpenAI endpoint", runtime.namespace)
    } else {
        runtime.label.clone()
    };
    active.endpoint != runtime.endpoint
        || active.env_var != runtime.env_var
        || active.free != runtime.free
        || active.label != expected_label
        || active.seed_models != runtime.seed_models
}

pub(super) fn azure_provider_pending_restart(config: &forge_config::AzureConfig) -> bool {
    let Ok(configured) = config.clone().into_provider() else {
        return true;
    };
    forge_config::azure_provider().is_none_or(|active| active != &configured)
}

pub(super) fn after_credential_change(
    store: &forge_store::Store,
    provider: &str,
    clear_health: bool,
) {
    crate::cli::commands::models::invalidate_catalog_cache();
    forge_provider::invalidate_plan_cache();
    if clear_health {
        crate::cli::commands::local::clear_auth_exclusion_in(store, provider);
    }
}

pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
