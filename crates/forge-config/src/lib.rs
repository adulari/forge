//! Layered configuration (defaults -> user file -> project file -> `FORGE_*` env) and
//! secret resolution. Secrets are never part of the config surface (ADR-0007): API keys
//! come from environment variables first, then the OS keyring (`forge auth`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use forge_types::{
    CreditMode, PermissionDecision, PermissionMode, PermissionRule, RuleSource, TaskTier,
};
use serde::{Deserialize, Serialize};

mod adaptive_effort;
pub mod advisor;
mod statusline_toggle;
pub use statusline_toggle::WidgetToggle;
mod agent_builtins;
pub mod agents;
pub mod browser;
pub mod cc_rules;
pub mod chat;
pub mod codemode;
mod commit_policy;
mod compact_model;
mod config_diagnostics;
mod harness;
pub mod setting_sources;
pub mod skill_usage;
mod system_prompt_overrides;
use system_prompt_overrides::resolve_system_prompt_overrides;
mod edit_mode;
pub mod goal;
mod hard_stop;
mod harness_profile;
mod hooks;
mod install_notices;
mod key_adoption;
mod keybind_migration;
mod keybindings_file;
mod keybinds;
pub mod model_family;
mod tui_config;
pub use tui_config::TuiConfig;
mod progress_guard;
pub use key_adoption::{adopt_parent_provider_keys, injected_provider_key_vars};
pub mod account_sync;
pub mod commit_language;
pub mod host;
mod lsp_diagnostics;
pub mod mcp;
pub use mcp::mcp_toml_problem;
pub mod mcp_sync;
pub mod notifications;
pub mod oauth;
mod paths;
pub mod power;
pub mod pr;
pub mod preview;
pub mod privacy;
pub mod project_actions;
mod project_dirs;
pub mod project_trust;
pub mod provider_oauth;
pub mod quota_status;
pub mod reporting;
pub mod secret_health;
pub mod secret_store;
pub mod sessions;
pub mod terminal;
mod vision_fallback;
mod workspace_state;
pub mod worktree;
pub use adaptive_effort::CACHE_SAFE_EFFORT_PROVIDERS;
pub use agents::{agent_catalog, load_agents, load_agents_layered, map_tool_name, AgentDef};
pub use commit_policy::CommitPolicy;
pub use compact_model::CompactModel;
pub use edit_mode::EditMode;
pub use hard_stop::HardStop;
pub use harness::{AutoRefineMode, HarnessConfig};
pub use harness_profile::{resolve_harness_profile, HarnessProfile};
pub use hooks::{cc_hooks_from_settings, is_regex_matcher, HookConfig, HookEvent, HookKind};
pub use install_notices::{install_notice_seen, mark_install_notice_seen, CLI_BRIDGE_TERMS};
pub use keybindings_file::{
    keybindings_path, overlay_tui, read_keybindings_file, write_app_overrides,
    write_keybindings_file, KeybindingsFile,
};
pub use keybinds::{
    default_leader, remove_keybind, write_keybind, write_keybind_preset, KeyCombo, KeySequence,
    KeybindPreset, KeybindsConfig, UNBOUND_KEY,
};
pub use lsp_diagnostics::LspDiagnosticsMode;
pub use mcp::{
    discover_import_sources, embeds_credentials, import_mcp_json, load_mcp_toml,
    load_mcp_toml_with_diagnostic, looks_secret, write_mcp_toml, ImportSource, McpAllowlist,
    McpAuth, McpConfig, McpServerConfig, McpTransport, ParsedServers,
};
pub use oauth::{
    add_oauth_account, authorize_url, clear_oauth_tokens, list_oauth_accounts, load_oauth_tokens,
    next_oauth_account_id, oauth_keyring_key, random_state, remove_oauth_account,
    store_oauth_tokens, switch_oauth_account, AuthServerMetadata, OAuthAccountPool,
    OAuthAccountStore, OAuthConfig, OAuthTokens, Pkce, ProtectedResourceMetadata,
};
pub use paths::{
    claude_dir, codex_dir, config_dir, cursor_dir, data_dir, home_dir, is_global_worktree_path,
    mark_project_auto_setup_attempted, project_auto_setup_attempted, project_initialization,
    ProjectInitialization,
};
pub use progress_guard::ProgressGuardMode;
pub use project_dirs::{project_resource_dirs, repo_top};
pub use vision_fallback::VisionFallback;
pub use workspace_state::{
    adopt_legacy_state, legacy_state_subdir, prepare_workspace_state, state_subdir,
    workspace_state_dir, workspace_state_root, STATE_DIR_ENV,
};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to load configuration: {0}")]
    Load(Box<figment::Error>),
    #[error("no API key found for provider '{0}' (set {1} or run `forge auth {0}`)")]
    MissingKey(String, String),
    #[error("secret store error: {0}")]
    Keyring(String),
    #[error("no per-user config directory available on this platform")]
    NoConfigDir,
    #[error("writing config failed: {0}")]
    Write(String),
    #[error("invalid configuration: {0}")]
    Invalid(String),
    #[error("failed to read system-prompt override file '{0}': {1}")]
    PromptFile(String, String),
}

impl From<figment::Error> for ConfigError {
    fn from(e: figment::Error) -> Self {
        ConfigError::Load(Box::new(e))
    }
}

/// The fully resolved Forge configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Default permission posture for new sessions (ADR-0008).
    pub permission_mode: PermissionMode,
    /// Model Mesh settings (ADR-0006).
    pub mesh: MeshConfig,
    /// Fine-grained allow/ask/deny rules layered on top of the mode (FR-10).
    #[serde(default)]
    pub permissions: PermissionsConfig,
    /// External MCP servers Forge connects to as a client (mcp-client.md). Empty = inert.
    #[serde(default)]
    pub mcp: McpConfig,
    /// Slash-command + skill discovery and trust (command-skill-system.md).
    #[serde(default)]
    pub commands: CommandsConfig,
    /// Native code-intelligence graph (code-intelligence.md).
    #[serde(default)]
    pub lattice: LatticeConfig,
    /// Shell tool behaviour, incl. the AI error interceptor (shell-error-interceptor.md).
    #[serde(default)]
    pub shell: ShellConfig,
    /// Pre/post tool-use shell hooks (hooks.md). Each `[[hooks]]` entry runs a command around a
    /// matching tool call.
    #[serde(default)]
    pub hooks: Vec<HookConfig>,
    /// Max consecutive times `Stop` hooks may block a turn from ending before Forge forces it to
    /// end (Claude Code caps this at 8). The count resets whenever a continuation runs a tool.
    #[serde(default = "default_stop_hook_max_blocks")]
    pub stop_hook_max_blocks: u32,
    /// Git integration settings (co-authoring, hook installation).
    #[serde(default)]
    pub git: GitConfig,
    /// LSP-backed live diagnostics fed back into the turn after edits (lsp.md). Off = inert.
    #[serde(default)]
    pub lsp: LspConfig,
    /// Auto-lint / auto-test self-healing loop after edits (autofix.md). Off = inert.
    #[serde(default)]
    pub autofix: AutofixConfig,
    /// Assay-gated auto-review of write turns before they finish (assay-gate.md). Off = inert.
    #[serde(default)]
    pub assay: AssayConfig,
    /// Per-turn recap: a one-line AI-generated summary after each completed turn. On by default;
    /// disable in /config if you find it noisy.
    #[serde(default)]
    pub recap: RecapConfig,
    /// Next-prompt suggestion: a cheap trivial-tier call after each completed turn that predicts
    /// the user's likely next prompt, shown as dim ghost text in an empty input (Tab accepts it).
    /// On by default; disable in /config if you find it noisy.
    #[serde(default)]
    pub suggest: SuggestConfig,
    /// Project setup behavior for repositories that have no Forge guidance or customizations.
    #[serde(default)]
    pub project: ProjectConfig,
    /// Interactive TUI rendering (chat). Controls inline vs. full-screen (alternate-screen) mode.
    #[serde(default)]
    pub tui: TuiConfig,
    /// Local-LLM runtime (Ollama): which model to auto-start and whether to start it with Forge.
    #[serde(default)]
    pub local: LocalConfig,
    /// Startup update check (GitHub releases). On by default; checked at most every 6 hours.
    #[serde(default)]
    pub update: UpdateConfig,
    /// Worktree location, setup/cleanup commands and ignored-file copying (user config only).
    #[serde(default)]
    pub worktree: worktree::WorktreeConfig,
    /// Anonymous product-count telemetry. Enabled by default; no stable identifier or user
    /// content is ever sent. Disable here, with `FORGE_TELEMETRY=0`, or with `DO_NOT_TRACK=1`.
    #[serde(default)]
    pub telemetry: TelemetryConfig,
    /// Automatic issue reports for Forge's own bugs: scrubbed, content-free, on by default.
    /// `FORGE_NO_REPORTS=1` forces it off. See `docs/features/issue-reporting.md`.
    #[serde(default)]
    pub reporting: reporting::ReportingConfig,
    /// When true, Forge starts a sub-Forge session as an MCP server so the agent can use
    /// forge_chat / forge_assay as native tools (self-driving mode). Toggle with
    /// `forge self enable` / `forge self disable`. Off by default.
    #[serde(default)]
    pub self_mcp: bool,
    /// Customizable statusline layout (left / center / right widget segments).
    #[serde(default)]
    pub statusline: StatuslineConfig,
    /// Desktop notification + terminal bell when a turn finishes or Forge waits on the user.
    #[serde(default)]
    pub notifications: notifications::NotificationsConfig,
    /// Attach the browser tool to an existing logged-in browser (`[browser] attach`).
    #[serde(default)]
    pub browser: browser::BrowserConfig,
    /// What Enter does with a prompt typed mid-turn (`[chat] follow_up = "steer" | "queue"`).
    #[serde(default)]
    pub chat: chat::ChatConfig,
    /// Public preview URLs for agent-started dev servers (`shell_job` expose).
    #[serde(default)]
    pub preview: preview::PreviewConfig,
    /// Whether the host keeps itself awake while Forge works (`[power] keep_awake`).
    #[serde(default)]
    pub power: power::PowerConfig,
    /// Ceilings on what one daemon runs at once (`[host] max_active_sessions` and friends).
    #[serde(default)]
    pub host: host::HostConfig,
    /// Which shell terminal tabs and the agent's shell tool use (`[terminal]`).
    #[serde(default)]
    pub terminal: terminal::TerminalConfig,
    /// One-click dev scripts (`[[actions]]`), from the project's `.forge/config.toml`.
    #[serde(default)]
    pub actions: Vec<project_actions::ProjectActionConfig>,
    /// The `run_code` tool on the direct path (`[codemode]`).
    #[serde(default)]
    pub codemode: codemode::CodemodeConfig,
    /// How `open_pr` publishes pull requests (`[pr] publish_evidence_images`).
    #[serde(default)]
    pub pr: pr::PrConfig,
    /// Idle unload and auto-archive of sessions (`[sessions]`).
    #[serde(default)]
    pub sessions: sessions::SessionsConfig,
    /// User-configurable keybind map (action → key combo). Defaults to the built-in map.
    #[serde(default)]
    pub keybinds: KeybindsConfig,
    /// Runtime-registered providers: custom OpenAI-compatible endpoints (LM Studio, vLLM, llama.cpp
    /// `--server`, text-generation-webui, local/proxy servers) the user adds without recompiling.
    /// Merged with the built-in [`CUSTOM_OPENAI_PROVIDERS`] at startup so they participate in
    /// discovery + routing identically. Empty = inert.
    #[serde(default)]
    pub providers: ProvidersConfig,
    /// Per-provider system-prompt overrides, keyed by provider namespace (e.g. `"kimi"`). When a
    /// namespace has an entry, its models get that text as the base coding-agent prompt in the main
    /// loop, replacing the built-in [`FORGE_SYSTEM`] prompt (the live env block and any minimal-diff
    /// bias are kept). For experimentation; empty = inert.
    #[serde(default)]
    pub system_prompt_overrides: HashMap<String, String>,
    /// Per-provider reasoning prefill, keyed by provider namespace (e.g. `"kimi"`). An entry seeds
    /// the START of the model's thinking with its text, via a trailing assistant message flagged
    /// `partial: true` (Moonshot/Kimi "Partial Mode"). Absent or empty = off, which is the default:
    /// the flag is only understood by providers that implement Partial Mode, and a seeded thought
    /// changes how the model reasons, so it is never applied unless asked for.
    #[serde(default)]
    pub reasoning_prefill: HashMap<String, String>,
    /// Exact reasoning rung per model, keyed by full id (`"kimi::k3"`) or provider namespace
    /// (`"kimi"`), the id winning where both match. Unlike `mesh.default_effort` — which is a
    /// CEILING the mesh then optimises under, using measured benchmark data — an entry here is an
    /// INSTRUCTION: that rung is sent verbatim, provided the provider actually offers it
    /// (`effort::ladder`). It exists because the benchmark ladder is not the provider's ladder: a
    /// rung a provider supports but nobody measured (Kimi's `high`) is otherwise unreachable.
    /// Empty = inert, and the mesh's own best-value choice stands.
    #[serde(default)]
    pub model_effort: HashMap<String, String>,
    /// Auto-compaction ceiling in tokens per model or provider, keyed like `model_effort` (full
    /// id wins over a bare namespace). An entry replaces `mesh.compact_cap_tokens` (or the free
    /// cap) for that model; the 80%-of-window trigger still applies on top. For a subscription
    /// metered per request, a smaller ceiling trades continuity for a cheaper prompt on every step
    /// of a long tool loop. Ships with `kimi = 120000`: Kimi Code's K3 window is 262K, its replies
    /// carry thinking that goes back on every step, and a measured 198-step session averaged a
    /// 130K prompt under the global 217,600 ceiling. Set a provider to `0` to fall back to the
    /// global ceiling.
    #[serde(default = "default_compact_cap")]
    pub compact_cap: HashMap<String, u64>,
    /// Remote control server (`/remote`): drive this session from a phone or browser. `auto`
    /// starts it at chat launch so the session is reachable without typing `/remote` first.
    #[serde(default)]
    pub remote: RemoteConfig,
    /// Optional managed Forge Anywhere companion. Credentials and device keys are deliberately
    /// stored outside config.toml in the owner-only platform data directory.
    #[serde(default)]
    pub anywhere: AnywhereConfig,
    /// Local whisper.cpp speech-to-text (voice.md, V1): which model size to run, transcription
    /// language, and auto-stop-on-silence. Models download on first use into
    /// `{data_dir}/models/whisper/`.
    #[serde(default)]
    pub voice: VoiceConfig,
    /// Continual Harness (`/refine`, port of prime-agent's `/refine`): the agent proposes durable
    /// edits to its own prompts/skills/subagents, persisted via forge-store's harness tables.
    #[serde(default)]
    pub harness: HarnessConfig,
    /// Structured file-tool workspace confinement (`tools.extra_roots`).
    #[serde(default)]
    pub tools: ToolsConfig,
    /// Zero-data-retention routing (`[privacy] zero_retention`, zero-retention.md).
    #[serde(default)]
    pub privacy: privacy::PrivacyConfig,
    /// The `/goal` progress evaluator (`[goal] evaluator`, `[goal] evaluator_model`).
    #[serde(default)]
    pub goal: goal::GoalConfig,
}

/// `[remote]` config block — the phone/browser remote-control server (`remote-control.md`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    /// Auto-start exposure when `forge chat` launches. `off` (default) leaves remote control off
    /// until you type `/remote`; `local` binds loopback (this machine only); `lan` binds the LAN
    /// with self-signed HTTPS; `anywhere` opens a public tunnel (cloudflared/ngrok).
    #[serde(default)]
    pub auto: RemoteAuto,
    /// Override the host used in the LAN connect URL / QR code / TLS cert SANs. By default the
    /// outbound-interface IP is auto-discovered; set this on multi-homed or VPN'd machines where
    /// the discovered interface isn't the one the phone reaches (e.g. `host = "192.168.1.5"`).
    #[serde(default)]
    pub host: Option<String>,
    /// The stable port `forge serve` (the multi-session daemon) listens on. Stable so the
    /// installed PWA's origin never changes across daemon restarts. Defaults to 7420; the
    /// in-chat `/remote` server is unaffected (it keeps its ephemeral per-session port).
    #[serde(default)]
    pub port: Option<u16>,
    /// A named cloudflared tunnel to run for `--anywhere` instead of an ephemeral quick tunnel,
    /// so the public URL is stable across launches. The tunnel must already exist
    /// (`cloudflared tunnel create <name>`) and have a DNS route to `tunnel_hostname`
    /// (`cloudflared tunnel route dns <name> <hostname>`) — both one-time setup steps. Ignored
    /// by ngrok (it has no named-tunnel concept). Absent = quick-tunnel behavior, unchanged.
    #[serde(default)]
    pub tunnel_name: Option<String>,
    /// The stable public hostname to advertise for `--anywhere`. Paired with `tunnel_name` for
    /// cloudflared (the DNS-routed hostname of the named tunnel — cloudflared's named-tunnel run
    /// prints no URL, so this is how Forge knows what to show/print/QR-code). Set alone (no
    /// `tunnel_name`) it instead selects ngrok's reserved-domain flow
    /// (`ngrok http --domain=<hostname> <port>`, needs a domain reserved on the ngrok account).
    #[serde(default)]
    pub tunnel_hostname: Option<String>,
    /// An externally managed HTTPS origin that reaches this daemon (a reverse proxy, a Tailscale
    /// Funnel, a fixed domain). When set, pairing links and QR codes point here instead of the
    /// LAN address, and the daemon expects the proxy to terminate a trusted certificate.
    #[serde(default)]
    pub public_url: Option<String>,
    /// Directories the remote project browser may enumerate. Paths are canonicalized at daemon
    /// startup; missing/unreadable entries are ignored. The daemon working directory is always
    /// included even when this list is empty. Use this to expose a parent such as `~/Projects`
    /// without exposing the rest of the host filesystem.
    #[serde(default)]
    pub project_roots: Vec<String>,
    /// Run a standalone `forge run` IN the local daemon, so the run shows up in the Anywhere
    /// fleet (phone and desktop apps) and can be watched and steered from either.
    ///
    /// ON by default, like [`Self::publish_local_sessions`]: a one-shot run that nobody can see
    /// from the phone is the odd case, not the norm. This moves where the turn EXECUTES rather
    /// than only advertising it, so it fail-softs — if the daemon is down, unreachable, or
    /// rejects the request, the run proceeds locally exactly as if this were off. Set to `false`
    /// (or pass `--no-publish-to-fleet`) to keep one-shot runs purely local.
    #[serde(default = "default_true")]
    pub publish_local_runs: bool,
    /// Advertise terminal-local `forge` chat sessions in the fleet, so the phone and desktop apps
    /// list what this machine is working on alongside `forge serve`-hosted sessions.
    ///
    /// ON by default. This publishes presence only — id, title, cwd, busy — for sessions already
    /// in the shared store. Whether those rows can also be *driven* from the apps is
    /// [`Self::interactive_local_sessions`]; with that off they are listed read-only. Set this to
    /// `false` to keep local chat out of the fleet entirely.
    #[serde(default = "default_true")]
    pub publish_local_sessions: bool,
    /// Let the phone and desktop apps watch and DRIVE a terminal `forge` chat session, not just
    /// list it. Each terminal session opens a token-gated control channel on loopback and records
    /// it in the shared store; `forge serve` proxies the app's WebSocket to it, so a remote prompt,
    /// permission answer, or interrupt arrives exactly as if typed into that terminal.
    ///
    /// ON by default: a session you can see but not answer is the frustrating case, not the safe
    /// one — a permission prompt raised on the laptop should be answerable from the phone. Nothing
    /// is exposed beyond loopback by this setting; the daemon is the only thing that can reach the
    /// channel, and it already requires its own authentication. Set to `false` to publish terminal
    /// sessions read-only, as before.
    #[serde(default = "default_true")]
    pub interactive_local_sessions: bool,
    /// Extra `Host` header values the daemon accepts on its token routes (exact names, or
    /// `*.example.com`). IP literals, `localhost`, single-label names, `*.local`, `*.ts.net`, the
    /// well-known tunnel domains and `tunnel_hostname` are always accepted; any other name is
    /// refused so a DNS-rebinding page cannot reach the daemon through a hostname it controls.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Reverse-proxy addresses whose `X-Forwarded-For` the daemon believes when it records a
    /// client address in the remote audit log. Loopback is always trusted (tunnel agents).
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
}

// Hand-written, NOT derived. `Config` carries `#[serde(default)] pub remote: RemoteConfig`, so a
// config file with no `[remote]` block at all builds this struct through `Default` and never
// touches the per-field `#[serde(default = "default_true")]` attributes — those only apply when
// the block EXISTS and the key is missing. Deriving `Default` therefore silently gave
// `publish_local_runs`/`publish_local_sessions` the `bool` default of `false` for every user
// without a `[remote]` block, which is most of them: terminal sessions stopped appearing in the
// fleet entirely. Any future `default_true` field here must be added in both places.
impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            auto: RemoteAuto::default(),
            host: None,
            port: None,
            tunnel_name: None,
            tunnel_hostname: None,
            public_url: None,
            project_roots: Vec::new(),
            publish_local_runs: default_true(),
            publish_local_sessions: default_true(),
            interactive_local_sessions: default_true(),
            allowed_hosts: Vec::new(),
            trusted_proxies: Vec::new(),
        }
    }
}

/// `[anywhere]` config block for the optional managed companion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnywhereConfig {
    /// Connect this host to the managed relay when `forge serve` is running.
    #[serde(default)]
    pub enabled: bool,
    /// Stable user-facing name for this host. The service still assigns an opaque host ID.
    #[serde(default)]
    pub host_name: Option<String>,
    /// Upload eligible encrypted journal records (sessions, memory). Credentials are never part of
    /// this stream; they travel only through `account_sync.keys`, which is off until this device
    /// opts in.
    #[serde(default = "default_true")]
    pub sync: bool,
    /// Alternate service origin for local development. Release builds ignore this field.
    #[serde(default)]
    pub service_url: Option<String>,
    /// Which parts of this machine's setup follow the account to the user's other devices. Every
    /// category is per device and off by default.
    #[serde(default)]
    pub account_sync: AccountSyncConfig,
}

/// `[anywhere.account_sync]`: per-device opt-ins for syncing setup across the account's machines.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountSyncConfig {
    /// Provider API keys and static MCP secrets. Rotating OAuth logins never sync, only a marker.
    #[serde(default)]
    pub keys: bool,
    /// User skills, commands, agents and installed skill packs.
    #[serde(default)]
    pub skills: bool,
    /// Portable preferences (theme, statusline, notifications, permission rules) and keybindings.
    #[serde(default)]
    pub settings: bool,
    /// MCP server definitions from `mcp.toml`.
    #[serde(default)]
    pub mcp: bool,
    /// The user-level `AGENTS.md`.
    #[serde(default)]
    pub instructions: bool,
}

impl AccountSyncConfig {
    pub const CATEGORIES: [&'static str; 5] = ["keys", "skills", "settings", "mcp", "instructions"];

    pub fn enabled(&self, category: &str) -> bool {
        match category {
            "keys" => self.keys,
            "skills" => self.skills,
            "settings" => self.settings,
            "mcp" => self.mcp,
            "instructions" => self.instructions,
            _ => false,
        }
    }

    pub fn any_enabled(&self) -> bool {
        Self::CATEGORIES.iter().any(|c| self.enabled(c))
    }
}

impl Default for AnywhereConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host_name: None,
            sync: true,
            service_url: None,
            account_sync: AccountSyncConfig::default(),
        }
    }
}

impl AnywhereConfig {
    /// Resolve the service origin. Custom origins are honored only in debug builds.
    pub fn service_url(&self) -> &str {
        #[cfg(debug_assertions)]
        if let Some(url) = self.service_url.as_deref() {
            return url.trim_end_matches('/');
        }
        "https://app.forge.adulari.dev"
    }
}

/// Keys `AnywhereConfig` actually recognizes. A user typo here (`nabled` for `enabled`) is
/// silently dropped by Figment/serde — the field just keeps its default, with no error and no
/// warning — so `[anywhere] nabled = true` left the connector disabled with `forge anywhere
/// doctor` reporting only "connector: not configured", no pointer at the real cause (#5).
const ANYWHERE_KNOWN_KEYS: &[&str] = &[
    "enabled",
    "host_name",
    "sync",
    "service_url",
    "account_sync",
];

/// Unrecognized keys in `raw_toml`'s `[anywhere]` table, each paired with the closest known key
/// when the two are close enough (edit distance <= 2) to be a plausible typo.
fn anywhere_unknown_keys(raw_toml: &str) -> Vec<(String, Option<&'static str>)> {
    let Ok(value) = toml::from_str::<toml::Value>(raw_toml) else {
        return Vec::new();
    };
    let Some(table) = value.get("anywhere").and_then(toml::Value::as_table) else {
        return Vec::new();
    };
    table
        .keys()
        .filter(|key| !ANYWHERE_KNOWN_KEYS.contains(&key.as_str()))
        .map(|key| (key.clone(), closest_anywhere_key(key)))
        .collect()
}

fn closest_anywhere_key(key: &str) -> Option<&'static str> {
    ANYWHERE_KNOWN_KEYS
        .iter()
        .copied()
        .map(|known| (known, levenshtein(key, known)))
        .filter(|(_, dist)| *dist <= 2)
        .min_by_key(|(_, dist)| *dist)
        .map(|(known, _)| known)
}

/// Classic Wagner-Fischer edit distance, used only for the small "did you mean" hint above — not
/// performance-sensitive (a handful of short config-key comparisons per load).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, &cb) in b.iter().enumerate() {
            cur[j + 1] = if ca == cb {
                prev[j]
            } else {
                1 + prev[j].min(prev[j + 1]).min(cur[j])
            };
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Human-readable warnings for unrecognized `[anywhere]` keys across every config file
/// [`load`] reads (user config dir + project `./.forge/config.toml`). Shared by the loader's own
/// warning and `forge anywhere doctor`, so both name the same typo the same way.
pub fn anywhere_config_warnings() -> Vec<String> {
    anywhere_config_warnings_in(std::path::Path::new("."))
}

/// [`anywhere_config_warnings`] for the project rooted at `root`.
pub fn anywhere_config_warnings_in(root: &std::path::Path) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(dir) = config_dir() {
        paths.push(dir.join("config.toml"));
    }
    paths.push(root.join(".forge/config.toml"));
    let mut warnings = Vec::new();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (key, suggestion) in anywhere_unknown_keys(&text) {
            warnings.push(match suggestion {
                Some(known) => {
                    format!("unknown key `{key}` in [anywhere] — did you mean `{known}`?")
                }
                None => format!("unknown key `{key}` in [anywhere]"),
            });
        }
    }
    warnings
}

/// The default `forge serve` port when `[remote] port` is unset (see [`RemoteConfig::port`]).
pub const DEFAULT_SERVE_PORT: u16 = 7420;

impl RemoteConfig {
    /// The exposure to auto-start at chat launch, or `None` when remote control is left off.
    pub fn startup_exposure(&self) -> Option<RemoteAuto> {
        match self.auto {
            RemoteAuto::Off => None,
            other => Some(other),
        }
    }

    /// The `forge serve` daemon port: `[remote] port`, else [`DEFAULT_SERVE_PORT`].
    pub fn serve_port(&self) -> u16 {
        self.port.unwrap_or(DEFAULT_SERVE_PORT)
    }
}

/// How (if at all) remote control auto-starts at chat launch. Mirrors the `/remote` exposures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAuto {
    /// Off — remote control starts only when the user types `/remote`. Default.
    #[default]
    Off,
    /// Loopback only (control from this machine).
    Local,
    /// LAN-reachable over self-signed HTTPS.
    Lan,
    /// Public tunnel (cloudflared / ngrok) — reachable from any network.
    Anywhere,
}

/// `[voice]` config block — local whisper.cpp speech-to-text (voice.md).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// Whisper model size: `tiny`, `base` (default), `small`, or `medium`. Bigger is more
    /// accurate and slower; parsed by `forge_voice::ModelKind` at the call site (this crate
    /// keeps it a plain string so config loading never depends on forge-voice).
    #[serde(default = "default_voice_model")]
    pub model: String,
    /// Transcription language: a whisper.cpp language code (`"en"`, `"nl"`, ...) or `"auto"`
    /// (default) to detect it from the audio.
    #[serde(default = "default_voice_language")]
    pub language: String,
    /// Auto-stop a recording after this many milliseconds of silence. `0` (default) disables
    /// auto-stop — recording only ends when the user explicitly stops it.
    #[serde(default)]
    pub auto_stop_silence_ms: u64,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            model: default_voice_model(),
            language: default_voice_language(),
            auto_stop_silence_ms: 0,
        }
    }
}

fn default_voice_model() -> String {
    "base".to_string()
}

fn default_voice_language() -> String {
    "auto".to_string()
}

/// `[providers]` config block. Today just custom OpenAI-compatible endpoints; a home for future
/// runtime provider registration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProvidersConfig {
    /// Each `[[providers.custom]]` entry registers one OpenAI-compatible endpoint at runtime.
    #[serde(default)]
    pub custom: Vec<CustomProviderConfig>,
    /// `[providers.azure]` — an Azure OpenAI resource. Unlike the standard OpenAI shape, Azure scopes
    /// the URL to a deployment and authenticates with an `api-key` header (not `Authorization:
    /// Bearer`), so it needs its own block rather than a `[[providers.custom]]` row. Absent = inert.
    #[serde(default)]
    pub azure: Option<AzureConfig>,
    /// `[providers.bedrock]` — which Bedrock models to offer. Bedrock has no model list Forge can
    /// enumerate with an API key, so without this a `bedrock::` id works only when typed by hand
    /// and never appears in a model picker. Absent = inert.
    #[serde(default)]
    pub bedrock: Option<BedrockConfig>,
}

/// `[providers.bedrock]`: the Bedrock model ids to seed into the catalog when a `bedrock` key is
/// stored. Use the id Bedrock invokes, which for most third-party models is an inference profile
/// rather than the bare model id:
/// ```toml
/// [providers.bedrock]
/// models = ["global.moonshotai.kimi-k3"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BedrockConfig {
    #[serde(default)]
    pub models: Vec<String>,
}

