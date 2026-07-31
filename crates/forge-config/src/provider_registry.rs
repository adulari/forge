//! Provider credentials, discovery, and runtime custom-provider registry.

use super::*;

/// Providers that authenticate with an API key, paired with the environment variable the
/// genai client reads for that provider. The env var names must match genai's
/// `API_KEY_DEFAULT_ENV_NAME` per adapter exactly (note OpenRouter's underscore). Local
/// providers (e.g. ollama) need no key and are intentionally absent.
// Provider prefix -> API-key env var. The prefix matches the `provider::` namespace in model
// ids (and, except `openrouter`→`open_router`, the genai adapter namespace), and the env var
// matches the name the genai adapter reads — so a key set here is picked up end-to-end. Every
// provider here has a NATIVE genai adapter. Providers genai has no adapter for live in
// [`CUSTOM_OPENAI_PROVIDERS`] (OpenAI-compatible endpoints Forge wires via a custom resolver);
// the key/discovery accessors below chain both tables so adding either kind is one row.
pub(crate) const PROVIDER_ENV_VARS: &[(&str, &str)] = &[
    ("anthropic", "ANTHROPIC_API_KEY"),
    ("openai", "OPENAI_API_KEY"),
    ("gemini", "GEMINI_API_KEY"),
    ("xai", "XAI_API_KEY"),
    ("deepseek", "DEEPSEEK_API_KEY"),
    ("openrouter", "OPEN_ROUTER_API_KEY"),
    // Free / free-tier providers.
    ("groq", "GROQ_API_KEY"),
    ("opencode_go", "OPENCODE_GO_API_KEY"), // OpenCode Zen free curated coding models
    ("github_copilot", "GITHUB_TOKEN"),     // GitHub Models free inference
    ("mimo", "MIMO_API_KEY"),               // Xiaomi MiMo
    ("minimax", "MINIMAX_API_KEY"),
    ("cohere", "COHERE_API_KEY"), // native adapter — Command A (218B), free trial tier
    // Enterprise gateways with a NATIVE genai 0.6 adapter (Bearer/key auth). They route fine but are
    // marked non-listable in `forge-provider::is_discoverable` (no enumerable models endpoint for our
    // flow) so discovery skips them quietly; users pin `bedrock::…` / `vertex::…` model ids.
    //   • Bedrock → genai's `bedrock_api` adapter (`forge-provider::normalize_namespace` maps it).
    //     AWS Bedrock Converse with a long-lived Bedrock API key (Bearer). SigV4 auth is NOT wired.
    //   • Vertex → genai's `vertex` adapter. ALSO needs `VERTEX_PROJECT_ID` (+ optional
    //     `VERTEX_LOCATION`) exported in the environment — genai reads those directly; Forge only
    //     manages the `VERTEX_API_KEY`. See docs / `forge provider list`.
    ("bedrock", "BEDROCK_API_KEY"),
    ("vertex", "VERTEX_API_KEY"),
];

/// Enterprise gateways Forge has config/CLI scaffolding for but CANNOT route to with the pinned genai
/// version — shown by `forge provider list` with the reason, never entered into routing (honest: not
/// faked). `(namespace, why)`. Empty now that Azure OpenAI is wired through genai's per-request
/// URL+header override (see [`AzureConfig`] / `[providers.azure]`); kept as the home for the next
/// gateway that genai can't yet reach.
pub const UNWIRED_ENTERPRISE_PROVIDERS: &[(&str, &str)] = &[];

/// Provider namespace for Azure OpenAI model ids (`azure::<deployment>`).
pub const AZURE_NS: &str = "azure";

/// Default env var Forge reads for the Azure OpenAI API key (overridable via `api_key_env`).
pub const AZURE_DEFAULT_KEY_ENV: &str = "AZURE_OPENAI_API_KEY";

/// Default Azure REST `api-version` when the config omits one. A recent GA version that supports
/// tool/function calling; the user can pin any version their resource exposes via `api_version`.
pub const DEFAULT_AZURE_API_VERSION: &str = "2024-10-21";

