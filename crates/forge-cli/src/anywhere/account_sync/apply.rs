//! Writes an incoming item onto this machine. Every path is derived from a validated item name,
//! never from a path the sender chose.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use super::catalog::{config_dir, on_path, slot_is_syncable};
use super::files::{plain_name, remove, write_bundle};
use super::{Category, Payload};

pub(super) enum Outcome {
    Applied,
    /// Not applicable on this machine; the reason is shown to the user.
    Skipped(String),
}

fn dir(sub: &str) -> Result<PathBuf> {
    Ok(config_dir()
        .context("no Forge config directory on this machine")?
        .join(sub))
}

/// `kind/name` -> a directory/file name that cannot escape its parent.
fn split(item: &str) -> Result<(&str, &str)> {
    let (kind, name) = item.split_once('/').context("malformed synced item name")?;
    if !plain_name(name) && kind != "login" && kind != "setting" && kind != "tool" {
        bail!("synced item has an unsafe name");
    }
    Ok((kind, name))
}

fn content_dir(kind: &str) -> Option<&'static str> {
    match kind {
        "skill" => Some("skills"),
        "command" => Some("commands"),
        "agent" => Some("agents"),
        _ => None,
    }
}

pub(super) async fn apply(
    category: Category,
    payload: &Payload,
    secret: Option<String>,
    keep_conflict: bool,
) -> Result<Outcome> {
    let value = payload.value.clone().unwrap_or(Value::Null);
    match category {
        Category::Keys => apply_key(&payload.item, secret),
        Category::Skills => apply_skill(&payload.item, &value, keep_conflict).await,
        Category::Settings => apply_setting(&payload.item, &value, keep_conflict),
        Category::Mcp => apply_mcp(&payload.item, &value),
        Category::Instructions => {
            if payload.item != "file/AGENTS.md" {
                bail!("unknown instructions item");
            }
            write_bundle(&dir("AGENTS.md")?, &value, keep_conflict)?;
            Ok(Outcome::Applied)
        }
    }
}

pub(super) fn remove_local(category: Category, item: &str) -> Result<()> {
    match category {
        Category::Keys => {
            if item.starts_with("login/") {
                return Ok(());
            }
            if !slot_is_syncable(item) {
                bail!("refusing to remove a secret slot that does not sync");
            }
            forge_config::delete_secret(item)?;
        }
        Category::Skills => {
            let (kind, name) = split(item)?;
            match (content_dir(kind), kind) {
                (Some(sub), _) => remove(&dir(sub)?.join(name))?,
                (None, "pack") => remove_pack(name)?,
                (None, "marketplace") => {
                    edit_toml_table("marketplaces.toml", "marketplaces", name, None)?
                }
                _ => bail!("unknown skills item"),
            }
        }
        Category::Settings => {
            let (kind, name) = split(item)?;
            match kind {
                "setting" => forge_config::account_sync::apply_portable_settings(&BTreeMap::from(
                    [(name.to_string(), None)],
                ))?,
                "file" if name == "keybindings.json" => remove(&dir("keybindings.json")?)?,
                _ => {}
            }
        }
        Category::Mcp => {
            let name = item.strip_prefix("server/").context("unknown mcp item")?;
            let path = dir("mcp.toml")?;
            let mut config = forge_config::load_mcp_toml(&path);
            config.servers.retain(|s| s.name != name);
            forge_config::write_mcp_toml(&path, &config)?;
        }
        Category::Instructions => remove(&dir("AGENTS.md")?)?,
    }
    Ok(())
}

fn apply_key(item: &str, secret: Option<String>) -> Result<Outcome> {
    if item.starts_with("login/") || item.starts_with("tool/") {
        return Ok(Outcome::Applied);
    }
    if !slot_is_syncable(item) {
        bail!("a synced key names a slot that does not sync");
    }
    let secret = secret.context("synced key carries no secret")?;
    forge_config::store_secret(item, &secret)?;
    Ok(Outcome::Applied)
}