/// `[providers.azure]`: an Azure OpenAI resource the user configures without recompiling. Azure's
/// chat URL is `https://<resource>.openai.azure.com/openai/deployments/<deployment>/chat/completions?
/// api-version=<ver>` with an `api-key` header — Forge reaches it through genai's OpenAI adapter via a
/// per-request URL+header override (`AuthData::RequestOverride`), so the request body is standard
/// OpenAI chat-completions (tool calls included). Each model id is `azure::<deployment>`; the
/// deployment name is both the routing key and the body `model`. Example:
/// ```toml
/// [providers.azure]
/// resource    = "my-resource"          # -> https://my-resource.openai.azure.com  (or set `endpoint`)
/// api_version = "2024-10-21"           # optional — defaults to a recent GA version
/// api_key_env = "AZURE_OPENAI_API_KEY" # optional — this is the default
/// deployments = ["gpt-4o", "gpt-4o-mini"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AzureConfig {
    /// Azure resource name → `https://<resource>.openai.azure.com`. Ignored if `endpoint` is set.
    #[serde(default)]
    pub resource: Option<String>,
    /// Full resource endpoint base (e.g. `https://my-resource.openai.azure.com`). Overrides
    /// `resource`; lets sovereign/custom Azure clouds (`*.openai.azure.us`, proxies) be targeted.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Azure REST `api-version` query value. Optional — defaults to [`DEFAULT_AZURE_API_VERSION`].
    #[serde(default)]
    pub api_version: Option<String>,
    /// Env var holding the Azure API key. Optional — defaults to [`AZURE_DEFAULT_KEY_ENV`].
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Deployment names (each becomes an `azure::<deployment>` model id). May be empty (the user pins
    /// `azure::<deployment>` ids manually); Azure has no enumerable deployments endpoint in our flow.
    #[serde(default)]
    pub deployments: Vec<String>,
    /// Whether these deployments are free to call. Azure is metered, so this is `false` by default.
    #[serde(default)]
    pub free: bool,
    /// Human label shown in `forge provider list`. Optional.
    #[serde(default)]
    pub label: Option<String>,
}

/// One user-declared `[[providers.custom]]` endpoint. Mirrors a [`CustomProvider`] row but owned and
/// deserializable; converted (with validation + endpoint normalization) into the static registry at
/// startup. Example:
/// ```toml
/// [[providers.custom]]
/// namespace = "lmstudio"
/// base_url  = "http://localhost:1234/v1"   # trailing /v1 or /v1/ both accepted
/// api_key_env = "LMSTUDIO_API_KEY"          # optional — omit for a keyless local server
/// free = true                                # optional (default false)
/// models = ["qwen2.5-coder-32b"]            # optional — else discovered via /v1/models
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomProviderConfig {
    /// The `provider::` namespace in model ids and the `forge provider` name.
    pub namespace: String,
    /// Full base URL of the OpenAI-compatible server (the part before `chat/completions`).
    pub base_url: String,
    /// Env var holding the API key. Omit for a keyless local server (a placeholder is sent).
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Whether this endpoint's models are genuinely free to call (e.g. a local server).
    #[serde(default)]
    pub free: bool,
    /// Explicit model ids (bare, no namespace). Empty → discover live via `/v1/models`.
    #[serde(default)]
    pub models: Vec<String>,
    /// Human label shown in `forge provider list`. Optional.
    #[serde(default)]
    pub label: Option<String>,
    /// How to turn a marketplace price list into one rate, for an endpoint whose `/models`
    /// publishes several asks per model (InferHub: `pricing.asks_in` / `asks_out`, plus the
    /// vendor's `official_in` / `official_out`). Default `median`. Ignored by endpoints that
    /// publish no such shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_stat: Option<PricingStat>,
}

/// Which statistic of a marketplace's asks prices a model (see
/// [`CustomProviderConfig::pricing_stat`]).
///
/// `min` is what the cheapest seller asks, which UNDERSTATES what a routed request costs: the
/// gateway may serve it from any seller. `median` is the default because it is robust to a single
/// outlier ask on either side; `mean` follows a skewed list; `official` uses the vendor's list
/// price. Every statistic falls back to the official price when a model has no asks, and actual
/// spend comes from the response's reported cost whatever is chosen here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PricingStat {
    #[default]
    Median,
    Min,
    Mean,
    Official,
}

fn default_stop_hook_max_blocks() -> u32 {
    8
}

/// Map a Forge tool name to its Claude-Code equivalent, so a CC hook matcher written against CC
/// tool names (`Write`, `Edit`, `Bash`, …) still fires on the corresponding Forge tool. Returns the
/// original name when there's no distinct CC alias.
pub fn cc_tool_alias(forge_tool: &str) -> &str {
    match forge_tool {
        "shell" => "Bash",
        "edit_file" | "apply_patch" => "Edit",
        "write_file" | "create_file" => "Write",
        "read_file" => "Read",
        "list_dir" | "list_files" | "ls" => "LS",
        "search" | "grep" | "ripgrep" => "Grep",
        "glob" => "Glob",
        "web_fetch" => "WebFetch",
        "web_search" => "WebSearch",
        other => other,
    }
}

/// Map a Claude-Code tool name (`Bash`, `Edit`, `Read`, …) to Forge's tool name, for translating a
/// CC `settings.json` permission entry (`Bash(npm run *)`) into a Forge `[[permissions.rules]]`
/// block. Unknown names (e.g. an MCP tool) pass through unchanged.
pub fn forge_tools_from_cc(cc_tool: &str) -> Vec<&'static str> {
    match cc_tool {
        "Bash" => vec!["shell"],
        "Edit" | "MultiEdit" => vec!["edit_file", "apply_patch"],
        "Write" => vec!["write_file", "create_file"],
        "Read" => vec!["read_file"],
        "LS" => vec!["list_dir"],
        "Grep" => vec!["search"],
        "Glob" => vec!["glob"],
        "WebFetch" => vec!["web_fetch"],
        "WebSearch" => vec!["web_search"],
        _ => vec![],
    }
}

/// Map a Claude-Code tool name to Forge's primary tool name for compatibility callers.
pub fn forge_tool_from_cc(cc_tool: &str) -> &str {
    forge_tools_from_cc(cc_tool)
        .into_iter()
        .next()
        .unwrap_or(cc_tool)
}

/// Settings for the Lattice code-intelligence subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatticeConfig {
    /// Build + maintain the structural graph. Off → the `forge lattice` commands are inert.
    #[serde(default = "default_lattice_enabled")]
    pub enabled: bool,
    /// Auto-inject relevant code into each turn (the killer step). Off → the index still builds
    /// and the CLI/tool work, but no system message is injected before the model call.
    #[serde(default = "default_lattice_inject")]
    pub inject: bool,
    /// Token ceiling for the auto-injected context block. Scaled down as the daily budget tightens.
    #[serde(default = "default_inject_budget")]
    pub inject_token_budget: usize,
    /// Watch the working tree and reindex changed files automatically (external editor edits), so
    /// retrieval stays fresh without a manual `forge lattice update`. Off → index only updates on
    /// explicit `update`/agent edits.
    #[serde(default = "default_lattice_watch")]
    pub watch: bool,
    /// Semantic retrieval via embeddings (code-intelligence.md §5.6). On by default with
    /// `backend = "auto"`: embeddings are computed in the background when a backend is reachable
    /// and blended into retrieval, and it's a no-op (zero cost, structural-only) when none is.
    #[serde(default)]
    pub embeddings: EmbeddingsConfig,
    /// Inject the *source body* of the top-ranked retrieved symbols (not just their signature),
    /// so the model can read a function directly from context instead of spending a `read_file`
    /// (which dumps the whole file into the transcript). The single biggest token-saving lever —
    /// see docs/features/lattice-token-savings.md. Off → signature-only (legacy behaviour).
    #[serde(default = "default_inject_bodies")]
    pub inject_bodies: bool,
    /// Per-symbol token ceiling for an injected body. Symbols whose body exceeds this are kept as
    /// a signature line instead (injecting a huge body would cost more than the read it saves).
    #[serde(default = "default_body_max_tokens")]
    pub body_max_tokens: usize,
    /// How many of the top-ranked symbols get their FULL body injected (the rest stay signature
    /// lines). Higher = more aggressive front-loading: the model reads more from context up front
    /// instead of `read_file`/`search`-ing for it. Capped further by `inject_token_budget`.
    #[serde(default = "default_inject_body_hits")]
    pub inject_body_hits: usize,
    /// Future hook for `forge lattice map`: when true, group the map output by importance tier
    /// (high / medium / low pagerank bands) rather than by file path. Not yet wired into the
    /// agent turn loop — present so it can be set in config ahead of the feature landing.
    #[serde(default)]
    pub map_orientation: bool,
}

/// Embedding-backed semantic retrieval settings. On by default with `backend = "auto"`, which
/// picks the cheapest available backend and is a zero-cost no-op when none is reachable; node
/// embeddings are computed via the chosen backend and blended into retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsConfig {
    /// Master switch. Off → no embeddings are computed or used; retrieval is structural/lexical.
    /// Defaults to `true`: the `auto` backend picks the cheapest available provider (Gemini
    /// free-tier first) and gracefully no-ops if nothing is reachable — so it's safe always-on.
    #[serde(default = "default_embed_enabled")]
    pub enabled: bool,
    /// Embedding backend: `"auto"` (default — pick the cheapest available, see below), `"ollama"`
    /// (local HTTP), or a provider namespace genai can embed with (`"openai"`, `"gemini"`). `auto`
    /// prefers a free/cheap cloud model when a key exists (gemini free-tier → openai), else falls
    /// back to local ollama; if nothing is reachable it costs nothing (retrieval stays structural).
    #[serde(default = "default_embed_backend")]
    pub backend: String,
    /// Embedding model id. Used as-is for `ollama`/explicit-provider backends (e.g. ollama's
    /// `nomic-embed-text`); ignored under `auto`, which picks the model per chosen provider.
    #[serde(default = "default_embed_model")]
    pub model: String,
    /// ollama HTTP API root (only used by the ollama backend / auto's local fallback).
    #[serde(default = "default_embed_endpoint")]
    pub endpoint: String,
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: default_embed_backend(),
            model: default_embed_model(),
            endpoint: default_embed_endpoint(),
        }
    }
}

fn default_embed_enabled() -> bool {
    true
}

fn default_embed_backend() -> String {
    "auto".to_string()
}

fn default_embed_model() -> String {
    "nomic-embed-text".to_string()
}

fn default_embed_endpoint() -> String {
    "http://localhost:11434".to_string()
}

impl Default for LatticeConfig {
    fn default() -> Self {
        Self {
            enabled: default_lattice_enabled(),
            inject: default_lattice_inject(),
            inject_token_budget: default_inject_budget(),
            watch: default_lattice_watch(),
            embeddings: EmbeddingsConfig::default(),
            inject_bodies: default_inject_bodies(),
            body_max_tokens: default_body_max_tokens(),
            inject_body_hits: default_inject_body_hits(),
            map_orientation: false,
        }
    }
}

fn default_inject_bodies() -> bool {
    true
}

fn default_inject_body_hits() -> usize {
    3
}

fn default_body_max_tokens() -> usize {
    800
}

fn default_lattice_watch() -> bool {
    true
}

fn default_lattice_enabled() -> bool {
    true
}

fn default_lattice_inject() -> bool {
    true
}

fn default_inject_budget() -> usize {
    // Sized so the top few symbol *bodies* fit (body injection is the token-saving lever): a body
    // costs up to `body_max_tokens` (~800) and we inject up to 3, so ~3000 covers bodies + a tail
    // of signature lines. Prompt-adaptive scaling in retrieval avoids over-injecting on simple
    // prompts. See docs/features/lattice-token-savings.md.
    3000
}

/// A three-state switch for integrations Forge can detect on its own: `auto` (default) turns the
/// feature on when its prerequisite is present, `on` forces it, `off` disables it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AutoToggle {
    #[default]
    Auto,
    On,
    Off,
}

impl AutoToggle {
    /// Resolve against whether the prerequisite was detected.
    pub fn resolve(self, detected: bool) -> bool {
        match self {
            AutoToggle::Auto => detected,
            AutoToggle::On => true,
            AutoToggle::Off => false,
        }
    }
}

/// Settings for the `shell` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellConfig {
    /// When a shell command fails (non-zero exit, timeout, spawn error), make one cheap
    /// trivial-tier model call to explain the likely cause and suggest a fix, surfaced
    /// alongside the result — no prompt needed (shell-error-interceptor.md). Skipped when the
    /// budget is exhausted.
    #[serde(default = "default_explain_errors")]
    pub explain_errors: bool,
    /// Opt-in OS-level sandbox using Linux Landlock (Linux 5.13+). When true, shell commands
    /// run under a kernel-enforced ruleset that confines filesystem **writes** to the workspace
    /// (cwd) and the system temp directory; reads stay broad so tools continue to work. A clean
    /// no-op on macOS/Windows and on Linux kernels without Landlock support — never hard-fails
    /// a command. Default: false (opt-in).
    ///
    /// TOML key: `shell.sandbox`
    #[serde(default)]
    pub sandbox: bool,
    /// Extra writable paths beyond the workspace (cwd) and the system temp directory. Each
    /// entry is an absolute path (relative entries are resolved against cwd at the time the
    /// shell command runs). Ignored when `sandbox = false`.
    ///
    /// TOML key: `shell.sandbox_writable`
    #[serde(default)]
    pub sandbox_writable: Vec<String>,
    /// When true, cargo/rust build commands run by the `shell` tool get a scoped, writable
    /// `CARGO_TARGET_DIR` pointing OUTSIDE the workspace (a per-project subdir of
    /// `scoped_cargo_target_dir`, or `<system-temp>/forge-cargo-target` when unset). This lets an
    /// autonomous/bypass-mode agent run `cargo check`/`build` to verify its own edits even when the
    /// workspace tree is read-only under confinement — without loosening the sandbox for arbitrary
    /// writes (only the build-target dir is carved out, and only for recognized cargo commands). An
    /// explicit `CARGO_TARGET_DIR` already in the environment always wins. Works with or without
    /// `shell.sandbox` (an outer container can confine writes too). Default: false (opt-in).
    ///
    /// TOML key: `shell.scoped_cargo_target`
    #[serde(default)]
    pub scoped_cargo_target: bool,
    /// Base directory for the scoped `CARGO_TARGET_DIR` (see `scoped_cargo_target`). Each project
    /// gets a stable subdir here. When unset, defaults to `<system-temp>/forge-cargo-target`.
    /// Ignored when `scoped_cargo_target = false`.
    ///
    /// TOML key: `shell.scoped_cargo_target_dir`
    #[serde(default)]
    pub scoped_cargo_target_dir: Option<String>,
    /// Route eligible shell commands through RTK (`rtk`, the Rust Token Killer) so the model reads
    /// its compacted output — `cargo test` collapses to a one-line summary plus the failures,
    /// `ls`/`find`/`wc` to their essentials (docs/features/token-savings.md). `auto` (default):
    /// on when `rtk` is on `PATH`; `on` / `off` force it. Only commands whose RTK rendering is
    /// lossless for an agent are rewritten; the result header says when it happened.
    ///
    /// TOML key: `shell.rtk`
    #[serde(default)]
    pub rtk: AutoToggle,
    /// Programs (first word, e.g. `"cargo"`) to never route through RTK.
    ///
    /// TOML key: `shell.rtk_skip`
    #[serde(default)]
    pub rtk_skip: Vec<String>,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            explain_errors: default_explain_errors(),
            sandbox: false,
            sandbox_writable: Vec::new(),
            scoped_cargo_target: false,
            scoped_cargo_target_dir: None,
            rtk: AutoToggle::Auto,
            rtk_skip: Vec::new(),
        }
    }
}

fn default_explain_errors() -> bool {
    true
}

/// Per-turn recap: a one-line AI-generated summary shown in scrollback after each completed turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecapConfig {
    /// Emit a `※ recap …` line after each completed turn. Default: true. Disable in /config.
    #[serde(default = "default_recap_enabled")]
    pub enabled: bool,
}

impl Default for RecapConfig {
    fn default() -> Self {
        Self {
            enabled: default_recap_enabled(),
        }
    }
}

fn default_recap_enabled() -> bool {
    true
}

/// Structured file-tool workspace confinement: an opt-in allowlist of extra roots that
/// read_file/write_file/edit_file/multi_edit/apply_patch/append_file/notebook_edit/delete_file/
/// list_dir/search/glob may read and write, in addition to the session workspace itself.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Absolute paths outside the session workspace that the structured file tools may read and
    /// write, in addition to the workspace itself. Relative entries are ignored. Default empty =
    /// file tools stay confined to the workspace. Separate from `shell.sandbox_writable` (which
    /// governs the Landlock write-sandbox) and does not affect `shell` (already unconfined).
    #[serde(default)]
    pub extra_roots: Vec<String>,
    /// Hard-confine the structured file tools to the workspace plus `extra_roots`. Off by
    /// default: a path outside the workspace goes through the permission broker like any other
    /// side effect (Full allows, Ask/Auto/Auto-edit prompt, Read-only denies; the secret denylist
    /// applies in every mode). Hard confinement never protected anything `shell` could not
    /// already reach, and refusing the file tools pushed models into writing files via `shell`.
    #[serde(default)]
    pub confine_to_workspace: bool,
}

impl ToolsConfig {
    /// The absolute roots the file tools may use beyond the workspace: the configured
    /// `extra_roots`, or the filesystem root when the tools are not confined.
    pub fn effective_extra_roots(&self) -> Vec<String> {
        let mut roots: Vec<String> = self
            .extra_roots
            .iter()
            .filter(|root| std::path::Path::new(root).is_absolute())
            .cloned()
            .collect();
        if !self.confine_to_workspace {
            roots.push("/".to_string());
        }
        roots
    }
}

/// Next-prompt suggestion: predicts the user's likely next prompt after each completed turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestConfig {
    /// Generate a next-prompt suggestion after each completed turn. Default: true. Disable in
    /// /config if you find it noisy.
    #[serde(default = "default_suggest_enabled")]
    pub enabled: bool,
}

impl Default for SuggestConfig {
    fn default() -> Self {
        Self {
            enabled: default_suggest_enabled(),
        }
    }
}

fn default_suggest_enabled() -> bool {
    true
}

/// Project initialization behavior. Disabled by default because setup performs a model-backed,
/// repository-specific analysis and consumes the configured model quota.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Automatically run the tailored project setup once when opening an uninitialized project.
    #[serde(default)]
    pub auto_initialize: bool,
}

/// Local-LLM runtime settings. Forge runs local models through Ollama (exposed in the mesh as
/// `ollama::<tag>`). Off by default — nothing starts unless the user opts in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    /// Start the local runtime + `model` automatically when `forge chat` launches. Default false
    /// (no surprise multi-GB model load / background server); use `forge local start` to run it
    /// on demand.
    #[serde(default)]
    pub autostart: bool,
    /// The Ollama tag to auto-start / treat as the default local model (e.g. `gemma4:12b`).
    #[serde(default)]
    pub model: Option<String>,
    /// Ollama HTTP endpoint. Defaults to the local server.
    #[serde(default = "default_local_endpoint")]
    pub endpoint: String,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            autostart: false,
            model: None,
            endpoint: default_local_endpoint(),
        }
    }
}

fn default_local_endpoint() -> String {
    "http://localhost:11434".to_string()
}

/// Startup update-check settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    /// Check GitHub releases on startup and notify when a newer version exists. Default: true.
    /// Checked at most every 6 hours, and offered as an in-app card (or a prompt in plain chat); the env var `FORGE_NO_UPDATE_CHECK=1` also disables it.
    #[serde(default = "default_update_check")]
    pub check: bool,
    /// Install a newer release automatically on an interactive launch: a standalone install
    /// swaps itself before the TUI opens, Homebrew/Scoop installs upgrade in the background, and
    /// the `forge serve` daemon updates itself when idle. Default: true. `FORGE_NO_AUTO_UPDATE=1`
    /// also disables it; `check = false` / `FORGE_NO_UPDATE_CHECK` disable everything.
    #[serde(default = "default_update_check")]
    pub auto: bool,
}

/// `[telemetry]` config block. Forge reports only fixed, anonymous counters; see
/// `docs/telemetry.md` for the complete wire contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Send anonymous install and activity counters. This defaults on so aggregate adoption
    /// numbers are representative; users can opt out without losing any Forge functionality.
    #[serde(default = "default_telemetry_enabled")]
    pub enabled: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: default_telemetry_enabled(),
        }
    }
}

fn default_telemetry_enabled() -> bool {
    true
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            check: default_update_check(),
            auto: default_update_check(),
        }
    }
}

fn default_update_check() -> bool {
    true
}

/// Git integration settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    /// When true, `forge git setup` installs a prepare-commit-msg hook that strips
    /// Claude/Codex co-author lines and adds `Co-Authored-By: Forge <forge@adulari.dev>`.
    #[serde(default)]
    pub coauthor: bool,
    /// Commit discipline (docs/features/commit-discipline.md). While the files this session has
    /// edited are still uncommitted, every turn starts with a one-line reminder of them, and a
    /// further reminder lands mid-turn once `commit_nudge_edits` more successful edits have
    /// accumulated since the last commit or reminder. A week-long session that never committed
    /// motivated this: 49 commits and 2 pushes against 2,600 edits, with two whole days at zero.
    /// `false` turns both reminders (and the push reminder) off; the system prompt still asks the
    /// model to commit verified units of work.
    #[serde(default = "default_commit_nudge")]
    pub commit_nudge: bool,
    /// Successful write-tool calls since the last commit (or the last reminder) before the
    /// mid-turn reminder fires again. `0` disables the mid-turn reminder only.
    #[serde(default = "default_commit_nudge_edits")]
    pub commit_nudge_edits: u32,
    /// Commits the current branch may run ahead of its upstream before the turn-start reminder
    /// asks the model to offer the user a push (Forge never pushes on its own). `0` disables it.
    #[serde(default = "default_push_nudge_ahead")]
    pub push_nudge_ahead: u32,
    /// `auto` (default: `unit` interactively, `end` for headless one-shot runs), `unit`, `end`, or
    /// `never`: swaps the system prompt's version-control paragraph and, for anything but `unit`,
    /// silences the commit/push reminders above.
    #[serde(default)]
    pub commit_policy: CommitPolicy,
    /// Language the review dock's "generate commit message" button writes in (default English).
    #[serde(default = "default_commit_language")]
    pub commit_language: String,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            coauthor: false,
            commit_nudge: default_commit_nudge(),
            commit_nudge_edits: default_commit_nudge_edits(),
            push_nudge_ahead: default_push_nudge_ahead(),
            commit_policy: CommitPolicy::default(),
            commit_language: default_commit_language(),
        }
    }
}

fn default_commit_language() -> String {
    commit_language::DEFAULT_COMMIT_LANGUAGE.to_string()
}

fn default_commit_nudge() -> bool {
    true
}

fn default_commit_nudge_edits() -> u32 {
    10
}

fn default_push_nudge_ahead() -> u32 {
    3
}

/// LSP-backed live diagnostics. After an edit, Forge asks a language server for diagnostics on the
/// touched file and feeds any errors back into the turn so the model self-corrects. Off = inert;
/// when no server binary is on PATH the path degrades silently to no diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    #[serde(default)]
    pub enabled: bool,
    /// How long to wait for a server's `publishDiagnostics` before giving up.
    #[serde(default = "default_lsp_timeout_ms")]
    pub timeout_ms: u64,
    /// Maximum resident memory for one analyzer process tree, in MiB. `0` disables the guard.
    ///
    /// Forge runs at most one analyzer process tree at a time, so this is also the process-wide
    /// aggregate LSP memory ceiling.
    #[serde(default = "default_lsp_memory_limit_mb")]
    pub memory_limit_mb: u64,
    /// Stop an analyzer after this many idle seconds. A later edit starts it again on demand.
    #[serde(default = "default_lsp_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    /// How much of the post-edit report reaches the model: `delta` (default), `full` or `off`.
    #[serde(default)]
    pub diagnostics: LspDiagnosticsMode,
    /// Per-language server command, keyed by language id (`rust`, `typescript`, `python`, …).
    /// Empty = use the built-in defaults for languages whose server binary is found on PATH.
    #[serde(default)]
    pub servers: std::collections::HashMap<String, LspServerEntry>,
}

/// One language server invocation: a binary plus extra args (e.g. `["--stdio"]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerEntry {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_ms: default_lsp_timeout_ms(),
            memory_limit_mb: default_lsp_memory_limit_mb(),
            idle_timeout_secs: default_lsp_idle_timeout_secs(),
            diagnostics: LspDiagnosticsMode::default(),
            servers: std::collections::HashMap::new(),
        }
    }
}

fn default_lsp_timeout_ms() -> u64 {
    3000
}

fn default_lsp_memory_limit_mb() -> u64 {
    2048
}

fn default_lsp_idle_timeout_secs() -> u64 {
    120
}

/// Auto-lint / auto-test self-healing. After a turn makes edits, run the configured lint and/or
/// test command; on a non-zero exit, feed the output back into the turn so the model fixes it,
/// looping up to `max_iterations` times. Off = inert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutofixConfig {
    #[serde(default)]
    pub auto_lint: bool,
    #[serde(default)]
    pub auto_test: bool,
    /// Lint command line (e.g. `cargo clippy --all-targets` / `npm run lint`). Empty disables lint.
    #[serde(default)]
    pub lint_cmd: String,
    /// Test command line (e.g. `cargo test` / `pytest -x`). Empty disables test.
    #[serde(default)]
    pub test_cmd: String,
    /// Max fix attempts per turn before giving up and surfacing the remaining failures.
    #[serde(default = "default_autofix_iterations")]
    pub max_iterations: u32,
    /// Detect lint/test commands from project structure when lint_cmd/test_cmd are empty.
    /// Cargo.toml → `cargo check --all-targets 2>&1`; package.json → `npm run lint 2>&1`.
    /// auto_lint / auto_test must still be true; this only fills in the command.
    #[serde(default = "default_true")]
    pub auto_detect: bool,
}

impl Default for AutofixConfig {
    fn default() -> Self {
        Self {
            auto_lint: false,
            auto_test: false,
            lint_cmd: String::new(),
            test_cmd: String::new(),
            max_iterations: default_autofix_iterations(),
            auto_detect: true,
        }
    }
}

fn default_autofix_iterations() -> u32 {
    3
}

pub(crate) fn default_true() -> bool {
    true
}

/// Assay-gated auto-review: before a write turn finishes, run the Assay critic crew on the turn's
/// diff and warn (or block) on findings at/above `gate_severity`. Off = inert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssayConfig {
    #[serde(default)]
    pub auto_review: bool,
    /// Minimum severity that triggers the gate: `low` | `medium` | `high`.
    #[serde(default = "default_assay_gate_severity")]
    pub gate_severity: String,
    /// `warn` surfaces findings but lets the turn finish; `block` fails the turn.
    #[serde(default = "default_assay_gate_mode")]
    pub gate_mode: String,
    /// Skip review when the turn's diff is smaller than this (bytes) — trivial edits aren't worth it.
    #[serde(default = "default_assay_min_diff_bytes")]
    pub min_diff_bytes: usize,
    /// Maximum estimated USD cost for one auto-review gate run. When the pre-estimate exceeds
    /// this cap the gate is skipped with a warning instead of running away cost. 0.0 = unlimited.
    #[serde(default = "default_assay_max_cost_usd")]
    pub max_cost_usd: f64,
}

impl Default for AssayConfig {
    fn default() -> Self {
        Self {
            auto_review: false,
            gate_severity: default_assay_gate_severity(),
            gate_mode: default_assay_gate_mode(),
            min_diff_bytes: default_assay_min_diff_bytes(),
            max_cost_usd: default_assay_max_cost_usd(),
        }
    }
}

fn default_assay_gate_severity() -> String {
    "high".to_string()
}

fn default_assay_gate_mode() -> String {
    "warn".to_string()
}

fn default_assay_min_diff_bytes() -> usize {
    200
}

fn default_assay_max_cost_usd() -> f64 {
    0.50
}

/// Settings for the slash-command + skill system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandsConfig {
    /// Trust project-scope (`./.forge/`) commands/skills without a first-run confirmation. A
    /// project file is a *prompt*, not an instruction to the harness — but it can still try to
    /// steer the model, so by default the first use of a project-scope definition is confirmed.
    #[serde(default)]
    pub trust_project: bool,
    /// Max rows the command palette shows at once before overflow.
    #[serde(default = "default_max_palette")]
    pub max_palette: usize,
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self {
            trust_project: false,
            max_palette: default_max_palette(),
        }
    }
}

fn default_max_palette() -> usize {
    8
}

/// Fine-grained permission rules (FR-10). Resolution is by specificity/precedence, not file
/// order; see `forge_core::permission`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
    /// `auto` temper: ask one cheap side-call model about a shell command the heuristics do not
    /// recognise. Off means such a command always asks.
    #[serde(default = "default_true")]
    pub auto_classifier: bool,
    /// `ask` temper: run shell commands the broker can prove read-only (`git status`, `ls`,
    /// `cat file`, `grep -rn x src`) without a prompt, as Claude Code does. Deny and ask rules,
    /// the secret-path rules and every other temper are unaffected; false makes every shell
    /// command prompt again (ADR-0008).
    #[serde(default = "default_true")]
    pub auto_allow_read_only_shell: bool,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            auto_classifier: true,
            auto_allow_read_only_shell: true,
        }
    }
}

