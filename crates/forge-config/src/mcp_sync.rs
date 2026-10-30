//! `forge mcp sync`: mirror Forge's enabled MCP servers into Claude Code's and Codex's own
//! configs so all three agents see the same tool set.
//!
//! Only the sync'd servers' entries are written; every other key and server in the target is left
//! alone. Secret values are never written: a token is referenced by environment variable name
//! (`${VAR}` for Claude, `bearer_token_env_var` / `env_vars` for Codex).

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::mcp::{McpServerConfig, McpTransport};
use crate::ConfigError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTarget {
    Claude,
    Codex,
}

impl SyncTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    pub fn path(self, home: &Path) -> PathBuf {
        match self {
            Self::Claude => home.join(".claude.json"),
            Self::Codex => home.join(".codex").join("config.toml"),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub unchanged: Vec<String>,
}

impl SyncReport {
    fn record(&mut self, name: &str, existing: bool, same: bool) {
        let bucket = match (existing, same) {
            (false, _) => &mut self.added,
            (true, false) => &mut self.updated,
            (true, true) => &mut self.unchanged,
        };
        bucket.push(name.to_string());
    }

    pub fn changed(&self) -> bool {
        !self.added.is_empty() || !self.updated.is_empty()
    }
}

fn bearer_header(auth: &crate::mcp::McpAuth) -> Option<(String, String)> {
    let var = auth.token_env.as_deref()?;
    Some(match auth.header.as_deref() {
        None | Some("Authorization") => ("Authorization".into(), format!("Bearer ${{{var}}}")),
        Some(other) => (other.to_string(), format!("${{{var}}}")),
    })
}

fn claude_entry(server: &McpServerConfig) -> Value {
    let token_env = server.auth.as_ref().and_then(|a| a.token_env.clone());
    match &server.transport {
        McpTransport::Stdio { command, args, env } => {
            let mut vars: Map<String, Value> =
                env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            if let Some(var) = token_env {
                vars.insert(var.clone(), json!(format!("${{{var}}}")));
            }
            let mut entry = json!({ "type": "stdio", "command": command, "args": args });
            if !vars.is_empty() {
                entry["env"] = Value::Object(vars);
            }
            entry
        }
        McpTransport::Http { url, headers } | McpTransport::Sse { url, headers } => {
            let kind = if matches!(server.transport, McpTransport::Sse { .. }) {
                "sse"
            } else {
                "http"
            };
            let mut map: Map<String, Value> =
                headers.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            if let Some((name, value)) = server.auth.as_ref().and_then(bearer_header) {
                map.insert(name, json!(value));
            }
            let mut entry = json!({ "type": kind, "url": url });
            if !map.is_empty() {
                entry["headers"] = Value::Object(map);
            }
            entry
        }
    }
}

pub fn sync_claude(
    path: &Path,
    servers: &[&McpServerConfig],
    dry_run: bool,
) -> Result<SyncReport, ConfigError> {
    let mut root: Value = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            ConfigError::Write(format!(
                "refusing to overwrite invalid {}: {e}",
                path.display()
            ))
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(ConfigError::Write(format!("{}: {e}", path.display()))),
    };
    let object = root
        .as_object_mut()
        .ok_or_else(|| ConfigError::Write(format!("{} is not a JSON object", path.display())))?;
    let table = object
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| ConfigError::Write("`mcpServers` is not an object".into()))?;
    let mut report = SyncReport::default();
    for server in servers {
        let entry = claude_entry(server);
        let previous = table.get(&server.name);
        report.record(&server.name, previous.is_some(), previous == Some(&entry));
        table.insert(server.name.clone(), entry);
    }
    if report.changed() && !dry_run {
        write_atomic(
            path,
            &serde_json::to_string_pretty(&root).unwrap_or_default(),
        )?;
    }
    Ok(report)
}

fn codex_entry(server: &McpServerConfig) -> toml_edit::Table {
    use toml_edit::{value, Array, Table};
    let mut table = Table::new();
    let token_env = server.auth.as_ref().and_then(|a| a.token_env.clone());
    match &server.transport {
        McpTransport::Stdio { command, args, env } => {
            table["command"] = value(command.as_str());
            if !args.is_empty() {
                table["args"] = value(args.iter().map(String::as_str).collect::<Array>());
            }
            if !env.is_empty() {
                let mut vars = toml_edit::InlineTable::new();
                let mut keys: Vec<_> = env.iter().collect();
                keys.sort();
                for (k, v) in keys {
                    vars.insert(k, v.as_str().into());
                }
                table["env"] = value(vars);
            }
            if let Some(var) = token_env {
                table["env_vars"] = value(Array::from_iter([var.as_str()]));
            }
        }
        McpTransport::Http { url, headers } | McpTransport::Sse { url, headers } => {
            table["url"] = value(url.as_str());
            let auth_header = server.auth.as_ref().and_then(bearer_header);
            if let (Some(var), Some((name, _))) = (token_env, auth_header) {
                if name == "Authorization" {
                    table["bearer_token_env_var"] = value(var);
                } else {
                    let mut map = toml_edit::InlineTable::new();
                    map.insert(&name, var.as_str().into());
                    table["env_http_headers"] = value(map);
                }
            }
            if !headers.is_empty() {
                let mut map = toml_edit::InlineTable::new();
                let mut keys: Vec<_> = headers.iter().collect();
                keys.sort();
                for (k, v) in keys {
                    map.insert(k, v.as_str().into());
                }
                table["http_headers"] = value(map);
            }
        }
    }
    table
}