/// A resolved, validated Azure OpenAI provider (from `[providers.azure]`). The genai client builds a
/// per-request `AuthData::RequestOverride` from this to retarget the OpenAI adapter at Azure's
/// deployment-scoped URL with an `api-key` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzureProvider {
    /// Resource endpoint base, no trailing slash (e.g. `https://my-resource.openai.azure.com`).
    pub endpoint: String,
    /// Azure REST `api-version` query value.
    pub api_version: String,
    /// Env var holding the API key.
    pub env_var: String,
    /// Deployment names → `azure::<deployment>` model ids.
    pub deployments: Vec<String>,
    /// Whether these deployments are free (Azure is metered, so normally false).
    pub free: bool,
    /// Human label for `forge provider list`.
    pub label: String,
}

impl AzureProvider {
    /// The full Azure chat-completions URL for a deployment, including the `api-version` query. This
    /// is what the genai OpenAI adapter's request is redirected to via `AuthData::RequestOverride`.
    pub fn chat_completions_url(&self, deployment: &str) -> String {
        format!(
            "{}/openai/deployments/{}/chat/completions?api-version={}",
            self.endpoint.trim_end_matches('/'),
            deployment,
            self.api_version
        )
    }
}

impl AzureConfig {
    /// Resolve the endpoint base (no trailing slash) from `endpoint` (preferred) or `resource`.
    fn resolved_endpoint(&self) -> Option<String> {
        if let Some(ep) = self
            .endpoint
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            return Some(ep.trim_end_matches('/').to_string());
        }
        let res = self
            .resource
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())?;
        Some(format!("https://{res}.openai.azure.com"))
    }

    /// Validate + resolve into an [`AzureProvider`]. Requires `endpoint` or `resource`; the
    /// endpoint must be `http(s)://`. `api_version`/`api_key_env` fall back to the defaults.
    pub fn into_provider(self) -> Result<AzureProvider, String> {
        let endpoint = self
            .resolved_endpoint()
            .ok_or_else(|| "[providers.azure] needs `resource` or `endpoint`".to_string())?;
        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
            return Err(format!(
                "azure endpoint '{endpoint}' must start with http(s)://"
            ));
        }
        let api_version = self
            .api_version
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_AZURE_API_VERSION.to_string());
        let env_var = self
            .api_key_env
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| AZURE_DEFAULT_KEY_ENV.to_string());
        Ok(AzureProvider {
            endpoint,
            api_version,
            env_var,
            deployments: self
                .deployments
                .into_iter()
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
                .collect(),
            free: self.free,
            label: self.label.unwrap_or_default(),
        })
    }
}

/// The resolved Azure provider, cached process-wide (build-once, like the custom-provider registry).
/// `None` when no valid `[providers.azure]` block is configured.
static AZURE_REGISTRY: std::sync::OnceLock<Option<AzureProvider>> = std::sync::OnceLock::new();

pub(crate) fn azure_registry() -> Option<&'static AzureProvider> {
    AZURE_REGISTRY
        .get_or_init(|| {
            let cfg = load().ok().and_then(|c| c.providers.azure)?;
            match cfg.into_provider() {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!("ignoring invalid [providers.azure]: {e}");
                    None
                }
            }
        })
        .as_ref()
}

/// The configured Azure OpenAI provider, if `[providers.azure]` is present and valid. Used by the
/// provider client to build the per-request Azure override, and by discovery to seed deployments.
pub fn azure_provider() -> Option<&'static AzureProvider> {
    azure_registry()
}

/// An OpenAI-compatible API provider that genai has **no native adapter** for. Forge reaches it
/// by retargeting genai's OpenAI adapter at `endpoint` with the key from `env_var` (see the
/// service-target resolver in `forge-provider`). These providers expose a standard
/// `/chat/completions` but cannot be model-LISTED through genai, so the mesh seeds `seed_models`
/// for them at discovery instead of enumerating live.
///
/// Adding a new OpenAI-compatible provider is a single row here: that wires auth (`forge auth
/// <namespace>`), env injection, routing, mesh discovery, and the free/paid flag end-to-end.
#[derive(Debug, Clone, Copy)]
pub struct CustomProvider {
    /// The `provider::` namespace in model ids and the `forge auth` name.
    pub namespace: &'static str,
    /// Full base URL, trailing slash included (genai appends `chat/completions`).
    pub endpoint: &'static str,
    /// Environment variable holding the API key.
    pub env_var: &'static str,
    /// Whether the provider's models are genuinely free to call (standing free tier).
    pub free: bool,
    /// Human label + tier hint shown in `forge init` / `forge auth`.
    pub label: &'static str,
    /// Curated bare model ids (no namespace) seeded into the mesh when a key is present, since
    /// these providers have no live model-listing API. Users can pin any `namespace::model`.
    pub seed_models: &'static [&'static str],
}

