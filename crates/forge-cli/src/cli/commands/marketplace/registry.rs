//! Persisted marketplace registry and installed-skill lockfile state.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// marketplaces.toml — the name → source registry
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Marketplaces {
    #[serde(default)]
    pub(crate) marketplaces: BTreeMap<String, MarketplaceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MarketplaceEntry {
    /// A GitHub `owner/repo` (top-level dirs = packages), or a full git URL.
    pub(crate) source: String,
    /// Optional pinned branch/tag for the whole marketplace.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "ref")]
    pub(crate) git_ref: Option<String>,
}

pub(crate) fn load_marketplaces_at(path: &Path) -> Result<Marketplaces> {
    load_toml_or_default(path, "marketplaces.toml")
}

fn save_marketplaces_at(path: &Path, m: &Marketplaces) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let body = toml::to_string_pretty(m).context("serializing marketplaces.toml")?;
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))
}

/// Add (or overwrite) a marketplace entry. Pure over the given path. Returns whether it replaced one.
pub(crate) fn add_marketplace_at(
    path: &Path,
    name: &str,
    source: &str,
    git_ref: Option<String>,
) -> Result<bool> {
    if name.trim().is_empty() {
        anyhow::bail!("marketplace name cannot be empty");
    }
    let mut m = load_marketplaces_at(path)?;
    let replaced = m
        .marketplaces
        .insert(
            name.to_string(),
            MarketplaceEntry {
                source: source.to_string(),
                git_ref,
            },
        )
        .is_some();
    save_marketplaces_at(path, &m)?;
    Ok(replaced)
}

/// Remove a marketplace entry. Returns whether one existed.
pub(crate) fn remove_marketplace_at(path: &Path, name: &str) -> Result<bool> {
    let mut m = load_marketplaces_at(path)?;
    let existed = m.marketplaces.remove(name).is_some();
    if existed {
        save_marketplaces_at(path, &m)?;
    }
    Ok(existed)
}

// ---------------------------------------------------------------------------
// installed-skills.toml — the install lockfile
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct InstalledSkills {
    #[serde(default)]
    pub(crate) skills: BTreeMap<String, InstalledEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct InstalledEntry {
    /// The repo/URL the pack was fetched from (the clone target).
    pub(crate) source: String,
    /// The marketplace it was resolved through, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) marketplace: Option<String>,
    /// The subdirectory within `source` holding this package (marketplace installs), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) subdir: Option<String>,
    /// The pinned branch/tag/ref, if any.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "ref")]
    pub(crate) git_ref: Option<String>,
    /// The skill file/dir names written into the skills dir (for update/removal).
    #[serde(default)]
    pub(crate) files: Vec<String>,
}

pub(crate) fn load_installed_at(path: &Path) -> Result<InstalledSkills> {
    load_toml_or_default(path, "installed-skills.toml")
}

fn load_toml_or_default<T: serde::de::DeserializeOwned + Default>(
    path: &Path,
    name: &str,
) -> Result<T> {
    match std::fs::read_to_string(path) {
        Ok(body) => {
            toml::from_str(&body).with_context(|| format!("parsing {name} at {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error).with_context(|| format!("reading {name} at {}", path.display())),
    }
}

fn save_installed_at(path: &Path, lock: &InstalledSkills) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let body = toml::to_string_pretty(lock).context("serializing installed-skills.toml")?;
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))
}

/// Record (or replace) an installed pack in the lockfile. Pure over the given path.
pub(crate) fn record_installed_at(path: &Path, name: &str, entry: InstalledEntry) -> Result<()> {
    let mut lock = load_installed_at(path)?;
    lock.skills.insert(name.to_string(), entry);
    save_installed_at(path, &lock)
}

/// Drop a pack from the lockfile (used by `forge plugin remove`). Returns whether it existed.
pub(crate) fn remove_installed_at(path: &Path, name: &str) -> Result<bool> {
    let mut lock = load_installed_at(path)?;
    let existed = lock.skills.remove(name).is_some();
    if existed {
        save_installed_at(path, &lock)?;
    }
    Ok(existed)
}

/// Drop a pack from the default lockfile.
pub(crate) fn remove_installed_entry(name: &str) -> Result<bool> {
    remove_installed_at(&installed_path()?, name)
}

// ---------------------------------------------------------------------------
// Default config paths
// ---------------------------------------------------------------------------

fn config_dir() -> Result<PathBuf> {
    forge_config::config_dir().context("no user config directory on this platform")
}
pub(crate) fn marketplaces_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("marketplaces.toml"))
}
pub(crate) fn installed_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("installed-skills.toml"))
}
pub(crate) fn skills_dir() -> Result<PathBuf> {
    Ok(config_dir()?.join("skills"))
}
