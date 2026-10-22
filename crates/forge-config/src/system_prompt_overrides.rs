//! `system_prompt_overrides` values that name a file (`@path`) are read at load time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{config_dir, Config, ConfigError};

/// Resolve `@path` system-prompt overrides. A value beginning with `@` names a file whose contents
/// become the prompt (so large or shared prompts can live in a file instead of being inlined).
/// `~/…` expands to the home directory, a bare relative path resolves against the Forge config
/// directory, and an absolute path is used as-is. Inline values (no leading `@`) are left untouched.
pub(crate) fn resolve_system_prompt_overrides(config: &mut Config) -> Result<(), ConfigError> {
    let base = config_dir();
    let overrides = std::mem::take(&mut config.system_prompt_overrides);
    let mut resolved = HashMap::new();
    for (provider, value) in overrides {
        let Some(path_str) = value.strip_prefix('@') else {
            resolved.insert(provider, value);
            continue;
        };
        let path = resolve_prompt_path(path_str, base.as_deref());
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| ConfigError::PromptFile(path.display().to_string(), e.to_string()))?;
        resolved.insert(provider, contents);
    }
    config.system_prompt_overrides = resolved;
    Ok(())
}

fn resolve_prompt_path(raw: &str, base: Option<&Path>) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        base.map(|b| b.join(&path)).unwrap_or(path)
    }
}
