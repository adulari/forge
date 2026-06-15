//! MCP-client configuration (docs/features/mcp-client.md): declaring external MCP servers,
//! the allowlist, token resolution (env/keyring — never inline in TOML, ADR-0007), and a
//! Claude-Code-compatible `.mcp.json` importer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{ConfigError, KEYRING_SERVICE};

fn default_call_timeout_secs() -> u64 {
    60
}
fn default_connect_timeout_secs() -> u64 {
    20
}
fn default_max_reconnect_attempts() -> usize {
    3
}
fn default_true() -> bool {
    true
}

/// The `[mcp]` config section: declared servers + global knobs. Empty (no servers) means the
/// whole MCP path is inert — zero overhead for users who don't use MCP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
    /// Allowlist: if `servers` is non-empty, only those server names may connect; if `tools` is
    /// non-empty, only those qualified tool names may be exposed/called.
    #[serde(default)]
    pub allow: McpAllowlist,
    /// Per-`tools/call` timeout (default 60s) — a slow/hung server returns a tool error, not a hang.
    #[serde(default = "default_call_timeout_secs")]
    pub call_timeout_secs: u64,
    /// Connect/initialize budget per server (default 20s) — a slow server lands `failed`/`connecting`
    /// without delaying session start beyond this.
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    /// How many of a server's tools to advertise to the model eagerly (per server). The rest are
    /// discovered but loaded on demand via `mcp_search_tools`→`mcp_expose_tool`. Default 0 = all
    /// deferred (keeps the per-turn tool list bounded for big servers).
    #[serde(default)]
    pub max_eager_tools: usize,
    /// Bounded reconnect attempts after a stdio child exits / an HTTP stream drops (default 3).
    #[serde(default = "default_max_reconnect_attempts")]
    pub max_reconnect_attempts: usize,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            servers: Vec::new(),
            allow: McpAllowlist::default(),
            call_timeout_secs: default_call_timeout_secs(),
            connect_timeout_secs: default_connect_timeout_secs(),
            max_eager_tools: 0,
            max_reconnect_attempts: default_max_reconnect_attempts(),
        }
    }
}

impl McpConfig {
    /// Servers that are enabled AND pass the server allowlist. The set Forge actually connects to.
    pub fn active_servers(&self) -> impl Iterator<Item = &McpServerConfig> {
        self.servers
            .iter()
            .filter(|s| s.enabled && self.server_allowed(&s.name))
    }

    /// Is this server permitted by the allowlist? Empty `allow.servers` = all declared servers.
    pub fn server_allowed(&self, name: &str) -> bool {
        self.allow.servers.is_empty() || self.allow.servers.iter().any(|s| s == name)
    }

    /// Is this qualified tool (`server__tool`) permitted? Empty `allow.tools` = every tool of an
    /// allowed server. Otherwise the qualified name must be listed explicitly.
    pub fn tool_allowed(&self, qualified: &str) -> bool {
        self.allow.tools.is_empty() || self.allow.tools.iter().any(|t| t == qualified)
    }

