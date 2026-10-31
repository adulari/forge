//! Saved crews: a named roster of subagent roles that a session or a prompt can summon at once.
//!
//! A crew is one TOML file, `<name>.toml`, in `<config>/forge/crews`, `~/.forge/crews` or a
//! project's `.forge/crews` (nearer layers override farther ones by name):
//!
//! ```toml
//! description = "Review a change from three angles"
//!
//! [[member]]
//! role = "security"
//! tier = "complex"
//! effort = "high"
//! no_edits = true
//! brief = "Look for injection, authz gaps and secret handling."
//! ```
//!
//! Each member becomes a subagent type named `<crew>-<role>` ([`crew_agents`]), merged into the
//! agent catalog so `spawn_agents` can address it. `@crew:<name>` in a prompt, or a crew selected
//! for the session, adds [`directive`] to the turn so the model delegates to exactly that roster.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use forge_types::{EffortLevel, PermissionMode, TaskTier};
use serde::{Deserialize, Serialize};

use super::AgentDef;

/// Members per crew and characters per brief: a roster, not a prompt library.
pub const MAX_MEMBERS: usize = 8;
pub const MAX_BRIEF_CHARS: usize = 4000;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CrewMember {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default)]
    pub no_edits: bool,
    #[serde(default)]
    pub brief: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Crew {
    /// The file stem; set on load, never read from the file.
    #[serde(skip)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "member")]
    pub members: Vec<CrewMember>,
    /// The file the crew came from.
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

/// A crew or role name as a file stem and agent-name part: lowercase `[a-z0-9_-]`, 1-32 chars.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

impl Crew {
    /// What is wrong with this crew, if anything: a bad name, no or too many members, a repeated
    /// role, an unknown tier or effort, an oversized brief.
    pub fn problem(&self) -> Option<String> {
        if !valid_name(&self.name) {
            return Some(format!(
                "crew name '{}' must be 1-32 characters of a-z, 0-9, - or _",
                self.name
            ));
        }
        if self.members.is_empty() || self.members.len() > MAX_MEMBERS {
            return Some(format!("a crew needs 1 to {MAX_MEMBERS} members"));
        }
        let mut seen: Vec<&str> = Vec::new();
        for m in &self.members {
            if !valid_name(&m.role) {
                return Some(format!(
                    "role '{}' must be 1-32 characters of a-z, 0-9, - or _",
                    m.role
                ));
            }
            if seen.contains(&m.role.as_str()) {
                return Some(format!("role '{}' appears twice", m.role));
            }
            seen.push(&m.role);
            if m.tier
                .as_deref()
                .is_some_and(|t| super::parse_tier(t).is_none())
            {
                return Some(format!(
                    "role '{}': tier must be trivial, standard or complex",
                    m.role
                ));
            }
            if m.effort
                .as_deref()
                .is_some_and(|e| super::parse_effort(e).is_none())
            {
                return Some(format!("role '{}': unknown effort level", m.role));
            }
            if m.brief.chars().count() > MAX_BRIEF_CHARS {
                return Some(format!(
                    "role '{}': brief is over {MAX_BRIEF_CHARS} characters",
                    m.role
                ));
            }
        }
        None
    }

    /// The subagent type name of one member.
    pub fn agent_name(&self, member: &CrewMember) -> String {
        format!("{}-{}", self.name, member.role)
    }
}

/// Where each layer's crews live, farthest first (so a later layer overrides an earlier one).
fn layer_dirs(project_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = crate::home_dir() {
        dirs.push(home.join(".forge/crews"));
    }
    if let Some(config) = crate::config_dir() {
        dirs.push(config.join("crews"));
    }
    for project in crate::project_resource_dirs(project_root).iter().rev() {
        dirs.push(project.join(".forge/crews"));
    }
    dirs
}

/// The directory a saved crew is written to: the user's config directory.
pub fn user_crews_dir() -> Option<PathBuf> {
    crate::config_dir().map(|c| c.join("crews"))
}

/// Parse one crew file. A malformed or invalid file is skipped, never an error.
pub fn parse_crew(name: &str, text: &str) -> Option<Crew> {
    let mut crew: Crew = toml::from_str(text).ok()?;
    crew.name = name.to_string();
    crew.problem().is_none().then_some(crew)
}

fn load_dir(dir: &Path) -> Vec<Crew> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let stem = path.file_stem()?.to_str()?.to_string();
            let mut crew = parse_crew(&stem, &std::fs::read_to_string(&path).ok()?)?;
            crew.path = Some(path);
            Some(crew)
        })
        .collect()
}

/// Every crew visible from `project_root`, sorted by name.
pub fn load_crews(project_root: &Path) -> Vec<Crew> {
    let mut by_name: HashMap<String, Crew> = HashMap::new();
    for dir in layer_dirs(project_root) {
        for crew in load_dir(&dir) {
            by_name.insert(crew.name.clone(), crew);
        }
    }
    let mut crews: Vec<Crew> = by_name.into_values().collect();
    crews.sort_by(|a, b| a.name.cmp(&b.name));
    crews
}

/// Write `crew` to `dir/<name>.toml`.
pub fn save_crew_in(dir: &Path, crew: &Crew) -> Result<PathBuf, String> {
    if let Some(problem) = crew.problem() {
        return Err(problem);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.toml", crew.name));
    let text = toml::to_string_pretty(crew).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("could not save {}: {e}", path.display()))?;
    Ok(path)
}