async fn apply_skill(item: &str, value: &Value, keep_conflict: bool) -> Result<Outcome> {
    let (kind, name) = split(item)?;
    if let Some(sub) = content_dir(kind) {
        write_bundle(&dir(sub)?.join(name), value, keep_conflict)?;
        return Ok(Outcome::Applied);
    }
    match kind {
        "marketplace" => {
            edit_toml_table("marketplaces.toml", "marketplaces", name, Some(value))?;
            Ok(Outcome::Applied)
        }
        "pack" => install_pack(name, value).await,
        _ => bail!("unknown skills item"),
    }
}

async fn install_pack(name: &str, entry: &Value) -> Result<Outcome> {
    let lock = dir("installed-skills.toml")?;
    let installed = crate::cli::commands::marketplace::registry_has_pack(&lock, name);
    if installed {
        return Ok(Outcome::Applied);
    }
    let marketplace = entry.get("marketplace").and_then(Value::as_str);
    match crate::cli::commands::marketplace::install_synced_pack(name, marketplace).await {
        Ok(()) => Ok(Outcome::Applied),
        Err(error) => Ok(Outcome::Skipped(format!(
            "could not install pack: {}",
            first_line(&error)
        ))),
    }
}

fn remove_pack(name: &str) -> Result<()> {
    crate::cli::commands::marketplace::remove_plugin(name).map(|_| ())
}

fn first_line(error: &anyhow::Error) -> String {
    error
        .to_string()
        .lines()
        .next()
        .unwrap_or("failed")
        .chars()
        .take(120)
        .collect()
}

fn edit_toml_table(file: &str, table: &str, name: &str, value: Option<&Value>) -> Result<()> {
    let path = dir(file)?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut root: toml::Table = toml::from_str(&text).unwrap_or_default();
    let section = root
        .entry(table.to_string())
        .or_insert_with(|| toml::Value::Table(Default::default()));
    let toml::Value::Table(section) = section else {
        bail!("{file} has an unexpected shape")
    };
    match value {
        Some(v) => {
            let v: toml::Value = serde_json::from_value(v.clone())?;
            section.insert(name.to_string(), v);
        }
        None => {
            section.remove(name);
        }
    }
    forge_config::write_atomic(&path, toml::to_string_pretty(&root)?)?;
    Ok(())
}

fn apply_setting(item: &str, value: &Value, keep_conflict: bool) -> Result<Outcome> {
    let (kind, name) = split(item)?;
    let _lock = crate::serve::config_mutation_lock().lock();
    match kind {
        "setting" => {
            forge_config::account_sync::apply_portable_settings(&BTreeMap::from([(
                name.to_string(),
                Some(value.clone()),
            )]))?;
        }
        "file" if name == "keybindings.json" => {
            write_bundle(&dir("keybindings.json")?, value, keep_conflict)?
        }
        "tool" => {}
        _ => bail!("unknown settings item"),
    }
    Ok(Outcome::Applied)
}

fn apply_mcp(item: &str, value: &Value) -> Result<Outcome> {
    let name = item.strip_prefix("server/").context("unknown mcp item")?;
    let server: forge_config::mcp::McpServerConfig =
        serde_json::from_value(value.clone()).context("synced MCP server is malformed")?;
    if server.name != name || !plain_name(name) {
        bail!("synced MCP server name does not match its item");
    }
    if let forge_config::mcp::McpTransport::Stdio { command, .. } = &server.transport {
        if !on_path(command) {
            return Ok(Outcome::Skipped(format!(
                "needs `{command}`, not installed here"
            )));
        }
    }
    let _lock = crate::serve::config_mutation_lock().lock();
    let path = dir("mcp.toml")?;
    let mut config = forge_config::load_mcp_toml(&path);
    match config.servers.iter_mut().find(|s| s.name == name) {
        Some(existing) => *existing = server,
        None => config.servers.push(server),
    }
    forge_config::write_mcp_toml(&path, &config)?;
    Ok(Outcome::Applied)
}