    /// Reject duplicate server names (they'd collide as tool-name prefixes) and empty names.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for s in &self.servers {
            if s.name.trim().is_empty() {
                return Err("mcp: a server has an empty name".to_string());
            }
            if !seen.insert(&s.name) {
                return Err(format!("mcp: duplicate server name '{}'", s.name));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique namespace prefix for this server's tools (`<name>__<tool>`).
    pub name: String,
    pub transport: McpTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<McpAuth>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl McpServerConfig {
    /// Resolve this server's bearer token from env/keyring (ADR-0007). `None` if no auth declared.
    pub fn token(&self) -> Option<String> {
        self.auth.as_ref().and_then(resolve_token)
    }

    /// "stdio" / "http", for status display.
    pub fn transport_label(&self) -> &'static str {
        match self.transport {
            McpTransport::Stdio { .. } => "stdio",
            McpTransport::Http { .. } => "http",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpTransport {
    /// A child process speaking MCP over stdio.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    /// A remote MCP server over streamable-HTTP / SSE.
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

/// Where a server's token comes from — never the value itself in config (ADR-0007).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpAuth {
    /// Environment variable holding the token (e.g. `GITLAB_TOKEN`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
    /// Keyring entry name (looked up under the `forge` service), e.g. `mcp:gitlab`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_keyring: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpAllowlist {
    #[serde(default)]
    pub servers: Vec<String>,
    /// Qualified `server__tool` names.
    #[serde(default)]
    pub tools: Vec<String>,
}

/// Resolve a token: env var first, then keyring. `None` if neither yields a non-empty value.
pub fn resolve_token(auth: &McpAuth) -> Option<String> {
    if let Some(var) = &auth.token_env {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    if let Some(key) = &auth.token_keyring {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, key) {
            if let Ok(v) = entry.get_password() {
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Env-var name patterns that almost certainly hold a secret — used by the importer to avoid
/// copying a plaintext token out of `.mcp.json` into Forge's TOML.
fn looks_secret(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    [
        "TOKEN",
        "KEY",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "PAT",
        "CREDENTIAL",
    ]
    .iter()
    .any(|m| k.contains(m))
}

/// Parse one server entry from a JSON spec (the `{type?, command/args/env | url/headers}` object
/// used by Claude Code, Cursor, Windsurf, …). Pushes secret-stripping warnings into `warnings`;
/// returns `None` for an entry that is neither stdio nor http. Secrets are NEVER copied.
fn server_from_json(
    name: &str,
    spec: &serde_json::Value,
    warnings: &mut Vec<String>,
) -> Option<McpServerConfig> {
    let (transport, auth) = if let Some(cmd) = spec.get("command").and_then(|v| v.as_str()) {
        let args = spec
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut env = HashMap::new();
        let mut auth: Option<McpAuth> = None;
        if let Some(env_obj) = spec.get("env").and_then(|v| v.as_object()) {
            for (k, v) in env_obj {
                if looks_secret(k) {
                    auth.get_or_insert_with(McpAuth::default).token_env = Some(k.clone());
                    warnings.push(format!(
                        "server '{name}': not copying secret env '{k}' into mcp.toml — set \
                         token_env = \"{k}\" and export it (or use the keyring)"
                    ));
                } else if let Some(val) = v.as_str() {
                    env.insert(k.clone(), val.to_string());
                }
            }
        }
        (
            McpTransport::Stdio {
                command: cmd.to_string(),
                args,
                env,
            },
            auth,
        )
    } else if let Some(url) = spec.get("url").and_then(|v| v.as_str()) {
        let mut headers = HashMap::new();
        let mut had_secret_header = false;
        if let Some(h) = spec.get("headers").and_then(|v| v.as_object()) {
            for (k, v) in h {
                if looks_secret(k) || k.eq_ignore_ascii_case("authorization") {
                    had_secret_header = true;
                    warnings.push(format!(
                        "server '{name}': not copying header '{k}' into mcp.toml — put the token \
                         in token_keyring/token_env (Forge sends it as a bearer token)"
                    ));
                } else if let Some(val) = v.as_str() {
                    headers.insert(k.clone(), val.to_string());
                }
            }
        }
        // Leave a placeholder keyring slot so the user knows where the token belongs.
        let auth = had_secret_header.then(|| McpAuth {
            token_keyring: Some(format!("mcp:{name}")),
            token_env: None,
        });
        (
            McpTransport::Http {
                url: url.to_string(),
                headers,
            },
            auth,
        )
    } else {
        warnings.push(format!(
            "server '{name}': skipped — neither `command` (stdio) nor `url` (http)"
        ));
        return None;
    };
    Some(McpServerConfig {
        name: name.to_string(),
        transport,
        auth,
        enabled: true,
    })
}

/// Parse a JSON `mcpServers` (Claude/Cursor/Windsurf) **or** `servers` (VS Code) object into
/// server configs + secret-stripping warnings.
fn servers_from_json(root: &serde_json::Value) -> (Vec<McpServerConfig>, Vec<String>) {
    let obj = root
        .get("mcpServers")
        .or_else(|| root.get("servers"))
        .and_then(|v| v.as_object());
    let mut servers = Vec::new();
    let mut warnings = Vec::new();
    if let Some(obj) = obj {
        for (name, spec) in obj {
            if let Some(s) = server_from_json(name, spec, &mut warnings) {
                servers.push(s);
            }
        }
    }
    (servers, warnings)
}

/// Translate a Claude-Code-style `.mcp.json` into an [`McpConfig`]. Returns the config plus any
/// warnings (a secret that was NOT copied — the user is told to move it to `token_env`/keyring).
/// Secrets are never written into the resulting config (ADR-0007).
pub fn import_mcp_json(path: &Path) -> Result<(McpConfig, Vec<String>), ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError::Write(format!("reading {}: {e}", path.display())))?;
    let root: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| ConfigError::Write(format!("parsing {}: {e}", path.display())))?;
    if root
        .get("mcpServers")
        .or_else(|| root.get("servers"))
        .is_none()
    {
        return Err(ConfigError::Write(
            "no `mcpServers` (or `servers`) object in the file".into(),
        ));
    }
    let (servers, warnings) = servers_from_json(&root);
    Ok((
        McpConfig {
            servers,
            ..Default::default()
        },
        warnings,
    ))
}

/// One place Forge found MCP servers declared (a specific tool's config file). Surfaced by
/// [`discover_import_sources`] so the user can pick which servers to import.
#[derive(Debug, Clone)]
pub struct ImportSource {
    /// Human label, e.g. `claude-code (global)`, `codex`, `cursor (project)`.
    pub label: String,
    pub path: PathBuf,
    pub servers: Vec<McpServerConfig>,
    /// Secret-stripping notes for this source.
    pub warnings: Vec<String>,
}

/// Scan every AI-CLI MCP config Forge knows about (Claude Code, Claude Desktop, Codex, Cursor,
/// Windsurf, VS Code) and return the sources that exist and declare ≥1 server. Read-only.
/// Secrets are stripped during parsing — an [`ImportSource`]'s servers never carry a token value.
pub fn discover_import_sources(cwd: &Path) -> Vec<ImportSource> {
    let mut out = Vec::new();
    let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
    let config_dir = directories::BaseDirs::new().map(|b| b.config_dir().to_path_buf());

    // --- Claude Code: ~/.claude.json (global `mcpServers` + per-project) ---
    if let Some(home) = &home {
        let claude = home.join(".claude.json");
        if let Ok(text) = std::fs::read_to_string(&claude) {
            if let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) {
                let (servers, warnings) = servers_from_json(&root);
                push_source(&mut out, "claude-code (global)", &claude, servers, warnings);
                // Project-scoped: projects.<abs-cwd>.mcpServers
                if let Some(proj) = root
                    .get("projects")
                    .and_then(|p| p.get(cwd.to_string_lossy().as_ref()))
                {
                    let (servers, warnings) = servers_from_json(proj);
                    push_source(
                        &mut out,
                        "claude-code (this project)",
                        &claude,
                        servers,
                        warnings,
                    );
                }
            }
        }
    }

    // --- Claude Code / generic project file: ./.mcp.json ---
    scan_json(&mut out, "claude-code (.mcp.json)", &cwd.join(".mcp.json"));

    // --- Codex: ~/.codex/config.toml ([mcp_servers.<name>]) ---
    if let Some(home) = &home {
        let codex = home.join(".codex/config.toml");
        if let Ok(text) = std::fs::read_to_string(&codex) {
            let (servers, warnings) = servers_from_codex_toml(&text);
            push_source(&mut out, "codex", &codex, servers, warnings);
        }
    }

    // --- Cursor: ~/.cursor/mcp.json (global) + ./.cursor/mcp.json (project) ---
    if let Some(home) = &home {
        scan_json(&mut out, "cursor (global)", &home.join(".cursor/mcp.json"));
    }
    scan_json(&mut out, "cursor (project)", &cwd.join(".cursor/mcp.json"));

    // --- Claude Desktop: <config>/Claude/claude_desktop_config.json ---
    if let Some(cfg) = &config_dir {
        scan_json(
            &mut out,
            "claude-desktop",
            &cfg.join("Claude/claude_desktop_config.json"),
        );
    }

    // --- Windsurf: ~/.codeium/windsurf/mcp_config.json ---
    if let Some(home) = &home {
        scan_json(
            &mut out,
            "windsurf",
            &home.join(".codeium/windsurf/mcp_config.json"),
        );
    }

    // --- VS Code project: ./.vscode/mcp.json (uses the `servers` key) ---
    scan_json(&mut out, "vscode (project)", &cwd.join(".vscode/mcp.json"));

    out
}

/// Read a JSON MCP config and, if it has servers, push it as a source.
fn scan_json(out: &mut Vec<ImportSource>, label: &str, path: &Path) {
    if let Ok(text) = std::fs::read_to_string(path) {
        if let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) {
            let (servers, warnings) = servers_from_json(&root);
            push_source(out, label, path, servers, warnings);
        }
    }
}

fn push_source(
    out: &mut Vec<ImportSource>,
    label: &str,
    path: &Path,
    servers: Vec<McpServerConfig>,
    warnings: Vec<String>,
) {
    if !servers.is_empty() {
        out.push(ImportSource {
            label: label.to_string(),
            path: path.to_path_buf(),
            servers,
            warnings,
        });
    }
}

/// Parse Codex's `~/.codex/config.toml` `[mcp_servers.<name>]` tables. Stdio entries use
/// `command`/`args`/`env`; http entries use `url`/`headers`. Secrets stripped like the JSON path.
fn servers_from_codex_toml(text: &str) -> (Vec<McpServerConfig>, Vec<String>) {
    let mut servers = Vec::new();
    let mut warnings = Vec::new();
    let root: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(_) => return (servers, warnings),
    };
    let Some(table) = root.get("mcp_servers").and_then(|v| v.as_table()) else {
        return (servers, warnings);
    };
    for (name, spec) in table {
        // Reuse the JSON parser by converting the TOML value to JSON (same field shapes).
        let json = serde_json::to_value(spec).unwrap_or(serde_json::Value::Null);
        if let Some(s) = server_from_json(name, &json, &mut warnings) {
            servers.push(s);
        }
    }
    (servers, warnings)
}

/// Read an existing `.forge/mcp.toml` into an [`McpConfig`], or the default if it's absent or
/// malformed. Used when merging newly-imported servers into a file that may already exist.
pub fn load_mcp_toml(path: &Path) -> McpConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

/// Serialize an [`McpConfig`] to a `.forge/mcp.toml` file (creating parent dirs). Secrets are
/// never present in `McpConfig`, so this is safe to write.
pub fn write_mcp_toml(path: &Path, config: &McpConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Write(e.to_string()))?;
    }
    let body = toml::to_string_pretty(config).map_err(|e| ConfigError::Write(e.to_string()))?;
    std::fs::write(path, body).map_err(|e| ConfigError::Write(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_empty_allows_all_declared() {
        let c = McpConfig::default();
        assert!(c.server_allowed("anything"));
        assert!(c.tool_allowed("gitlab__list_merge_requests"));
    }

    #[test]
    fn allowlist_restricts_when_set() {
        let c = McpConfig {
            allow: McpAllowlist {
                servers: vec!["gitlab".into()],
                tools: vec!["gitlab__get_mr_diff".into()],
            },
            ..Default::default()
        };
        assert!(c.server_allowed("gitlab"));
        assert!(!c.server_allowed("evil"));
        assert!(c.tool_allowed("gitlab__get_mr_diff"));
        assert!(!c.tool_allowed("gitlab__delete_repo"));
    }

    #[test]
    fn duplicate_server_names_rejected() {
        let stdio = || McpTransport::Stdio {
            command: "x".into(),
            args: vec![],
            env: HashMap::new(),
        };
        let c = McpConfig {
            servers: vec![
                McpServerConfig {
                    name: "a".into(),
                    transport: stdio(),
                    auth: None,
                    enabled: true,
                },
                McpServerConfig {
                    name: "a".into(),
                    transport: stdio(),
                    auth: None,
                    enabled: true,
                },
            ],
            ..Default::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn token_resolves_from_env_first() {
        std::env::set_var("FORGE_TEST_MCP_TOKEN", "tok-123");
        let auth = McpAuth {
            token_env: Some("FORGE_TEST_MCP_TOKEN".into()),
            token_keyring: None,
        };
        assert_eq!(resolve_token(&auth).as_deref(), Some("tok-123"));
        std::env::remove_var("FORGE_TEST_MCP_TOKEN");
        assert_eq!(resolve_token(&auth), None);
    }

    #[test]
    fn parses_mcp_toml_section() {
        let toml = r#"
call_timeout_secs = 30
max_eager_tools = 2

[[servers]]
name = "gitlab"
[servers.transport]
type = "stdio"
command = "gitlab-mcp-server"
args = ["--read-only"]
[servers.transport.env]
GITLAB_URL = "https://gitlab.example.com"
[servers.auth]
token_env = "GITLAB_TOKEN"

[[servers]]
name = "docs"
[servers.transport]
type = "http"
url = "https://mcp.example.com/mcp"
"#;
        let c: McpConfig = toml::from_str(toml).unwrap();
        assert_eq!(c.call_timeout_secs, 30);
        assert_eq!(c.max_eager_tools, 2);
        assert_eq!(c.servers.len(), 2);
        assert_eq!(c.servers[0].name, "gitlab");
        assert_eq!(c.servers[0].transport_label(), "stdio");
        assert_eq!(c.servers[1].transport_label(), "http");
        match &c.servers[0].transport {
            McpTransport::Stdio { command, args, env } => {
                assert_eq!(command, "gitlab-mcp-server");
                assert_eq!(args, &["--read-only"]);
                assert_eq!(env.get("GITLAB_URL").unwrap(), "https://gitlab.example.com");
            }
            _ => panic!("expected stdio"),
        }
        c.validate().unwrap();
    }

    #[test]
    fn codex_toml_servers_parse_and_infer_transport() {
        let toml = r#"
[mcp_servers.github]
command = "/home/x/.local/bin/claude-code-mcp"
args = ["github"]

[mcp_servers.remote]
url = "https://mcp.example.com/mcp"
[mcp_servers.remote.headers]
Authorization = "Bearer SECRET-TOKEN"
"#;
        let (servers, warnings) = servers_from_codex_toml(toml);
        assert_eq!(servers.len(), 2);
        let gh = servers.iter().find(|s| s.name == "github").unwrap();
        assert_eq!(gh.transport_label(), "stdio");
        let remote = servers.iter().find(|s| s.name == "remote").unwrap();
        assert_eq!(remote.transport_label(), "http");
        // The Authorization header value is never copied; a keyring placeholder is left instead.
        assert!(warnings.iter().any(|w| w.contains("Authorization")));
        assert_eq!(
            remote.auth.as_ref().unwrap().token_keyring.as_deref(),
            Some("mcp:remote")
        );
        // Round-trip the parsed config: the secret must not appear in the serialized TOML.
        let cfg = McpConfig {
            servers,
            ..Default::default()
        };
        let body = toml::to_string_pretty(&cfg).unwrap();
        assert!(!body.contains("SECRET-TOKEN"));
    }

    #[test]
    fn discovers_sources_across_clis() {
        // A fake HOME + cwd holding a Claude global config, a Codex config, and a project .mcp.json.
        let root = std::env::temp_dir().join(format!("forge-disco-{}", forge_types::new_id()));
        let home = root.join("home");
        let cwd = root.join("proj");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(
            home.join(".claude.json"),
            serde_json::json!({
                "mcpServers": { "helm": { "type": "http", "url": "https://h.example/mcp",
                    "headers": { "Authorization": "Bearer X" } } },
                "projects": { cwd.to_string_lossy(): { "mcpServers": {
                    "vectra": { "type": "stdio", "command": "npx", "args": ["-y", "vectra"] } } } }
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            home.join(".codex/config.toml"),
            "[mcp_servers.github]\ncommand = \"x\"\nargs = [\"github\"]\n",
        )
        .unwrap();
        std::fs::write(
            cwd.join(".mcp.json"),
            serde_json::json!({ "mcpServers": {
                "local": { "command": "./srv", "args": [] } } })
            .to_string(),
        )
        .unwrap();

        // Point discovery at the fake HOME by overriding it for this thread.
        std::env::set_var("HOME", &home);
        let sources = discover_import_sources(&cwd);
        std::env::remove_var("HOME");

        let labels: Vec<&str> = sources.iter().map(|s| s.label.as_str()).collect();
        assert!(labels.contains(&"claude-code (global)"), "{labels:?}");
        assert!(labels.contains(&"claude-code (this project)"), "{labels:?}");
        assert!(labels.contains(&"codex"), "{labels:?}");
        assert!(labels.contains(&"claude-code (.mcp.json)"), "{labels:?}");
        // The helm secret never lands in a parsed server.
        let helm = sources
            .iter()
            .flat_map(|s| &s.servers)
            .find(|s| s.name == "helm")
            .unwrap();
        match &helm.transport {
            McpTransport::Http { headers, .. } => assert!(!headers.contains_key("Authorization")),
            _ => panic!("http"),
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn import_mcp_json_translates_and_protects_secrets() {
        let dir = std::env::temp_dir().join(format!("forge-mcpimp-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let json = r#"{
          "mcpServers": {
            "gitlab": {
              "command": "gitlab-mcp",
              "args": ["--read-only"],
              "env": { "GITLAB_URL": "https://gl.example.com", "GITLAB_TOKEN": "glpat-SECRET" }
            },
            "docs": { "url": "https://mcp.example.com/mcp" }
          }
        }"#;
        let path = dir.join(".mcp.json");
        std::fs::write(&path, json).unwrap();

        let (cfg, warnings) = import_mcp_json(&path).unwrap();
        assert_eq!(cfg.servers.len(), 2);
        let gl = cfg.servers.iter().find(|s| s.name == "gitlab").unwrap();
        match &gl.transport {
            McpTransport::Stdio { env, .. } => {
                assert_eq!(env.get("GITLAB_URL").unwrap(), "https://gl.example.com");
                // The secret env value is NOT copied into config.
                assert!(!env.contains_key("GITLAB_TOKEN"));
            }
            _ => panic!("stdio"),
        }
        // Instead the server points at the env var by name.
        assert_eq!(
            gl.auth.as_ref().unwrap().token_env.as_deref(),
            Some("GITLAB_TOKEN")
        );
        assert!(warnings.iter().any(|w| w.contains("GITLAB_TOKEN")));

        // Round-trips through write_mcp_toml without leaking the secret.
        let out = dir.join("mcp.toml");
        write_mcp_toml(&out, &cfg).unwrap();
        let written = std::fs::read_to_string(&out).unwrap();
        assert!(
            !written.contains("glpat-SECRET"),
            "no secret in written TOML"
        );
        let reparsed: McpConfig = toml::from_str(&written).unwrap();
        assert_eq!(reparsed.servers.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }
}