/// One TOML rule block: a tool plus exactly one of `allow`/`ask`/`deny` (string or list).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConfig {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<OneOrMany>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<OneOrMany>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny: Option<OneOrMany>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A TOML scalar-or-array of strings (so `allow = "git *"` and `allow = ["a","b"]` both work).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }

    /// The first entry (the single value, or the head of the list). Empty `Many` → `""`.
    pub fn first(&self) -> &str {
        match self {
            OneOrMany::One(s) => s,
            OneOrMany::Many(v) => v.first().map(String::as_str).unwrap_or(""),
        }
    }

    /// All entries as owned strings (one-element vec for the single form).
    pub fn all(&self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

impl RuleConfig {
    /// Convert to a runtime rule. Deny is highest precedence if more than one is set.
    fn to_rule(&self) -> Option<PermissionRule> {
        let (decision, pats) = if let Some(d) = &self.deny {
            (PermissionDecision::Deny, d.clone())
        } else if let Some(a) = &self.ask {
            (PermissionDecision::Ask, a.clone())
        } else if let Some(a) = &self.allow {
            (PermissionDecision::Allow, a.clone())
        } else {
            return None; // a block with no decision is ignored
        };
        Some(PermissionRule {
            tool: self.tool.clone(),
            patterns: pats.into_vec(),
            decision,
            source: RuleSource::Configured,
            reason: self.reason.clone(),
        })
    }
}

/// Built-in safety deny rules — present even with zero config, unoverridable (`Builtin`),
/// active in every mode including `bypass`.
pub fn builtin_deny_rules() -> Vec<PermissionRule> {
    let deny = |tool: &str, pats: &[&str]| PermissionRule {
        tool: tool.to_string(),
        patterns: pats.iter().map(|s| s.to_string()).collect(),
        decision: PermissionDecision::Deny,
        source: RuleSource::Builtin,
        reason: Some("built-in safety rule".into()),
    };
    let secrets = [
        "**/.env",
        // `**/.env` does NOT match `.env.local` / `.env.production` / `.env.*` — a real gap, since
        // those dotenv variants are where deploy secrets usually live. Cover them explicitly.
        "**/.env.*",
        "**/*.pem",
        "**/*.key",
        "**/*.p12",
        "**/*.pfx",
        "**/*.keystore",
        "**/*.jks",
        // SSH private keys (all common key types) + the whole ~/.ssh dir.
        "**/id_rsa",
        "**/id_ed25519",
        "**/id_ecdsa",
        "**/id_dsa",
        "**/.ssh/**",
        // Cloud / cluster / registry credentials.
        "**/.aws/credentials",
        "**/.git-credentials",
        "**/.netrc",
        "**/.npmrc",
        "**/.pypirc",
        "**/.kube/config",
        "**/.docker/config.json",
        "**/.config/gcloud/**",
        "**/.gnupg/**",
    ];
    vec![
        deny(
            "shell",
            &[
                // catastrophic filesystem / disk (Unix)
                "rm -rf /",
                "rm -rf ~",
                "rm -rf /*",
                ":(){ :|:& };:",
                "dd of=/dev/*",
                "mkfs*",
                "mkfs.*",
                // catastrophic filesystem / disk (Windows)
                "del /s *",
                "del /f /s *",
                "del /q /s *",
                "rd /s *",
                "rmdir /s *",
                "format ?:*",
                // remote-to-shell pipe (matched against the raw command line)
                "*| sh",
                "*|sh",
                "*| bash",
                "*|bash",
                "*| zsh",
                "*|zsh",
                // Windows: catastrophic filesystem / disk (cmd.exe + PowerShell)
                "rd /s /q *\\",
                "rd /s /q /",
                "rmdir /s /q *\\",
                "rmdir /s /q /",
                "del /f /s /q *\\*",
                "format c:*",
                "format *: /q*",
                "Remove-Item -Recurse -Force /*",
                "Remove-Item -Recurse -Force C:\\*",
                "rm -Recurse -Force /*",
                // secret-file reads via common verbs
                "cat *.env",
                "cat *.env.*",
                "cat *.pem",
                "cat *.key",
                "cat *id_rsa*",
                "cat *id_ed25519*",
                "cat *id_ecdsa*",
                "cat *id_dsa*",
                "cat */.ssh/*",
                "cat *.aws/credentials*",
                "cat *.git-credentials*",
                "cat *.netrc*",
                "cat *.npmrc*",
                "cat *.pypirc*",
                "cat */.kube/config*",
                "less *.env",
                "less *.env.*",
                "head *.env",
                "head *.env.*",
                "tail *.env",
                "tail *.env.*",
                "cp *.env *",
                "cp *.env.* *",
                "cp */.ssh/* *",
                // Secret reads via the OTHER common non-interactive verbs an agent reaches for when
                // `cat` is the obvious-but-blocked choice: text tools (grep/awk/sed/nl/sort/cut),
                // binary dumps / encoders for exfil (xxd/od/strings/base64), and `source`/`.` which
                // execute a dotenv straight into the environment. Best-effort defense-in-depth — the
                // primary block is the read_file/list_dir tool denylist; this catches the shell-out.
                "grep *.env",
                "grep *.env.*",
                "grep *.pem",
                "grep *.key",
                "grep *id_rsa*",
                "grep */.ssh/*",
                "grep *.aws/credentials*",
                "egrep *.env",
                "rg *.env",
                "awk *.env",
                "awk *.env.*",
                "sed *.env",
                "sed *.env.*",
                "nl *.env",
                "sort *.env",
                "cut *.env",
                "xxd *.env",
                "xxd */.ssh/*",
                "od *.env",
                "strings *.env",
                "strings */.ssh/*",
                "base64 *.env",
                "base64 *.env.*",
                "base64 */.ssh/*",
                "base64 *.pem",
                "base64 *.key",
                "source *.env",
                "source *.env.*",
                ". *.env",
                ". *.env.*",
                // Windows: secret-file reads via type/more
                "type *.env",
                "type *.env.*",
                "type *.pem",
                "type *.key",
                "type *id_rsa*",
                "type */.ssh/*",
                "more *.env",
                "more *.env.*",
                "copy *.env *",
                "copy *.env.* *",
            ],
        ),
        deny("read_file", &secrets),
        deny("list_dir", &secrets),
        // `search`/`glob` walk an arbitrary root with std::fs directly (like read_file/list_dir),
        // so they need the same secrets-glob floor.
        deny("search", &secrets),
        deny("glob", &secrets),
        deny("lsp", &secrets),
        // Secrets must also be blocked for write/edit/delete — overwriting a .env or deleting an
        // SSH key is as dangerous as reading it. Also block /etc writes (system config tampering).
        deny("write_file", &{
            let mut v = secrets.to_vec();
            v.extend(["**/.ssh/**", "/etc/**"]);
            v
        }),
        deny("edit_file", &{
            let mut v = secrets.to_vec();
            v.extend(["**/.ssh/**", "/etc/**"]);
            v
        }),
        // The other file-mutating tools carry paths in the same places and need the same floor.
        deny("append_file", &{
            let mut v = secrets.to_vec();
            v.extend(["**/.ssh/**", "/etc/**"]);
            v
        }),
        deny("multi_edit", &{
            let mut v = secrets.to_vec();
            v.extend(["**/.ssh/**", "/etc/**"]);
            v
        }),
        deny("apply_patch", &{
            let mut v = secrets.to_vec();
            v.extend(["**/.ssh/**", "/etc/**"]);
            v
        }),
        // delete_file had no deny rules at all — a model could delete .env, SSH keys, etc.
        deny("delete_file", &{
            let mut v = secrets.to_vec();
            v.extend(["**/.ssh/**", "/etc/**"]);
            v
        }),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshConfig {
    /// Tier -> model id, or an ordered list of candidate model ids for that tier. With a
    /// list, the router picks the cheapest *usable* candidate (cost-aware routing); a single
    /// string behaves as a one-element list (back-compat).
    pub models: HashMap<String, OneOrMany>,
    /// Tier names that a *loaded config file* set under `[mesh.models]`, as opposed to the
    /// shipped cold-start seed. Only these count as an explicit user instruction and therefore
    /// override auto-discovery for their tier (`Config::explicit_candidates_for`). Never
    /// (de)serialized — `load()` fills it from the raw TOML of each layer.
    #[serde(skip)]
    pub explicit_models: Vec<String>,
    /// Prefer an already-paid subscription (the `claude-cli::`/`codex-cli::` bridges, $0
    /// marginal cost) over a metered API model when both are usable candidates. Default true.
    #[serde(default = "default_prefer_subscription")]
    pub prefer_subscription: bool,
    /// Daily spend cap in USD across all sessions (FR-5). `daily_cap_usd` is the preferred
    /// key; `daily_budget_usd` is kept as a backward-compatible alias.
    #[serde(alias = "daily_cap_usd", skip_serializing_if = "Option::is_none")]
    pub daily_budget_usd: Option<f64>,
    /// Monthly spend cap in USD across all sessions. Absent = unlimited.
    #[serde(default)]
    pub monthly_cap_usd: Option<f64>,
    /// Weekly spend cap in USD (Monday 00:00 local → Sunday). Absent = unlimited.
    #[serde(default)]
    pub weekly_budget_usd: Option<f64>,
    /// Fraction of a cap that triggers a warning (default 0.8).
    #[serde(default = "default_warn_threshold")]
    pub warn_threshold: f64,
    /// Which task classifier the mesh uses (ADR-0006). Default = LLM, falling back to the
    /// deterministic heuristic when classification is unavailable.
    #[serde(default)]
    pub classifier: ClassifierKind,
    /// Fixed model id the `llm` classifier calls to label the tier. A fixed default keeps
    /// identical inputs on one capable classifier instead of mesh-routing the classifier itself.
    /// Ignored for the heuristic classifier.
    #[serde(default = "default_classifier_model")]
    pub classifier_model: Option<String>,
    /// Compatibility mode for callers that still concatenate standing context and the active
    /// task into one unstructured prompt. When enabled, classification uses only the final
    /// non-empty paragraph. Prefer role-separated API messages or `forge run --system`.
    #[serde(default)]
    pub classifier_activity_focused: bool,
    /// How a CLI-bridge (`claude-cli::`/`codex-cli::`) turn runs (RFC cli-bridge-full-harness):
    /// `harness` (default) routes the model's tools through Forge's own MCP server + permission
    /// gate; `text` runs the CLI as its own agent with its own tools.
    #[serde(default)]
    pub bridge_mode: BridgeMode,
    /// Lean bridge tool surface: `forge mcp-serve` advertises only the core coding tools to a
    /// bridged CLI, which re-ingests every schema on every request of its own loop. The rest
    /// (device, browser, proxy, subagents, MCP meta-tools, ...) stay reachable through the
    /// `tool_search`/`tool_call` pair. `FORGE_BRIDGE_LEAN=0` also opts out. Default true.
    #[serde(default = "default_true")]
    pub bridge_lean: bool,
    /// Lean direct-path tool surface (docs/features/deferred-tools.md): direct providers get the
    /// core coding tools plus `tool_search`/`tool_call`; platform tools load on demand. The kill
    /// switch: `false` here, or `FORGE_DIRECT_LEAN=0`. Default true.
    #[serde(default = "default_true")]
    pub direct_lean: bool,
    /// Deferred tools to keep advertised on the lean direct surface (e.g. `["web_fetch"]`).
    #[serde(default)]
    pub lean_core_extra: Vec<String>,
    /// Connect external project MCP servers (dual-graph/token-counter/helm/…) inside the CLI
    /// bridge. Default true: servers connect concurrently in the background, each under the
    /// configured MCP connect/discovery timeout, so a slow or auth-gated server is skipped rather
    /// than stalling the bridged turn. Set false or `FORGE_BRIDGE_MCP_EXTERNAL=0` to disable this
    /// external project-MCP surface (and its `mcp_search_tools`/`mcp_call` meta-tools); Forge CORE
    /// tools remain available either way. The environment override wins.
    #[serde(default = "default_true")]
    pub bridge_mcp_external: bool,
    /// The lowest tier a turn may be routed at (`"standard"` or `"complex"`; unset = no floor).
    /// A turn the classifier rates below it is routed as the floor tier instead. For embedders
    /// whose every question needs tools and a capable model: a `trivial` turn is routed to the
    /// cheapest models and gets no tool surface unless its prompt names a workspace action, so an
    /// assistant asked "how many habits do I have" answered "I'll check your habits now" and
    /// stopped, having nothing to call.
    #[serde(default)]
    pub min_tier: Option<forge_types::TaskTier>,
    /// Enforcement behavior once a cap is reached.
    #[serde(default)]
    pub budget: BudgetBehavior,
    /// Per-model pricing overrides (USD per 1k tokens), applied on top of bundled
    /// defaults so a price change needs no release (A-7). Keyed by model id.
    #[serde(default)]
    pub pricing: HashMap<String, PriceOverride>,
    /// Per-model context-window overrides (tokens), keyed by full model id. Wins over every
    /// fetched or inferred window, for endpoints whose model list publishes none (a custom
    /// gateway serving a renamed model would otherwise fall to the 32k conservative floor).
    #[serde(default)]
    pub context_windows: HashMap<String, u32>,
    /// Per-model subscription plan-burn weight overrides (relative to the cheapest sibling of a
    /// family = 1.0), applied on top of the bundled defaults in
    /// `forge-mesh::capability::subscription_burn_weight`. Keyed by the BARE model name (no
    /// `provider::` prefix), e.g. `"gpt-5.5" = 4.0`.
    #[serde(default)]
    pub burn_weights: HashMap<String, f64>,
    /// Subagent orchestration (RFC subagent-orchestration): the `spawn_agents` tool.
    #[serde(default)]
    pub subagents: SubagentsConfig,
    /// Workflow scripts (docs/rfcs/forge-workflow.md): the `run_workflow` tool. Concurrency is
    /// shared with `subagents.max_concurrency`/`max_per_provider` — one real budget for both.
    #[serde(default)]
    pub workflows: WorkflowsConfig,
    /// Auto-discovery routing (docs/features/mesh-routing.md): when true (default), the
    /// mesh discovers the models the user can actually use and ranks the best per tier itself,
    /// rather than relying on the `[mesh.models]` lists. Set false to route strictly from the
    /// configured `[mesh.models]` candidates.
    #[serde(default = "default_auto_discover")]
    pub auto_discover: bool,
    /// Failover (docs/features/mesh-routing.md): when true (default), a model that
    /// errors with a retryable failure (rate-limit / unavailable / auth) is benched and the
    /// turn transparently retries on the next-best healthy model. Set false for single-shot.
    #[serde(default = "default_failover")]
    pub failover: bool,
    /// Escape hatch for strict pin semantics (harness-robustness wave 2): when FALSE (default), an
    /// EXPLICITLY pinned model (`--model` / `/model`) never fails over to a different model — a
    /// rate limit is waited out on the same model (pinned backoff) and a permanent error fails the
    /// turn with the real cause, so a pinned run can't be silently contaminated by another model's
    /// output (the SWE-bench pin-violation defect). Set true to restore the old switch-away
    /// behaviour for pinned turns.
    #[serde(default)]
    pub pin_failover: bool,
    /// Wait budget (seconds) for a pinned model to recover from a TRANSIENT OUTAGE (provider
    /// unavailable / stream dropped, after the hot same-model retries in the turn loop are
    /// exhausted) — separate from the rate-limit backoff budget above, since an outage recovery
    /// takes minutes rather than a signaled `Retry-After`. Same `pinned_backoff_delay` schedule
    /// (5s/15s/45s, then 60s-capped, ±20% jitter), bounded by this total budget rather than a
    /// fixed attempt count. `0` disables outage backoff: a pinned model's outage fails the turn
    /// immediately with the real error, same as a permanent error (pinned-outage-resilience §1).
    #[serde(default = "default_pin_outage_wait_secs")]
    pub pin_outage_wait_secs: u64,
    /// Default bench duration (seconds) when a transient failure (rate-limit / 5xx) gives no
    /// server `Retry-After`. Kept short because free-tier rate limits typically reset per MINUTE
    /// (NVIDIA NIM, Groq, Gemini RPM) — a long bench would strand the best free models and push
    /// routing onto worse or paid ones for minutes. Providers that DO send `Retry-After` use that
    /// exact value instead.
    #[serde(default = "default_failover_cooldown_secs")]
    pub failover_cooldown_secs: u64,
    /// Abort a model stream that goes silent for this many seconds (a half-open/stalled
    /// connection) and fail over, instead of hanging the turn forever. `0` disables the watchdog.
    #[serde(default = "default_stream_idle_timeout_secs")]
    pub stream_idle_timeout_secs: u64,
    /// Abort a model call that yields no stream event at all (text, reasoning delta or keepalive)
    /// for this many seconds and fail over, instead of waiting the full `stream_idle_timeout_secs`.
    /// Only governs the wait for the *first* event; an established stream keeps the idle window.
    /// `0` (or a value at or above the idle window) disables the separate deadline.
    #[serde(default = "default_first_event_timeout_secs")]
    pub first_event_timeout_secs: u64,
    /// Absolute token ceiling at which a session auto-compacts, regardless of the model's real
    /// context window. Fires at `min(compact_cap_tokens, 0.80 * window)`, so large-window models
    /// (gpt-5.6 ~1.05M) compact early to cut subscription usage — later compaction burns more.
    /// Does NOT change the displayed window.
    /// Default 217_600 = 80% of 272K: OpenAI rolled gpt-5.6 (Sol) back 372K->272K on 2026-07-13 as
    /// a main source of high usage burn. OpenRouter reports the API context window, not the
    /// subscription-side effective window, so it cannot justify raising this ceiling. Revisit
    /// only after an observed subscription change or an OpenAI statement confirms the effective
    /// window has been restored; keeping the cap conservative avoids unexpected usage burn.
    #[serde(default = "default_compact_cap_tokens")]
    pub compact_cap_tokens: u64,
    /// The same absolute ceiling for a model that costs nothing. Free models skip
    /// [`Self::compact_cap_tokens`] — that one is a spend control and there is no spend to
    /// control — but they still need A ceiling: exempting them from the spend cap entirely left
    /// the model's own window as the only limit, which on a 1M-window free model meant compaction
    /// did not fire until 838,860 tokens.
    ///
    /// Free buys tokens, not attention. A turn carrying 800k of mostly-stale tool output is not
    /// better than one carrying a compacted 400k, and every later turn in that session re-sends
    /// the whole thing. Default 400_000: well above [`default_compact_cap_tokens`], so it never
    /// second-guesses the spend cap for a paid model, while still halving the runaway that was
    /// observed in practice. A backstop chosen from measured behaviour, not a benchmarked optimum.
    #[serde(default = "default_free_model_cap_tokens")]
    pub free_model_cap_tokens: u64,
    /// Longest rate-limit reset (seconds) Forge will WAIT OUT in-turn to retry the best model rather
    /// than degrade to a lower-ranked one (per-minute free tiers: NIM/Groq/Gemini). A reset longer
    /// than this falls through to failover. `0` disables in-turn waiting entirely.
    #[serde(default = "default_rate_limit_wait_secs")]
    pub rate_limit_wait_secs: u64,
    /// Explicit cap on model↔tool rounds in one turn. `0` (the default) means none: like Claude
    /// Code and Codex, the loop runs until the model stops calling tools, the person interrupts, or
    /// the progress supervisor (`progress_guard`) sees a turn that is not getting anywhere. Set a
    /// number only when you want a hard count: reaching it pauses an attended turn with a warning
    /// (type `continue` to resume) and fails an unattended one. `--max-turns N` sets it per run.
    /// The count is per turn, across every re-drive the turn makes.
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
    /// Explicit cap on rounds for an UNATTENDED turn (headless `forge run`, a detached session, or
    /// `--mode bypass`), where nobody can type `continue`. `0` (the default) means none; the
    /// unattended safety net is progress-based (`idle_stop_steps`) and the spend ceilings. When
    /// set it wins over `max_steps` on unattended turns; the turn ends with an error naming the
    /// uncommitted work. Interactive sessions never read it.
    #[serde(default = "default_max_steps_unattended")]
    pub max_steps_unattended: usize,
    /// What the progress supervisor does to a turn that stopped making progress: `enforce` (the
    /// default) nudges once and then ends the turn cleanly, `warn` only reports where it would
    /// have stopped, `off` disables stops (loop nudges stay).
    #[serde(default)]
    pub progress_guard: ProgressGuardMode,
    /// Steps without a file change, a closed task, a passing check or a steer after which the
    /// supervisor asks the model to report what it found or make the change. Never stops a turn.
    /// `0` disables the nudge. Default 60: the longest healthy stretch measured was 37.
    #[serde(default = "default_idle_nudge_steps")]
    pub idle_nudge_steps: usize,
    /// Steps without any of those after which a HEADLESS turn (nobody watching: `forge run`, a
    /// detached session, JSON output) is stopped. A person at a terminal is never stopped by this,
    /// even in `--mode bypass`: they can see the turn and press escape. A headless turn that does
    /// nothing but wait on a job that never finishes is stopped on the same switch, after 96 wait
    /// steps. `0` disables both. Default 200.
    #[serde(default = "default_idle_stop_steps")]
    pub idle_stop_steps: usize,
    /// Cumulative input tokens a single turn may consume before it is ended with an error, on any
    /// surface. The step cap alone does not bound cost (one runaway turn burned 7.0M input tokens
    /// inside 100 steps). `0` disables the ceiling.
    #[serde(default = "default_max_turn_input_tokens")]
    pub max_turn_input_tokens: u64,
    /// Proactively spread complex/standard tasks off the subscription bridges (claude-cli/
    /// codex-cli) onto the free-frontier pool, scaling with how full the weekly/session window is
    /// and how much headroom the plan has (subscription-conservation routing). When true (default)
    /// a fraction of complex/standard tasks route to a free frontier model even while the
    /// subscription is fresh, so a complex-heavy workload doesn't exhaust the plan. Set false for
    /// the old greedy behaviour (always the subscription flagship until the hard limit).
    #[serde(default = "default_subscription_conserve")]
    pub subscription_conserve: bool,
    /// Rank models on REAL measured performance (Artificial Analysis benchmark indices, ADR-0011)
    /// instead of the family-name heuristic, when benchmark data is available. Default true; a
    /// no-op without a cached dataset / API key (falls back to the heuristic).
    #[serde(default = "default_benchmark_ranking")]
    pub benchmark_ranking: bool,
    /// Fetch the published benchmark mirror (forge.adulari.dev, then GitHub) when no
    /// Artificial Analysis key is set. Default true. Set false to keep Forge off the network for
    /// benchmarks: ranking then uses the snapshot bundled with the binary (or your own key). The
    /// `FORGE_BENCHMARK_MIRROR=0` environment variable does the same without editing config.
    #[serde(default = "default_benchmark_mirror")]
    pub benchmark_mirror: bool,
    /// "Max-resolve" mode: run a bounded, evidence-grounded completeness pass before a code-change
    /// turn finishes. CLI bridges re-check every stated requirement. Direct providers run the
    /// additional final-diff and related-production-sibling search only for prompts that explicitly
    /// describe an identifier migration (for example a deprecation, rename, or compatibility
    /// alias). Direct prompts that explicitly name a `Class.method` API receive lightweight
    /// pre-solve scope guidance rather than another review loop. Default true; disable to skip all
    /// extra completeness guidance and passes when latency matters more than maximum resolve
    /// quality.
    #[serde(default = "default_verify_completeness")]
    pub verify_completeness: bool,
    /// Empty-diff completion nudge (harness-robustness wave 2): when a headless code-change run
    /// (`bench swe` marks the session as expecting a code change) ends with tools having run but
    /// NO file edits and a clean git tree, push back ONCE — "implement it, don't describe it" —
    /// before accepting the turn. Default true; only sessions explicitly marked as code-change
    /// runs are affected, so interactive use never sees it. Set false to disable even for bench.
    #[serde(default = "default_nudge_empty_diff")]
    pub nudge_empty_diff: bool,
    /// Existing-tests guard (quality guards wave 4): when a code-change run (`bench swe`, a
    /// worktree build session), an unattended "keep the tests green" run, or a request that says
    /// to leave the existing tests alone is about to complete, edits to tracked test files are
    /// sorted. Edits that hide a failure (a deleted test file, removed assertions, an added
    /// skip/xfail marker, an assertion that cannot fail, changed expectations with no code change
    /// behind them) are put back and the model is pushed back ONCE; changed lines of an existing
    /// test with a code change behind them stay and the model is asked once to justify them. New
    /// test files and appended tests are fine. A request that says to leave the tests alone
    /// restores any change. Default true.
    #[serde(default = "default_guard_test_edits")]
    pub guard_test_edits: bool,
    /// Timeout reconciliation window (quality guards wave 4): when the caller has set a turn
    /// deadline (`bench swe` reserves ~120s before its hard tokio timeout), a turn past the
    /// deadline stops launching new model calls and gets ONE final instruction — revert
    /// unverified speculative changes, keep the minimal verified fix, stop. Without it the hard
    /// kill ships whatever mid-refactor state the tree happens to be in (the xarray-3364 timeout:
    /// "submit partial work" shipped the riskiest state). Default true; inert unless a deadline
    /// is set, so interactive use never sees it.
    #[serde(default = "default_deadline_reconcile")]
    pub deadline_reconcile: bool,
    /// Env-fight spend cap (quality guards wave 4): after an environment-provisioning command
    /// (pip install / venv / virtualenv / ensurepip / apt / uv …) fails, allow one alternate
    /// recovery attempt, inject ONE nudge, then block further provisioning/native builds that turn.
    /// SWE-bench turns burned minutes fighting host-python/repo-era mismatches (venv archaeology)
    /// that provisioning was never going to win. Default true; latched once per turn.
    #[serde(default = "default_env_fight_nudge")]
    pub env_fight_nudge: bool,
    /// Bridge MCP-tool health guard (wave 7): on a headless code-change run (`bench swe` marks the
    /// session), require that the bridged CLI actually had Forge's write tools. Forge serves them
    /// via `forge mcp-serve`, which intermittently FAILS TO START under the sandbox (codex logs
    /// `resources/list failed: MCP startup failed`) — the turn then completes with an empty patch
    /// and NO error, so a benchmark silently scores a toolless run as a clean completion (observed
    /// on ~7/15 instances of a codex-cli::gpt-5.5 SWE-bench sweep). When true, such a turn is
    /// classified TOOLS-UNAVAILABLE and the harness retries it ONCE on a fresh process, then records
    /// an ERROR rather than a fair empty attempt. Default true; only `expect_code_change` runs are
    /// affected, so interactive use never sees it. Set false to disable even for bench.
    #[serde(default = "default_bridge_require_tools")]
    pub bridge_require_tools: bool,
    /// Per-result token budget for older tool output in model requests. The newest tool results
    /// stay verbatim; full output remains persisted for replay. `0` disables request elision.
    #[serde(default = "default_tool_result_context_token_budget")]
    pub tool_result_context_token_budget: usize,
    /// Number of newest tool results kept verbatim before older oversized results are elided.
    #[serde(default = "default_tool_result_context_keep_recent")]
    pub tool_result_context_keep_recent: usize,
    /// Smallest stretch of verbatim context (tokens) the elision floor releases at once. Old tool
    /// output is only shortened when the floor advances, and every advance rewrites the prompt
    /// from that point on — so it moves in blocks of at least this many tokens (or 25% of the
    /// compaction trigger, whichever is larger) instead of on every turn.
    #[serde(default = "default_context_floor_block_tokens")]
    pub context_floor_block_tokens: usize,
    /// Fraction of the compaction trigger an auto-compaction must get the context down to. The
    /// prune or summary that fires at the trigger goes this far in one step so the following
    /// requests append to a stable prefix instead of re-trimming on every call.
    #[serde(default = "default_compact_low_water")]
    pub compact_low_water: f64,
    /// Which model writes the compaction summary: `session` (default; the model the session is
    /// running on, then its fallbacks, then the free shortlist) or `auto` (the best free capable
    /// model first and the session's own last, the order before this setting existed).
    #[serde(default)]
    pub compact_model: CompactModel,
    /// Per-turn cumulative input-token ceiling for a CLI bridge turn (wave 5). A bridge runs its
    /// own tool loop inside a subprocess, so the direct-path cost guards (which only see
    /// `resp.tool_calls`) never fire on it — an unbounded bridge turn is the dominant tail-cost
    /// risk because bridge input scales ~quadratically with round-trips (each re-drive re-sends the
    /// transcript). When the accumulated input across a turn's bridge completions crosses this cap,
    /// Forge stops re-driving and ends the turn cleanly, submitting whatever verified diff exists.
    /// A backstop, not a target: 0 disables it. Default 2_500_000.
    #[serde(default = "default_bridge_turn_token_cap")]
    pub bridge_turn_token_cap: u64,
    /// Which subscription plan backs each CLI bridge (`claude-cli` → "max-20x", `codex-cli` →
    /// "plus"), captured by `forge init`. Records the usage headroom the user has: the
    /// subscription-conservation layer reads it so a larger plan (more headroom) is spent more
    /// freely than a smaller one. Also shown by `forge init`/`forge models`.
    #[serde(default)]
    pub subscriptions: HashMap<String, String>,
    /// Override which models a CLI bridge exposes to auto-discovery, keyed by bridge prefix
    /// (`claude-cli` / `codex-cli`); each value is a list of model aliases/ids the CLI's `--model`
    /// flag accepts (e.g. `["opus","sonnet","haiku"]`). Empty/absent → the bridge's built-in
    /// defaults. The CLIs expose no machine-readable model list, so this is how a user pins the
    /// exact set; a stale alias just benches itself via failover.
    #[serde(default)]
    pub bridge_models: HashMap<String, Vec<String>>,
    /// Models/providers excluded from discovery + routing. Each entry is either a full model id
    /// (`provider::model`) or a bare provider prefix (`provider`, matching every `provider::*`).
    /// Use it to drop a flaky or unwanted model without deleting its key (known-issues.md). A
    /// disabled model never enters the catalog, so the mesh won't route to or fail over onto it.
    #[serde(default)]
    pub disabled: Vec<String>,
    /// Model ids a picker pins above the rest. Pure presentation: routing ignores it.
    #[serde(default)]
    pub favorites: Vec<String>,
    /// Cap on the output tokens requested per completion. Providers otherwise default to a
    /// model's full max (often 65k), which a free / low-credit account can't afford — OpenRouter
    /// then returns HTTP 402 ("requested 65536 tokens, can only afford 669") and the model looks
    /// "down". Capping keeps free-tier models usable and bounds runaway generations. `0` = no cap
    /// (use the provider default).
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
    /// How aggressively to conserve metered API credits. This changes routing policy only; it
    /// never truncates a model response. Strict restricts auto-routing + failover to free +
    /// subscription models — a paid, metered model (including a priced "free-tier" model that
    /// could bill once its quota runs out) is dropped from the candidate set, so neither the
    /// primary pick nor the failover chain can spend API credit without an explicit `--model` pin.
    /// (Enforced in `HeuristicRouter::allowed_under_credit_mode`.)
    #[serde(default)]
    pub credit_mode: CreditMode,
    /// Auto-memory: capture durable facts (preferences/decisions/conventions) at the end of a turn
    /// and recall the most relevant ones into context at the start of a session, scoped per project.
    /// On by default — the built-in cross-session memory. Set false to disable capture + recall (the
    /// `forge memory` command still works for manual entries).
    #[serde(default = "default_auto_memory")]
    pub auto_memory: bool,
    /// Also capture memories after a one-shot `forge run` turn. Off by default: the capture is a
    /// model call the process waits on before it exits (run after the result is printed).
    #[serde(default)]
    pub auto_memory_one_shot: bool,
    /// Route direct-API model calls through a local Headroom proxy (`headroom proxy`, the
    /// context-optimization layer) so its compression/caching applies to Forge's traffic
    /// (docs/features/token-savings.md). `off` (default — measured on Forge's own traffic the
    /// proxy removed 0.2% of input tokens for ~0.4 s added latency per request, because Forge
    /// already bounds and prunes tool output itself); `auto`: on when a healthy proxy answers at
    /// `headroom_url`; `on` forces it. Covers OpenAI-wire, Anthropic and Gemini adapters plus the
    /// `claude` CLI bridge; Responses-API adapters (OpenCode Go/Zen) go direct.
    ///
    /// TOML key: `mesh.headroom`
    #[serde(default = "default_headroom_toggle")]
    pub headroom: AutoToggle,
    /// Base URL of the Headroom proxy. Default `http://127.0.0.1:8787`.
    ///
    /// TOML key: `mesh.headroom_url`
    #[serde(default)]
    pub headroom_url: Option<String>,
    /// Auto-orchestrate: inject the Forge orchestration framework as a standing system instruction
    /// at the start of every session. The model is guided to check skills first, choose the
    /// highest-level tool that fits, and use subagents/MCP/web/Lattice appropriately — without
    /// requiring the user to type `/orchestrate` on each prompt. Off by default (opt-in).
    #[serde(default)]
    pub auto_orchestrate: bool,
    /// Self-review pass (opt-in): after a turn makes edits, the SAME model re-examines its own
    /// changes against the task and may fix them — one bounded round, only on edit turns. OFF by
    /// default: a same-model SWE-bench A/B showed the always-on version REGRESSED (the extra round
    /// over-revised correct fixes). Kept as a lever to refine, not a default-on win.
    #[serde(default = "default_self_review")]
    pub self_review: bool,
    /// Architect mode (dual-model pipeline): when true, each turn runs a plan phase on a strong
    /// model then an apply phase on a cheaper one. Off = single-model turns (default).
    #[serde(default)]
    pub architect_mode: bool,
    /// Planner model id for architect mode. Empty → mesh-route at the Complex tier.
    #[serde(default)]
    pub architect_model: Option<String>,
    /// Editor (apply) model id for architect mode. Empty → mesh-route at the Standard tier.
    #[serde(default)]
    pub editor_model: Option<String>,
    /// Default reasoning effort sent to API providers (`low`|`medium`|`high`|`xhigh`). Absent →
    /// no effort param (provider default). Overridable per session with `/effort`.
    #[serde(default)]
    pub default_effort: Option<String>,
    /// Give a session's provider cache key a stable hash of (model, tools, system prompt,
    /// workspace) instead of the session id, so a new session can reuse the fixed prefix an
    /// earlier one warmed. Frozen at the session's first request (mesh-routing.md, prompt caching).
    #[serde(default = "default_true")]
    pub cross_session_cache_key: bool,
    /// Advisory steering level (docs/features/harness-profile.md). `guarded` (default) keeps every
    /// nudge; `minimal` drops the advisory ones (progress asks, stall/stagnation nudges, commit
    /// reminders, auto-orchestrate, mid-intent and continue nudges) while the safety guards stay.
    #[serde(default)]
    pub harness_profile: HarnessProfile,
    /// Per-model overrides of `harness_profile`, keyed by a model-id glob (`*` = any run of
    /// characters): `[mesh.harness_profiles] "bedrock::*" = "minimal"`. The most specific match
    /// wins.
    #[serde(default)]
    pub harness_profiles: std::collections::BTreeMap<String, HarnessProfile>,
    /// Low reasoning effort for the step after a successful read-only tool round, restored after
    /// a failure, a write or a user turn (docs/features/adaptive-effort.md).
    #[serde(default = "default_true")]
    pub adaptive_effort: bool,
    /// Providers `adaptive_effort` applies to: by default only those where an effort change
    /// leaves the prompt cache intact ([`CACHE_SAFE_EFFORT_PROVIDERS`]).
    #[serde(default = "adaptive_effort::default_adaptive_effort_providers")]
    pub adaptive_effort_providers: Vec<String>,
    /// What to do when the selected model cannot read the images a request carries: `warn`
    /// (default; send text only and say so), `describe` (a vision side call describes the image
    /// once) or `switch` (route to a vision-capable model even when pinned).
    #[serde(default)]
    pub vision_fallback: VisionFallback,
}

impl MeshConfig {
    /// The harness profile for `model`, after per-model glob overrides.
    pub fn harness_profile_for(&self, model: &str) -> HarnessProfile {
        resolve_harness_profile(self.harness_profile, &self.harness_profiles, model)
    }

    /// Whether adaptive effort may lower `model`'s effort (switch on and provider listed).
    pub fn adaptive_effort_for(&self, model: &str) -> bool {
        self.adaptive_effort
            && self
                .adaptive_effort_providers
                .iter()
                .any(|p| p == provider_of(model))
    }

    /// Effective per-completion output token cap. Credit conservation affects routing only; a
    /// response limit exists solely when the user explicitly configures this field above zero.
    pub fn effective_max_output_tokens(&self) -> u32 {
        self.max_output_tokens
    }
}

/// Whether `model_id` is excluded by a `[mesh] disabled` list — exact id match or a bare provider
/// prefix matching `provider::*`. Pure so it's unit-testable.
pub fn is_model_disabled(model_id: &str, disabled: &[String]) -> bool {
    disabled.iter().any(|d| {
        let d = d.trim();
        !d.is_empty() && (model_id == d || model_id.starts_with(&format!("{d}::")))
    })
}

fn default_auto_discover() -> bool {
    true
}

/// Do not truncate model output unless the user explicitly configures a positive cap. Providers
/// already enforce each model's native output limit; silently imposing a smaller Forge default can
/// cut a structured tool call in the middle of its JSON payload and turn a valid large file write
/// into a corrupt partial write.
fn default_max_output_tokens() -> u32 {
    0
}

/// No step cap by default (`0`). Before the progress supervisor this was 100 (a pause on attended turns, a warning on
/// unattended ones), which interrupted productive turns and bounded nothing about spend: the loop
/// ends when the model stops, the person interrupts, or the progress supervisor sees a stall.
fn default_max_steps() -> usize {
    0
}

/// No unattended step ceiling by default (`0`; it was 400). The unattended safety net is the
/// progress supervisor's idle stop plus the spend ceilings.
fn default_max_steps_unattended() -> usize {
    0
}

fn default_idle_nudge_steps() -> usize {
    60
}

fn default_idle_stop_steps() -> usize {
    200
}

/// Default per-turn cumulative input-token ceiling. Configurable via `mesh.max_turn_input_tokens`.
fn default_max_turn_input_tokens() -> u64 {
    10_000_000
}

fn default_subscription_conserve() -> bool {
    true
}

fn default_benchmark_ranking() -> bool {
    true
}

fn default_benchmark_mirror() -> bool {
    true
}

fn default_verify_completeness() -> bool {
    true
}

fn default_nudge_empty_diff() -> bool {
    true
}

fn default_guard_test_edits() -> bool {
    true
}

fn default_deadline_reconcile() -> bool {
    true
}

fn default_env_fight_nudge() -> bool {
    true
}

fn default_bridge_require_tools() -> bool {
    true
}

fn default_tool_result_context_token_budget() -> usize {
    4_096
}

fn default_tool_result_context_keep_recent() -> usize {
    2
}

fn default_context_floor_block_tokens() -> usize {
    32_768
}

fn default_compact_low_water() -> f64 {
    0.62
}

fn default_bridge_turn_token_cap() -> u64 {
    2_500_000
}

/// The Artificial Analysis Data API key (ADR-0011), for benchmark-driven ranking. Read from
/// `ARTIFICIALANALYSIS_API_KEY` first, then the `artificialanalysis` keyring/file entry. `None`
/// skips the direct fetch (ranking uses the Forge mirror, then the bundled snapshot, then the
/// heuristic).
pub fn benchmark_api_key() -> Option<String> {
    if let Ok(k) = std::env::var("ARTIFICIALANALYSIS_API_KEY") {
        if !k.is_empty() {
            return Some(k);
        }
    }
    secret_store::get("artificialanalysis").filter(|k| !k.is_empty())
}

/// Secret-store slot holding the long-lived token from `claude setup-token`, for the claude-cli
/// bridge. Set with `forge auth claude_oauth_token`.
pub const CLAUDE_OAUTH_TOKEN_SLOT: &str = "claude_oauth_token";

/// The long-lived Claude Code OAuth token the claude bridge authenticates with:
/// `CLAUDE_CODE_OAUTH_TOKEN` from the environment first, then the [`CLAUDE_OAUTH_TOKEN_SLOT`]
/// secret. Unlike the interactive login in `~/.claude/.credentials.json` it never refreshes, so
/// the bridge can use it without rotating anything the user's own `claude` depends on.
pub fn claude_oauth_token() -> Option<String> {
    let env = std::env::var("CLAUDE_CODE_OAUTH_TOKEN").ok();
    env.map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .or_else(|| {
            secret_store::get(CLAUDE_OAUTH_TOKEN_SLOT)
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        })
}

fn default_failover() -> bool {
    true
}

fn default_self_review() -> bool {
    // OFF by default: on a same-model SWE-bench A/B the always-on review REGRESSED results
    // (4/6 → 3/6) — the extra round let the model second-guess and break a fix that was already
    // correct. Kept as an opt-in lever; needs a more conservative trigger before it's a net win.
    false
}

fn default_failover_cooldown_secs() -> u64 {
    60
}

fn default_headroom_toggle() -> AutoToggle {
    AutoToggle::Off
}

fn default_auto_memory() -> bool {
    true
}

fn default_rate_limit_wait_secs() -> u64 {
    // Covers the common per-minute free-tier reset (NIM/Groq/Gemini ~60s) plus slack; a longer
    // (hourly/daily) quota exceeds this and falls through to failover instead of blocking the turn.
    75
}

fn default_pin_outage_wait_secs() -> u64 {
    // ~10 minutes: generous enough to ride out a real provider blip/redeploy without silently
    // switching a pinned model, short enough that a genuinely dead provider still fails the turn
    // rather than blocking indefinitely.
    600
}

fn default_first_event_timeout_secs() -> u64 {
    // Above the ~90s a reasoning model on a large-context session can stay silent before its
    // first token (see `default_stream_idle_timeout_secs`), well below the 180s idle window.
    120
}

fn default_stream_idle_timeout_secs() -> u64 {
    // Long enough to never trip during normal generation, short enough to recover from a genuine
    // stall in reasonable time. Raised from 120s: a reasoning model on a large-context session
    // (~100k input tokens) can go silent for ~90s during a single reasoning burst before it emits,
    // so 120 left too little margin and working streams were being killed and re-driven. A tool in
    // flight still doubles this (see `stream_with_idle_timeout`).
    180
}

fn default_compact_cap_tokens() -> u64 {
    217_600
}

fn default_free_model_cap_tokens() -> u64 {
    400_000
}

/// Subagent orchestration settings (RFC subagent-orchestration).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentsConfig {
    /// Advertise the `spawn_agents` tool to the top-level model. Default true.
    #[serde(default = "default_subagents_enabled")]
    pub enabled: bool,
    /// Max child agents per `spawn_agents` call (hard cap).
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,
    /// Max child agents running concurrently (parallel fan-out is Phase 2).
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,
    /// How many additional spawn generations children may create. `0` lets the top-level turn
    /// spawn children but prevents those children from recursively delegating. Higher values are
    /// opt-in because fan-out grows exponentially; the per-call `max_agents`/`max_concurrency`
    /// caps still apply at every level (RFC subagent-orchestration Phase 3c).
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    /// Directory holding named agent-type files (`<name>.md`), relative to the cwd.
    #[serde(default = "default_agents_dir")]
    pub agents_dir: String,
    /// Give each write-capable child its own git worktree so concurrent edits can't corrupt the
    /// shared working tree; changes are merged back after the child finishes. Read-only children
    /// always skip this. Off by default (requires the repo to be a git work tree).
    #[serde(default)]
    pub worktree_isolation: bool,
    /// Max child agents that may run concurrently *on the same provider*. A burst of subagents all
    /// routed to one subscription (claude/codex bridge) or one metered key would otherwise hammer a
    /// single quota in parallel — the global `max_concurrency` doesn't see provider. This sub-cap
    /// throttles per provider so fan-out spreads the load (and protects the subscription thesis).
    /// `0` disables the per-provider cap (global cap only).
    #[serde(default = "default_max_per_provider")]
    pub max_per_provider: usize,
    /// Whether a pin active on the session (`--model` or in-session `/model`) is inherited by
    /// every subagent, strictly (same model, `pinned: true`, no cross-model fallback chain) —
    /// matching the strict-pin the parent gets. Default true: the user asked for this exact model,
    /// so a delegated child must run the same model, and an agent-type tier default must not beat
    /// it. When false, children route through the mesh independently (their normal failover chain
    /// intact). A hard per-child pin (`/duel`'s `ResolvedAgent.pinned_model`) always wins regardless.
    /// An absent key means `true`.
    #[serde(default = "default_inherit_pin")]
    pub inherit_pin: bool,
}

fn default_subagents_enabled() -> bool {
    true
}
fn default_max_agents() -> usize {
    8
}
fn default_max_concurrency() -> usize {
    4
}
fn default_max_depth() -> usize {
    0
}
fn default_max_per_provider() -> usize {
    2
}
fn default_inherit_pin() -> bool {
    true
}
fn default_agents_dir() -> String {
    ".forge/agents".to_string()
}

impl Default for SubagentsConfig {
    fn default() -> Self {
        Self {
            enabled: default_subagents_enabled(),
            max_agents: default_max_agents(),
            max_concurrency: default_max_concurrency(),
            max_depth: default_max_depth(),
            agents_dir: default_agents_dir(),
            worktree_isolation: false,
            max_per_provider: default_max_per_provider(),
            inherit_pin: default_inherit_pin(),
        }
    }
}

/// Workflow-script settings (docs/rfcs/forge-workflow.md).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowsConfig {
    /// Hard safety backstop on total `agent()` calls a single `run_workflow` script (including
    /// any nested `workflow()` calls) may make — bounds a runaway or pathological loop, mirroring
    /// the reference Workflow tool's own total-agent cap.
    #[serde(default = "default_max_total_agents")]
    pub max_total_agents: usize,
}

fn default_max_total_agents() -> usize {
    200
}

impl Default for WorkflowsConfig {
    fn default() -> Self {
        Self {
            max_total_agents: default_max_total_agents(),
        }
    }
}

fn default_warn_threshold() -> f64 {
    0.8
}

fn default_prefer_subscription() -> bool {
    true
}

fn default_classifier_model() -> Option<String> {
    // NOT `groq/compound-mini`: that is Groq's agentic *compound* system with server-side web
    // search and code execution, not a plain chat model. Two disqualifiers for a classifier whose
    // whole contract is determinism — its answer can vary with live web results for identical
    // input, and the classifier prompt embeds the user's task text, which an agentic search
    // pipeline may forward to third-party infrastructure. `openai/gpt-oss-20b` is the plain, fast
    // model actually observed answering a classify call ("classified by groq::openai/gpt-oss-20b
    // as trivial"); compound-mini stays in the chain as a backup but must not lead it.
    Some("groq::openai/gpt-oss-20b".to_string())
}

/// How a CLI-bridge turn runs (RFC cli-bridge-full-harness).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BridgeMode {
    /// Forge serves its own tools to the CLI via `forge mcp-serve` (MCP) and gates them with
    /// the permission engine; the CLI's built-in tools are disabled. The full Forge harness.
    #[default]
    Harness,
    /// The CLI runs as its own agent with its own tools (no Forge tools/permission gate).
    Text,
}