/// OpenAI-compatible providers with no native genai adapter, reached via the custom endpoint
/// resolver in `forge-provider`. Single source of truth for their endpoint, key env var, free
/// flag, and curated seed models — every key/discovery accessor chains this with
/// [`PROVIDER_ENV_VARS`]. Add a provider by appending one row.
pub const CUSTOM_OPENAI_PROVIDERS: &[CustomProvider] = &[
    CustomProvider {
        namespace: "qwencloud",
        endpoint: "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/",
        env_var: "QWENCLOUD_API_KEY",
        free: false,
        label: "Qwen Cloud — individual Token Plan (Singapore)",
        // Token-plan availability is subscription-specific; authenticated `/models` discovery is
        // authoritative, so do not route to speculative fallback ids when discovery fails.
        seed_models: &[],
    },
    CustomProvider {
        namespace: "cerebras",
        endpoint: "https://api.cerebras.ai/v1/",
        env_var: "CEREBRAS_API_KEY",
        free: true,
        label: "Cerebras — free tier (very fast)",
        seed_models: &["llama-3.3-70b", "gpt-oss-120b", "qwen-3-coder-480b"],
    },
    CustomProvider {
        namespace: "nvidia",
        endpoint: "https://integrate.api.nvidia.com/v1/",
        env_var: "NVIDIA_API_KEY",
        free: true,
        label: "NVIDIA NIM — free developer tier (100+ models)",
        seed_models: &[
            "deepseek-ai/deepseek-r1",
            "meta/llama-3.1-405b-instruct",
            "meta/llama-3.3-70b-instruct",
            "qwen/qwen2.5-coder-32b-instruct",
            "nvidia/llama-3.1-nemotron-70b-instruct",
        ],
    },
    CustomProvider {
        namespace: "sambanova",
        endpoint: "https://api.sambanova.ai/v1/",
        env_var: "SAMBANOVA_API_KEY",
        free: true,
        label: "SambaNova — free tier (fast, frontier OSS)",
        seed_models: &[
            "DeepSeek-V3.1",
            "DeepSeek-R1",
            "Meta-Llama-3.3-70B-Instruct",
            "Llama-4-Maverick-17B-128E-Instruct",
        ],
    },
    CustomProvider {
        namespace: "mistral",
        endpoint: "https://api.mistral.ai/v1/",
        env_var: "MISTRAL_API_KEY",
        free: true,
        label: "Mistral — free Experiment tier (La Plateforme)",
        seed_models: &[
            "mistral-large-latest",
            "mistral-small-latest",
            "codestral-latest",
            "magistral-medium-latest",
        ],
    },
    // Popular OSS gateways — all OpenAI-compatible, reached via the same custom resolver. Paid
    // (metered), so `free: false`: priced-by-token, not a standing free tier.
    CustomProvider {
        namespace: "together",
        endpoint: "https://api.together.xyz/v1/",
        env_var: "TOGETHER_API_KEY",
        free: false,
        label: "Together AI — gateway (OSS frontier, metered)",
        seed_models: &[
            "deepseek-ai/DeepSeek-V3",
            "meta-llama/Llama-3.3-70B-Instruct-Turbo",
            "Qwen/Qwen2.5-Coder-32B-Instruct",
        ],
    },
    CustomProvider {
        namespace: "fireworks",
        endpoint: "https://api.fireworks.ai/inference/v1/",
        env_var: "FIREWORKS_API_KEY",
        free: false,
        label: "Fireworks AI — gateway (fast OSS, metered)",
        seed_models: &[
            "accounts/fireworks/models/deepseek-v3",
            "accounts/fireworks/models/llama-v3p3-70b-instruct",
            "accounts/fireworks/models/qwen2p5-coder-32b-instruct",
        ],
    },
    CustomProvider {
        namespace: "perplexity",
        endpoint: "https://api.perplexity.ai/",
        env_var: "PERPLEXITY_API_KEY",
        free: false,
        label: "Perplexity — Sonar (online + reasoning, metered)",
        seed_models: &[
            "sonar",
            "sonar-pro",
            "sonar-reasoning",
            "sonar-reasoning-pro",
        ],
    },
];

