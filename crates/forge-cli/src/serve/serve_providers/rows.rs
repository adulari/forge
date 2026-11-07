//! One [`ProviderRow`] per provider kind: API-key, Azure OpenAI and OAuth-account providers.

use super::*;

pub(super) fn api_key_row(
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
        key_verdict: None,
    }
}

pub(super) fn azure_row(config: forge_config::AzureConfig, disabled: &[String]) -> ProviderRow {
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
        key_verdict: None,
    }
}

pub(super) fn oauth_row(
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
        key_verdict: None,
    }
}