/// How the mesh decides a task's tier (ADR-0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ClassifierKind {
    /// Ask a cheap model to label the tier on every turn, falling back to the heuristic on
    /// any error. One extra round-trip per turn regardless of how obvious the task is.
    #[default]
    Llm,
    /// Deterministic weighted-signal heuristic — zero added cost/latency.
    Heuristic,
    /// Legacy alias for `llm`, retained so existing configurations keep working. Every unhinted
    /// turn is classified by the LLM; the heuristic is only the final availability fallback.
    Hybrid,
}

/// What Forge does once a budget cap is reached (FR-5).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BudgetBehavior {
    /// Refuse model calls once a cap is exceeded (overridable per-turn via
    /// `FORGE_BUDGET_OVERRIDE=1`). `true` stops everywhere, `false` only warns on the USD caps, and
    /// `"auto"` (the default) stops an unattended turn and only warns an attended one: the person
    /// at an attended turn sees the spend and can interrupt. Session, daily, weekly and monthly
    /// caps are enforced either way.
    pub hard_stop: HardStop,
    /// A cap downshifts/stops even an explicitly pinned model. Default true. (Model pinning
    /// is not yet a feature; this is forward-compatible config.)
    pub cap_overrides_pin: bool,
    /// One user turn's spend (USD, every model call in it) at which a warning is shown, once.
    /// `0` disables. The day/week/month caps never see a single runaway turn coming: the measured
    /// outliers were 19 turns over $5 that made up 68% of all recorded spend.
    #[serde(default = "default_turn_warn_usd")]
    pub turn_warn_usd: f64,
    /// One user turn's spend (USD) at which the turn is ended. The next prompt (or `continue`) gets
    /// a fresh allowance, which is the confirmation. `0` disables; `hard_stop = false` downgrades
    /// it to a warning; `FORGE_BUDGET_OVERRIDE=1` bypasses it.
    #[serde(default = "default_turn_cap_usd")]
    pub turn_cap_usd: f64,
    /// A session's lifetime spend (USD, including side calls) at which a warning is shown, once per
    /// process. `0` disables.
    #[serde(default = "default_session_warn_usd")]
    pub session_warn_usd: f64,
    /// A session's lifetime spend (USD) at which further turns are refused until the cap is raised
    /// or `FORGE_BUDGET_OVERRIDE=1` is set. Resuming the session does not reset it. `0` disables.
    #[serde(default = "default_session_cap_usd")]
    pub session_cap_usd: f64,
}

fn default_turn_warn_usd() -> f64 {
    5.0
}

fn default_turn_cap_usd() -> f64 {
    25.0
}

fn default_session_warn_usd() -> f64 {
    25.0
}

fn default_session_cap_usd() -> f64 {
    100.0
}

impl Default for BudgetBehavior {
    fn default() -> Self {
        Self {
            hard_stop: HardStop::Auto,
            cap_overrides_pin: true,
            turn_warn_usd: default_turn_warn_usd(),
            turn_cap_usd: default_turn_cap_usd(),
            session_warn_usd: default_session_warn_usd(),
            session_cap_usd: default_session_cap_usd(),
        }
    }
}

/// A user-supplied price for one model (USD per 1,000 tokens).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PriceOverride {
    pub input_per_1k: f64,
    pub output_per_1k: f64,
    /// The rate for cached prompt tokens. Absent = cached tokens are billed at `input_per_1k`,
    /// which overstates a long session on a provider that caches automatically (Bedrock's Kimi K3
    /// reads cache at a tenth of its input price).
    #[serde(default)]
    pub cache_read_per_1k: Option<f64>,
}

/// A widget shown in the statusline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatuslineWidget {
    /// Spinner + working indicator + model name + tier bracket (compound). Shows spinner while busy.
    Model,
    /// Session cost "◈ $X.XXXX"
    SessionCost,
    /// Reasoning-effort pin label (only renders when effort is pinned)
    Effort,
    /// Operating mode/temper "◆ {label}" (only renders when temper is set)
    Mode,
    /// Per-turn elapsed timer "⧖ Xs"
    TurnElapsed,
    /// Per-turn input tokens "↑N"
    TokensIn,
    /// Per-turn output tokens "↓N"
    TokensOut,
    /// Session total tokens "Σ ↑N ↓N"
    SessionTokens,
    /// Live output throughput with a trend sparkline, "⚡ 64 tok/s ▂▄▇█". Measured over a short
    /// sliding window while the model generates, then the finished turn's average (dimmed).
    /// Absent until there is enough signal to state a rate.
    Throughput,
    /// Current git branch "⎇ branch"
    GitBranch,
    /// Claude.ai usage % "claude N%"
    QuotaClaude,
    /// Codex usage % "codex N%" (only when data is available)
    QuotaCodex,
    /// OpenCode Go usage % for its MOST CONSTRAINED window, e.g. "go 5h 42%". The provider reports
    /// three simultaneous windows (5-hour / weekly / monthly) and the statusline has room for one,
    /// so the window is named alongside the percentage rather than left ambiguous.
    QuotaOpencodeGo,
    /// Consumption-pace meter (mesh-routing.md) for whichever subscription window is
    /// projected closest to exhaustion, e.g. "⏱ claude 5h → 120%" — only renders once enough
    /// usage history has built up to derive a rate (silently absent otherwise, never a guess).
    QuotaPace,
    /// MCP server count "⌬ N mcp" (only when servers are connected)
    McpStatus,
    /// Account-sync health "sync ✓" / "sync 2↑" / "sync !" (hidden while Anywhere account sync is off)
    AnywhereSync,
    /// Tier bracket only "[tier]"
    Tier,
    /// Current project/repo directory name "⚑ repo"
    RepoName,
    /// Static text, or — when `shell` is set — the cached stdout of a user-supplied shell
    /// command, refreshed every `refresh_secs` (Starship-style custom module). `text` is the
    /// fallback shown before the first refresh completes (or if `shell` is unset).
    Custom {
        text: String,
        #[serde(default)]
        shell: Option<String>,
        #[serde(default = "default_custom_refresh_secs")]
        refresh_secs: u64,
    },
}

fn default_custom_refresh_secs() -> u64 {
    5
}

/// Customizable statusline layout (the left segment and extra rows render today).
/// Default matches the current built-in layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatuslineConfig {
    /// Left segment widgets (rendered left-aligned).
    #[serde(default = "default_sl_left")]
    pub left: Vec<StatuslineWidget>,
    /// Reserved: center widgets are parsed but not drawn yet (use `left` or `extra_rows`).
    #[serde(default)]
    pub center: Vec<StatuslineWidget>,
    /// Reserved: right widgets are parsed but not drawn yet (use `left` or `extra_rows`).
    #[serde(default)]
    pub right: Vec<StatuslineWidget>,
    /// Separator between widgets within a segment.
    #[serde(default = "default_sl_separator")]
    pub separator: String,
    /// Additional left-aligned rows rendered below the built-in two (widgets + turn/gauge/session
    /// totals). Each inner `Vec` is one row. Empty by default; grow this to fit more context —
    /// the statusline's reserved height grows with it (see `statusline_height`).
    #[serde(default)]
    pub extra_rows: Vec<Vec<StatuslineWidget>>,
    /// A Claude-Code-style statusline command. It gets session JSON on stdin (the same shape CC
    /// sends: `session_id`, `model`, `workspace`, `cost`, `context_window`, ...) and its first
    /// stdout line is shown as one extra row, so an existing `~/.claude/statusline.sh` works
    /// unchanged. `forge import claude` fills this from CC's `statusLine.command`.
    #[serde(default)]
    pub command: Option<String>,
    /// Hide the account-sync badge that otherwise appears on its own while any account-sync
    /// category is on and no `anywhere_sync` widget is configured. `/statusline toggle sync`
    /// flips it.
    #[serde(default)]
    pub hide_sync_badge: bool,
}

impl StatuslineConfig {
    /// True when `widget` sits in any configured segment.
    pub fn lists(&self, widget: &StatuslineWidget) -> bool {
        self.left
            .iter()
            .chain(&self.center)
            .chain(&self.right)
            .chain(self.extra_rows.iter().flatten())
            .any(|w| w == widget)
    }

    /// The sync badge shows by itself: not listed anywhere, and not switched off.
    pub fn auto_sync_badge(&self) -> bool {
        !self.hide_sync_badge && !self.lists(&StatuslineWidget::AnywhereSync)
    }
}

fn default_sl_left() -> Vec<StatuslineWidget> {
    vec![
        StatuslineWidget::Model,
        StatuslineWidget::SessionCost,
        StatuslineWidget::Effort,
        StatuslineWidget::Mode,
    ]
}

fn default_sl_separator() -> String {
    "  │  ".to_string()
}

impl Default for StatuslineConfig {
    fn default() -> Self {
        Self {
            left: default_sl_left(),
            center: Vec::new(),
            right: Vec::new(),
            separator: default_sl_separator(),
            extra_rows: Vec::new(),
            command: None,
            hide_sync_badge: false,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut models = HashMap::new();
        let many = |s: &[&str]| OneOrMany::Many(s.iter().map(|x| x.to_string()).collect());
        // Cost-aware routing (FR-5) picks the cheapest *usable* candidate regardless of list order,
        // so a configured free provider ($0, unlisted in pricing) still wins for the actual route.
        // Order matters only for code paths that take the FIRST candidate (e.g. an architect
        // planner/editor fallback): those key-filter now, but we deliberately DON'T lead any tier
        // with `groq::…` — it needs a key many users don't have, and leading with it made groq the
        // face of every "first candidate" failure. Lead instead with a keyless/bridge option, groq
        // last. Free model ids change over time; edit `[mesh.models]` to taste (mesh-routing.md).
        models.insert(
            TaskTier::Trivial.as_str().into(),
            many(&["ollama::llama3.2", "groq::llama-3.1-8b-instant"]),
        );
        models.insert(
            TaskTier::Standard.as_str().into(),
            many(&[
                "gemini::gemini-2.5-flash",
                "openai::gpt-4o-mini",
                "groq::llama-3.3-70b-versatile",
            ]),
        );
        models.insert(
            TaskTier::Complex.as_str().into(),
            many(&[
                "claude-cli::",
                // Both subscription bridges belong in the seed, not just one: on a fresh install
                // with no API keys, whichever CLI the user is logged into is the only thing that
                // can serve a turn, and an unavailable bridge is filtered by availability anyway.
                // Listing only claude meant a codex-only machine failed its first prompt.
                "codex-cli::",
                "anthropic::claude-opus-5",
                "groq::llama-3.3-70b-versatile",
            ]),
        );
        Self {
            permission_mode: PermissionMode::AcceptEdits,
            mesh: MeshConfig {
                models,
                prefer_subscription: default_prefer_subscription(),
                classifier: ClassifierKind::default(),
                classifier_model: default_classifier_model(),
                classifier_activity_focused: false,
                bridge_mode: BridgeMode::default(),
                bridge_lean: true,
                direct_lean: true,
                lean_core_extra: Vec::new(),
                bridge_mcp_external: true,
                min_tier: None,
                daily_budget_usd: None,
                monthly_cap_usd: None,
                weekly_budget_usd: None,
                warn_threshold: default_warn_threshold(),
                budget: BudgetBehavior::default(),
                pricing: HashMap::new(),
                context_windows: HashMap::new(),
                burn_weights: HashMap::new(),
                subagents: SubagentsConfig::default(),
                workflows: WorkflowsConfig::default(),
                auto_discover: default_auto_discover(),
                failover: default_failover(),
                pin_failover: false,
                pin_outage_wait_secs: default_pin_outage_wait_secs(),
                failover_cooldown_secs: default_failover_cooldown_secs(),
                rate_limit_wait_secs: default_rate_limit_wait_secs(),
                stream_idle_timeout_secs: default_stream_idle_timeout_secs(),
                first_event_timeout_secs: default_first_event_timeout_secs(),
                compact_cap_tokens: default_compact_cap_tokens(),
                free_model_cap_tokens: default_free_model_cap_tokens(),
                max_steps: default_max_steps(),
                max_steps_unattended: default_max_steps_unattended(),
                progress_guard: ProgressGuardMode::default(),
                idle_nudge_steps: default_idle_nudge_steps(),
                idle_stop_steps: default_idle_stop_steps(),
                max_turn_input_tokens: default_max_turn_input_tokens(),
                subscription_conserve: default_subscription_conserve(),
                benchmark_ranking: default_benchmark_ranking(),
                benchmark_mirror: default_benchmark_mirror(),
                verify_completeness: default_verify_completeness(),
                nudge_empty_diff: default_nudge_empty_diff(),
                guard_test_edits: default_guard_test_edits(),
                deadline_reconcile: default_deadline_reconcile(),
                env_fight_nudge: default_env_fight_nudge(),
                bridge_require_tools: default_bridge_require_tools(),
                tool_result_context_token_budget: default_tool_result_context_token_budget(),
                tool_result_context_keep_recent: default_tool_result_context_keep_recent(),
                context_floor_block_tokens: default_context_floor_block_tokens(),
                compact_low_water: default_compact_low_water(),
                compact_model: CompactModel::default(),
                bridge_turn_token_cap: default_bridge_turn_token_cap(),
                explicit_models: Vec::new(),
                bridge_models: HashMap::new(),
                subscriptions: HashMap::new(),
                disabled: Vec::new(),
                favorites: Vec::new(),
                max_output_tokens: default_max_output_tokens(),
                credit_mode: CreditMode::Normal,
                auto_memory: default_auto_memory(),
                auto_memory_one_shot: false,
                headroom: default_headroom_toggle(),
                headroom_url: None,
                auto_orchestrate: false,
                self_review: default_self_review(),
                architect_mode: false,
                architect_model: None,
                editor_model: None,
                default_effort: None,
                cross_session_cache_key: true,
                harness_profile: HarnessProfile::Guarded,
                harness_profiles: std::collections::BTreeMap::new(),
                adaptive_effort: true,
                adaptive_effort_providers: adaptive_effort::default_adaptive_effort_providers(),
                vision_fallback: VisionFallback::default(),
            },
            permissions: PermissionsConfig::default(),
            mcp: McpConfig::default(),
            commands: CommandsConfig::default(),
            lattice: LatticeConfig::default(),
            shell: ShellConfig::default(),
            hooks: Vec::new(),
            stop_hook_max_blocks: default_stop_hook_max_blocks(),
            git: GitConfig::default(),
            lsp: LspConfig::default(),
            autofix: AutofixConfig::default(),
            assay: AssayConfig::default(),
            recap: RecapConfig::default(),
            suggest: SuggestConfig::default(),
            project: ProjectConfig::default(),
            tui: TuiConfig::default(),
            local: LocalConfig::default(),
            update: UpdateConfig::default(),
            worktree: worktree::WorktreeConfig::default(),
            telemetry: TelemetryConfig::default(),
            reporting: reporting::ReportingConfig::default(),
            self_mcp: false,
            statusline: StatuslineConfig::default(),
            notifications: notifications::NotificationsConfig::default(),
            browser: browser::BrowserConfig::default(),
            chat: chat::ChatConfig::default(),
            preview: preview::PreviewConfig::default(),
            power: power::PowerConfig::default(),
            host: host::HostConfig::default(),
            terminal: terminal::TerminalConfig::default(),
            actions: Vec::new(),
            codemode: codemode::CodemodeConfig::default(),
            pr: pr::PrConfig::default(),
            sessions: sessions::SessionsConfig::default(),
            keybinds: KeybindsConfig::default(),
            providers: ProvidersConfig::default(),
            system_prompt_overrides: HashMap::new(),
            reasoning_prefill: HashMap::new(),
            model_effort: HashMap::new(),
            compact_cap: default_compact_cap(),
            remote: RemoteConfig::default(),
            anywhere: AnywhereConfig::default(),
            voice: VoiceConfig::default(),
            harness: HarnessConfig::default(),
            tools: ToolsConfig::default(),
            privacy: privacy::PrivacyConfig::default(),
            goal: goal::GoalConfig::default(),
        }
    }
}

impl Config {
    /// Resolve the primary model id for a tier (the single value, or the first candidate),
    /// falling back to the standard tier.
    pub fn model_for(&self, tier: TaskTier) -> Option<&str> {
        self.mesh
            .models
            .get(tier.as_str())
            .or_else(|| self.mesh.models.get(TaskTier::Standard.as_str()))
            .map(OneOrMany::first)
    }