/// Owned form of a runtime-registered custom provider (from a `[[providers.custom]]` block), after
/// validation + endpoint normalization. Leaked into a `'static` [`CustomProvider`] by
/// [`build_custom_registry`] so it joins the built-ins transparently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCustomProvider {
    pub namespace: String,
    pub endpoint: String,
    /// Env var holding the key; `""` = keyless (a placeholder is sent by the provider client).
    pub env_var: String,
    pub free: bool,
    pub label: String,
    pub seed_models: Vec<String>,
}

/// The merged custom-provider registry: built-in [`CUSTOM_OPENAI_PROVIDERS`] + any runtime
/// `[[providers.custom]]` entries. Initialized once (lazily from config, or explicitly via
/// [`register_custom_providers`]) and then immutable for the process — matching the build-once
/// nature of the genai client and the const it replaces.
static CUSTOM_PROVIDER_REGISTRY: std::sync::OnceLock<Vec<CustomProvider>> =
    std::sync::OnceLock::new();

/// The active custom-provider registry. On first access, lazily merges the built-ins with the
/// runtime `[[providers.custom]]` entries read from the loaded config (zero call-site wiring), unless
/// [`register_custom_providers`] already seeded it. Process-lifetime `'static`.
pub(crate) fn custom_provider_registry() -> &'static [CustomProvider] {
    CUSTOM_PROVIDER_REGISTRY
        .get_or_init(|| build_custom_registry(&load_runtime_custom_providers()))
        .as_slice()
}

/// Read + validate the runtime `[[providers.custom]]` entries from the loaded config. Best-effort:
/// a malformed entry is skipped (logged), never fatal.
fn load_runtime_custom_providers() -> Vec<RuntimeCustomProvider> {
    let custom = load().map(|c| c.providers.custom).unwrap_or_default();
    custom
        .into_iter()
        .filter_map(|c| match c.clone().into_runtime() {
            Ok(rp) => Some(rp),
            Err(e) => {
                tracing::warn!(
                    "ignoring invalid [[providers.custom]] '{}': {e}",
                    c.namespace
                );
                None
            }
        })
        .collect()
}

/// Merge built-in custom providers with `runtime` ones, leaking the owned runtime strings to
/// `'static`. A runtime entry whose namespace collides with a built-in custom provider OR a native
/// adapter is dropped — built-ins always win, so a config typo can't shadow a first-class provider.
fn build_custom_registry(runtime: &[RuntimeCustomProvider]) -> Vec<CustomProvider> {
    let mut out: Vec<CustomProvider> = CUSTOM_OPENAI_PROVIDERS.to_vec();
    for rp in runtime {
        let collides = out.iter().any(|p| p.namespace == rp.namespace)
            || PROVIDER_ENV_VARS.iter().any(|(n, _)| *n == rp.namespace);
        if collides {
            tracing::warn!(
                "[[providers.custom]] '{}' collides with a built-in provider — ignored",
                rp.namespace
            );
            continue;
        }
        let seeds: Vec<&'static str> = rp.seed_models.iter().cloned().map(leak_str).collect();
        let label = if rp.label.is_empty() {
            format!("{} — custom OpenAI endpoint", rp.namespace)
        } else {
            rp.label.clone()
        };
        out.push(CustomProvider {
            namespace: leak_str(rp.namespace.clone()),
            endpoint: leak_str(rp.endpoint.clone()),
            env_var: leak_str(rp.env_var.clone()),
            free: rp.free,
            label: leak_str(label),
            seed_models: Box::leak(seeds.into_boxed_slice()),
        });
    }
    out
}

