//! Claude Code's `--setting-sources user,project,local`: which configuration layers load.
//!
//! Process-wide on purpose: the flag scopes a whole `forge run` invocation, and that process calls
//! `load()` from many places, every one of which must see the same layers. Unset means every
//! layer, exactly as before. Forge has one project layer (`.forge/`), so `project` and `local`
//! (Claude Code's `.claude/settings.local.json`) both select it.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingSources {
    /// The user config directory: `config.toml`, `mcp.toml`, `settings.json`.
    pub user: bool,
    /// The project's `.forge/`: `config.toml`, `mcp.toml`, `settings.json`.
    pub project: bool,
}

impl Default for SettingSources {
    fn default() -> Self {
        Self {
            user: true,
            project: true,
        }
    }
}

impl SettingSources {
    /// Parse a comma-separated list; an empty list loads neither layer (built-in defaults only).
    pub fn parse(list: &str) -> Result<Self, String> {
        let mut sources = Self {
            user: false,
            project: false,
        };
        for item in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
            match item {
                "user" => sources.user = true,
                "project" | "local" => sources.project = true,
                other => {
                    return Err(format!(
                        "unknown setting source '{other}' (expected user, project or local)"
                    ))
                }
            }
        }
        Ok(sources)
    }
}

static SOURCES: OnceLock<SettingSources> = OnceLock::new();

/// Restrict every later config load in this process. The first call wins.
pub fn set_setting_sources(sources: SettingSources) {
    let _ = SOURCES.set(sources);
}

pub fn setting_sources() -> SettingSources {
    SOURCES.get().copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_code_source_lists() {
        assert_eq!(
            SettingSources::parse("user,project").unwrap(),
            SettingSources::default()
        );
        assert_eq!(
            SettingSources::parse("local").unwrap(),
            SettingSources {
                user: false,
                project: true
            }
        );
        assert_eq!(
            SettingSources::parse("").unwrap(),
            SettingSources {
                user: false,
                project: false
            }
        );
        assert!(SettingSources::parse("user,policy").is_err());
    }
}