    /// The candidates a config *file* explicitly set for `tier`, in the order written. Empty
    /// when the tier only carries the shipped seed (or nothing) — no standard-tier fallback,
    /// because an override is a statement about that one tier.
    pub fn explicit_candidates_for(&self, tier: TaskTier) -> Vec<String> {
        if !self.mesh.explicit_models.iter().any(|t| t == tier.as_str()) {
            return Vec::new();
        }
        self.mesh
            .models
            .get(tier.as_str())
            .map(OneOrMany::all)
            .unwrap_or_default()
    }

    /// All candidate model ids configured for a tier (one element for the single-string form),
    /// falling back to the standard tier. The cost-aware router ranks these.
    pub fn candidates_for(&self, tier: TaskTier) -> Vec<String> {
        self.mesh
            .models
            .get(tier.as_str())
            .or_else(|| self.mesh.models.get(TaskTier::Standard.as_str()))
            .map(OneOrMany::all)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| !is_model_disabled(m, &self.mesh.disabled))
            .collect()
    }

    /// The full ordered rule set the broker resolves against: built-in safety denies first,
    /// then configured rules. Precedence is decided in `forge_core::permission`, not order.
    pub fn permission_rules(&self) -> Vec<PermissionRule> {
        let mut rules = builtin_deny_rules();
        rules.extend(
            self.permissions
                .rules
                .iter()
                .filter_map(RuleConfig::to_rule),
        );
        if !self.permissions.auto_allow_read_only_shell {
            rules.push(forge_types::read_only_shell_opt_out_rule());
        }
        rules
    }
}

/// The command/skill discovery sources, scope-tagged: user scope (`<config>/forge/{commands,
/// skills}`, present only when a config dir resolves) then project scope (`.forge/{commands,
/// skills}` in the working directory and each parent up to the repository top, nearest first, see
/// [`project_resource_dirs`]). Project wins on a name collision, and among project directories the
/// nearest one does (see `forge_skills::Catalog`).
pub fn command_sources() -> forge_skills::Sources {
    command_sources_in(std::path::Path::new("."))
}

/// [`command_sources`] with the project scope rooted at `root` (a daemon session's workspace).
pub fn command_sources_in(root: &std::path::Path) -> forge_skills::Sources {
    use forge_skills::{Scope, ScopedDir};
    let mut commands = Vec::new();
    let mut skills = Vec::new();
    if let Some(dir) = config_dir() {
        commands.push(ScopedDir {
            scope: Scope::User,
            path: dir.join("commands"),
        });
        skills.push(ScopedDir {
            scope: Scope::User,
            path: dir.join("skills"),
        });
    }
    for dir in project_resource_dirs(root) {
        commands.push(ScopedDir {
            scope: Scope::Project,
            path: dir.join(".forge/commands"),
        });
        skills.push(ScopedDir {
            scope: Scope::Project,
            path: dir.join(".forge/skills"),
        });
    }
    forge_skills::Sources { commands, skills }
}

fn merge_mcp_toml(config: &mut McpConfig, path: &std::path::Path) {
    if let Ok(text) = std::fs::read_to_string(path) {
        let has_allow = toml::from_str::<toml::Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .as_table()
                    .and_then(|table| table.get("allow"))
                    .cloned()
            })
            .is_some();
        let table = toml::from_str::<toml::Value>(&text).ok();
        let has_key = |key: &str| {
            table
                .as_ref()
                .and_then(|value| value.as_table())
                .is_some_and(|table| table.contains_key(key))
        };
        match toml::from_str::<McpConfig>(&text) {
            Ok(mcp) => {
                if has_key("call_timeout_secs") {
                    config.call_timeout_secs = mcp.call_timeout_secs;
                }
                if has_key("connect_timeout_secs") {
                    config.connect_timeout_secs = mcp.connect_timeout_secs;
                }
                if has_key("max_reconnect_attempts") {
                    config.max_reconnect_attempts = mcp.max_reconnect_attempts;
                }
                mcp::merge_mcp_servers(&mut config.servers, mcp.servers);
                if has_allow {
                    config.allow = mcp.allow;
                }
            }
            Err(e) => tracing::warn!("ignoring malformed {}: {e}", path.display()),
        }
    }
}

/// The `[providers.bedrock] models` entries as routable `bedrock::<id>` ids, blanks dropped.
pub fn bedrock_models() -> Vec<String> {
    load()
        .ok()
        .and_then(|config| config.providers.bedrock)
        .map(|bedrock| bedrock_model_ids(&bedrock))
        .unwrap_or_default()
}

fn bedrock_model_ids(bedrock: &BedrockConfig) -> Vec<String> {
    bedrock
        .models
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
        .map(|model| format!("bedrock::{}", model.trim_start_matches("bedrock::")))
        .collect()
}

/// Per-provider reasoning-prefill seeds (`[reasoning_prefill]`), read once for the process.
static REASONING_PREFILL: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();

/// The reasoning seed configured for `provider` (a namespace such as `"kimi"`), or `None` when it
/// has no entry — the default, so no provider is prefilled unless asked for.
///
/// Cached: this is consulted on every completion, and re-reading the config file per request would
/// put a disk read on the hot path.
pub fn reasoning_prefill_for(provider: &str) -> Option<String> {
    REASONING_PREFILL
        .get_or_init(|| load().map(|c| c.reasoning_prefill).unwrap_or_default())
        .get(provider)
        .filter(|seed| !seed.trim().is_empty())
        .cloned()
}

/// Exact per-model reasoning rungs (`[model_effort]`), read once for the process.
static MODEL_EFFORT: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();

/// The exact rung configured for `model` (a full `provider::model` id), as written in config.
///
/// The full id wins over a bare provider entry, so a provider-wide choice can be overridden for one
/// model. Returned unparsed: the rung vocabulary lives in `forge-types`, and the caller validates it
/// against what the provider actually offers. `None` — the default — leaves the mesh's own
/// best-value choice untouched. Cached, since this is consulted per routing decision.
pub fn model_effort_for(model: &str) -> Option<String> {
    let configured =
        MODEL_EFFORT.get_or_init(|| load().map(|c| c.model_effort).unwrap_or_default());
    by_model_or_provider(configured, model)
        .filter(|rung| !rung.trim().is_empty())
        .cloned()
}

/// The entry for `model` in a table keyed by full id or provider namespace, the id winning.
fn by_model_or_provider<'a, V>(table: &'a HashMap<String, V>, model: &str) -> Option<&'a V> {
    let namespace = model.split_once("::").map(|(provider, _)| provider);
    table
        .get(model)
        .or_else(|| namespace.and_then(|provider| table.get(provider)))
}

fn default_compact_cap() -> HashMap<String, u64> {
    HashMap::from([("kimi".to_string(), 120_000)])
}

/// Per-model auto-compaction ceilings (`[compact_cap]`), read once for the process.
static COMPACT_CAP: std::sync::OnceLock<HashMap<String, u64>> = std::sync::OnceLock::new();

/// The auto-compaction ceiling configured for `model`, in tokens. `None` — the common case — means
/// the global `mesh.compact_cap_tokens` / free-model ceiling applies; so does an explicit `0`,
/// which is how a user switches off a shipped default for one provider.
pub fn compact_cap_for(model: &str) -> Option<u64> {
    let configured = COMPACT_CAP.get_or_init(|| {
        load()
            .map(|c| c.compact_cap)
            .unwrap_or_else(|_| default_compact_cap())
    });
    by_model_or_provider(configured, model)
        .copied()
        .filter(|cap| *cap > 0)
}

/// Permission rules accumulate across scopes (Claude Code semantics): a figment merge replaces
/// arrays, so a project defining any `[[permissions.rules]]` used to erase every user-level rule,
/// user denies included. Re-add the user's rules ahead of the project's when both define some.
fn accumulate_permission_rules(
    config: &mut Config,
    user_config: Option<&std::path::Path>,
    project_has_rules: bool,
) {
    if !project_has_rules {
        return;
    }
    let user_rules = user_config.filter(|path| path.is_file()).and_then(|path| {
        Figment::from(Toml::file_exact(path))
            .extract_inner::<Vec<RuleConfig>>("permissions.rules")
            .ok()
    });
    if let Some(mut user_rules) = user_rules {
        user_rules.append(&mut config.permissions.rules);
        config.permissions.rules = user_rules;
    }
}

/// Replace `path` with `contents` via a synced temp file and rename, so a crash, a full disk or
/// an interrupt leaves the previous file intact instead of a truncated, unparsable config.
pub fn write_atomic(
    path: impl AsRef<std::path::Path>,
    contents: impl AsRef<[u8]>,
) -> std::io::Result<()> {
    use std::io::Write;
    let path = path.as_ref();
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = target.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        if let Ok(meta) = std::fs::metadata(&target) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        file.write_all(contents.as_ref())?;
        file.sync_all()?;
        std::fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The project layer of `.forge/config.toml`: as written when the project is trusted, otherwise
/// only its safe subset ([`project_trust::sanitize_untrusted`]), plus what was dropped.
fn project_layer(path: &Path, trusted: bool) -> Option<(toml::Table, Vec<String>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let table: toml::Table = match toml::from_str(&text) {
        Ok(table) => table,
        Err(e) => {
            tracing::warn!("ignoring malformed {}: {e}", path.display());
            return None;
        }
    };
    Some(if trusted {
        (table, Vec::new())
    } else {
        project_trust::sanitize_untrusted(table)
    })
}

fn has_permission_rules(table: &toml::Table) -> bool {
    table
        .get("permissions")
        .and_then(|p| p.get("rules"))
        .and_then(toml::Value::as_array)
        .is_some_and(|rules| !rules.is_empty())
}

/// Load configuration with full layered precedence (lowest -> highest):
/// built-in defaults -> user config -> project `./.forge/config.toml` -> `FORGE_*` env.
/// An untrusted project contributes only its safe subset (docs/features/project-trust.md), and
/// `--setting-sources` can leave out the user or project layer ([`setting_sources`]).
pub fn load() -> Result<Config, ConfigError> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    load_in(&cwd)
}

/// [`load`] for a project rooted at `root` rather than the process's cwd. A daemon hosts sessions
/// for many workspaces from one process whose own cwd is unrelated to any of them, so its project
/// config, trust decision, `.forge/mcp.toml` and `.forge/settings.json` hooks must all be resolved
/// against the SESSION's workspace root.
pub fn load_in(root: &std::path::Path) -> Result<Config, ConfigError> {
    let mut fig = Figment::from(Serialized::defaults(Config::default()));

    // `file_exact`, not `file`: figment's `file` searches every parent directory, so a stray
    // `~/.forge/config.toml` silently became the project config of every directory under `$HOME`.
    let layers = setting_sources::setting_sources();
    let user_dir = config_dir().filter(|_| layers.user);
    let user_config = user_dir.as_ref().map(|dir| dir.join("config.toml"));
    // `file_exact` errors on a missing file where `file` skipped it, so guard on existence.
    if let Some(path) = user_config.as_ref().filter(|p| p.is_file()) {
        fig = fig.merge(Toml::file_exact(path));
    }
    let project_config_buf = root.join(".forge/config.toml");
    let project_config = project_config_buf.as_path();
    let cwd = root.to_path_buf();
    let trusted = project_trust::is_trusted(&cwd);
    let mut ignored = Vec::new();
    let mut project_has_rules = false;
    if layers.project && trusted && project_config.is_file() {
        fig = fig.merge(Toml::file_exact(project_config));
        project_has_rules =
            project_layer(project_config, true).is_some_and(|(t, _)| has_permission_rules(&t));
    } else if let Some((table, dropped)) = layers
        .project
        .then(|| project_layer(project_config, false))
        .flatten()
    {
        project_has_rules = has_permission_rules(&table);
        ignored.extend(dropped);
        let text = toml::to_string(&table).map_err(|e| ConfigError::Write(e.to_string()))?;
        fig = fig.merge(Toml::string(&text));
    }
    // `FORGE_TELEMETRY=0` is the telemetry opt-out flag (read in forge-cli), not a `[telemetry]` table.
    fig = fig.merge(
        Env::prefixed("FORGE_")
            .ignore(&["telemetry", "no_reports", "reporting_url", "reporting_dir"])
            .split("__"),
    );

    let mut config: Config = fig.extract().map_err(|error| {
        let mut files: Vec<&Path> = Vec::new();
        if trusted && project_config.is_file() {
            files.push(project_config);
        }
        files.extend(user_config.as_deref().filter(|p| p.is_file()));
        match config_diagnostics::locate(&error.path, &files) {
            Some(at) => ConfigError::Invalid(format!("{error} (at {at})")),
            None => ConfigError::from(error),
        }
    })?;
    accumulate_permission_rules(&mut config, user_config.as_deref(), project_has_rules);
    config.harness.explicit_flags = model_family::explicit_harness_flags(
        user_config.as_deref(),
        (layers.project && trusted).then_some(project_config),
    );
    if !trusted {
        config
            .permissions
            .rules
            .extend(project_trust::user_rules_for(&cwd));
    }
    keybinds::normalize(&mut config.keybinds);
    keybindings_file::overlay_user_file(&mut config.keybinds);
    resolve_system_prompt_overrides(&mut config)?;
    // Project-local `.forge/mcp.toml` is the dedicated home for MCP server declarations; when
    // present it sets the whole `[mcp]` section (overriding any `[mcp]` in config.toml). Keeping
    // it a separate file matches Claude-Code's `.mcp.json` convention and keeps server lists out
    // of the main config.
    if let Some(dir) = user_dir.as_ref() {
        merge_mcp_toml(&mut config.mcp, &dir.join("mcp.toml"));
    }
    let project_mcp_buf = root.join(".forge/mcp.toml");
    let project_mcp = project_mcp_buf.as_path();
    if layers.project && trusted {
        merge_mcp_toml(&mut config.mcp, project_mcp);
    } else if layers.project
        && project_mcp.is_file()
        && !load_mcp_toml(project_mcp).servers.is_empty()
    {
        ignored.push("MCP servers (.forge/mcp.toml)".to_string());
    }

    // Claude-Code-compatible hooks: a `settings.json` (user config dir + project `.forge/`) can
    // declare hooks in CC's event-nested shape. They're appended (CC-compat mode) so existing CC
    // hook scripts run unmodified alongside any native `[[hooks]]`. User scope first, project last.
    let mut cc_paths: Vec<PathBuf> = user_dir.iter().map(|d| d.join("settings.json")).collect();
    let project_settings = root.join(".forge/settings.json");
    cc_paths.extend(layers.project.then(|| project_settings.clone()));
    for path in cc_paths {
        if let Ok(text) = std::fs::read_to_string(&path) {
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) => {
                    let hooks = cc_hooks_from_settings(&v);
                    if !trusted && path == project_settings {
                        if !hooks.is_empty() {
                            ignored.push("hooks (.forge/settings.json)".to_string());
                        }
                        continue;
                    }
                    config.hooks.extend(hooks);
                }
                Err(e) => tracing::warn!("ignoring malformed {}: {e}", path.display()),
            }
        }
    }
    project_trust::record_ignored(ignored);

    // Record which tiers a real config FILE pinned under `[mesh.models]`. The merged struct
    // cannot tell those apart from the shipped cold-start seed, and only a file-set tier is an
    // explicit user instruction strong enough to override auto-discovery (forge-mesh
    // `candidates_for_tier`; docs/features/mesh-routing.md §3.1).
    let mut model_config_paths: Vec<PathBuf> = user_config.iter().cloned().collect();
    model_config_paths.extend(layers.project.then(|| project_config_buf.clone()));
    for path in model_config_paths {
        if let Ok(text) = std::fs::read_to_string(&path) {
            for tier in configured_model_tiers(&text) {
                if !config.mesh.explicit_models.contains(&tier) {
                    config.mesh.explicit_models.push(tier);
                }
            }
        }
    }

    if let Some(path) = user_config.as_deref().filter(|p| p.is_file()) {
        config_diagnostics::warn_unknown_keys(path);
        config_diagnostics::warn_legacy_step_limits(path);
    }
    if layers.project && trusted && project_config.is_file() {
        config_diagnostics::warn_unknown_keys(project_config);
        config_diagnostics::warn_legacy_step_limits(project_config);
    }

    // A typo in `[anywhere]` (e.g. `nabled` for `enabled`) is otherwise silent: Figment/serde drop
    // an unrecognized key with no error, so the connector just stays off (#5). `forge anywhere
    // doctor` surfaces the same warnings on request; this is the passive path for anyone who never
    // runs doctor.
    for warning in anywhere_config_warnings_in(root) {
        tracing::warn!("{warning}");
    }

    Ok(config)
}

/// The `[mesh.models]` tier keys one config file's text sets, in file order. Empty for a file
/// with no such table (or unparseable text — `load` has already surfaced that failure).
fn configured_model_tiers(raw_toml: &str) -> Vec<String> {
    let Ok(value) = toml::from_str::<toml::Value>(raw_toml) else {
        return Vec::new();
    };
    value
        .get("mesh")
        .and_then(|m| m.get("models"))
        .and_then(toml::Value::as_table)
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default()
}

/// Whether the user has a persisted config file (the onboarding "first run" signal — combined
/// with "no provider keys / no bridges" by the caller).
pub fn user_config_exists() -> bool {
    config_dir().is_some_and(|d| d.join("config.toml").exists())
}

/// Persist `permission_mode` and `mesh.credit_mode` into the user config TOML, preserving all
/// other keys. Returns the path written.
pub fn write_settings(
    permission: PermissionMode,
    credit_mode: CreditMode,
) -> Result<PathBuf, ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir).map_err(|e| ConfigError::Write(e.to_string()))?;
    let path = dir.join("config.toml");
    write_settings_at(&path, permission, credit_mode)?;
    Ok(path)
}

/// Persist the public `[anywhere]` host settings while preserving unrelated user configuration.
/// Credentials and keys are never accepted by this API.
pub fn write_anywhere_settings(
    enabled: bool,
    host_name: Option<&str>,
    sync: bool,
) -> Result<PathBuf, ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir).map_err(|e| ConfigError::Write(e.to_string()))?;
    let path = dir.join("config.toml");
    write_anywhere_settings_at(&path, enabled, host_name, sync)?;
    Ok(path)
}

fn write_anywhere_settings_at(
    path: &Path,
    enabled: bool,
    host_name: Option<&str>,
    sync: bool,
) -> Result<(), ConfigError> {
    let mut doc = toml_write::load_doc(path)?;
    let anywhere = toml_write::table_at(&mut doc, &["anywhere"]);
    toml_write::put(anywhere, "enabled", toml_edit::value(enabled));
    toml_write::put(anywhere, "sync", toml_edit::value(sync));
    match host_name {
        Some(name) => toml_write::put(anywhere, "host_name", toml_edit::value(name)),
        None => {
            anywhere.remove("host_name");
        }
    }
    toml_write::save_doc(path, &doc)
}

fn write_settings_at(
    path: &std::path::Path,
    permission: PermissionMode,
    credit_mode: CreditMode,
) -> Result<(), ConfigError> {
    let mut doc = toml_write::load_doc(path)?;
    // permission_mode uses kebab-case serde names
    toml_write::put(
        doc.as_table_mut(),
        "permission_mode",
        toml_edit::value(permission.key()),
    );
    let credit_str = match credit_mode {
        CreditMode::Normal => "normal",
        CreditMode::Frugal => "frugal",
        CreditMode::Strict => "strict",
    };
    toml_write::put(
        toml_write::table_at(&mut doc, &["mesh"]),
        "credit_mode",
        toml_edit::value(credit_str),
    );
    toml_write::save_doc(path, &doc)
}

/// Persist `self_mcp = true/false` to the global user config, preserving all other keys.
/// Active on the next session start.
pub fn write_self_mcp(enabled: bool) -> Result<PathBuf, ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir).map_err(|e| ConfigError::Write(e.to_string()))?;
    let path = dir.join("config.toml");
    let mut doc = toml_write::load_doc(&path)?;
    toml_write::put(doc.as_table_mut(), "self_mcp", toml_edit::value(enabled));
    toml_write::save_doc(&path, &doc)?;
    Ok(path)
}

/// Persist `[statusline]` to the user config TOML, preserving all other keys.
pub fn write_statusline_config(cfg: &StatuslineConfig) -> Result<PathBuf, ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir).map_err(|e| ConfigError::Write(e.to_string()))?;
    let path = dir.join("config.toml");
    let mut doc = toml_write::load_doc(&path)?;
    let value = toml::Value::try_from(cfg).map_err(|e| ConfigError::Write(e.to_string()))?;
    let toml::Value::Table(table) = value else {
        return Err(ConfigError::Write(
            "[statusline] must serialize to a table".into(),
        ));
    };
    toml_write::sync_table(toml_write::table_at(&mut doc, &["statusline"]), &table);
    toml_write::save_doc(&path, &doc)?;
    Ok(path)
}

/// Persist only `permission_mode` to the user config TOML, preserving every other key (notably
/// `mesh.credit_mode`, which [`write_settings`] would also rewrite). Used when the temper is
/// switched at runtime (SHIFT+TAB / `/mode`) so the chosen posture becomes the default next launch.
pub fn write_permission_mode(permission: PermissionMode) -> Result<PathBuf, ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir).map_err(|e| ConfigError::Write(e.to_string()))?;
    let path = dir.join("config.toml");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let body = set_top_level_permission_mode(&existing, permission.key());
    write_atomic(&path, body).map_err(|e| ConfigError::Write(e.to_string()))?;
    Ok(path)
}

/// Set the top-level `permission_mode` key by editing the text, so comments, ordering and every
/// other key survive (a parse-and-reserialize drops all comments).
fn set_top_level_permission_mode(existing: &str, mode: &str) -> String {
    let line = format!("permission_mode = \"{mode}\"");
    let mut out: Vec<String> = Vec::new();
    let mut replaced = false;
    let mut in_table = false;
    for raw in existing.lines() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with('[') {
            in_table = true;
        }
        let is_key = !in_table
            && trimmed
                .strip_prefix("permission_mode")
                .is_some_and(|rest| rest.trim_start().starts_with('='));
        if is_key && !replaced {
            out.push(line.clone());
            replaced = true;
        } else {
            out.push(raw.to_string());
        }
    }
    if !replaced {
        out.insert(0, line);
    }
    let mut body = out.join("\n");
    body.push('\n');
    body
}

/// Append a `[[permissions.rules]]` allow entry for `tool` to the project `.forge/config.toml`.
/// Creates the file (and `.forge/` dir) if absent. Idempotent at the file level — duplicate
/// entries are harmless (first match wins in the permission broker).
pub fn append_allow_rule(tool: &str) -> std::io::Result<()> {
    append_allow_rule_in(std::path::Path::new("."), tool, &[])
}

/// Persist an "always allow" answer into `<root>/.forge/config.toml`, scoped to `patterns` (empty =
/// every call of `tool`). Shell must always be scoped: a bare `allow = "*"` for shell approves
/// every later command in that project, which is how 40 such rules once ended up auto-approving
/// everything under `$HOME`.
pub fn append_allow_rule_in(
    root: &std::path::Path,
    tool: &str,
    patterns: &[String],
) -> std::io::Result<()> {
    if tool == "shell" && patterns.is_empty() {
        return Err(std::io::Error::other(
            "refusing to persist an unscoped shell allow rule",
        ));
    }
    // An untrusted project's own allow rules are ignored (project-trust.md), so the user's answer
    // is kept in their config directory instead, where the repository cannot forge it.
    if !project_trust::is_trusted(root) {
        let allow = if patterns.is_empty() {
            OneOrMany::One("*".to_string())
        } else {
            OneOrMany::Many(patterns.to_vec())
        };
        return project_trust::append_user_rule(
            root,
            RuleConfig {
                tool: tool.to_string(),
                allow: Some(allow),
                ask: None,
                deny: None,
                reason: None,
            },
        );
    }
    let dir = root.join(".forge");
    std::fs::create_dir_all(&dir)?;
    let allow = if patterns.is_empty() {
        "\"*\"".to_string()
    } else {
        let quoted: Vec<String> = patterns.iter().map(|p| format!("{p:?}")).collect();
        format!("[{}]", quoted.join(", "))
    };
    let entry = format!("\n[[permissions.rules]]\ntool = {tool:?}\nallow = {allow}\n");
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("config.toml"))?
        .write_all(entry.as_bytes())
}

// ----------------------------------------------------------------------------------------------
mod editor;
mod editor_harness;
mod endpoint_check;
mod provider_registry;
mod provider_reload;
mod toml_write;
pub use editor::*;
pub use provider_registry::*;
pub(crate) use provider_registry::{
    azure_registry, custom_provider_registry, PROVIDER_ENV_VARS, SEARCH_ENV_VARS,
};
pub use provider_reload::{provider_set_fingerprint, reload_provider_registries};

#[cfg(test)]
use editor::set_model_disabled_at as editor_set_model_disabled_at;
#[cfg(test)]
use editor::test_support::{coerce_value, flatten_value, priority_rank, write_subscriptions_at};
#[cfg(test)]
use provider_registry::test_support::{
    add_azure_provider_at, add_custom_provider_at, build_custom_registry, remove_azure_provider_at,
    remove_custom_provider_at,
};

/// Search providers Forge can authenticate (`forge auth brave`).
pub fn known_search_providers() -> impl Iterator<Item = &'static str> {
    SEARCH_ENV_VARS.iter().map(|(name, _)| *name)
}

/// Whether a search-API key is configured (env var or keyring) for `provider`. Unlike
/// [`has_api_key`], this checks the `SEARCH_ENV_VARS` table (search keys are not model
/// providers, so `has_api_key` would wrongly treat them as keyless).
pub fn has_search_key(provider: &str) -> bool {
    let Some((_, var)) = SEARCH_ENV_VARS.iter().find(|(n, _)| *n == provider) else {
        return false;
    };
    if std::env::var(var).map(|v| !v.is_empty()).unwrap_or(false) {
        return true;
    }
    secret_store::get(provider)
        .map(|k| !k.is_empty())
        .unwrap_or(false)
}

/// Whether a `provider::model` id is a NON-chat model — image/video/audio generation, embeddings,
/// reranking, OCR, moderation/safety, or extraction/detection — that can't serve `/chat/completions`
/// and so should be excluded from the routable mesh catalog (otherwise it pollutes routing, never
/// gets a chat-intelligence benchmark, and shows as a heuristic "—"). Conservative: matches only
/// unambiguously non-conversational families. Multimodal CHAT models (…-vision, …-vl, flash) are
/// kept — "vision"/"vl" are intentionally NOT in the deny list.
pub fn is_non_chat_model(id: &str) -> bool {
    let l = id.to_ascii_lowercase();
    const DENY: &[&str] = &[
        // Embedding / reranking.
        "embed",
        "embedqa",
        "rerank",
        "/bge",
        "reranker",
        // Image / video generation.
        "imagen",
        "veo-",
        "veo3",
        "lyria",
        "nano-banana",
        "grok-imagine",
        "diffusion",
        "stable-diffusion",
        "flux",
        "text-to-image",
        "image-generation",
        // Alibaba Wan is a dedicated image/video generation family exposed beside Qwen chat
        // models by Token Plan discovery.
        "wan2.",
        // Audio / speech / TTS.
        "whisper",
        "voxtral",
        "orpheus",
        "-tts",
        "tts-",
        "text-to-speech",
        "speech-to-text",
        // OCR / document parsing / extraction / detection.
        "-ocr",
        "ocr-",
        "mistral-ocr",
        "deplot",
        "-parse",
        "parse-",
        "gliner",
        "detector",
        // Moderation / safety classifiers.
        "moderation",
        "content-safety",
        "llama-guard",
        "shieldgemma",
        "guard-",
    ];
    DENY.iter().any(|p| l.contains(p))
}