/// Remove the user-scope crew `name`. `Ok(false)` when there was none.
pub fn delete_crew_in(dir: &Path, name: &str) -> Result<bool, String> {
    if !valid_name(name) {
        return Err(format!("'{name}' is not a crew name"));
    }
    match std::fs::remove_file(dir.join(format!("{name}.toml"))) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

/// One subagent type per crew member, named `<crew>-<role>`.
pub fn crew_agents(crews: &[Crew]) -> HashMap<String, AgentDef> {
    let mut out = HashMap::new();
    for crew in crews {
        for member in &crew.members {
            let name = crew.agent_name(member);
            out.insert(
                name.clone(),
                AgentDef {
                    description: format!("{} in the {} crew", member.role, crew.name),
                    tier: member.tier.as_deref().and_then(super::parse_tier),
                    effort: member.effort.as_deref().and_then(super::parse_effort),
                    permission_mode: member.no_edits.then_some(PermissionMode::Plan),
                    system_prompt: if member.brief.trim().is_empty() {
                        format!("You are the {} on the {} crew.", member.role, crew.name)
                    } else {
                        member.brief.trim().to_string()
                    },
                    name,
                    path: crew.path.clone(),
                    ..AgentDef::default()
                },
            );
        }
    }
    out
}

/// Crew names mentioned in `prompt` as `@crew:<name>`, in order, once each, limited to `crews`.
pub fn mentioned<'a>(prompt: &str, crews: &'a [Crew]) -> Vec<&'a Crew> {
    let mut found: Vec<&Crew> = Vec::new();
    for token in prompt.split_whitespace() {
        let Some(name) = token.strip_prefix("@crew:") else {
            continue;
        };
        let name =
            name.trim_end_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        if let Some(crew) = crews.iter().find(|c| c.name == name) {
            if !found.iter().any(|f| f.name == crew.name) {
                found.push(crew);
            }
        }
    }
    found
}

/// The instruction that makes the model delegate to `crew`.
pub fn directive(crew: &Crew) -> String {
    let mut text = format!(
        "[crew: {}] The user summoned the saved crew `{}`{}. Delegate this task with ONE \
         spawn_agents call, one entry per member below, each `agent` set to the member's type. \
         Give every member a self-contained `task` drawn from the user's request and that \
         member's focus; then combine their results.\n",
        crew.name,
        crew.name,
        if crew.description.is_empty() {
            String::new()
        } else {
            format!(" ({})", crew.description)
        }
    );
    for m in &crew.members {
        let mut traits: Vec<String> = Vec::new();
        if let Some(t) = &m.tier {
            traits.push(format!("tier {t}"));
        }
        if let Some(e) = &m.effort {
            traits.push(format!("effort {e}"));
        }
        if m.no_edits {
            traits.push("read-only".to_string());
        }
        text.push_str(&format!(
            "- agent `{}`: {}{}\n",
            crew.agent_name(m),
            m.role,
            if traits.is_empty() {
                String::new()
            } else {
                format!(" ({})", traits.join(", "))
            }
        ));
    }
    text
}

/// Effort of a member as the typed enum, for surfaces that show it.
pub fn member_effort(member: &CrewMember) -> Option<EffortLevel> {
    member.effort.as_deref().and_then(super::parse_effort)
}

/// Tier of a member as the typed enum.
pub fn member_tier(member: &CrewMember) -> Option<TaskTier> {
    member.tier.as_deref().and_then(super::parse_tier)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REVIEW: &str = r#"
description = "Review a change"

[[member]]
role = "security"
tier = "complex"
effort = "high"
no_edits = true
brief = "Audit authz."

[[member]]
role = "tests"
"#;

    #[test]
    fn a_crew_file_becomes_named_subagent_types() {
        let crew = parse_crew("review", REVIEW).unwrap();
        assert_eq!(crew.members.len(), 2);
        let agents = crew_agents(&[crew]);
        let sec = &agents["review-security"];
        assert_eq!(sec.tier, Some(TaskTier::Complex));
        assert_eq!(sec.permission_mode, Some(PermissionMode::Plan));
        assert_eq!(sec.system_prompt, "Audit authz.");
        assert_eq!(agents["review-tests"].permission_mode, None);
        assert!(agents["review-tests"].system_prompt.contains("tests"));
    }

    #[test]
    fn invalid_crews_are_rejected_with_a_reason() {
        let mut crew = parse_crew("review", REVIEW).unwrap();
        crew.members[1].role = "security".into();
        assert!(crew.problem().unwrap().contains("twice"));
        crew.members[1].role = "tests".into();
        crew.members[1].tier = Some("huge".into());
        assert!(crew.problem().unwrap().contains("tier"));
        assert!(parse_crew("Bad Name", REVIEW).is_none());
        assert!(parse_crew("empty", "description = 'x'").is_none());
    }

    #[test]
    fn at_crew_mentions_resolve_once_and_ignore_unknown_names() {
        let crews = vec![parse_crew("review", REVIEW).unwrap()];
        let hit = mentioned(
            "please @crew:review, then @crew:review and @crew:nope",
            &crews,
        );
        assert_eq!(hit.len(), 1);
        assert!(directive(hit[0]).contains("`review-security`"));
        assert!(mentioned("email me@crew.com", &crews).is_empty());
    }

    #[test]
    fn save_then_load_round_trips_and_delete_removes() {
        let dir = std::env::temp_dir().join(format!("forge-crews-{}", std::process::id()));
        let crew = parse_crew("review", REVIEW).unwrap();
        let path = save_crew_in(&dir, &crew).unwrap();
        let loaded = load_dir(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].members, crew.members);
        assert_eq!(loaded[0].path.as_deref(), Some(path.as_path()));
        assert_eq!(delete_crew_in(&dir, "review"), Ok(true));
        assert_eq!(delete_crew_in(&dir, "review"), Ok(false));
        assert!(delete_crew_in(&dir, "../x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