/// Leak a `String` to `&'static str`. Only ever called on the bounded, build-once provider registry
/// (a few entries for the whole process), so the leak is intentional and negligible.
fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Explicitly seed the custom-provider registry (built-ins + `runtime`). No-op if already
/// initialized (lazily or by a prior call). Primarily for tests / explicit startup wiring; normal
/// runs initialize lazily from config on first [`custom_provider`]/[`custom_providers`] access.
pub fn register_custom_providers(runtime: &[RuntimeCustomProvider]) {
    let _ = CUSTOM_PROVIDER_REGISTRY.set(build_custom_registry(runtime));
}

/// The custom OpenAI-compatible provider registered under `namespace` (built-in or runtime), if any.
pub fn custom_provider(namespace: &str) -> Option<&'static CustomProvider> {
    custom_provider_registry()
        .iter()
        .find(|p| p.namespace == namespace)
}

/// All custom OpenAI-compatible providers (built-in + runtime-registered).
pub fn custom_providers() -> impl Iterator<Item = &'static CustomProvider> {
    custom_provider_registry().iter()
}

/// Normalize an OpenAI-compatible base URL to the form the resolver + `/models` listing expect: a
/// trailing slash (so `{endpoint}models` / `{endpoint}chat/completions` join correctly). Accepts
/// `http://h/v1` and `http://h/v1/` identically.
pub fn normalize_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim();
    if trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    }
}

impl CustomProviderConfig {
    /// Validate + normalize into a [`RuntimeCustomProvider`]. Rejects an empty/odd namespace or a
    /// non-HTTP base URL; an absent `api_key_env` means keyless (`env_var = ""`).
    pub fn into_runtime(self) -> Result<RuntimeCustomProvider, String> {
        let namespace = self.namespace.trim().to_string();
        if namespace.is_empty()
            || !namespace
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!(
                "namespace '{namespace}' must be non-empty and only [A-Za-z0-9_-]"
            ));
        }
        let endpoint = normalize_endpoint(&self.base_url);
        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
            return Err(format!(
                "base_url '{}' must start with http(s)://",
                self.base_url
            ));
        }
        let env_var = self
            .api_key_env
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        Ok(RuntimeCustomProvider {
            namespace,
            endpoint,
            env_var,
            free: self.free,
            label: self.label.unwrap_or_default(),
            seed_models: self
                .models
                .into_iter()
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect(),
        })
    }
}

/// Persist a `[[providers.custom]]` block to the user `config.toml`, replacing any existing entry
/// with the same namespace and preserving every other key. Validates first (so a bad endpoint fails
/// before writing) and that the whole file still extracts to a [`Config`]. Returns the path written.
/// Active on the next session start (the registry is build-once per process).
pub fn add_custom_provider(p: &CustomProviderConfig) -> Result<PathBuf, ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir).map_err(|e| ConfigError::Write(e.to_string()))?;
    let path = dir.join("config.toml");
    add_custom_provider_at(&path, p)?;
    Ok(path)
}