/// A human label + hint for a search provider, shown in `forge init` / `/config`.
pub fn search_provider_label(provider: &str) -> &'static str {
    match provider {
        "brave" => "Brave Search (web_search) — metered, ~$0.005/query",
        _ => "Web search API key",
    }
}

/// Export keyring-stored search keys into the environment so the `web_search` tool (which
/// reads `BRAVE_API_KEY`) sees them. Mirrors [`inject_provider_keys`]; best-effort.
pub fn inject_search_keys() {
    finish_warming_secrets();
    for (provider, var) in SEARCH_ENV_VARS {
        if std::env::var(var).is_ok() {
            continue;
        }
        if let Some(key) = secret_store::get(provider) {
            std::env::set_var(var, key);
        }
    }
}

/// The conventional environment variable for a provider's API key, if it needs one. Chains the
/// native-adapter table and the custom OpenAI-compatible registry.
fn env_var_for(provider: &str) -> Option<&'static str> {
    PROVIDER_ENV_VARS
        .iter()
        .find(|(name, _)| *name == provider)
        .map(|(_, var)| *var)
        // A custom provider with an empty `env_var` is KEYLESS (local server) — `None` here makes
        // `has_api_key` treat it as not-key-blocked and `api_key` return `Ok("")` without erroring.
        .or_else(|| {
            custom_provider(provider).and_then(|p| (!p.env_var.is_empty()).then_some(p.env_var))
        })
        // Azure carries a configurable key env var (default `AZURE_OPENAI_API_KEY`); the resolved
        // provider is build-once `'static`, so the borrowed name is `'static` too.
        .or_else(|| {
            (provider == AZURE_NS)
                .then(azure_registry)
                .flatten()
                .map(|a| a.env_var.as_str())
        })
}

/// Conventional environment variable for a provider key, if this provider is keyed.
///
/// This exposes only the variable *name* for account-management UI; key contents never cross the
/// config boundary.
pub fn provider_key_env_var(provider: &str) -> Option<&'static str> {
    env_var_for(provider)
}

/// Whether this process received a provider key from its environment (canonical name or alias).
/// The value itself is deliberately never returned.
pub fn provider_environment_key_present(provider: &str) -> bool {
    env_var_for(provider).is_some_and(env_set) || env_aliases_for(provider).any(env_set)
}

/// Provider names Forge knows how to authenticate (for `forge auth` validation/help). Includes
/// native-adapter providers and the custom OpenAI-compatible registry (built-in + runtime).
pub fn known_key_providers() -> impl Iterator<Item = &'static str> {
    PROVIDER_ENV_VARS
        .iter()
        .map(|(name, _)| *name)
        .chain(custom_provider_registry().iter().map(|p| p.namespace))
        // Azure is a known keyed provider only when configured (so `forge auth azure` is valid then).
        .chain(azure_registry().map(|_| AZURE_NS))
}

/// Conventional / legacy env-var aliases accepted IN ADDITION to the canonical name in
/// [`PROVIDER_ENV_VARS`]. OpenRouter's own docs and most users use `OPENROUTER_API_KEY`, but the
/// canonical var (the one genai's `open_router` adapter reads) is `OPEN_ROUTER_API_KEY` — so a user
/// who exported the conventional name was silently treated as keyless, the mesh skipped OpenRouter
/// discovery, and routing fell back to the built-in groq defaults. Accept both: read either name,
/// and [`inject_provider_keys`] copies an alias into the canonical var genai authenticates with.
const PROVIDER_ENV_ALIASES: &[(&str, &str)] = &[("openrouter", "OPENROUTER_API_KEY")];

/// Accepted env-var aliases for `provider` (besides its canonical [`env_var_for`] name).
fn env_aliases_for(provider: &str) -> impl Iterator<Item = &'static str> + '_ {
    PROVIDER_ENV_ALIASES
        .iter()
        .filter(move |(p, _)| *p == provider)
        .map(|(_, v)| *v)
}

/// Whether an env var is set to a non-empty value.
fn env_set(var: &str) -> bool {
    std::env::var(var).map(|v| !v.is_empty()).unwrap_or(false)
}

/// The provider prefix of a `"provider::model"` id (the part before the first `::`), or `""`
/// when the id is unprefixed.
pub fn provider_of(model: &str) -> &str {
    model.split_once("::").map(|(p, _)| p).unwrap_or("")
}

/// Whether a usable API key is available for `model`'s provider *without* erroring. True for
/// keyless providers (local `ollama::`, the `claude-cli::`/`codex-cli::` bridges that own
/// their own auth, and unprefixed ids we can't classify). For key-based providers, true iff
/// the env var is set or the keyring holds an entry. The mesh uses this for provider fallback.
pub fn has_api_key(provider: &str) -> bool {
    let Some(var) = env_var_for(provider) else {
        return true; // keyless / unknown -> don't block routing on it
    };
    if env_set(var) || env_aliases_for(provider).any(env_set) {
        return true;
    }
    secret_store::get(provider)
        .map(|k| !k.is_empty())
        .unwrap_or(false)
}

/// Whether `provider` is one Forge actually knows how to dispatch to — a keyed provider (native or
/// custom OpenAI-compatible, incl. Azure), a keyless local server (`ollama`), or a subscription CLI
/// bridge (`claude-cli`/`codex-cli`/`agy-cli`). Distinct from [`has_api_key`], which returns `true`
/// for UNKNOWN providers (so it never blocks routing) — this returns `false` for them, letting a
/// caller that must reject an unroutable pin (e.g. the OpenAI-compatible `forge api` endpoint) tell
/// a real provider from a typo instead of dispatching a doomed request.
pub fn is_known_provider(provider: &str) -> bool {
    known_key_providers().any(|p| p == provider)
        || matches!(
            provider,
            "ollama" | "claude-cli" | "codex-cli" | "agy-cli" | "xai-oauth" | "codex-oauth"
        )
}

/// Resolve a single API key for a provider (the first usable one). Environment first, then the OS
/// keyring. Used wherever exactly one key is needed (model listing, balance probes); for rotation
/// across all configured keys see [`api_keys`].
pub fn api_key(provider: &str) -> Result<String, ConfigError> {
    if let Some(first) = api_keys(provider).into_iter().next() {
        return Ok(first);
    }
    // Keyless / unknown providers (e.g. local `ollama`) have no env var → not an error, just "".
    let Some(var) = env_var_for(provider) else {
        return Ok(String::new());
    };
    Err(ConfigError::MissingKey(provider.into(), var.into()))
}

/// All usable API keys configured for a provider, in priority order, de-duplicated. Sources: the
/// canonical env var and its numbered siblings (`VAR`, `VAR_2`, `VAR_3`, …), comma-separated values
/// within any of those, accepted env aliases (+ their numbered siblings), and the OS keyring entry
/// (a newline-separated list appended by repeated `forge auth`). Multiple keys let the provider
/// client round-robin across them to multiply a free tier's per-key rate limit and fail over within
/// one provider on a 429. Empty for keyless/unknown providers.
pub fn api_keys(provider: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |raw: &str| {
        for part in raw.split([',', '\n']) {
            let k = part.trim();
            if !k.is_empty() && !out.iter().any(|e| e == k) {
                out.push(k.to_string());
            }
        }
    };
    if let Some(var) = env_var_for(provider) {
        let vars =
            std::iter::once(var.to_string()).chain(env_aliases_for(provider).map(str::to_string));
        for base in vars {
            if let Ok(v) = std::env::var(&base) {
                push(&v);
            }
            for n in 2..=MAX_NUMBERED_KEY_ENV {
                if let Ok(v) = std::env::var(format!("{base}_{n}")) {
                    push(&v);
                }
            }
        }
    }
    if let Some(stored) = secret_store::get(provider) {
        push(&stored);
    }
    out
}

/// Upper bound when scanning numbered env-var siblings (`VAR_2`..=`VAR_N`). Gaps are tolerated.
const MAX_NUMBERED_KEY_ENV: u8 = 16;

/// The Forge provider that authenticates with environment variable `var`, if any. Reverse of
/// [`env_var_for`]; the provider client uses it to map a genai `AuthData::FromEnv` back to a
/// provider so it can substitute a rotated key. Aliases canonicalize to the primary var.
pub fn provider_for_env_var(var: &str) -> Option<&'static str> {
    if var.is_empty() {
        return None; // keyless custom providers carry an empty env_var — never match on it
    }
    PROVIDER_ENV_VARS
        .iter()
        .find(|(_, v)| *v == var)
        .map(|(p, _)| *p)
        .or_else(|| {
            custom_provider_registry()
                .iter()
                .find(|p| p.env_var == var)
                .map(|p| p.namespace)
        })
        .or_else(|| {
            azure_registry()
                .filter(|a| a.env_var == var)
                .map(|_| AZURE_NS)
        })
}

/// Store a provider API key, REPLACING any existing key(s) with this single one
/// (`forge auth <p> --replace`). For additive multi-key setup use [`add_api_key`].
pub fn store_api_key(provider: &str, key: &str) -> Result<(), ConfigError> {
    secret_store::set(provider, key.trim())
}

