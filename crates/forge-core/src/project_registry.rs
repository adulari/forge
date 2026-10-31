//! User-level project bookkeeping the session history cannot express: a display name, "hide this
//! project", and "this folder moved". Lives in `<config dir>/projects.json`; the project catalog
//! applies it on read, so nothing in the session store is rewritten.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    /// The project now lives here; its old path resolves to this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<String>,
}

impl ProjectEntry {
    fn is_empty(&self) -> bool {
        self.name.is_none() && !self.disabled && self.moved_to.is_none()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRegistry {
    #[serde(default)]
    pub entries: BTreeMap<String, ProjectEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ProjectPatch {
    /// `Some("")` clears the custom name.
    pub name: Option<String>,
    pub disabled: Option<bool>,
    /// An absolute, existing directory; `Some("")` clears the relocation.
    pub relocate_to: Option<String>,
}

fn registry_path() -> Option<PathBuf> {
    forge_config::config_dir().map(|d| d.join("projects.json"))
}

impl ProjectRegistry {
    pub fn load() -> Self {
        registry_path().map_or_else(Self::default, |p| Self::load_from(&p))
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = registry_path().ok_or("no config directory")?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn entry(&self, path: &str) -> Option<&ProjectEntry> {
        self.entries.get(path)
    }

    /// Where `path` really is now, following relocations (bounded, so a cycle cannot hang).
    pub fn resolve(&self, path: &str) -> String {
        let mut current = path.to_string();
        for _ in 0..8 {
            match self.entries.get(&current).and_then(|e| e.moved_to.clone()) {
                Some(next) if next != current => current = next,
                _ => break,
            }
        }
        current
    }

    pub fn apply(&mut self, path: &str, patch: &ProjectPatch) -> Result<(), String> {
        if !Path::new(path).is_absolute() {
            return Err("project path must be absolute".into());
        }
        if let Some(target) = patch.relocate_to.as_deref().filter(|t| !t.is_empty()) {
            let target_path = Path::new(target);
            if !target_path.is_absolute() || !target_path.is_dir() {
                return Err("the new location must be an existing absolute directory".into());
            }
            if self.resolve(target) == path {
                return Err("that would point the project at itself".into());
            }
        }
        let entry = self.entries.entry(path.to_string()).or_default();
        if let Some(name) = &patch.name {
            let name = name.trim();
            entry.name = (!name.is_empty()).then(|| name.chars().take(80).collect());
        }
        if let Some(disabled) = patch.disabled {
            entry.disabled = disabled;
        }
        if let Some(target) = &patch.relocate_to {
            entry.moved_to = (!target.is_empty()).then(|| target.clone());
        }
        if entry.is_empty() {
            self.entries.remove(path);
        }
        Ok(())
    }

    /// Point every path in `duplicates` at `keep` (used by duplicate repair).
    pub fn merge_into(&mut self, keep: &str, duplicates: &[String]) {
        for dup in duplicates.iter().filter(|d| d.as_str() != keep) {
            self.entries.entry(dup.clone()).or_default().moved_to = Some(keep.to_string());
        }
    }
}

#[cfg(test)]
#[path = "tests/project_registry.rs"]
mod tests;
