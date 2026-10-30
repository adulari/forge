//! `--mcp-config` / `--strict-mcp-config`: MCP servers for one run, in Claude Code's `.mcp.json`
//! shape, layered over (or, strict, instead of) the configured servers.
//!
//! The servers live only in this process's config; nothing is written to `.forge/mcp.toml` or the
//! keyring, so inline secrets in a per-run config stay where the caller put them. `${VAR}` and
//! `${VAR:-default}` expand from the environment in commands, args, env, URLs and headers, as in
//! Claude Code.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use forge_config::{McpConfig, McpServerConfig, McpTransport};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub(crate) struct McpOverlay {
    servers: Vec<McpServerConfig>,
    strict: bool,
}

impl McpOverlay {
    /// Each value is a path to a JSON file or an inline JSON object.
    pub(crate) fn load(values: &[String], strict: bool) -> Result<Self> {
        let mut servers: Vec<McpServerConfig> = Vec::new();
        for value in values {
            let root = read_source(value)?;
            for server in servers_from(&root, &|name| std::env::var(name).ok())
                .with_context(|| format!("--mcp-config {}", abbreviate(value)))?
            {
                servers.retain(|s| s.name != server.name);
                servers.push(server);
            }
        }
        Ok(Self { servers, strict })
    }

    pub(crate) fn strict(&self) -> bool {
        self.strict
    }

    /// Strict replaces every configured server; otherwise a same-named server is overridden.
    pub(crate) fn apply(&self, mcp: &mut McpConfig) {
        if self.strict {
            mcp.servers.clear();
        }
        for server in &self.servers {
            mcp.servers.retain(|s| s.name != server.name);
            mcp.servers.push(server.clone());
        }
    }
}

fn abbreviate(value: &str) -> String {
    let flat: String = value.split_whitespace().collect::<Vec<_>>().join(" ");
    forge_types::truncate_ellipsis(&flat, 60)
}

fn read_source(value: &str) -> Result<Value> {
    let text = if value.trim_start().starts_with('{') {
        value.to_string()
    } else {
        std::fs::read_to_string(value).with_context(|| format!("reading --mcp-config {value}"))?
    };
    serde_json::from_str(&text)
        .with_context(|| format!("parsing --mcp-config {}", abbreviate(value)))
}

fn servers_from(
    root: &Value,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<McpServerConfig>> {
    let Some(map) = root
        .get("mcpServers")
        .or_else(|| root.get("servers"))
        .and_then(Value::as_object)
    else {
        bail!("no `mcpServers` object");
    };
    let mut servers = Vec::new();
    for (name, spec) in map {
        servers.push(server_from(name, spec, env)?);
    }
    Ok(servers)
}

fn server_from(
    name: &str,
    spec: &Value,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<McpServerConfig> {
    let text = |key: &str| {
        spec.get(key)
            .and_then(Value::as_str)
            .map(|v| expand(v, env))
    };
    let strings = |key: &str| -> HashMap<String, String> {
        spec.get(key)
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), expand(v, env))))
                    .collect()
            })
            .unwrap_or_default()
    };
    let kind = text("type");
    let transport = if let Some(command) = text("command") {
        let args = spec
            .get("args")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|v| expand(v, env))
                    .collect()
            })
            .unwrap_or_default();
        McpTransport::Stdio {
            command,
            args,
            env: strings("env"),
        }
    } else if let Some(url) = text("url") {
        let headers = strings("headers");
        match kind.as_deref() {
            Some("sse") => McpTransport::Sse { url, headers },
            Some("http") | Some("streamable-http") | None => McpTransport::Http { url, headers },
            Some(other) => bail!("server '{name}': unknown type '{other}'"),
        }
    } else {
        bail!("server '{name}': needs `command` (stdio) or `url` (http/sse)");
    };
    Ok(McpServerConfig {
        disabled_tools: Vec::new(),
        name: name.to_string(),
        transport,
        auth: None,
        secret_env: Vec::new(),
        enabled: true,
        shared: None,
    })
}

/// `${VAR}` and `${VAR:-default}`; an unset variable without a default expands to nothing.
fn expand(raw: &str, env: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..end];
        let (var, default) = match inner.split_once(":-") {
            Some((var, default)) => (var, Some(default)),
            None => (inner, None),
        };
        match env(var).filter(|v| !v.is_empty()) {
            Some(value) => out.push_str(&value),
            None => out.push_str(default.unwrap_or("")),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> Option<String> {
        (name == "TOKEN").then(|| "s3cret".to_string())
    }

    #[test]
    fn parses_claude_code_mcp_json_with_env_expansion() {
        let root = serde_json::json!({ "mcpServers": {
            "git": { "command": "uvx", "args": ["mcp-git", "--repo", "${REPO:-.}"],
                     "env": { "TOKEN": "${TOKEN}" } },
            "docs": { "type": "http", "url": "https://x.dev/mcp",
                      "headers": { "Authorization": "Bearer ${TOKEN}" } },
            "old": { "type": "sse", "url": "https://x.dev/sse" }
        }});
        let servers = servers_from(&root, &env).unwrap();
        let by = |n: &str| servers.iter().find(|s| s.name == n).unwrap();
        match &by("git").transport {
            McpTransport::Stdio { command, args, env } => {
                assert_eq!(command, "uvx");
                assert_eq!(args, &["mcp-git", "--repo", "."]);
                assert_eq!(env["TOKEN"], "s3cret");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&by("docs").transport,
            McpTransport::Http { headers, .. } if headers["Authorization"] == "Bearer s3cret"));
        assert!(matches!(by("old").transport, McpTransport::Sse { .. }));
    }

    #[test]
    fn a_server_without_command_or_url_is_an_error() {
        let root = serde_json::json!({ "mcpServers": { "broken": { "type": "stdio" } } });
        assert!(servers_from(&root, &env).is_err());
        assert!(servers_from(&serde_json::json!({}), &env).is_err());
    }

    fn named(name: &str) -> McpServerConfig {
        McpServerConfig {
            disabled_tools: Vec::new(),
            name: name.to_string(),
            transport: McpTransport::Http {
                url: "https://x".into(),
                headers: HashMap::new(),
            },
            auth: None,
            secret_env: Vec::new(),
            enabled: true,
            shared: None,
        }
    }

    #[test]
    fn strict_replaces_the_configured_servers_and_plain_adds_to_them() {
        let configured = McpConfig {
            servers: vec![named("kept"), named("docs")],
            ..Default::default()
        };
        let overlay = McpOverlay {
            servers: vec![named("docs"), named("new")],
            strict: false,
        };
        let mut mcp = configured.clone();
        overlay.apply(&mut mcp);
        let names: Vec<_> = mcp.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["kept", "docs", "new"]);

        let mut mcp = configured;
        McpOverlay {
            strict: true,
            ..overlay
        }
        .apply(&mut mcp);
        let names: Vec<_> = mcp.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["docs", "new"]);
    }

    #[test]
    fn inline_json_and_files_are_both_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("mcp.json");
        std::fs::write(&file, r#"{"mcpServers":{"a":{"command":"a-server"}}}"#).unwrap();
        let overlay = McpOverlay::load(
            &[
                file.display().to_string(),
                r#"{"mcpServers":{"b":{"url":"https://b"}}}"#.to_string(),
            ],
            false,
        )
        .unwrap();
        assert_eq!(overlay.servers.len(), 2);
        assert!(McpOverlay::load(&["/no/such/file.json".to_string()], false).is_err());
    }
}