/// Append a key to a provider's keyring list (idempotent — a duplicate is ignored). Returns the
/// number of keys in the keyring afterwards. Default `forge auth` behaviour, so repeated runs
/// accumulate keys to rotate across.
pub fn add_api_key(provider: &str, key: &str) -> Result<usize, ConfigError> {
    let key = key.trim();
    let mut keys: Vec<String> = secret_store::get(provider)
        .map(|s| {
            s.split('\n')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if !keys.iter().any(|k| k == key) {
        keys.push(key.to_string());
    }
    secret_store::set(provider, &keys.join("\n"))?;
    Ok(keys.len())
}

/// Masked fingerprints (`…last4`) of every configured key for a provider, for `forge auth --list`.
/// Never returns the keys themselves.
pub fn api_key_fingerprints(provider: &str) -> Vec<String> {
    api_keys(provider)
        .iter()
        .map(|k| {
            let tail: String = k
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<char>>()
                .into_iter()
                .rev()
                .collect();
            format!("…{tail}")
        })
        .collect()
}

/// Masked fingerprints of only the keys Forge owns in its keyring/encrypted fallback.
///
/// Unlike [`api_key_fingerprints`], this excludes environment-provided keys so a management UI can
/// truthfully explain what its Remove action will and will not delete.
pub fn stored_api_key_fingerprints(provider: &str) -> Vec<String> {
    secret_store::get(provider)
        .into_iter()
        .flat_map(|stored| {
            stored
                .split('\n')
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(|key| {
                    let tail: String = key
                        .chars()
                        .rev()
                        .take(4)
                        .collect::<Vec<char>>()
                        .into_iter()
                        .rev()
                        .collect();
                    format!("…{tail}")
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Delete a provider's stored API key(s). `Ok(true)` if an entry was removed, `Ok(false)` if there
/// was nothing stored (so `forge auth --remove` is idempotent). Removes the whole keyring list;
/// env-var keys are not touched (Forge doesn't own the environment).
pub fn remove_api_key(provider: &str) -> Result<bool, ConfigError> {
    secret_store::delete(provider)
}

/// Store an arbitrary secret (e.g. an MCP server token, keyed `mcp:<server>`) in the OS keyring
/// under the `forge` service. Cross-platform via the `keyring` crate's native backends (macOS
/// Keychain, Windows Credential Manager, Linux Secret Service). ADR-0007: secrets live in the
/// keyring, never in config or logs. `forge mcp import` uses this to persist captured tokens.
pub fn store_secret(key: &str, value: &str) -> Result<(), ConfigError> {
    secret_store::set(key, value)
}

/// Read a stored secret by key (keyring, then the encrypted-file fallback). Used for MCP tokens.
pub fn load_secret(key: &str) -> Option<String> {
    secret_store::get(key)
}

/// Delete a stored secret by key. `Ok(false)` when nothing was stored.
pub fn delete_secret(key: &str) -> Result<bool, ConfigError> {
    secret_store::delete(key)
}

/// Every (provider, env var) pair a key can be injected for, and the providers whose secret has to
/// be asked of the secret store because no environment variable already answers.
fn provider_secret_lookups() -> (Vec<(&'static str, &'static str)>, Vec<&'static str>) {
    use key_adoption::INJECTED;
    let injected = |var: &str| {
        INJECTED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(var)
    };
    let user_set = |var: &str| env_set(var) && !injected(var);
    let native = PROVIDER_ENV_VARS.iter().copied();
    let custom = custom_provider_registry()
        .iter()
        .map(|p| (p.namespace, p.env_var));
    let azure = azure_registry()
        .into_iter()
        .map(|a| (AZURE_NS, a.env_var.as_str()));
    let all: Vec<(&str, &str)> = native.chain(custom).chain(azure).collect();
    let to_lookup: Vec<&str> = all
        .iter()
        .filter(|(p, var)| {
            !var.is_empty()
                && !user_set(var)
                && !env_aliases_for(p).any(|a| std::env::var(a).is_ok_and(|v| !v.is_empty()))
        })
        .map(|(p, _)| *p)
        .collect();
    (all, to_lookup)
}

/// Read each provider's secret on up to eight threads. The read cache in `secret_store` keeps the
/// answers, so whoever asks next gets them without another round trip.
fn lookup_secrets_in_parallel(providers: &[&str]) {
    std::thread::scope(|scope| {
        let per_thread = providers.len().div_ceil(8).max(1);
        for chunk in providers.chunks(per_thread) {
            scope.spawn(move || {
                for provider in chunk {
                    let _ = secret_store::get(provider);
                }
            });
        }
    });
}

/// Start reading the stored provider and search-API secrets on a detached thread, so the
/// round trips to the secret service overlap whatever the caller does before
/// [`inject_provider_keys`] and its lookups find them cached. Reads only: nothing is injected, no
/// environment variable is touched. Call it as early in a start-up as the process is allowed to
/// spawn a thread.
pub fn warm_provider_secrets() {
    let spawned = std::thread::Builder::new()
        .name("forge-secret-warm".into())
        .spawn(|| {
            let (_, mut lookups) = provider_secret_lookups();
            lookups.extend(
                SEARCH_ENV_VARS
                    .iter()
                    .filter(|(_, var)| std::env::var(var).is_err())
                    .map(|(provider, _)| *provider),
            );
            lookup_secrets_in_parallel(&lookups);
        });
    if let Ok(handle) = spawned {
        *WARMING_SECRETS.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }
}

static WARMING_SECRETS: std::sync::Mutex<Option<std::thread::JoinHandle<()>>> =
    std::sync::Mutex::new(None);

/// Wait for [`warm_provider_secrets`] to finish. The injectors set environment variables, which
/// must not overlap another thread's secret-service calls (they read the environment), so they
/// wait here first; with the warm-up started early there is nothing left to wait for.
fn finish_warming_secrets() {
    let handle = WARMING_SECRETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

/// Make stored keys visible to the provider client (genai reads keys from the environment): for
/// each known provider with no env var set, inject the PRIMARY stored key. Only the first key is
/// injected — the keyring may hold a newline-separated list of several, and the env var must be a
/// single key; the provider client rotates across the full list separately (see `api_keys`).
/// Best-effort — providers without a stored key are simply left unset.
pub fn inject_provider_keys() {
    use key_adoption::INJECTED;
    finish_warming_secrets();
    let injected = |var: &str| {
        INJECTED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(var)
    };
    let user_set = |var: &str| env_set(var) && !injected(var);
    let (all, to_lookup) = provider_secret_lookups();
    // ~144 serial secret-store lookups (a D-Bus round trip each) cost 218 ms before the first
    // frame. Ask for them in parallel; `secret_store::get` memoizes, so the loop below then only
    // reads the cache.
    lookup_secrets_in_parallel(&to_lookup);
    for (provider, var) in all {
        // Keyless custom providers (local servers) carry an empty env_var — nothing to inject.
        if var.is_empty() || user_set(var) {
            continue;
        }
        // A conventional alias the user exported (e.g. OPENROUTER_API_KEY) → copy into the
        // canonical var genai's adapter actually reads (OPEN_ROUTER_API_KEY), so it authenticates.
        let value = env_aliases_for(provider)
            .find_map(|a| std::env::var(a).ok().filter(|s| !s.is_empty()))
            .or_else(|| {
                secret_store::get(provider).and_then(|key| {
                    let primary = key.split('\n').next().unwrap_or(&key).trim().to_string();
                    (!primary.is_empty()).then_some(primary)
                })
            });
        let mut set = INJECTED.lock().unwrap_or_else(|e| e.into_inner());
        match value {
            Some(value) => {
                if std::env::var(var).ok().as_deref() != Some(value.as_str()) {
                    std::env::set_var(var, value);
                }
                set.insert(var.to_string());
                forge_types::child_env::scrub(var);
            }
            None if set.remove(var) => {
                std::env::remove_var(var);
                forge_types::child_env::unscrub(var);
            }
            None => {}
        }
    }
    key_adoption::mark_resolved();
}

#[cfg(test)]
mod tests {
    #[test]
    fn tui_look_settings_default_to_auto_and_parse_from_toml() {
        let cfg = TuiConfig::default();
        assert_eq!(
            (cfg.theme.as_str(), cfg.color.as_str(), cfg.glyphs.as_str()),
            ("auto", "auto", "auto")
        );
        assert!(!cfg.reduce_motion);
        // An older config with only the original keys still loads, and the new keys default.
        let old: TuiConfig = toml::from_str("fullscreen = false\nmouse_capture = false").unwrap();
        assert!(!old.fullscreen && !old.mouse_capture);
        assert_eq!(old.theme, "auto");
        assert!(!old.reduce_motion);
        let set: TuiConfig = toml::from_str(
            "theme = \"light\"\ncolor = \"256\"\nglyphs = \"ascii\"\nreduce_motion = true",
        )
        .unwrap();
        assert_eq!(
            (set.theme.as_str(), set.color.as_str(), set.glyphs.as_str()),
            ("light", "256", "ascii")
        );
        assert!(set.reduce_motion);
    }
    use super::*;

    #[test]
    fn permission_mode_edit_keeps_comments_and_other_keys() {
        let before =
            "# my notes\npermission_mode = \"default\" # inline\n[mesh]\npermission_mode = \"x\"\n";
        let after = set_top_level_permission_mode(before, "accept-edits");
        assert_eq!(
            after,
            "# my notes\npermission_mode = \"accept-edits\"\n[mesh]\npermission_mode = \"x\"\n"
        );
        assert_eq!(
            set_top_level_permission_mode("# c\n[a]\nk = 1\n", "default"),
            "permission_mode = \"default\"\n# c\n[a]\nk = 1\n"
        );
    }

    /// Every wave-1 config addition (cache floor, cross-session cache key, harness profile,
    /// privacy, preview) must deserialize, from a config file holding only the two required keys,
    /// to the same value `Config::default()` ships.
    #[test]
    fn wave1_config_fields_default_from_an_empty_file() {
        let parsed: Config = toml::from_str(
            r#"
permission_mode = "accept-edits"
[mesh]
models = {}
"#,
        )
        .expect("a config naming none of them parses");
        let shipped = Config::default();
        assert_eq!(
            parsed.mesh.context_floor_block_tokens,
            shipped.mesh.context_floor_block_tokens
        );
        assert_eq!(
            parsed.mesh.compact_low_water,
            shipped.mesh.compact_low_water
        );
        assert_eq!(parsed.mesh.compact_model, CompactModel::Session);
        let auto: Config = toml::from_str(
            r#"
permission_mode = "accept-edits"
[mesh]
models = {}
compact_model = "auto"
"#,
        )
        .expect("compact_model = \"auto\" parses");
        assert_eq!(auto.mesh.compact_model, CompactModel::Auto);
        assert!(parsed.mesh.cross_session_cache_key);
        assert_eq!(
            parsed.mesh.cross_session_cache_key,
            shipped.mesh.cross_session_cache_key
        );
        assert_eq!(parsed.mesh.harness_profile, HarnessProfile::Guarded);
        assert_eq!(parsed.mesh.harness_profile, shipped.mesh.harness_profile);
        assert!(parsed.mesh.harness_profiles.is_empty());
        assert_eq!(parsed.privacy, shipped.privacy);
        assert_eq!(parsed.preview, shipped.preview);
        assert_eq!(parsed.codemode, shipped.codemode);
        assert!(parsed.harness.edit_errors && parsed.harness.arg_coercion);
        assert!(!parsed.harness.read_shaping);
        assert!(!parsed.harness.script_parallel && !parsed.harness.shell_autobackground);
        assert_eq!(parsed.pr, shipped.pr);
    }

    /// The shipped `[compact_cap]` table halves Kimi Code's per-step prompt; every other provider
    /// stays on the global ceiling, and a full model id overrides its provider's entry.
    #[test]
    fn compact_cap_ships_a_kimi_ceiling_and_the_full_id_wins() {
        let table = default_compact_cap();
        assert_eq!(
            by_model_or_provider(&table, "kimi::k3-256k"),
            Some(&120_000)
        );
        assert_eq!(by_model_or_provider(&table, "openai::gpt-5.6"), None);
        let mut table = table;
        table.insert("kimi::k3".into(), 200_000);
        assert_eq!(by_model_or_provider(&table, "kimi::k3"), Some(&200_000));
        assert_eq!(
            by_model_or_provider(&table, "kimi::k3-256k"),
            Some(&120_000)
        );
        // A config that omits the table entirely still gets the shipped default; one that names
        // the provider with `0` opts back out of it.
        let parsed: Config =
            toml::from_str("permission_mode = \"accept-edits\"\n[mesh]\nmodels = {}\n")
                .expect("a config with no [compact_cap] table parses");
        assert_eq!(parsed.compact_cap.get("kimi"), Some(&120_000));
        let mut table = default_compact_cap();
        table.insert("kimi".into(), 0);
        assert_eq!(
            by_model_or_provider(&table, "kimi::k3")
                .copied()
                .filter(|c| *c > 0),
            None
        );
    }

    #[test]
    fn mesh_min_tier_parses_and_defaults_to_no_floor() {
        let floored: Config = toml::from_str(
            r#"
permission_mode = "accept-edits"
[mesh]
models = {}
min_tier = "standard"
"#,
        )
        .expect("min_tier parses");
        assert_eq!(floored.mesh.min_tier, Some(forge_types::TaskTier::Standard));
        let plain: Config = toml::from_str(
            r#"
permission_mode = "accept-edits"
[mesh]
models = {}
"#,
        )
        .expect("a config without min_tier parses");
        assert_eq!(plain.mesh.min_tier, None);
        assert_eq!(Config::default().mesh.min_tier, None);
    }

    #[test]
    fn local_sessions_and_runs_reach_the_fleet_unless_opted_out() {
        // Reaching the fleet is opt-OUT for both: a config with no `[remote]` block at all must
        // still advertise local chat AND host one-shot runs in the daemon, or the phone and
        // desktop apps show nothing of what this machine is working on.
        // Parse a WHOLE config with no `[remote]` block — the shape of a real config file, and
        // the case that regressed. `#[serde(default)]` on `Config::remote` builds the struct via
        // `Default`, which never consults the per-field `default_true` attributes; deriving
        // `Default` here silently disabled fleet publishing for every user without the block.
        // Asserting on `toml::from_str::<RemoteConfig>("")` alone does NOT cover this: that path
        // does use the field attributes, which is why the original test passed while terminal
        // sessions stopped appearing in the fleet.
        let whole: Config = toml::from_str(
            r#"
permission_mode = "accept-edits"
[mesh]
models = {}
"#,
        )
        .expect("a config with no [remote] block parses");
        assert!(
            whole.remote.publish_local_sessions,
            "an absent [remote] block must still publish local sessions"
        );
        assert!(
            whole.remote.publish_local_runs,
            "an absent [remote] block must still host one-shot runs in the daemon"
        );
        assert!(
            RemoteConfig::default().publish_local_sessions,
            "Default must agree with the serde defaults"
        );
        assert!(RemoteConfig::default().publish_local_runs);
        assert!(
            whole.remote.interactive_local_sessions,
            "an absent [remote] block must still make terminal sessions drivable from the apps"
        );
        assert!(RemoteConfig::default().interactive_local_sessions);

        let default: RemoteConfig = toml::from_str("").expect("an absent [remote] block parses");
        assert!(default.publish_local_sessions);
        assert!(
            default.publish_local_runs,
            "a one-shot run nobody can see from the phone is the odd case, not the norm"
        );

        assert!(default.interactive_local_sessions);

        let opted_out: RemoteConfig = toml::from_str(
            "publish_local_sessions = false\npublish_local_runs = false\ninteractive_local_sessions = false\n",
        )
        .expect("opt-out parses");
        assert!(!opted_out.publish_local_sessions);
        assert!(!opted_out.publish_local_runs);
        assert!(!opted_out.interactive_local_sessions);
    }

    #[test]
    fn system_prompt_override_resolves_at_file_paths() {
        let dir = std::env::temp_dir().join(format!("forge-prompt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("kimi.md");
        std::fs::write(&file, "custom prompt from a file").unwrap();

        let mut config = Config::default();
        config
            .system_prompt_overrides
            .insert("kimi".to_string(), format!("@{}", file.display()));
        config
            .system_prompt_overrides
            .insert("openai".to_string(), "inline prompt".to_string());

        resolve_system_prompt_overrides(&mut config).unwrap();
        assert_eq!(
            config.system_prompt_overrides["kimi"],
            "custom prompt from a file"
        );
        assert_eq!(config.system_prompt_overrides["openai"], "inline prompt");

        // A missing file is a clear startup error, not a literal prompt sent to the model.
        let mut bad = Config::default();
        bad.system_prompt_overrides
            .insert("kimi".to_string(), format!("@{}/nope.md", dir.display()));
        let err = resolve_system_prompt_overrides(&mut bad).unwrap_err();
        assert!(matches!(err, ConfigError::PromptFile(_, _)), "got: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn claude_aliases_cover_every_imported_permission_tool() {
        for (claude, expected) in [
            ("Edit", vec!["edit_file", "apply_patch"]),
            ("Write", vec!["write_file", "create_file"]),
            ("LS", vec!["list_dir"]),
        ] {
            assert_eq!(forge_tools_from_cc(claude), expected);
            for tool in forge_tools_from_cc(claude) {
                assert_eq!(cc_tool_alias(tool), claude);
            }
        }
    }

    #[test]
    fn file_set_mesh_models_tiers_are_recorded_as_explicit() {
        let tiers = configured_model_tiers(
            "[mesh.models]\ntrivial = \"claude-cli::haiku\"\ncomplex = \"claude-cli::opus\"\n",
        );
        assert!(tiers.contains(&"trivial".to_string()));
        assert!(tiers.contains(&"complex".to_string()));
    }

    #[test]
    fn no_explicit_tiers_without_a_mesh_models_table() {
        assert!(configured_model_tiers("[mesh]\nauto_discover = true\n").is_empty());
        assert!(configured_model_tiers("").is_empty());
    }

    #[test]
    fn anywhere_typo_is_caught_with_a_did_you_mean_hint() {
        // #5: `[anywhere] nabled = true` silently disabled the connector with zero diagnostic.
        let unknown = anywhere_unknown_keys("[anywhere]\nnabled = true\n");
        assert_eq!(unknown, vec![("nabled".to_string(), Some("enabled"))]);
    }

    #[test]
    fn anywhere_known_keys_produce_no_warning() {
        let unknown = anywhere_unknown_keys(
            "[anywhere]\nenabled = true\nhost_name = \"laptop\"\nsync = false\n\
             service_url = \"http://localhost:8080\"\n",
        );
        assert!(unknown.is_empty(), "{unknown:?}");
    }

    #[test]
    fn anywhere_unknown_key_far_from_any_known_key_gets_no_guess() {
        // A key with no plausible match should say so rather than propose a random near-miss.
        let unknown = anywhere_unknown_keys("[anywhere]\nxyz_totally_unrelated = 1\n");
        assert_eq!(unknown, vec![("xyz_totally_unrelated".to_string(), None)]);
    }

    #[test]
    fn anywhere_unknown_keys_ignore_other_tables_and_malformed_toml() {
        assert!(anywhere_unknown_keys("[mesh]\nnabled = true\n").is_empty());
        assert!(anywhere_unknown_keys("not valid toml {{{").is_empty());
        assert!(anywhere_unknown_keys("").is_empty());
    }

    #[test]
    fn levenshtein_matches_known_distances() {
        assert_eq!(levenshtein("enabled", "enabled"), 0);
        assert_eq!(levenshtein("nabled", "enabled"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn default_seed_models_are_not_explicit() {
        let cfg = Config::default();
        assert!(
            !cfg.mesh.models.is_empty(),
            "the cold-start seed should populate [mesh.models]"
        );
        for tier in [TaskTier::Trivial, TaskTier::Standard, TaskTier::Complex] {
            assert!(
                cfg.explicit_candidates_for(tier).is_empty(),
                "seed entries must not count as an explicit override ({tier:?})"
            );
        }
    }

    #[test]
    fn explicit_candidates_honour_list_order_and_do_not_fall_back_to_standard() {
        let mut cfg = Config::default();
        cfg.mesh.models.insert(
            "trivial".into(),
            OneOrMany::Many(vec!["claude-cli::haiku".into(), "groq::llama".into()]),
        );
        cfg.mesh.explicit_models = vec!["trivial".into()];
        assert_eq!(
            cfg.explicit_candidates_for(TaskTier::Trivial),
            vec!["claude-cli::haiku".to_string(), "groq::llama".to_string()]
        );
        assert!(cfg.explicit_candidates_for(TaskTier::Complex).is_empty());
    }

    static TEST_CWD_MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

    struct TestCwdGuard {
        prior: std::path::PathBuf,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for TestCwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.prior);
        }
    }

    fn test_cwd_guard(target: &std::path::Path) -> TestCwdGuard {
        let lock = TEST_CWD_MUTEX
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .expect("locking config test cwd mutex");
        let prior = std::env::current_dir().expect("reading config test process cwd");
        std::env::set_current_dir(target).expect("entering guarded config test cwd");
        TestCwdGuard { prior, _lock: lock }
    }

    #[test]
    fn completeness_verification_defaults_to_on() {
        assert!(Config::default().mesh.verify_completeness);
    }

    #[test]
    fn remote_config_defaults_to_off() {
        let c = RemoteConfig::default();
        assert_eq!(c.auto, RemoteAuto::Off);
        assert!(c.startup_exposure().is_none(), "off => no auto-start");
        assert!(c.host.is_none(), "no host override by default");
    }

    #[test]
    fn remote_config_parses_host_override() {
        let c: RemoteConfig = toml::from_str(r#"host = "192.168.1.5""#).unwrap();
        assert_eq!(c.host.as_deref(), Some("192.168.1.5"));
    }

    #[test]
    fn remote_config_parses_auto_exposure() {
        let c: RemoteConfig = toml::from_str(r#"auto = "lan""#).unwrap();
        assert_eq!(c.auto, RemoteAuto::Lan);
        assert_eq!(c.startup_exposure(), Some(RemoteAuto::Lan));

        let c: RemoteConfig = toml::from_str(r#"auto = "anywhere""#).unwrap();
        assert_eq!(c.startup_exposure(), Some(RemoteAuto::Anywhere));

        // Absent `auto` (empty block) => off.
        let c: RemoteConfig = toml::from_str("").unwrap();
        assert!(c.startup_exposure().is_none());
    }

    #[test]
    fn remote_config_tunnel_fields_default_to_absent() {
        let c = RemoteConfig::default();
        assert!(c.tunnel_name.is_none());
        assert!(c.tunnel_hostname.is_none());
    }

    #[test]
    fn remote_config_parses_fixed_tunnel_fields() {
        let c: RemoteConfig = toml::from_str(
            r#"
tunnel_name = "forge"
tunnel_hostname = "forge.example.com"
"#,
        )
        .unwrap();
        assert_eq!(c.tunnel_name.as_deref(), Some("forge"));
        assert_eq!(c.tunnel_hostname.as_deref(), Some("forge.example.com"));
    }

    #[test]
    fn remote_config_parses_project_browse_roots() {
        let c: RemoteConfig = toml::from_str(
            r#"
project_roots = ["~/Projects", "/srv/work"]
"#,
        )
        .unwrap();
        assert_eq!(c.project_roots, ["~/Projects", "/srv/work"]);
    }

    #[test]
    fn voice_config_defaults() {
        let c = VoiceConfig::default();
        assert_eq!(c.model, "base");
        assert_eq!(c.language, "auto");
        assert_eq!(c.auto_stop_silence_ms, 0);
    }

    #[test]
    fn voice_config_parses_overrides() {
        let c: VoiceConfig = toml::from_str(
            r#"
model = "small"
language = "nl"
auto_stop_silence_ms = 1500
"#,
        )
        .unwrap();
        assert_eq!(c.model, "small");
        assert_eq!(c.language, "nl");
        assert_eq!(c.auto_stop_silence_ms, 1500);
    }

    #[test]
    fn config_accepts_voice_block() {
        let c: Config = toml::from_str(
            r#"
permission_mode = "accept-edits"
[mesh]
models = {}
[voice]
model = "medium"
"#,
        )
        .unwrap();
        assert_eq!(c.voice.model, "medium");
        assert_eq!(c.voice.language, "auto");
    }

    #[test]
    fn config_accepts_suggest_block() {
        let c: Config = toml::from_str(
            r#"
permission_mode = "accept-edits"
[mesh]
models = {}
[suggest]
enabled = false
"#,
        )
        .unwrap();
        assert!(!c.suggest.enabled);
        assert!(Config::default().suggest.enabled, "default is on");
    }

    #[test]
    fn output_is_uncapped_by_default_and_credit_mode_never_changes_it() {
        let mut mesh = Config::default().mesh;
        assert_eq!(mesh.max_output_tokens, 0);
        for mode in [CreditMode::Normal, CreditMode::Frugal, CreditMode::Strict] {
            mesh.credit_mode = mode;
            assert_eq!(mesh.effective_max_output_tokens(), 0);
        }
        mesh.max_output_tokens = 12_345;
        mesh.credit_mode = CreditMode::Strict;
        assert_eq!(mesh.effective_max_output_tokens(), 12_345);
    }

    #[test]
    fn daily_cap_usd_alias_loads_over_serialized_defaults() {
        let config: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string("[mesh]\ndaily_cap_usd = 1.5"))
            .extract()
            .unwrap();
        assert_eq!(config.mesh.daily_budget_usd, Some(1.5));
    }

    #[test]
    fn disabled_models_are_not_tier_candidates() {
        let mut config = Config::default();
        config.mesh.models.insert(
            "trivial".into(),
            OneOrMany::Many(vec!["groq::a".into(), "fka::b".into()]),
        );
        config.mesh.disabled = vec!["groq".into()];
        assert_eq!(config.candidates_for(TaskTier::Trivial), vec!["fka::b"]);
    }

    #[test]
    fn config_accepts_anonymous_telemetry_opt_out() {
        let config: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string("[telemetry]\nenabled = false"))
            .extract()
            .unwrap();
        assert!(!config.telemetry.enabled);
        assert!(Config::default().telemetry.enabled, "default is on");
    }

    #[test]
    fn config_accepts_remote_block() {
        // A full config with a `[remote]` block round-trips through the loader's deserialize path.
        let c: Config = toml::from_str(
            r#"
permission_mode = "accept-edits"
[mesh]
models = {}
[remote]
auto = "local"
"#,
        )
        .unwrap();
        assert_eq!(c.remote.auto, RemoteAuto::Local);
    }

    #[test]
    fn config_leaves_discovers_scalars_and_skips_complex_sections() {
        let value = serde_json::to_value(Config::default()).unwrap();
        let mut leaves = Vec::new();
        flatten_value("", &value, &mut leaves);
        let paths: Vec<&str> = leaves.iter().map(|l| l.path.as_str()).collect();
        // Known scalars are discovered automatically.
        assert!(paths.contains(&"tui.fullscreen"));
        assert!(paths.contains(&"local.autostart"));
        assert!(paths.contains(&"recap.enabled"));
        assert!(paths.contains(&"mesh.disabled"));
        assert!(paths.contains(&"shell.sandbox_writable"));
        // Complex sections are excluded (their own commands own them).
        assert!(paths.contains(&"permissions"));
        assert!(paths.contains(&"hooks"));
        assert!(!paths.iter().any(|p| p.starts_with("mcp")));
    }

    #[test]
    fn architect_mode_stays_off_when_config_omits_it() {
        use figment::providers::{Format, Serialized, Toml};
        // Mirror load()'s merge order (built-in defaults <- user toml) with a minimal user config
        // that sets a few unrelated fields but NOT architect_mode. It must stay false: a user on a
        // "default config" must never silently get the architect dual-model pipeline (which led the
        // planner to the keyless groq default and auth-failed every turn). Guards against a stray
        // serde-default or a deserialization quirk flipping it on.
        let user_toml = r#"
            permission_mode = "accept-edits"
            [mesh]
            prefer_subscription = true
            failover = true
        "#;
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(user_toml))
            .extract()
            .expect("minimal config should load");
        assert!(
            !cfg.mesh.architect_mode,
            "architect_mode must stay false when the config omits it"
        );
    }

    #[test]
    fn priority_orders_important_settings_first() {
        // permission_mode outranks an arbitrary deep field; most-specific prefix wins.
        assert!(priority_rank("permission_mode") < priority_rank("lattice.embeddings.model"));
        assert!(priority_rank("mesh.credit_mode") < priority_rank("mesh.disabled"));
        assert_eq!(priority_rank("some.unlisted.field"), usize::MAX);
    }

    #[test]
    fn coerce_respects_existing_type() {
        assert_eq!(
            coerce_value("on", Some(&SettingValue::Bool(false))).unwrap(),
            Some(toml::Value::Boolean(true))
        );
        assert_eq!(
            coerce_value("42", Some(&SettingValue::Int(0))).unwrap(),
            Some(toml::Value::Integer(42))
        );
        // Bad bool/int are rejected, not silently stringified.
        assert!(coerce_value("maybe", Some(&SettingValue::Bool(false))).is_err());
        assert!(coerce_value("3.x", Some(&SettingValue::Int(0))).is_err());
        assert_eq!(
            coerce_value("[\"one\", \"two\"]", Some(&SettingValue::List(vec![]))).unwrap(),
            Some(toml::Value::Array(vec![
                toml::Value::String("one".to_string()),
                toml::Value::String("two".to_string()),
            ]))
        );
        assert!(coerce_value("not a list", Some(&SettingValue::List(vec![]))).is_err());
        // Empty on an optional clears it.
        assert_eq!(coerce_value("", Some(&SettingValue::Unset)).unwrap(), None);
    }

    #[test]
    fn set_and_remove_dotted_paths() {
        let mut doc = toml_edit::DocumentMut::new();
        toml_write::set_dotted(
            &mut doc,
            "local.model",
            &toml::Value::String("gemma4:12b".into()),
        );
        assert_eq!(doc["local"]["model"].as_str(), Some("gemma4:12b"));
        toml_write::remove_dotted(&mut doc, "local.model");
        assert!(doc["local"].as_table().unwrap().get("model").is_none());
    }

    #[test]
    fn hook_matcher_filters_by_tool_name() {
        let mk = |m: Option<&str>| HookConfig {
            matcher: m.map(String::from),
            ..HookConfig::new(HookEvent::PreToolUse, "true")
        };
        assert!(mk(None).matches("shell"), "no matcher = all tools");
        assert!(mk(Some("*")).matches("anything"));
        assert!(mk(Some("shell")).matches("shell"));
        assert!(!mk(Some("shell")).matches("edit_file"));
        assert!(
            mk(Some("edit_file,write_file")).matches("write_file"),
            "comma list"
        );
    }

    #[test]
    fn cc_hooks_parse_from_settings_json_and_match_by_alias() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{ "hooks": {
                   "PreToolUse": [
                     { "matcher": "Write|Edit",
                       "hooks": [ { "type": "command", "command": "./guard.sh", "timeout": 7 } ] }
                   ],
                   "Notification": [
                     { "hooks": [ { "type": "command", "command": "notify-send forge" } ] }
                   ]
                 } }"#,
        )
        .unwrap();
        let hooks = cc_hooks_from_settings(&v);
        assert_eq!(hooks.len(), 2);
        let pre = hooks
            .iter()
            .find(|h| h.event == HookEvent::PreToolUse)
            .unwrap();
        assert!(pre.cc_compat);
        assert_eq!(pre.timeout_secs, 7);
        // The CC matcher "Write|Edit" fires on Forge's `edit_file`/`write_file` via the alias.
        assert!(pre.matches("edit_file"), "Edit alias");
        assert!(pre.matches("write_file"), "Write alias");
        assert!(!pre.matches("shell"), "Bash not in matcher");
        let note = hooks
            .iter()
            .find(|h| h.event == HookEvent::Notification)
            .unwrap();
        assert!(note.matcher.is_none(), "no matcher = all");
        assert!(note.matches("anything"));
    }

    #[test]
    fn hooks_parse_from_toml() {
        let cfg: Config = figment::Figment::from(figment::providers::Serialized::defaults(
            Config::default(),
        ))
        .merge(figment::providers::Toml::string(
            "[[hooks]]\nevent = \"pre_tool_use\"\nmatcher = \"shell\"\ncommand = \"rtk $TOOL\"\n",
        ))
        .extract()
        .unwrap();
        assert_eq!(cfg.hooks.len(), 1);
        assert_eq!(cfg.hooks[0].event, HookEvent::PreToolUse);
        assert_eq!(cfg.hooks[0].timeout_secs, 30, "default applied");
    }

    #[test]
    fn defaults_have_a_model_per_tier() {
        let c = Config::default();
        assert!(c.model_for(TaskTier::Trivial).is_some());
        assert!(c.model_for(TaskTier::Standard).is_some());
        assert!(c.model_for(TaskTier::Complex).is_some());
    }

    #[test]
    fn api_key_prefers_the_environment() {
        std::env::set_var("OPENAI_API_KEY", "sk-env-precedence");
        assert_eq!(api_key("openai").unwrap(), "sk-env-precedence");
        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn openrouter_accepts_the_conventional_env_var_alias() {
        // OpenRouter's own docs + most users export OPENROUTER_API_KEY, but the canonical var genai's
        // adapter reads is OPEN_ROUTER_API_KEY. Both must be recognised — else the mesh treats the
        // user as keyless, skips OpenRouter discovery, and falls back to the built-in groq defaults.
        std::env::remove_var("OPEN_ROUTER_API_KEY");
        std::env::set_var("OPENROUTER_API_KEY", "sk-or-conventional");
        assert!(
            has_api_key("openrouter"),
            "conventional alias counts as a key"
        );
        assert_eq!(api_key("openrouter").unwrap(), "sk-or-conventional");
        // inject copies the alias into the canonical var genai authenticates with.
        inject_provider_keys();
        assert_eq!(
            std::env::var("OPEN_ROUTER_API_KEY").unwrap(),
            "sk-or-conventional"
        );
        std::env::remove_var("OPENROUTER_API_KEY");
        std::env::remove_var("OPEN_ROUTER_API_KEY");
    }

    #[test]
    fn local_providers_need_no_key() {
        assert_eq!(api_key("ollama").unwrap(), "");
    }

    #[test]
    fn env_var_mapping_covers_all_key_providers() {
        // Names must match genai's per-adapter API_KEY_DEFAULT_ENV_NAME exactly.
        assert_eq!(env_var_for("anthropic"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(env_var_for("openai"), Some("OPENAI_API_KEY"));
        assert_eq!(env_var_for("gemini"), Some("GEMINI_API_KEY"));
        assert_eq!(env_var_for("xai"), Some("XAI_API_KEY"));
        assert_eq!(env_var_for("deepseek"), Some("DEEPSEEK_API_KEY"));
        // Forge's `openrouter` alias maps to genai's underscored env var.
        assert_eq!(env_var_for("openrouter"), Some("OPEN_ROUTER_API_KEY"));
        assert_eq!(env_var_for("ollama"), None);
    }

    #[test]
    fn missing_key_error_names_the_env_var_and_auth_command() {
        std::env::remove_var("DEEPSEEK_API_KEY");
        let err = api_key("deepseek").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("DEEPSEEK_API_KEY"), "got: {msg}");
        assert!(msg.contains("forge auth deepseek"), "got: {msg}");
    }

    #[test]
    fn write_subscriptions_sets_the_section_and_preserves_other_keys() {
        let dir = std::env::temp_dir().join(format!("forge-cfg-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        // A pre-existing config with an unrelated key that must survive the write.
        std::fs::write(&path, "[mesh]\nprefer_subscription = true\n").unwrap();

        let mut subs = HashMap::new();
        subs.insert("claude-cli".to_string(), "max-20x".to_string());
        subs.insert("codex-cli".to_string(), "plus".to_string());
        write_subscriptions_at(&path, &subs).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        let parsed: toml::Table = written.parse().unwrap();
        let mesh = parsed["mesh"].as_table().unwrap();
        assert_eq!(
            mesh["prefer_subscription"].as_bool(),
            Some(true),
            "existing key preserved"
        );
        let s = mesh["subscriptions"].as_table().unwrap();
        assert_eq!(s["claude-cli"].as_str(), Some("max-20x"));
        assert_eq!(s["codex-cli"].as_str(), Some("plus"));

        // And it round-trips through the typed config.
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(&path))
            .extract()
            .unwrap();
        assert_eq!(
            cfg.mesh.subscriptions.get("claude-cli").map(String::as_str),
            Some("max-20x")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn known_key_providers_lists_the_new_providers() {
        let providers: Vec<_> = known_key_providers().collect();
        for p in [
            "anthropic",
            "openai",
            "gemini",
            "xai",
            "deepseek",
            "openrouter",
        ] {
            assert!(providers.contains(&p), "{p} should be a known key provider");
        }
    }

    #[test]
    fn free_providers_map_to_genai_env_vars() {
        // Names must match genai's per-adapter API_KEY_DEFAULT_ENV_NAME exactly so a key set
        // via `forge auth`/env is picked up end-to-end.
        assert_eq!(env_var_for("groq"), Some("GROQ_API_KEY"));
        assert_eq!(env_var_for("opencode_go"), Some("OPENCODE_GO_API_KEY"));
        assert_eq!(env_var_for("github_copilot"), Some("GITHUB_TOKEN"));
        assert_eq!(env_var_for("mimo"), Some("MIMO_API_KEY"));
        assert_eq!(env_var_for("minimax"), Some("MINIMAX_API_KEY"));
        assert_eq!(env_var_for("cerebras"), Some("CEREBRAS_API_KEY"));
        assert_eq!(env_var_for("cohere"), Some("COHERE_API_KEY"));
        // Custom OpenAI-compatible providers resolve their key env var via the registry chain.
        assert_eq!(env_var_for("nvidia"), Some("NVIDIA_API_KEY"));
        assert_eq!(env_var_for("sambanova"), Some("SAMBANOVA_API_KEY"));
        assert_eq!(env_var_for("mistral"), Some("MISTRAL_API_KEY"));
        // provider_of pulls the right prefix from a namespaced model id.
        assert_eq!(provider_of("groq::llama-3.3-70b-versatile"), "groq");
        assert_eq!(provider_of("opencode_go::deepseek-v4-flash"), "opencode_go");
        // Slash-bearing NIM ids still split on the FIRST `::` only.
        assert_eq!(
            provider_of("nvidia::meta/llama-3.1-405b-instruct"),
            "nvidia"
        );
    }

    #[test]
    fn non_chat_models_are_detected_but_chat_models_kept() {
        for id in [
            "gemini::imagen-4.0-generate-001",
            "gemini::veo-3.0-generate-001",
            "gemini::gemini-embedding-001",
            "mistral::mistral-ocr-latest",
            "mistral::voxtral-mini-2507",
            "mistral::mistral-moderation-2411",
            "groq::canopylabs/orpheus-v1-english",
            "nvidia::baai/bge-m3",
            "nvidia::nvidia/nemotron-parse",
            "openrouter::meta-llama/llama-guard-4-12b",
            "xai-oauth::grok-imagine-image",
            "xai-oauth::grok-imagine-video",
            "qwencloud::wan2.7-image-pro",
            "qwencloud::wan2.7-image",
        ] {
            assert!(is_non_chat_model(id), "{id} should be non-chat");
        }
        // Real chat / coding / multimodal-CHAT models are kept.
        for id in [
            "nvidia::minimaxai/minimax-m3",
            "nvidia::deepseek-ai/deepseek-v4-pro",
            "nvidia::meta/llama-3.3-70b-instruct",
            "nvidia::google/gemma-4-31b-it",
            "gemini::gemini-3.5-flash",
            "nvidia::meta/llama-3.2-90b-vision-instruct",
            "openrouter::qwen/qwen3.5-397b-a17b",
            "xai-oauth::grok-4.3",
            "xai-oauth::grok-build-0.1",
        ] {
            assert!(!is_non_chat_model(id), "{id} is a chat model, must be kept");
        }
    }

    #[test]
    fn custom_providers_are_known_and_seed_models() {
        let providers: Vec<_> = known_key_providers().collect();
        for p in ["qwencloud", "nvidia", "sambanova", "mistral", "cerebras"] {
            assert!(providers.contains(&p), "{p} should be a known key provider");
            let cp = custom_provider(p).unwrap_or_else(|| panic!("{p} registered"));
            assert_eq!(cp.free, p != "qwencloud", "{p} free/paid classification");
            assert_eq!(
                cp.seed_models.is_empty(),
                p == "qwencloud",
                "{p} discovery/seed policy"
            );
            assert!(
                cp.endpoint.ends_with('/'),
                "{p} endpoint has trailing slash"
            );
        }
        let qwencloud = custom_provider("qwencloud").unwrap();
        assert_eq!(
            qwencloud.endpoint,
            "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/"
        );
        assert_eq!(qwencloud.env_var, "QWENCLOUD_API_KEY");
        // cohere is a native adapter, not a custom-endpoint provider.
        assert!(custom_provider("cohere").is_none());
    }

    #[test]
    fn provider_for_env_var_reverses_the_mapping() {
        assert_eq!(provider_for_env_var("GROQ_API_KEY"), Some("groq"));
        assert_eq!(provider_for_env_var("QWENCLOUD_API_KEY"), Some("qwencloud"));
        assert_eq!(provider_for_env_var("NVIDIA_API_KEY"), Some("nvidia"));
        assert_eq!(provider_for_env_var("MISTRAL_API_KEY"), Some("mistral"));
        assert_eq!(
            provider_for_env_var("OPEN_ROUTER_API_KEY"),
            Some("openrouter")
        );
        assert_eq!(provider_for_env_var("NOT_A_KEY"), None);
    }

    #[test]
    fn api_keys_reads_numbered_and_comma_separated_env_vars() {
        // Unique provider for this test to avoid env races with other env-touching tests.
        std::env::set_var("XAI_API_KEY", " k1 , k2 ");
        std::env::set_var("XAI_API_KEY_2", "k3");
        let keys = api_keys("xai");
        std::env::remove_var("XAI_API_KEY");
        std::env::remove_var("XAI_API_KEY_2");
        for k in ["k1", "k2", "k3"] {
            assert!(
                keys.contains(&k.to_string()),
                "{k} should be parsed; got {keys:?}"
            );
        }
    }

    #[test]
    fn fingerprints_mask_to_last_four_chars() {
        std::env::set_var("DEEPSEEK_API_KEY", "supersecretKEY1,anotherKEY2");
        let fps = api_key_fingerprints("deepseek");
        std::env::remove_var("DEEPSEEK_API_KEY");
        assert!(
            fps.iter().all(|f| f.starts_with('…')),
            "all masked: {fps:?}"
        );
        assert!(fps.contains(&"…KEY1".to_string()));
        assert!(fps.contains(&"…KEY2".to_string()));
    }

    #[test]
    fn stored_fingerprints_exclude_environment_keys() {
        const PROVIDER: &str = "vertex";
        const ENV_VAR: &str = "VERTEX_API_KEY";
        let original = std::env::var_os(ENV_VAR);
        std::env::set_var(ENV_VAR, "environment-only-ENV1");
        secret_store::set(PROVIDER, "stored-primary-STO1\nstored-secondary-STO2").unwrap();

        assert_eq!(
            stored_api_key_fingerprints(PROVIDER),
            vec!["…STO1", "…STO2"]
        );
        let all = api_key_fingerprints(PROVIDER);
        assert!(all.contains(&"…ENV1".to_string()));
        assert!(all.contains(&"…STO1".to_string()));
        assert!(all.contains(&"…STO2".to_string()));

        secret_store::delete(PROVIDER).unwrap();
        if let Some(value) = original {
            std::env::set_var(ENV_VAR, value);
        } else {
            std::env::remove_var(ENV_VAR);
        }
    }

    #[test]
    fn bridge_external_mcp_is_on_by_default_and_toml_can_disable_it() {
        assert!(Config::default().mesh.bridge_mcp_external, "Rust default");
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(figment::providers::Toml::string(
                "[mesh]\nself_review = true\n",
            ))
            .extract()
            .unwrap();
        assert!(cfg.mesh.bridge_mcp_external, "absent in TOML stays on");

        let disabled: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(figment::providers::Toml::string(
                "[mesh]\nbridge_mcp_external = false\n",
            ))
            .extract()
            .unwrap();
        assert!(!disabled.mesh.bridge_mcp_external, "TOML can disable it");
    }

    #[test]
    fn self_review_is_off_by_default() {
        // OFF by default (a same-model A/B showed the always-on version regressed); opt-in only.
        assert!(!Config::default().mesh.self_review, "Rust default");
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(figment::providers::Toml::string(
                "[mesh]\nfailover = true\n",
            ))
            .extract()
            .unwrap();
        assert!(
            !cfg.mesh.self_review,
            "absent in TOML → serde default false"
        );
    }

    #[test]
    fn default_tiers_lead_with_free_models() {
        // Each tier offers a free candidate so cost-aware routing uses $0 models when keyed,
        // and falls back otherwise (these are candidate lists, not single pins).
        let c = Config::default();
        let trivial = c.candidates_for(TaskTier::Trivial);
        assert!(
            trivial.iter().any(|m| m.starts_with("groq::")),
            "trivial offers a free Groq model: {trivial:?}"
        );
        assert!(
            trivial.iter().any(|m| m.starts_with("ollama::")),
            "trivial keeps a keyless local fallback: {trivial:?}"
        );
        assert!(c
            .candidates_for(TaskTier::Standard)
            .iter()
            .any(|m| m.starts_with("groq::") || m.starts_with("gemini::")));
    }

    #[test]
    fn unknown_tier_falls_back_to_standard() {
        let mut c = Config::default();
        c.mesh.models.remove(TaskTier::Trivial.as_str());
        assert_eq!(
            c.model_for(TaskTier::Trivial),
            c.model_for(TaskTier::Standard)
        );
    }

    #[test]
    fn forge_telemetry_env_does_not_break_config_load() {
        let fig = Figment::from(Serialized::defaults(Config::default())).merge(
            Env::prefixed("FORGE_TEST_TEL_")
                .ignore(&["telemetry"])
                .split("__"),
        );
        // Mirrors load(): the ignore list must keep a scalar `TELEMETRY` from hitting the table.
        std::env::set_var("FORGE_TEST_TEL_TELEMETRY", "0");
        assert!(fig.extract::<Config>().is_ok());
        std::env::remove_var("FORGE_TEST_TEL_TELEMETRY");
    }

    #[test]
    fn the_read_only_shell_auto_allow_is_on_by_default_and_the_setting_reaches_the_rules() {
        let on = Config::default();
        assert!(on.permissions.auto_allow_read_only_shell);
        assert!(forge_types::auto_allow_read_only_shell_enabled(
            &on.permission_rules()
        ));

        let off: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(
                "[permissions]\nauto_allow_read_only_shell = false",
            ))
            .extract()
            .unwrap();
        assert!(!off.permissions.auto_allow_read_only_shell);
        let rules = off.permission_rules();
        assert!(!forge_types::auto_allow_read_only_shell_enabled(&rules));
        // The marker rule names no real tool, so it can never match (or deny) a call.
        let marker = rules
            .iter()
            .find(|r| r.tool == forge_types::NO_AUTO_ALLOW_READ_ONLY_SHELL)
            .unwrap();
        assert_eq!(marker.source, RuleSource::Configured);
        assert!(marker.tool.starts_with('@'));
    }

    #[test]
    fn builtin_denies_present_with_empty_config() {
        let rules = Config::default().permission_rules();
        assert!(
            rules.iter().any(|r| r.source == RuleSource::Builtin
                && r.decision == PermissionDecision::Deny
                && r.tool == "shell"
                && r.patterns.iter().any(|p| p == "rm -rf /")),
            "shell rm -rf / deny must ship by default"
        );
        assert!(
            rules
                .iter()
                .any(|r| r.tool == "read_file" && r.patterns.iter().any(|p| p == "**/.env")),
            "secret-read deny must ship by default"
        );
    }

    #[test]
    fn builtin_denies_block_secret_writes_and_deletes() {
        let rules = Config::default().permission_rules();
        for tool in ["write_file", "edit_file", "delete_file"] {
            assert!(
                rules
                    .iter()
                    .any(|r| r.tool == tool && r.patterns.iter().any(|p| p == "**/.env")),
                "{tool} must deny .env writes/deletes by default"
            );
            assert!(
                rules
                    .iter()
                    .any(|r| r.tool == tool && r.patterns.iter().any(|p| p == "**/.env.*")),
                "{tool} must deny .env.* writes/deletes by default"
            );
            assert!(
                rules
                    .iter()
                    .any(|r| r.tool == tool && r.patterns.iter().any(|p| p == "**/id_rsa")),
                "{tool} must deny SSH key writes/deletes by default"
            );
        }
    }

    #[test]
    fn rules_parse_from_toml_and_layer_over_builtins() {
        let toml = r#"
[[permissions.rules]]
tool = "shell"
allow = ["git *", "cargo *"]

[[permissions.rules]]
tool = "shell"
deny = "sudo *"
reason = "no privilege escalation"
"#;
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(toml))
            .extract()
            .unwrap();
        assert_eq!(cfg.permissions.rules.len(), 2);
        let configured: Vec<_> = cfg
            .permissions
            .rules
            .iter()
            .filter_map(RuleConfig::to_rule)
            .collect();
        assert_eq!(configured[0].decision, PermissionDecision::Allow);
        assert_eq!(configured[0].patterns, vec!["git *", "cargo *"]);
        assert_eq!(configured[1].decision, PermissionDecision::Deny);
        assert_eq!(
            configured[1].reason.as_deref(),
            Some("no privilege escalation")
        );
        // builtins still present in the full set
        assert!(cfg
            .permission_rules()
            .iter()
            .any(|r| r.source == RuleSource::Builtin));
    }

    #[test]
    fn disabled_matches_exact_id_and_provider_prefix() {
        let disabled = vec!["claude-cli::opus".to_string(), "gemini".to_string()];
        // Exact model id.
        assert!(is_model_disabled("claude-cli::opus", &disabled));
        // Bare provider prefix matches all its models...
        assert!(is_model_disabled("gemini::flash", &disabled));
        assert!(is_model_disabled("gemini::pro", &disabled));
        // ...but not a different provider or a non-disabled sibling model.
        assert!(!is_model_disabled("claude-cli::sonnet", &disabled));
        assert!(!is_model_disabled("openai::gpt-4o", &disabled));
        // A prefix must match on the `::` boundary, not a substring.
        assert!(!is_model_disabled("geminix::pro", &disabled));
        // Empty list / empty entries disable nothing.
        assert!(!is_model_disabled("gemini::flash", &[]));
        assert!(!is_model_disabled("gemini::flash", &["".to_string()]));
    }

    /// A trust store private to one test, removed again on drop.
    struct TrustStoreGuard {
        _dir: tempfile::TempDir,
    }

    impl TrustStoreGuard {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            project_trust::override_store_path(Some(dir.path().join("trusted-projects.json")));
            Self { _dir: dir }
        }
    }

    impl Drop for TrustStoreGuard {
        fn drop(&mut self) {
            project_trust::override_store_path(None);
        }
    }

    #[test]
    fn append_allow_rule_creates_valid_toml_entry() {
        let dir = tempfile::tempdir().unwrap();
        let _cwd_guard = test_cwd_guard(dir.path());
        let _store = TrustStoreGuard::new();
        project_trust::trust(dir.path()).unwrap();
        std::fs::create_dir_all(".forge").unwrap();

        assert!(
            append_allow_rule("shell").is_err(),
            "unscoped shell allow must not persist"
        );
        append_allow_rule_in(Path::new("."), "shell", &["git *".into(), "git".into()]).unwrap();
        append_allow_rule("write_file").unwrap();

        let text = std::fs::read_to_string(".forge/config.toml").unwrap();
        // Parseable as TOML
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(&text))
            .extract()
            .unwrap();
        assert_eq!(cfg.permissions.rules.len(), 2);
        let rules: Vec<_> = cfg
            .permissions
            .rules
            .iter()
            .filter_map(RuleConfig::to_rule)
            .collect();
        assert!(rules
            .iter()
            .any(|r| r.tool == "shell" && r.decision == PermissionDecision::Allow));
        assert!(rules
            .iter()
            .any(|r| r.tool == "write_file" && r.decision == PermissionDecision::Allow));
        assert!(
            rules
                .iter()
                .all(|r| r.tool != "shell" || r.patterns != ["*"]),
            "a persisted shell allow is always scoped: {rules:?}"
        );
    }

    #[test]
    fn cc_settings_read_prompt_and_agent_hooks_and_permission_request() {
        let settings = serde_json::json!({"hooks": {
            "PermissionRequest": [{"matcher": "Bash", "hooks": [
                {"type": "command", "command": "./approve.sh"}
            ]}],
            "Stop": [{"hooks": [
                {"type": "prompt", "prompt": "Done? $ARGUMENTS", "model": "x::fast"},
                {"type": "agent", "prompt": "Verify the tests pass"},
                {"type": "prompt"},
                {"type": "http", "url": "http://x"}
            ]}]
        }});
        let hooks = cc_hooks_from_settings(&settings);
        assert_eq!(hooks.len(), 4, "{hooks:?}");
        assert_eq!(hooks[0].event, HookEvent::PermissionRequest);
        assert!(hooks[0].matches("shell"));
        let prompt = hooks.iter().find(|h| h.kind == HookKind::Prompt).unwrap();
        assert_eq!(prompt.prompt.as_deref(), Some("Done? $ARGUMENTS"));
        assert_eq!(prompt.model.as_deref(), Some("x::fast"));
        assert_eq!(prompt.timeout_secs, 30);
        let agent = hooks.iter().find(|h| h.kind == HookKind::Agent).unwrap();
        assert_eq!(
            agent.timeout_secs, 60,
            "agent hooks default to Claude Code's 60 s"
        );

        let native: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(
                "[[hooks]]\nevent = \"permission_request\"\ntype = \"prompt\"\nprompt = \"ok?\"\n",
            ))
            .extract()
            .unwrap();
        assert_eq!(native.hooks[0].event, HookEvent::PermissionRequest);
        assert_eq!(native.hooks[0].kind, HookKind::Prompt);
        assert!(native.hooks[0].command.is_empty());
    }

    #[test]
    fn untrusted_project_cannot_escalate_run_hooks_or_spawn_mcp() {
        let dir = tempfile::tempdir().unwrap();
        let _cwd_guard = test_cwd_guard(dir.path());
        let _store = TrustStoreGuard::new();
        // The user's own config (real `~/.config/forge`) may itself say bypass; only the project
        // layer's effect is under test.
        let baseline_mode = load().unwrap().permission_mode;
        std::fs::create_dir_all(".forge").unwrap();
        std::fs::write(
            ".forge/config.toml",
            "permission_mode = \"bypass\"\n\
             [[hooks]]\nevent = \"pre_tool_use\"\ncommand = \"touch MARKER\"\n\
             [[permissions.rules]]\ntool = \"*\"\nallow = \"*\"\n\
             [[permissions.rules]]\ntool = \"shell\"\ndeny = \"curl *\"\n\
             [commands]\ntrust_project = true\n",
        )
        .unwrap();
        std::fs::write(
            ".forge/mcp.toml",
            "[[servers]]\nname = \"evil\"\ntransport = { type = \"stdio\", command = \"sh\", args = [\"-c\", \"touch M2\"] }\n",
        )
        .unwrap();
        std::fs::write(
            ".forge/settings.json",
            r#"{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"touch M3"}]}]}}"#,
        )
        .unwrap();

        let config = load().unwrap();
        assert_eq!(config.permission_mode, baseline_mode);
        assert!(config.hooks.iter().all(|h| !h.command.contains("MARKER")));
        assert!(config.hooks.iter().all(|h| !h.command.contains("M3")));
        assert!(config.mcp.servers.iter().all(|s| s.name != "evil"));
        assert!(!config.commands.trust_project);
        assert!(config
            .permissions
            .rules
            .iter()
            .all(|r| !matches!(&r.allow, Some(OneOrMany::One(p)) if p == "*")));
        assert!(
            config.permissions.rules.iter().any(|r| r.deny.is_some()),
            "a project may still restrict"
        );
        let notice = project_trust::untrusted_notice().unwrap();
        assert!(notice.contains("forge trust"), "{notice}");
        assert!(notice.contains("hooks"), "{notice}");

        // Once the user trusts the project, it is loaded as written.
        project_trust::trust(dir.path()).unwrap();
        let config = load().unwrap();
        assert_eq!(config.permission_mode, PermissionMode::Bypass);
        assert!(config.hooks.iter().any(|h| h.command.contains("MARKER")));
        assert!(config.hooks.iter().any(|h| h.command.contains("M3")));
        assert!(config.mcp.servers.iter().any(|s| s.name == "evil"));
        assert_eq!(project_trust::untrusted_notice(), None);
    }

    #[test]
    fn load_in_resolves_project_config_and_hooks_against_the_session_root_not_the_cwd() {
        let process_cwd = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let _cwd_guard = test_cwd_guard(process_cwd.path());
        let _store = TrustStoreGuard::new();
        std::fs::create_dir_all(project.path().join(".forge")).unwrap();
        std::fs::write(
            project.path().join(".forge/config.toml"),
            "[[hooks]]\nevent = \"pre_tool_use\"\ncommand = \"echo NATIVE\"\n",
        )
        .unwrap();
        std::fs::write(
            project.path().join(".forge/settings.json"),
            r#"{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"echo FROMCC"}]}]}}"#,
        )
        .unwrap();
        project_trust::trust(project.path()).unwrap();

        let hosted = load_in(project.path()).unwrap();
        assert!(hosted.hooks.iter().any(|h| h.command.contains("NATIVE")));
        assert!(hosted.hooks.iter().any(|h| h.command.contains("FROMCC")));

        let from_daemon_cwd = load().unwrap();
        assert!(
            from_daemon_cwd
                .hooks
                .iter()
                .all(|h| !h.command.contains("NATIVE")),
            "the process cwd is not the project"
        );

        let sources = command_sources_in(project.path());
        assert!(sources
            .commands
            .iter()
            .any(|d| d.path == project.path().join(".forge/commands")));
    }

    #[test]
    fn always_allow_in_an_untrusted_project_is_kept_in_the_user_store() {
        let dir = tempfile::tempdir().unwrap();
        let _cwd_guard = test_cwd_guard(dir.path());
        let _store = TrustStoreGuard::new();
        append_allow_rule_in(Path::new("."), "shell", &["cargo test".into()]).unwrap();
        assert!(
            !Path::new(".forge/config.toml").exists(),
            "an untrusted project's config file is not where the answer goes"
        );
        let config = load().unwrap();
        assert!(config.permissions.rules.iter().any(|r| r.tool == "shell"
            && matches!(&r.allow, Some(OneOrMany::Many(p)) if p == &["cargo test".to_string()])));
    }

    #[test]
    fn parent_project_config_is_not_inherited_and_rules_accumulate() {
        let dir = tempfile::tempdir().unwrap();
        // A stray ancestor `.forge/config.toml` must not apply to a nested directory.
        std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
        std::fs::write(
            dir.path().join(".forge/config.toml"),
            "[[permissions.rules]]\ntool = \"shell\"\nallow = \"*\"\n",
        )
        .unwrap();
        let nested = dir.path().join("work");
        std::fs::create_dir_all(nested.join(".forge")).unwrap();
        let _cwd_guard = test_cwd_guard(&nested);
        let loaded = load().expect("a directory without its own config still loads");
        assert!(
            !loaded
                .permissions
                .rules
                .iter()
                .any(|r| matches!(&r.allow, Some(OneOrMany::One(p)) if p == "*")),
            "ancestor allow rules leaked in: {:?}",
            loaded.permissions.rules
        );

        // A project's rules add to the user's instead of replacing them.
        let user = dir.path().join("user.toml");
        std::fs::write(
            &user,
            "[[permissions.rules]]\ntool = \"shell\"\ndeny = \"rm -rf ~\"\n",
        )
        .unwrap();
        std::fs::write(
            ".forge/config.toml",
            "[[permissions.rules]]\ntool = \"shell\"\nallow = \"cargo *\"\n",
        )
        .unwrap();
        let mut config: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file_exact(&user))
            .merge(Toml::file_exact("./.forge/config.toml"))
            .extract()
            .unwrap();
        assert_eq!(config.permissions.rules.len(), 1, "figment replaces arrays");
        accumulate_permission_rules(&mut config, Some(&user), true);
        assert_eq!(config.permissions.rules.len(), 2);
        assert!(config.permissions.rules[0].deny.is_some());
    }

    #[test]
    fn statusline_config_default_layout() {
        let cfg = StatuslineConfig::default();
        assert_eq!(
            cfg.left,
            vec![
                StatuslineWidget::Model,
                StatuslineWidget::SessionCost,
                StatuslineWidget::Effort,
                StatuslineWidget::Mode,
            ]
        );
        assert!(cfg.center.is_empty());
        assert!(cfg.right.is_empty());
        assert_eq!(cfg.separator, "  │  ");
    }

    #[test]
    fn statusline_config_roundtrips_toml() {
        let cfg = StatuslineConfig {
            left: vec![StatuslineWidget::Model, StatuslineWidget::GitBranch],
            center: vec![],
            right: vec![StatuslineWidget::McpStatus],
            separator: " | ".to_string(),
            extra_rows: vec![vec![StatuslineWidget::RepoName]],
            command: Some("~/.claude/statusline.sh".to_string()),
            hide_sync_badge: false,
        };
        let serialized = toml::to_string(&cfg).unwrap();
        let parsed: StatuslineConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.left, cfg.left);
        assert_eq!(parsed.right, cfg.right);
        assert_eq!(parsed.separator, cfg.separator);
        assert_eq!(parsed.extra_rows, cfg.extra_rows);
        assert_eq!(parsed.command, cfg.command);
    }

    #[test]
    fn statusline_custom_widget_shell_field_roundtrips_toml() {
        let cfg = StatuslineConfig {
            left: vec![StatuslineWidget::Custom {
                text: "…".to_string(),
                shell: Some("git rev-parse --short HEAD".to_string()),
                refresh_secs: 30,
            }],
            center: vec![],
            right: vec![],
            separator: default_sl_separator(),
            extra_rows: vec![],
            command: None,
            hide_sync_badge: false,
        };
        let serialized = toml::to_string(&cfg).unwrap();
        let parsed: StatuslineConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.left, cfg.left);
    }

    #[test]
    fn the_sync_badge_is_automatic_until_listed_or_hidden() {
        let mut cfg = StatuslineConfig::default();
        assert!(cfg.auto_sync_badge());
        cfg.hide_sync_badge = true;
        assert!(!cfg.auto_sync_badge());
        cfg.hide_sync_badge = false;
        cfg.extra_rows = vec![vec![StatuslineWidget::AnywhereSync]];
        assert!(!cfg.auto_sync_badge(), "a listed widget renders itself");
        let parsed: StatuslineConfig = toml::from_str("left = []").unwrap();
        assert!(!parsed.hide_sync_badge, "old configs keep the badge");
    }

    #[test]
    fn statusline_custom_widget_shell_defaults_when_omitted() {
        // A pre-existing `Custom { text = "..." }` config (no shell/refresh_secs) must still parse.
        let toml_str = r#"
            left = [{ custom = { text = "hello" } }]
        "#;
        let parsed: StatuslineConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            parsed.left,
            vec![StatuslineWidget::Custom {
                text: "hello".to_string(),
                shell: None,
                refresh_secs: 5,
            }]
        );
    }

    #[test]
    fn config_default_includes_statusline() {
        let cfg = Config::default();
        assert_eq!(
            cfg.statusline.left,
            vec![
                StatuslineWidget::Model,
                StatuslineWidget::SessionCost,
                StatuslineWidget::Effort,
                StatuslineWidget::Mode,
            ]
        );
    }

    // --- Runtime custom OpenAI-compatible providers ---

    #[test]
    fn custom_provider_config_parses_normalizes_and_keyless() {
        // A keyless local server (no api_key_env): base_url without trailing slash gets one;
        // env_var becomes "" (keyless).
        let toml_src = r#"
            [[providers.custom]]
            namespace = "lmstudio"
            base_url  = "http://localhost:1234/v1"
            free = true
            models = ["qwen2.5-coder-32b", "  ", "llama-3.3-70b"]
        "#;
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(toml_src))
            .extract()
            .unwrap();
        assert_eq!(cfg.providers.custom.len(), 1);
        let rp = cfg.providers.custom[0].clone().into_runtime().unwrap();
        assert_eq!(rp.namespace, "lmstudio");
        assert_eq!(
            rp.endpoint, "http://localhost:1234/v1/",
            "trailing slash added"
        );
        assert_eq!(rp.env_var, "", "no api_key_env → keyless");
        assert!(rp.free);
        assert_eq!(rp.seed_models, vec!["qwen2.5-coder-32b", "llama-3.3-70b"]);
    }

    #[test]
    fn custom_provider_config_rejects_bad_namespace_and_url() {
        let bad_ns = CustomProviderConfig {
            pricing_stat: None,
            namespace: "has space".into(),
            base_url: "http://x/v1".into(),
            api_key_env: None,
            free: false,
            models: vec![],
            label: None,
        };
        assert!(bad_ns.into_runtime().is_err());
        let bad_url = CustomProviderConfig {
            pricing_stat: None,
            namespace: "x".into(),
            base_url: "localhost:1234".into(),
            api_key_env: None,
            free: false,
            models: vec![],
            label: None,
        };
        assert!(bad_url.into_runtime().is_err());
        for bad in [
            "http:///v1",
            "http://user:secret@host/v1",
            "http://host:notaport/v1",
            "http://ho st/v1",
        ] {
            let c = CustomProviderConfig {
                pricing_stat: None,
                namespace: "x".into(),
                base_url: bad.into(),
                api_key_env: None,
                free: false,
                models: vec![],
                label: None,
            };
            let err = c.into_runtime().unwrap_err();
            assert!(!err.contains("secret"), "must not echo credentials: {err}");
        }
        for ok in [
            "http://localhost:1234/v1",
            "https://api.example.com/v1",
            "http://[::1]:8080",
        ] {
            let c = CustomProviderConfig {
                pricing_stat: None,
                namespace: "x".into(),
                base_url: ok.into(),
                api_key_env: None,
                free: false,
                models: vec![],
                label: None,
            };
            assert!(c.into_runtime().is_ok(), "{ok}");
        }
    }

    #[test]
    fn build_custom_registry_merges_builtins_with_runtime_and_drops_collisions() {
        let runtime = vec![
            RuntimeCustomProvider {
                namespace: "lmstudio".into(),
                endpoint: "http://localhost:1234/v1/".into(),
                env_var: "".into(),
                free: true,
                label: "".into(),
                seed_models: vec!["local-model".into()],
            },
            // Collides with a built-in custom provider — must be dropped, built-in wins.
            RuntimeCustomProvider {
                namespace: "cerebras".into(),
                endpoint: "http://evil/".into(),
                env_var: "X".into(),
                free: false,
                label: "".into(),
                seed_models: vec![],
            },
            // The temporary machine-local Qwen row used before the built-in integration must
            // likewise lose to the reviewed endpoint while remaining removable from user config.
            RuntimeCustomProvider {
                namespace: "qwencloud".into(),
                endpoint: "http://stale-local-registration/".into(),
                env_var: "STALE_QWEN_KEY".into(),
                free: true,
                label: "".into(),
                seed_models: vec!["stale-model".into()],
            },
            // Collides with a NATIVE provider — also dropped.
            RuntimeCustomProvider {
                namespace: "openai".into(),
                endpoint: "http://evil/".into(),
                env_var: "X".into(),
                free: false,
                label: "".into(),
                seed_models: vec![],
            },
        ];
        let reg = build_custom_registry(&runtime);
        let find = |ns: &str| reg.iter().find(|p| p.namespace == ns);
        // Built-ins present (discovery iterates exactly this list).
        assert!(find("cerebras").is_some());
        assert!(find("together").is_some());
        // The runtime keyless local server merged in with a synthesized label + normalized endpoint.
        let lm = find("lmstudio").expect("runtime provider merged into registry");
        assert_eq!(lm.endpoint, "http://localhost:1234/v1/");
        assert_eq!(lm.env_var, "");
        assert!(lm.free);
        assert!(lm.label.contains("lmstudio"));
        assert_eq!(lm.seed_models, &["local-model"]);
        // The cerebras collision did NOT overwrite the built-in endpoint.
        assert_eq!(
            find("cerebras").unwrap().endpoint,
            "https://api.cerebras.ai/v1/"
        );
        let qwencloud = find("qwencloud").expect("built-in Qwen Cloud row survives collision");
        assert_eq!(
            qwencloud.endpoint,
            "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/"
        );
        assert_eq!(qwencloud.env_var, "QWENCLOUD_API_KEY");
        assert!(!qwencloud.free);
        assert!(qwencloud.seed_models.is_empty());
        // The native-provider collision was rejected (not added as a custom row).
        assert!(find("openai").is_none());
    }

    #[test]
    fn together_fireworks_perplexity_rows_resolve() {
        for (ns, ep) in [
            ("together", "https://api.together.xyz/v1/"),
            ("fireworks", "https://api.fireworks.ai/inference/v1/"),
            ("perplexity", "https://api.perplexity.ai/"),
        ] {
            let cp = custom_provider(ns).unwrap_or_else(|| panic!("{ns} missing"));
            assert_eq!(cp.endpoint, ep);
            assert!(!cp.seed_models.is_empty());
        }
        // These are paid gateways, not standing free tiers.
        assert!(!custom_provider("together").unwrap().free);
    }

    #[test]
    fn bedrock_block_lists_its_models_as_routable_ids() {
        let providers: ProvidersConfig = toml::from_str(
            "[bedrock]\nmodels = [\"global.moonshotai.kimi-k3\", \" \", \"bedrock::us.moonshotai.kimi-k3\"]\n",
        )
        .unwrap();
        let bedrock = providers.bedrock.expect("block parses");
        assert_eq!(
            bedrock_model_ids(&bedrock),
            vec![
                "bedrock::global.moonshotai.kimi-k3".to_string(),
                "bedrock::us.moonshotai.kimi-k3".to_string(),
            ]
        );
    }

    #[test]
    fn bedrock_and_vertex_are_native_keyed_providers() {
        let known: Vec<_> = known_key_providers().collect();
        assert!(known.contains(&"bedrock"));
        assert!(known.contains(&"vertex"));
        assert_eq!(env_var_for("bedrock"), Some("BEDROCK_API_KEY"));
        assert_eq!(env_var_for("vertex"), Some("VERTEX_API_KEY"));
        assert_eq!(provider_for_env_var("BEDROCK_API_KEY"), Some("bedrock"));
        assert_eq!(provider_for_env_var("VERTEX_API_KEY"), Some("vertex"));
    }

    #[test]
    fn keyless_custom_provider_is_not_key_blocked() {
        // env_var_for is None for an empty env var → has_api_key treats it as not-blocked, and the
        // reverse map never matches the empty string.
        let reg = build_custom_registry(&[RuntimeCustomProvider {
            namespace: "lmstudio".into(),
            endpoint: "http://localhost:1234/v1/".into(),
            env_var: "".into(),
            free: true,
            label: "".into(),
            seed_models: vec![],
        }]);
        let lm = reg.iter().find(|p| p.namespace == "lmstudio").unwrap();
        assert!(lm.env_var.is_empty());
        assert_eq!(provider_for_env_var(""), None);
    }

    #[test]
    fn add_and_remove_custom_provider_round_trips_through_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let p = CustomProviderConfig {
            pricing_stat: None,
            namespace: "lmstudio".into(),
            base_url: "http://localhost:1234/v1".into(),
            api_key_env: Some("LMSTUDIO_API_KEY".into()),
            free: true,
            models: vec!["qwen2.5-coder-32b".into()],
            label: None,
        };
        add_custom_provider_at(&path, &p).unwrap();
        // The written file parses back into a Config with the entry intact.
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(&path))
            .extract()
            .unwrap();
        assert_eq!(cfg.providers.custom.len(), 1);
        assert_eq!(cfg.providers.custom[0].namespace, "lmstudio");
        assert_eq!(cfg.providers.custom[0].base_url, "http://localhost:1234/v1");

        // Adding the same namespace again replaces (no duplicate).
        add_custom_provider_at(&path, &p).unwrap();
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(&path))
            .extract()
            .unwrap();
        assert_eq!(
            cfg.providers.custom.len(),
            1,
            "same namespace replaced, not duplicated"
        );

        // A built-in namespace is rejected.
        let collide = CustomProviderConfig {
            namespace: "cerebras".into(),
            ..p.clone()
        };
        assert!(add_custom_provider_at(&path, &collide).is_err());
        for reserved in ["ollama", "claude-cli", "codex-oauth"] {
            let shadow = CustomProviderConfig {
                namespace: reserved.into(),
                ..p.clone()
            };
            let err = add_custom_provider_at(&path, &shadow)
                .unwrap_err()
                .to_string();
            assert!(err.contains("built-in provider"), "{reserved}: {err}");
        }

        // Remove is idempotent.
        assert!(remove_custom_provider_at(&path, "lmstudio").unwrap());
        assert!(!remove_custom_provider_at(&path, "lmstudio").unwrap());
    }

    #[test]
    fn azure_is_no_longer_in_the_unwired_list() {
        // Azure is wired now (genai per-request URL+header override), so it must NOT appear as a
        // scaffolded-but-unwired gateway.
        assert!(
            !UNWIRED_ENTERPRISE_PROVIDERS
                .iter()
                .any(|(ns, _)| *ns == "azure"),
            "azure should be wired, not listed as unwired"
        );
    }

    #[test]
    fn azure_config_resolves_resource_and_builds_deployment_url() {
        let cfg = AzureConfig {
            resource: Some("my-resource".into()),
            deployments: vec!["gpt-4o".into(), "  ".into(), "gpt-4o-mini".into()],
            ..Default::default()
        };
        let p = cfg.into_provider().unwrap();
        assert_eq!(p.endpoint, "https://my-resource.openai.azure.com");
        // Defaults applied when omitted.
        assert_eq!(p.api_version, DEFAULT_AZURE_API_VERSION);
        assert_eq!(p.env_var, AZURE_DEFAULT_KEY_ENV);
        // Blank deployments filtered out.
        assert_eq!(p.deployments, vec!["gpt-4o", "gpt-4o-mini"]);
        // The deployment-scoped URL with the api-version query is the Azure REST shape.
        assert_eq!(
            p.chat_completions_url("gpt-4o"),
            format!(
                "https://my-resource.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version={DEFAULT_AZURE_API_VERSION}"
            )
        );
    }

    #[test]
    fn project_initialization_detects_guidance_agents_and_skills() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!project_initialization(dir.path()).initialized);

        std::fs::write(dir.path().join("AGENTS.md"), "guidance").unwrap();
        assert!(project_initialization(dir.path()).initialized);
        std::fs::remove_file(dir.path().join("AGENTS.md")).unwrap();

        std::fs::create_dir_all(dir.path().join(".forge/agents")).unwrap();
        std::fs::write(dir.path().join(".forge/agents/reviewer.md"), "agent").unwrap();
        assert!(project_initialization(dir.path()).initialized);
        std::fs::remove_dir_all(dir.path().join(".forge")).unwrap();

        std::fs::create_dir_all(dir.path().join(".claude/agents")).unwrap();
        std::fs::write(dir.path().join(".claude/agents/reviewer.md"), "agent").unwrap();
        assert!(project_initialization(dir.path()).initialized);
    }

    #[test]
    fn config_accepts_project_auto_initialize_block() {
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string("[project]\nauto_initialize = true"))
            .extract()
            .unwrap();
        assert!(cfg.project.auto_initialize);
        assert!(!Config::default().project.auto_initialize);
    }

    #[test]
    fn azure_explicit_endpoint_overrides_resource_and_strips_trailing_slash() {
        let cfg = AzureConfig {
            resource: Some("ignored".into()),
            endpoint: Some("https://sovereign.openai.azure.us/".into()),
            api_version: Some("2025-01-01".into()),
            api_key_env: Some("MY_AZURE_KEY".into()),
            ..Default::default()
        };
        let p = cfg.into_provider().unwrap();
        assert_eq!(p.endpoint, "https://sovereign.openai.azure.us");
        assert_eq!(p.api_version, "2025-01-01");
        assert_eq!(p.env_var, "MY_AZURE_KEY");
        assert!(p.chat_completions_url("d").starts_with(
            "https://sovereign.openai.azure.us/openai/deployments/d/chat/completions?"
        ));
    }

    #[test]
    fn azure_config_requires_resource_or_endpoint() {
        let err = AzureConfig::default().into_provider().unwrap_err();
        assert!(err.contains("resource") || err.contains("endpoint"));
    }

    #[test]
    fn azure_config_block_parses_from_toml() {
        let toml = r#"
[providers.azure]
resource = "acme"
api_version = "2024-10-21"
deployments = ["gpt-4o", "gpt-4o-mini"]
"#;
        let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(toml))
            .extract()
            .unwrap();
        let az = cfg.providers.azure.expect("azure block parsed");
        assert_eq!(az.resource.as_deref(), Some("acme"));
        assert_eq!(az.deployments.len(), 2);
        // And it resolves to a usable provider.
        let p = az.into_provider().unwrap();
        assert_eq!(p.endpoint, "https://acme.openai.azure.com");
    }

    #[test]
    fn add_azure_provider_round_trips_through_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = AzureConfig {
            resource: Some("acme".into()),
            api_version: Some("2024-10-21".into()),
            deployments: vec!["gpt-4o".into()],
            ..Default::default()
        };
        add_azure_provider_at(&path, &cfg).unwrap();
        let parsed: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(&path))
            .extract()
            .unwrap();
        let az = parsed.providers.azure.expect("azure persisted");
        assert_eq!(az.resource.as_deref(), Some("acme"));
        assert_eq!(az.deployments, vec!["gpt-4o"]);

        // Re-adding replaces the block (no duplicate / merge cruft).
        let cfg2 = AzureConfig {
            endpoint: Some("https://acme.openai.azure.com".into()),
            deployments: vec!["o3-mini".into()],
            ..Default::default()
        };
        add_azure_provider_at(&path, &cfg2).unwrap();
        let parsed: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(&path))
            .extract()
            .unwrap();
        let az = parsed.providers.azure.unwrap();
        assert_eq!(az.deployments, vec!["o3-mini"]);

        // An invalid block (no resource/endpoint) is rejected before writing.
        assert!(add_azure_provider_at(&path, &AzureConfig::default()).is_err());
    }

    #[test]
    fn remove_azure_provider_preserves_config_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[mesh]
