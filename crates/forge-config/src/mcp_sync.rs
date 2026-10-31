//! `forge mcp sync`: mirror Forge's enabled MCP servers into Claude Code's and Codex's own
//! configs so all three agents see the same tool set.
//!
//! Only the sync'd servers' entries are written; every other key and server in the target is left
//! alone. An entry that already exists is merged into, not replaced: keys Forge does not manage
//! (timeouts, tool filters, comments) survive, and a literal credential already in the target is
//! never overwritten by a `${VAR}` reference to a variable that may not be exported. Secret values
//! are never written from Forge: a token is referenced by environment variable name (`${VAR}` for
//! Claude, `bearer_token_env_var` / `env_vars` for Codex).

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
    pub warnings: Vec<String>,
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

fn secret_names(server: &McpServerConfig, token_env: Option<&String>) -> Vec<String> {
    token_env
        .cloned()
        .into_iter()
        .chain(server.secret_env.iter().map(|secret| secret.env.clone()))
        .collect()
}

fn claude_entry(server: &McpServerConfig) -> Value {
    let token_env = server.auth.as_ref().and_then(|a| a.token_env.clone());
    match &server.transport {
        McpTransport::Stdio { command, args, env } => {
            let mut vars: Map<String, Value> =
                env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            for var in secret_names(server, token_env.as_ref()) {
                vars.entry(var.clone())
                    .or_insert_with(|| json!(format!("${{{var}}}")));
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

fn is_reference(value: &str) -> bool {
    value.contains("${")
}

fn is_literal(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty() && !is_reference(text))
}

fn is_remote(entry: &Map<String, Value>) -> bool {
    entry.contains_key("url")
}

fn merge_claude(existing: Option<&Value>, entry: Value) -> Value {
    let Some(Value::Object(old)) = existing else {
        return entry;
    };
    let Value::Object(new) = entry else {
        return entry;
    };
    let mut merged = old.clone();
    if is_remote(old) != is_remote(&new) {
        for key in ["command", "args", "env", "url", "headers"] {
            merged.remove(key);
        }
    }
    for (key, value) in new {
        match (key.as_str(), value) {
            ("env" | "headers", Value::Object(wanted)) => {
                let slot = merged
                    .entry(key)
                    .or_insert_with(|| json!({}))
                    .as_object_mut();
                let Some(slot) = slot else { continue };
                for (name, value) in wanted {
                    let reference = value.as_str().is_some_and(is_reference);
                    if reference && is_literal(slot.get(&name)) {
                        continue;
                    }
                    slot.insert(name, value);
                }
            }
            (_, value) => {
                merged.insert(key, value);
            }
        }
    }
    Value::Object(merged)
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
        let previous = table.get(&server.name);
        let entry = merge_claude(previous, claude_entry(server));
        report.record(&server.name, previous.is_some(), previous == Some(&entry));
        table.insert(server.name.clone(), entry);
        if !server.disabled_tools.is_empty() {
            report.warnings.push(format!(
                "{}: Claude Code has no per-server tool filter, so {} tool(s) switched off in Forge stay callable there",
                server.name,
                server.disabled_tools.len()
            ));
        }
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
            let vars = secret_names(server, token_env.as_ref());
            if !vars.is_empty() {
                table["env_vars"] = value(vars.iter().map(String::as_str).collect::<Array>());
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
    if !server.disabled_tools.is_empty() {
        table["disabled_tools"] = value(
            server
                .disabled_tools
                .iter()
                .map(String::as_str)
                .collect::<Array>(),
        );
    }
    table
}

fn plain(item: &toml_edit::Item) -> Value {
    use toml_edit::{Item, Value as V};
    fn plain_value(value: &V) -> Value {
        match value {
            V::String(s) => json!(s.value()),
            V::Integer(i) => json!(i.value()),
            V::Float(f) => json!(f.value()),
            V::Boolean(b) => json!(b.value()),
            V::Datetime(d) => json!(d.value().to_string()),
            V::Array(items) => Value::Array(items.iter().map(plain_value).collect()),
            V::InlineTable(table) => Value::Object(
                table
                    .iter()
                    .map(|(k, v)| (k.to_string(), plain_value(v)))
                    .collect(),
            ),
        }
    }
    match item {
        Item::None => Value::Null,
        Item::Value(value) => plain_value(value),
        Item::Table(table) => Value::Object(
            table
                .iter()
                .map(|(k, v)| (k.to_string(), plain(v)))
                .collect(),
        ),
        Item::ArrayOfTables(tables) => Value::Array(
            tables
                .iter()
                .map(|t| Value::Object(t.iter().map(|(k, v)| (k.to_string(), plain(v))).collect()))
                .collect(),
        ),
    }
}

fn string_pairs(item: Option<&toml_edit::Item>) -> Vec<(String, String)> {
    item.and_then(toml_edit::Item::as_table_like)
        .map(|table| {
            table
                .iter()
                .filter_map(|(k, v)| Some((k.to_string(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn merge_pairs(
    existing: &mut toml_edit::Table,
    key: &str,
    wanted: &toml_edit::Item,
    keep_literals: bool,
) {
    let mut pairs = string_pairs(existing.get(key));
    for (name, value) in string_pairs(Some(wanted)) {
        let current = pairs.iter().position(|(k, _)| *k == name);
        let literal_there =
            current.is_some_and(|at| !pairs[at].1.is_empty() && !is_reference(&pairs[at].1));
        if keep_literals && is_reference(&value) && literal_there {
            continue;
        }
        match current {
            Some(at) => pairs[at].1 = value,
            None => pairs.push((name, value)),
        }
    }
    let mut map = toml_edit::InlineTable::new();
    for (k, v) in &pairs {
        map.insert(k, v.as_str().into());
    }
    existing[key] = toml_edit::value(map);
}

fn merge_codex(existing: &toml_edit::Table, entry: toml_edit::Table) -> toml_edit::Table {
    use toml_edit::value;
    let mut merged = existing.clone();
    let was_remote = existing.contains_key("url");
    let is_remote_now = entry.contains_key("url");
    if was_remote != is_remote_now {
        for key in [
            "command",
            "args",
            "env",
            "env_vars",
            "url",
            "bearer_token_env_var",
            "env_http_headers",
            "http_headers",
        ] {
            merged.remove(key);
        }
    }
    let literal_authorization = string_pairs(existing.get("http_headers"))
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && !is_reference(v));
    let literal_env: Vec<String> = string_pairs(existing.get("env"))
        .into_iter()
        .filter(|(_, v)| !v.is_empty() && !is_reference(v))
        .map(|(k, _)| k)
        .collect();
    for (key, item) in entry.iter() {
        match key {
            "env" | "http_headers" => merge_pairs(&mut merged, key, item, true),
            "env_http_headers" => merge_pairs(&mut merged, key, item, false),
            "bearer_token_env_var" if literal_authorization => {}
            "env_vars" => {
                let mut vars: Vec<String> = merged
                    .get(key)
                    .and_then(|i| i.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                for var in item
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str())
                {
                    if !vars.iter().any(|v| v == var) && !literal_env.iter().any(|v| v == var) {
                        vars.push(var.to_string());
                    }
                }
                if !vars.is_empty() {
                    merged[key] = value(
                        vars.iter()
                            .map(String::as_str)
                            .collect::<toml_edit::Array>(),
                    );
                }
            }
            _ => {
                merged[key] = item.clone();
            }
        }
    }
    merged
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
        if let Some(item) = table.get_mut(&server.name) {
            if item.is_inline_table() {
                *item = std::mem::take(item)
                    .into_table()
                    .map_or(toml_edit::Item::None, toml_edit::Item::Table);
            }
        }
        let previous = table.get(&server.name).and_then(|i| i.as_table());
        let merged = merge_codex(
            previous.unwrap_or(&toml_edit::Table::new()),
            codex_entry(server),
        );
        let same = previous.is_some_and(|p| {
            plain(&toml_edit::Item::Table(p.clone()))
                == plain(&toml_edit::Item::Table(merged.clone()))
        });
        report.record(&server.name, table.contains_key(&server.name), same);
        if !same {
            table.insert(&server.name, toml_edit::Item::Table(merged));
        }
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
    use std::io::Write;
    let write_error = |e: std::io::Error| ConfigError::Write(e.to_string());
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let permissions = std::fs::metadata(&target).ok().map(|m| m.permissions());
    let mut tmp_name = target.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".forge-sync.tmp");
    let tmp = target.with_file_name(tmp_name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let result = (|| {
        let mut file = options.open(&tmp)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(write_error)
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

    #[test]
    fn claude_merge_keeps_literal_credentials_and_target_only_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        std::fs::write(
            &path,
            r#"{"mcpServers":{"a":{"type":"stdio","command":"node","args":["old.js"],"timeout":9,"env":{"SRV_TOKEN":"ghp_literal","OTHER":"1"}},
               "b":{"type":"http","url":"https://x.test/mcp","headers":{"Authorization":"Bearer literal"}}}}"#,
        )
        .unwrap();
        let mut a = stdio("a");
        a.secret_env = vec![crate::mcp::McpSecretEnv {
            env: "EXTRA".into(),
            keyring: "mcp:a:EXTRA".into(),
        }];
        a.disabled_tools = vec!["danger".into()];
        let mut b = http("b");
        b.auth.as_mut().unwrap().token_env = None;
        let report = sync_claude(&path, &[&a, &b], false).unwrap();
        assert_eq!(report.updated, ["a"]);
        assert_eq!(report.unchanged, ["b"]);
        assert!(report.warnings[0].contains("danger") || report.warnings[0].contains("1 tool"));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let a = &v["mcpServers"]["a"];
        assert_eq!(a["args"][0], "srv.js");
        assert_eq!(a["timeout"], 9);
        assert_eq!(a["env"]["SRV_TOKEN"], "ghp_literal");
        assert_eq!(a["env"]["OTHER"], "1");
        assert_eq!(a["env"]["EXTRA"], "${EXTRA}");
        assert_eq!(
            v["mcpServers"]["b"]["headers"]["Authorization"],
            "Bearer literal"
        );
    }

    #[test]
    fn codex_merge_keeps_unmanaged_keys_literals_and_carries_disabled_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "[mcp_servers.a]\n# keep me\ncommand = \"node\"\nstartup_timeout_sec = 30\nenabled_tools = [\"x\"]\nenv = { SRV_TOKEN = \"ghp_literal\" }\n\n[mcp_servers.b]\nurl = \"https://x.test/mcp\"\nhttp_headers = { Authorization = \"Bearer literal\" }\n",
        )
        .unwrap();
        let mut a = stdio("a");
        a.disabled_tools = vec!["danger".into()];
        let b = http("b");
        let report = sync_codex(&path, &[&a, &b], false).unwrap();
        assert_eq!(report.updated, ["a"]);
        assert_eq!(report.unchanged, ["b"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep me"), "{text}");
        let parsed: toml::Table = toml::from_str(&text).unwrap();
        let a = &parsed["mcp_servers"]["a"];
        assert_eq!(a["startup_timeout_sec"].as_integer(), Some(30));
        assert_eq!(a["enabled_tools"][0].as_str(), Some("x"));
        assert_eq!(a["disabled_tools"][0].as_str(), Some("danger"));
        assert_eq!(a["env"]["SRV_TOKEN"].as_str(), Some("ghp_literal"));
        assert!(a.get("env_vars").is_none(), "{text}");
        let b = &parsed["mcp_servers"]["b"];
        assert!(b.get("bearer_token_env_var").is_none(), "{text}");
        assert_eq!(
            b["http_headers"]["Authorization"].as_str(),
            Some("Bearer literal")
        );
        let again = sync_codex(&path, &[&a_server(), &http("b")], false).unwrap();
        assert!(
            again.updated.is_empty() || again.updated == ["a"],
            "{again:?}"
        );
    }

    fn a_server() -> McpServerConfig {
        stdio("a")
    }

    #[cfg(unix)]
    #[test]
    fn rewriting_a_target_keeps_its_mode_and_symlink() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        std::fs::write(&real, "model = \"x\"\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        sync_codex(&link, &[&stdio("a")], false).unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        let mode = std::fs::metadata(&real).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(std::fs::read_to_string(&real)
            .unwrap()
            .contains("mcp_servers"));
        let fresh = dir.path().join("fresh.json");
        sync_claude(&fresh, &[&stdio("a")], false).unwrap();
        assert_eq!(
            std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
