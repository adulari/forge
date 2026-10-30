//! Comment-preserving edits of `config.toml`.
//!
//! Every programmatic config write goes through `toml_edit` so a user's comments, key order and
//! formatting survive. A file that does not parse is an error naming the line, never silently
//! replaced by an empty document.

use std::path::Path;

use toml_edit::{DocumentMut, Item, Table};

use crate::ConfigError;

/// 1-based line and column of byte `offset` in `text`.
pub(crate) fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text.as_bytes()[..offset];
    let line = before.iter().filter(|b| **b == b'\n').count() + 1;
    let col = offset
        - before
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |i| i + 1)
        + 1;
    (line, col)
}

/// Parse `text` for editing; `label` names the file in the error, which carries `line:column`.
pub(crate) fn parse_doc(text: &str, label: &str) -> Result<DocumentMut, ConfigError> {
    text.parse::<DocumentMut>().map_err(|error| {
        let at = error
            .span()
            .map(|span| line_col(text, span.start))
            .map(|(line, col)| format!(":{line}:{col}"))
            .unwrap_or_default();
        ConfigError::Write(format!(
            "cannot parse config {label}{at}: {}",
            error.message()
        ))
    })
}

/// Load `path` for editing. A missing file is an empty document; an unreadable or malformed one is
/// an error, so a write never destroys a config it could not understand.
pub(crate) fn load_doc(path: &Path) -> Result<DocumentMut, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_doc(&text, &path.display().to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(error) => Err(ConfigError::Write(format!(
            "cannot read config {}: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn save_doc(path: &Path, doc: &DocumentMut) -> Result<(), ConfigError> {
    crate::write_atomic(path, doc.to_string()).map_err(|e| ConfigError::Write(e.to_string()))
}

fn plain_value(value: &toml::Value) -> toml_edit::Value {
    match value {
        toml::Value::String(s) => toml_edit::Value::from(s.as_str()),
        toml::Value::Integer(i) => toml_edit::Value::from(*i),
        toml::Value::Float(f) => toml_edit::Value::from(*f),
        toml::Value::Boolean(b) => toml_edit::Value::from(*b),
        toml::Value::Datetime(d) => d
            .to_string()
            .parse()
            .unwrap_or_else(|_| toml_edit::Value::from(d.to_string())),
        toml::Value::Array(items) => {
            toml_edit::Value::Array(items.iter().map(plain_value).collect())
        }
        toml::Value::Table(table) => {
            let mut inline = toml_edit::InlineTable::new();
            for (key, item) in table {
                inline.insert(key, plain_value(item));
            }
            toml_edit::Value::InlineTable(inline)
        }
    }
}

/// A `toml::Value` as an editable item: tables become real `[tables]`, arrays of tables become
/// `[[arrays]]`, everything else an inline value.
pub(crate) fn value_item(value: &toml::Value) -> Item {
    match value {
        toml::Value::Table(table) => {
            let mut out = Table::new();
            for (key, item) in table {
                out.insert(key, value_item(item));
            }
            Item::Table(out)
        }
        toml::Value::Array(items)
            if !items.is_empty() && items.iter().all(toml::Value::is_table) =>
        {
            let mut out = toml_edit::ArrayOfTables::new();
            for item in items {
                if let Item::Table(table) = value_item(item) {
                    out.push(table);
                }
            }
            Item::ArrayOfTables(out)
        }
        other => Item::Value(plain_value(other)),
    }
}

/// Set `key` in `table`, keeping the position and the trailing comment of an existing value.
pub(crate) fn put(table: &mut Table, key: &str, item: Item) {
    let item = match (table.get(key), item) {
        (Some(Item::Value(old)), Item::Value(mut new)) => {
            *new.decor_mut() = old.decor().clone();
            Item::Value(new)
        }
        (_, item) => item,
    };
    // Assign through the slot rather than `insert`, which would replace the key and drop the
    // comment lines above it.
    match table.get_mut(key) {
        Some(slot) => *slot = item,
        None => {
            table.insert(key, item);
        }
    }
}

fn implicit_table() -> Table {
    let mut table = Table::new();
    table.set_implicit(true);
    table
}

/// The table at `path`, created (and a non-table in the way replaced) as needed. An empty `path`
/// is the document root.
pub(crate) fn table_at<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> &'a mut Table {
    let mut cur = doc.as_table_mut();
    for key in path {
        if !cur.get(key).is_some_and(Item::is_table) {
            cur.insert(key, Item::Table(implicit_table()));
        }
        cur = cur
            .get_mut(key)
            .and_then(Item::as_table_mut)
            .expect("table just ensured");
    }
    cur
}

/// Make `dst` hold exactly the keys of `src`, recursing into tables so comments on kept keys
/// survive; keys absent from `src` are removed.
pub(crate) fn sync_table(dst: &mut Table, src: &toml::Table) {
    let stale: Vec<String> = dst
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| !src.contains_key(key))
        .collect();
    for key in stale {
        dst.remove(&key);
    }
    for (key, value) in src {
        if let toml::Value::Table(inner) = value {
            if let Some(existing) = dst.get_mut(key).and_then(Item::as_table_mut) {
                sync_table(existing, inner);
                continue;
            }
        }
        if let toml::Value::Array(items) = value {
            if let Some(existing) = dst.get_mut(key).and_then(Item::as_array_of_tables_mut) {
                if items.iter().all(toml::Value::is_table) {
                    sync_array_of_tables(existing, items);
                    continue;
                }
            }
        }
        put(dst, key, value_item(value));
    }
}

/// Like [`sync_table`] for `[[arrays]]`: an element is matched to an existing one by its `name`
/// (so its comments survive a reorder), otherwise it is new.
fn sync_array_of_tables(dst: &mut toml_edit::ArrayOfTables, src: &[toml::Value]) {
    let mut old: Vec<Option<Table>> = dst.iter().cloned().map(Some).collect();
    let name_of = |table: &Table| table.get("name").and_then(Item::as_str).map(str::to_string);
    dst.clear();
    for value in src {
        let toml::Value::Table(new) = value else {
            continue;
        };
        let wanted = new.get("name").and_then(toml::Value::as_str);
        let slot = wanted.and_then(|wanted| {
            old.iter_mut()
                .find(|slot| slot.as_ref().and_then(name_of).as_deref() == Some(wanted))
        });
        let mut table = slot.and_then(Option::take).unwrap_or_default();
        sync_table(&mut table, new);
        dst.push(table);
    }
}

pub(crate) fn set_dotted(doc: &mut DocumentMut, path: &str, value: &toml::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let (leaf, parents) = parts.split_last().expect("split yields at least one part");
    put(table_at(doc, parents), leaf, value_item(value));
}

pub(crate) fn remove_dotted(doc: &mut DocumentMut, path: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    let (leaf, parents) = parts.split_last().expect("split yields at least one part");
    let mut cur = doc.as_table_mut();
    for key in parents {
        match cur.get_mut(key).and_then(Item::as_table_mut) {
            Some(next) => cur = next,
            None => return,
        }
    }
    cur.remove(leaf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use figment::providers::Format;

    #[test]
    fn line_col_counts_from_one() {
        assert_eq!(line_col("a\nbc\n", 0), (1, 1));
        assert_eq!(line_col("a\nbc\n", 3), (2, 2));
    }

    #[test]
    fn edits_keep_comments_order_and_trailing_comment() {
        let mut doc = parse_doc(
            "# top\nz = 1 # keep me\n\n[mesh] # hdr\n# note\ncredit_mode = \"normal\" # inline\n",
            "t",
        )
        .unwrap();
        set_dotted(
            &mut doc,
            "mesh.credit_mode",
            &toml::Value::String("frugal".into()),
        );
        set_dotted(&mut doc, "a.b", &toml::Value::Integer(2));
        let out = doc.to_string();
        assert!(out.starts_with("# top\nz = 1 # keep me\n"), "{out}");
        assert!(
            out.contains("# note\ncredit_mode = \"frugal\" # inline"),
            "{out}"
        );
        assert!(out.contains("[mesh] # hdr"), "{out}");
        assert!(out.contains("[a]\nb = 2"), "{out}");
        remove_dotted(&mut doc, "mesh.credit_mode");
        assert!(!doc.to_string().contains("credit_mode"));
    }

    #[test]
    fn parse_error_names_the_line() {
        let error = parse_doc("a = 1\nb = = 2\n", "config.toml").unwrap_err();
        assert!(error.to_string().contains("config.toml:2:"), "{error}");
    }

    #[test]
    fn load_doc_refuses_a_malformed_file_instead_of_emptying_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[mesh\n").unwrap();
        assert!(load_doc(&path).is_err());
        assert!(load_doc(&dir.path().join("absent.toml"))
            .unwrap()
            .as_table()
            .is_empty());
    }

    const COMMENTED: &str = "# my forge config\npermission_mode = \"default\" # posture\n\n# routing\n[mesh]\n# keep frugal\ncredit_mode = \"normal\" # tweak\n\n[tui]\n# unrelated\nx = 1\n";

    fn assert_survives(path: &Path) {
        let out = std::fs::read_to_string(path).unwrap();
        for needle in [
            "# my forge config",
            "# posture",
            "# routing",
            "# keep frugal",
            "# unrelated",
            "[tui]",
        ] {
            assert!(out.contains(needle), "lost {needle:?}:\n{out}");
        }
    }

    #[test]
    fn every_writer_keeps_user_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, COMMENTED).unwrap();

        crate::write_settings_at(
            &path,
            crate::PermissionMode::AcceptEdits,
            crate::CreditMode::Frugal,
        )
        .unwrap();
        assert_survives(&path);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("credit_mode = \"frugal\" # tweak"));

        crate::write_anywhere_settings_at(&path, true, Some("box"), false).unwrap();
        assert_survives(&path);

        let provider = crate::CustomProviderConfig {
            pricing_stat: None,
            namespace: "lmstudio".into(),
            base_url: "http://localhost:1234/v1".into(),
            api_key_env: None,
            free: true,
            models: vec!["m".into()],
            label: None,
        };
        crate::add_custom_provider_at(&path, &provider).unwrap();
        assert_survives(&path);
        assert!(crate::remove_custom_provider_at(&path, "lmstudio").unwrap());
        assert_survives(&path);

        let azure = crate::AzureConfig {
            resource: Some("acme".into()),
            deployments: vec!["gpt-4o".into()],
            ..Default::default()
        };
        crate::add_azure_provider_at(&path, &azure).unwrap();
        assert_survives(&path);
        assert!(crate::remove_azure_provider_at(&path).unwrap());
        assert_survives(&path);

        let mut subs = std::collections::HashMap::new();
        subs.insert("claude".to_string(), "max".to_string());
        crate::write_subscriptions_at(&path, &subs).unwrap();
        assert_survives(&path);

        assert!(crate::editor_set_model_disabled_at(&path, "openrouter", true).unwrap());
        assert_survives(&path);

        let cfg: crate::Config = figment::Figment::from(figment::providers::Serialized::defaults(
            crate::Config::default(),
        ))
        .merge(figment::providers::Toml::file(&path))
        .extract()
        .unwrap();
        assert_eq!(cfg.permission_mode, crate::PermissionMode::AcceptEdits);
        assert!(cfg.mesh.disabled.iter().any(|d| d == "openrouter"));
    }

    #[test]
    fn writers_refuse_a_malformed_file_and_leave_it_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let broken = "# precious\n[mesh\ncredit_mode = 1\n";
        std::fs::write(&path, broken).unwrap();
        let error = crate::write_anywhere_settings_at(&path, true, None, false).unwrap_err();
        assert!(error.to_string().contains(":2:"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn mcp_write_keeps_comments_on_untouched_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.toml");
        std::fs::write(
            &path,
            "# servers\ncall_timeout_secs = 30 # slow\n\n[[servers]]\n# the fs one\nname = \"fs\"\n\n[servers.transport]\ntype = \"stdio\"\ncommand = \"fs-mcp\"\n",
        )
        .unwrap();
        let mut config = crate::mcp::load_mcp_toml(&path);
        config.servers.push(crate::mcp::McpServerConfig {
            disabled_tools: Vec::new(),
            name: "git".into(),
            transport: crate::mcp::McpTransport::Stdio {
                command: "git-mcp".into(),
                args: Vec::new(),
                env: Default::default(),
            },
            auth: None,
            secret_env: Vec::new(),
            enabled: true,
            shared: None,
        });
        crate::mcp::write_mcp_toml(&path, &config).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# servers"), "{out}");
        assert!(out.contains("# slow"), "{out}");
        assert!(out.contains("# the fs one"), "{out}");
        assert_eq!(crate::mcp::load_mcp_toml(&path).servers.len(), 2);
    }
}