self_review = true

[providers.azure]
resource = "acme"
deployments = ["gpt-4o"]
"#,
        )
        .unwrap();

        assert!(remove_azure_provider_at(&path).unwrap());
        assert!(!remove_azure_provider_at(&path).unwrap());

        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("self_review = true"));
        assert!(!body.contains("[providers.azure]"));

        let parsed: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(&path))
            .extract()
            .unwrap();
        assert!(parsed.mesh.self_review);
        assert!(parsed.providers.azure.is_none());
    }

    #[test]
    fn classifier_defaults_are_fixed_and_activity_focus_is_documented() {
        let mesh = Config::default().mesh;
        assert_eq!(
            mesh.classifier_model.as_deref(),
            Some("groq::openai/gpt-oss-20b")
        );
        assert!(!mesh.classifier_activity_focused);
        // The help text must still say the setting names the FIRST model tried and that backups
        // exist behind it. The classifier is no longer a single point of failure, and the old
        // wording ("Fixed model") described exactly the behaviour that silently broke it.
        let classifier_help = setting_help("mesh.classifier_model").unwrap();
        assert!(classifier_help.contains("First model"));
        assert!(classifier_help.contains("backups"));
        assert!(setting_help("mesh.classifier_activity_focused")
            .unwrap()
            .contains("final non-empty paragraph"));

        let configured: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(
                "[mesh]\nclassifier_activity_focused = true\nclassifier_model = \"cerebras::gpt-oss-120b\"\n",
            ))
            .extract()
            .unwrap();
        assert!(configured.mesh.classifier_activity_focused);
        assert_eq!(
            configured.mesh.classifier_model.as_deref(),
            Some("cerebras::gpt-oss-120b")
        );
    }
}

#[cfg(test)]
mod mcp_toml_problem_tests {
    use super::mcp_toml_problem;

    #[test]
    fn a_malformed_mcp_file_reports_why_and_a_good_or_absent_one_is_silent() {
        let dir = std::env::temp_dir().join(format!("forge-mcp-problem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.toml");
        std::fs::write(&bad, "[[servers]]\nname = \"x\"\ncommand = \"python3\"\n").unwrap();
        assert!(mcp_toml_problem(&bad).unwrap().contains("transport"));
        let good = dir.join("good.toml");
        std::fs::write(&good, "").unwrap();
        assert_eq!(mcp_toml_problem(&good), None);
        assert_eq!(mcp_toml_problem(&dir.join("absent.toml")), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