/// The file half of [`add_custom_provider`] against an explicit path — split out so it's testable
/// without touching the real per-user config directory (mirrors `write_subscriptions_at`).
fn add_custom_provider_at(
    path: &std::path::Path,
    p: &CustomProviderConfig,
) -> Result<(), ConfigError> {
    // Validate the entry (and that it doesn't collide with a native/built-in provider).
    let rp = p.clone().into_runtime().map_err(ConfigError::Write)?;
    if PROVIDER_ENV_VARS.iter().any(|(n, _)| *n == rp.namespace)
        || CUSTOM_OPENAI_PROVIDERS
            .iter()
            .any(|c| c.namespace == rp.namespace)
    {
        return Err(ConfigError::Write(format!(
            "'{}' is a built-in provider — pick another namespace",
            rp.namespace
        )));
    }
    let mut root: toml::Table = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();

    let providers = root
        .entry("providers".to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if !providers.is_table() {
        *providers = toml::Value::Table(toml::Table::new());
    }
    let custom = providers
        .as_table_mut()
        .unwrap()
        .entry("custom".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()));
    if !custom.is_array() {
        *custom = toml::Value::Array(Vec::new());
    }
    let arr = custom.as_array_mut().unwrap();
    arr.retain(|v| v.get("namespace").and_then(|n| n.as_str()) != Some(rp.namespace.as_str()));

    let mut entry = toml::Table::new();
    entry.insert(
        "namespace".into(),
        toml::Value::String(rp.namespace.clone()),
    );
    entry.insert(
        "base_url".into(),
        toml::Value::String(p.base_url.trim().to_string()),
    );
    if let Some(env) = &p.api_key_env {
        if !env.trim().is_empty() {
            entry.insert(
                "api_key_env".into(),
                toml::Value::String(env.trim().to_string()),
            );
        }
    }
    if p.free {
        entry.insert("free".into(), toml::Value::Boolean(true));
    }
    if !rp.seed_models.is_empty() {
        entry.insert(
            "models".into(),
            toml::Value::Array(
                rp.seed_models
                    .iter()
                    .cloned()
                    .map(toml::Value::String)
                    .collect(),
            ),
        );
    }
    if let Some(label) = &p.label {
        if !label.trim().is_empty() {
            entry.insert(
                "label".into(),
                toml::Value::String(label.trim().to_string()),
            );
        }
    }
    arr.push(toml::Value::Table(entry));

    let body = toml::to_string_pretty(&root).map_err(|e| ConfigError::Write(e.to_string()))?;
    Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::string(&body))
        .extract::<Config>()
        .map_err(|e| ConfigError::Write(format!("invalid config after add: {e}")))?;
    std::fs::write(path, body).map_err(|e| ConfigError::Write(e.to_string()))?;
    Ok(())
}

/// Remove a user-registered `[[providers.custom]]` entry by namespace from the user `config.toml`.
/// `Ok(true)` if one was removed, `Ok(false)` if absent (idempotent). Built-ins can't be removed.
pub fn remove_custom_provider(namespace: &str) -> Result<bool, ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    let path = dir.join("config.toml");
    remove_custom_provider_at(&path, namespace)
}

fn remove_custom_provider_at(path: &std::path::Path, namespace: &str) -> Result<bool, ConfigError> {
    let Some(text) = std::fs::read_to_string(path).ok() else {
        return Ok(false);
    };
    let mut root: toml::Table = text.parse().unwrap_or_default();
    let mut removed = false;
    if let Some(arr) = root
        .get_mut("providers")
        .and_then(|p| p.as_table_mut())
        .and_then(|p| p.get_mut("custom"))
        .and_then(|c| c.as_array_mut())
    {
        let before = arr.len();
        arr.retain(|v| v.get("namespace").and_then(|n| n.as_str()) != Some(namespace));
        removed = arr.len() != before;
    }
    if removed {
        let body = toml::to_string_pretty(&root).map_err(|e| ConfigError::Write(e.to_string()))?;
        std::fs::write(path, body).map_err(|e| ConfigError::Write(e.to_string()))?;
    }
    Ok(removed)
}

/// The user-declared `[[providers.custom]]` entries from config (for `forge provider list`). Empty if
/// none / unreadable. Distinct from [`custom_providers`], which also includes the built-ins.
pub fn user_custom_providers() -> Vec<CustomProviderConfig> {
    load().map(|c| c.providers.custom).unwrap_or_default()
}

/// The raw `[providers.azure]` block from config (unresolved), for `forge provider list` to show what
/// the user set. `None` if absent. [`azure_provider`] returns the validated, resolved form.
pub fn user_azure_config() -> Option<AzureConfig> {
    load().ok().and_then(|c| c.providers.azure)
}

/// Persist a `[providers.azure]` block to the user `config.toml`, validating first (a bad
/// resource/endpoint fails before writing) and that the whole file still extracts to a [`Config`].
/// Returns the path written. Active on the next session start (the registry is build-once).
pub fn add_azure_provider(cfg: &AzureConfig) -> Result<PathBuf, ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir).map_err(|e| ConfigError::Write(e.to_string()))?;
    let path = dir.join("config.toml");
    add_azure_provider_at(&path, cfg)?;
    Ok(path)
}