pub fn sync_codex(
    path: &Path,
    servers: &[&McpServerConfig],
    dry_run: bool,
) -> Result<SyncReport, ConfigError> {
    let mut doc = if path.exists() {
        crate::toml_write::load_doc(path)?
    } else {
        toml_edit::DocumentMut::new()
    };
    let root = doc.as_table_mut();
    if !root.contains_key("mcp_servers") {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        root.insert("mcp_servers", toml_edit::Item::Table(table));
    }
    let table = root["mcp_servers"]
        .as_table_mut()
        .ok_or_else(|| ConfigError::Write("`mcp_servers` is not a table".into()))?;
    let mut report = SyncReport::default();
    for server in servers {
        let entry = codex_entry(server);
        let previous = table.get(&server.name).and_then(|i| i.as_table());
        let same = previous.is_some_and(|p| p.to_string() == entry.to_string());
        report.record(&server.name, table.contains_key(&server.name), same);
        table.insert(&server.name, toml_edit::Item::Table(entry));
    }
    if report.changed() && !dry_run {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError::Write(e.to_string()))?;
        }
        write_atomic(path, &doc.to_string())?;
    }
    Ok(report)
}

fn write_atomic(path: &Path, text: &str) -> Result<(), ConfigError> {
    let tmp = path.with_extension("forge-sync.tmp");
    std::fs::write(&tmp, text).map_err(|e| ConfigError::Write(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| ConfigError::Write(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpAuth;
    use std::collections::HashMap;

    fn stdio(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.into(),
            transport: McpTransport::Stdio {
                command: "node".into(),
                args: vec!["srv.js".into()],
                env: HashMap::new(),
            },
            auth: Some(McpAuth {
                token_env: Some("SRV_TOKEN".into()),
                token_keyring: None,
                header: None,
                oauth: None,
            }),
            secret_env: vec![],
            enabled: true,
            shared: None,
            disabled_tools: Vec::new(),
        }
    }

    fn http(name: &str) -> McpServerConfig {
        McpServerConfig {
            transport: McpTransport::Http {
                url: "https://x.test/mcp".into(),
                headers: HashMap::new(),
            },
            ..stdio(name)
        }
    }

    #[test]
    fn claude_sync_keeps_other_keys_and_never_writes_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        std::fs::write(
            &path,
            r#"{"theme":"dark","mcpServers":{"keep":{"type":"stdio","command":"x"}}}"#,
        )
        .unwrap();
        let (a, b) = (stdio("a"), http("b"));
        let report = sync_claude(&path, &[&a, &b], false).unwrap();
        assert_eq!(report.added, ["a", "b"]);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["mcpServers"]["keep"]["command"], "x");
        assert_eq!(v["mcpServers"]["a"]["env"]["SRV_TOKEN"], "${SRV_TOKEN}");
        assert_eq!(
            v["mcpServers"]["b"]["headers"]["Authorization"],
            "Bearer ${SRV_TOKEN}"
        );
        let again = sync_claude(&path, &[&a, &b], false).unwrap();
        assert_eq!(again.unchanged, ["a", "b"]);
        assert!(!again.changed());
    }

    #[test]
    fn codex_sync_preserves_comments_and_references_token_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# mine\nmodel = \"x\"\n").unwrap();
        let (a, b) = (stdio("a"), http("b"));
        let report = sync_codex(&path, &[&a, &b], false).unwrap();
        assert_eq!(report.added, ["a", "b"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# mine\nmodel = \"x\""), "{text}");
        let parsed: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(parsed["mcp_servers"]["a"]["command"].as_str(), Some("node"));
        assert_eq!(
            parsed["mcp_servers"]["b"]["bearer_token_env_var"].as_str(),
            Some("SRV_TOKEN")
        );
        assert!(!sync_codex(&path, &[&a, &b], false).unwrap().changed());
    }

    #[test]
    fn dry_run_writes_nothing_and_invalid_target_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        let a = stdio("a");
        assert_eq!(sync_claude(&path, &[&a], true).unwrap().added, ["a"]);
        assert!(!path.exists());
        std::fs::write(&path, "{nope").unwrap();
        assert!(sync_claude(&path, &[&a], false).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{nope");
    }
}
