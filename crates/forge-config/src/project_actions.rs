//! `[[actions]]`: one-click dev scripts a project declares in `.forge/config.toml`.
//!
//! ```toml
//! [[actions]]
//! name = "dev"
//! command = "npm run dev"
//! cwd = "web"        # optional, relative to the worktree
//! icon = "play"      # optional, a short label the UIs map to a glyph
//! ```
//!
//! An action runs a shell command, so only a trusted project contributes them
//! ([`crate::project_trust`] keeps `actions` out of the untrusted subset).

use serde::{Deserialize, Serialize};

/// How many actions one project may declare; the rest are ignored.
pub const MAX_ACTIONS: usize = 24;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectActionConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// A validated action with its stable id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectAction {
    pub id: String,
    pub name: String,
    pub command: String,
    pub cwd: Option<String>,
    pub icon: Option<String>,
}

/// The action id for a name: lowercase letters, digits and `-`, so it is safe in a terminal id.
pub fn action_id(name: &str) -> String {
    let mut id = String::new();
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            id.push(ch.to_ascii_lowercase());
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    id.truncate(40);
    id.trim_end_matches('-').to_string()
}

/// A relative path that stays inside the worktree.
fn safe_cwd(cwd: &str) -> bool {
    let path = std::path::Path::new(cwd);
    !cwd.trim().is_empty()
        && path.is_relative()
        && path.components().all(|part| {
            matches!(
                part,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

/// The configured actions that have a name, a command and a unique id, in declared order.
pub fn resolve_actions(configured: &[ProjectActionConfig]) -> Vec<ProjectAction> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for entry in configured {
        let id = action_id(&entry.name);
        if id.is_empty() || entry.command.trim().is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        out.push(ProjectAction {
            id,
            name: entry.name.trim().to_string(),
            command: entry.command.trim().to_string(),
            cwd: entry.cwd.clone().filter(|cwd| safe_cwd(cwd)),
            icon: entry
                .icon
                .clone()
                .map(|icon| icon.trim().to_string())
                .filter(|icon| !icon.is_empty()),
        });
        if out.len() == MAX_ACTIONS {
            break;
        }
    }
    out
}

/// The actions of the project rooted at `root`, honouring project trust.
pub fn actions_for(root: &std::path::Path) -> Vec<ProjectAction> {
    crate::load_in(root)
        .map(|config| resolve_actions(&config.actions))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, command: &str, cwd: Option<&str>) -> ProjectActionConfig {
        ProjectActionConfig {
            name: name.into(),
            command: command.into(),
            cwd: cwd.map(Into::into),
            icon: None,
        }
    }

    #[test]
    fn ids_are_slugs() {
        assert_eq!(action_id("Dev server"), "dev-server");
        assert_eq!(action_id("  ../x!! "), "x");
        assert_eq!(action_id("!!"), "");
    }

    #[test]
    fn invalid_duplicate_and_escaping_entries_are_dropped_or_clamped() {
        let actions = resolve_actions(&[
            entry("dev", "npm run dev", Some("web")),
            entry("Dev", "again", None),
            entry("", "x", None),
            entry("empty", "  ", None),
            entry("escape", "ls", Some("../outside")),
            entry("abs", "ls", Some("/etc")),
        ]);
        let ids: Vec<_> = actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["dev", "escape", "abs"]);
        assert_eq!(actions[0].cwd.as_deref(), Some("web"));
        assert_eq!(actions[1].cwd, None);
        assert_eq!(actions[2].cwd, None);
    }

    #[test]
    fn parses_from_a_project_config() {
        #[derive(Deserialize)]
        struct Doc {
            actions: Vec<ProjectActionConfig>,
        }
        let doc: Doc = toml::from_str(
            "[[actions]]\nname = \"dev\"\ncommand = \"npm run dev\"\nicon = \"play\"\n",
        )
        .unwrap();
        let actions = resolve_actions(&doc.actions);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].icon.as_deref(), Some("play"));
    }

    #[test]
    fn an_untrusted_project_cannot_declare_actions() {
        let table: toml::Table =
            toml::from_str("[[actions]]\nname = \"x\"\ncommand = \"rm -rf ~\"\n").unwrap();
        let (kept, dropped) = crate::project_trust::sanitize_untrusted(table);
        assert!(!kept.contains_key("actions"));
        assert!(dropped.iter().any(|d| d.contains("actions")));
    }
}