/// Remove the user `[providers.azure]` block while preserving every other config key.
/// `Ok(true)` means a block was removed; absent config is an idempotent `Ok(false)`.
pub fn remove_azure_provider() -> Result<bool, ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    remove_azure_provider_at(&dir.join("config.toml"))
}

fn remove_azure_provider_at(path: &std::path::Path) -> Result<bool, ConfigError> {
    let Some(text) = std::fs::read_to_string(path).ok() else {
        return Ok(false);
    };
    let mut root: toml::Table = text.parse().unwrap_or_default();
    let removed = root
        .get_mut("providers")
        .and_then(toml::Value::as_table_mut)
        .is_some_and(|providers| providers.remove("azure").is_some());
    if removed {
        let body = toml::to_string_pretty(&root).map_err(|e| ConfigError::Write(e.to_string()))?;
        Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(&body))
            .extract::<Config>()
            .map_err(|e| ConfigError::Write(format!("invalid config after Azure removal: {e}")))?;
        std::fs::write(path, body).map_err(|e| ConfigError::Write(e.to_string()))?;
    }
    Ok(removed)
}

/// The file half of [`add_azure_provider`] against an explicit path — testable without the real
/// per-user config dir (mirrors [`add_custom_provider_at`]).
fn add_azure_provider_at(path: &std::path::Path, cfg: &AzureConfig) -> Result<(), ConfigError> {
    // Validate (resource/endpoint present + well-formed) before touching the file.
    cfg.clone().into_provider().map_err(ConfigError::Write)?;

    let mut root: toml::Table = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();

    let providers = root
        .entry("providers".to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if !providers.is_table() {
        *providers = toml::Value::Table(toml::Table::new());
    }
    let azure = providers
        .as_table_mut()
        .unwrap()
        .entry("azure".to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let mut entry = toml::Table::new();
    let mut put = |k: &str, v: Option<&String>| {
        if let Some(s) = v.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            entry.insert(k.into(), toml::Value::String(s.to_string()));
        }
    };
    put("resource", cfg.resource.as_ref());
    put("endpoint", cfg.endpoint.as_ref());
    put("api_version", cfg.api_version.as_ref());
    put("api_key_env", cfg.api_key_env.as_ref());
    put("label", cfg.label.as_ref());
    if cfg.free {
        entry.insert("free".into(), toml::Value::Boolean(true));
    }
    let deps: Vec<toml::Value> = cfg
        .deployments
        .iter()
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .map(toml::Value::String)
        .collect();
    if !deps.is_empty() {
        entry.insert("deployments".into(), toml::Value::Array(deps));
    }
    *azure = toml::Value::Table(entry);

    let body = toml::to_string_pretty(&root).map_err(|e| ConfigError::Write(e.to_string()))?;
    Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::string(&body))
        .extract::<Config>()
        .map_err(|e| ConfigError::Write(format!("invalid config after azure add: {e}")))?;
    std::fs::write(path, body).map_err(|e| ConfigError::Write(e.to_string()))?;
    Ok(())
}

// Search-API providers for the `web_search` tool. Kept separate from PROVIDER_ENV_VARS so
// they never enter model discovery / the mesh — they authenticate a tool, not a model.
pub(crate) const SEARCH_ENV_VARS: &[(&str, &str)] = &[("brave", "BRAVE_API_KEY")];

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn add_custom_provider_at(
        path: &std::path::Path,
        provider: &CustomProviderConfig,
    ) -> Result<(), ConfigError> {
        super::add_custom_provider_at(path, provider)
    }

    pub(crate) fn remove_custom_provider_at(
        path: &std::path::Path,
        namespace: &str,
    ) -> Result<bool, ConfigError> {
        super::remove_custom_provider_at(path, namespace)
    }

    pub(crate) fn add_azure_provider_at(
        path: &std::path::Path,
        config: &AzureConfig,
    ) -> Result<(), ConfigError> {
        super::add_azure_provider_at(path, config)
    }

    pub(crate) fn remove_azure_provider_at(path: &std::path::Path) -> Result<bool, ConfigError> {
        super::remove_azure_provider_at(path)
    }

    pub(crate) fn build_custom_registry(runtime: &[RuntimeCustomProvider]) -> Vec<CustomProvider> {
        super::build_custom_registry(runtime)
    }
}
