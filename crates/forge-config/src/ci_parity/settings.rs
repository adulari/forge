//! `.forge/ci.toml` (or a `[ci]` table in `.forge/config.toml`): gate mode, timeouts, and
//! include / exclude overrides for the derived plan.
//!
//! ```toml
//! [ci]
//! gate = "auto"                  # "off" disables the finish gate and the pre-push check
//! step_timeout_minutes = 30
//! total_budget_minutes = 120
//! exclude = ["ci.yml::test*"]    # drop steps; `*` globs over `workflow::job::name`
//! include = ["ci.yml::*::Enforce docs freshness"]   # pull a skipped step back in
//! ```

use std::path::Path;

use super::plan::{CiPlan, CiSkip};

pub const DEFAULT_STEP_TIMEOUT_SECS: u64 = 30 * 60;
pub const DEFAULT_TOTAL_BUDGET_SECS: u64 = 120 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateMode {
    Auto,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiSettings {
    pub gate: GateMode,
    pub step_timeout_secs: u64,
    pub total_budget_secs: u64,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// A `.forge/ci.toml` that could not be read; defaults applied meanwhile.
    pub warning: Option<String>,
}

impl Default for CiSettings {
    fn default() -> Self {
        Self {
            gate: GateMode::Auto,
            step_timeout_secs: DEFAULT_STEP_TIMEOUT_SECS,
            total_budget_secs: DEFAULT_TOTAL_BUDGET_SECS,
            include: Vec::new(),
            exclude: Vec::new(),
            warning: None,
        }
    }
}

pub fn load_settings(root: &Path) -> CiSettings {
    for file in ["ci.toml", "config.toml"] {
        let path = root.join(".forge").join(file);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match parse_settings(&text) {
            Ok(Some(s)) => return s,
            Ok(None) => {}
            Err(e) => {
                return CiSettings {
                    warning: Some(format!("{}: {e}", path.display())),
                    ..CiSettings::default()
                }
            }
        }
    }
    CiSettings::default()
}

/// `Ok(None)` when the file has no `[ci]` table.
pub fn parse_settings(text: &str) -> Result<Option<CiSettings>, String> {
    let doc: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let Some(ci) = doc.get("ci").and_then(toml::Value::as_table) else {
        return Ok(None);
    };
    let mut s = CiSettings::default();
    match ci.get("gate").and_then(toml::Value::as_str) {
        None | Some("auto") => {}
        Some("off") => s.gate = GateMode::Off,
        Some(other) => {
            return Err(format!(
                "[ci] gate must be \"auto\" or \"off\", not \"{other}\""
            ))
        }
    }
    let minutes = |key: &str| {
        ci.get(key)
            .and_then(toml::Value::as_integer)
            .filter(|m| *m > 0)
    };
    if let Some(m) = minutes("step_timeout_minutes") {
        s.step_timeout_secs = m as u64 * 60;
    }
    if let Some(m) = minutes("total_budget_minutes") {
        s.total_budget_secs = m as u64 * 60;
    }
    let list = |key: &str| -> Vec<String> {
        ci.get(key)
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    s.include = list("include");
    s.exclude = list("exclude");
    Ok(Some(s))
}

fn selected(selectors: &[String], key: &str, name: &str) -> bool {
    selectors.iter().any(|p| glob_star(p, key) || p == name)
}

/// `*` matches anything, including `::` and spaces.
fn glob_star(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.replace("**", "*"), text);
    fn go(p: &[u8], t: &[u8]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some(b'*') => (0..=t.len()).any(|i| go(&p[1..], &t[i..])),
            Some(c) => t.first() == Some(c) && go(&p[1..], &t[1..]),
        }
    }
    go(p.as_bytes(), t.as_bytes())
}

/// Apply `include` / `exclude`: excluded steps become skips, included skips become steps.
pub fn apply_overrides(plan: &mut CiPlan, settings: &CiSettings) {
    let (kept, dropped): (Vec<_>, Vec<_>) = std::mem::take(&mut plan.steps)
        .into_iter()
        .partition(|s| !selected(&settings.exclude, &s.key, &s.name));
    plan.steps = kept;
    for s in dropped {
        plan.skips.push(CiSkip {
            workflow: s.workflow.clone(),
            job: Some(s.job.clone()),
            step: Some(s.name.clone()),
            reason: "excluded by .forge/ci.toml".to_string(),
            candidate: Some(s),
        });
    }
    let mut remaining = Vec::new();
    for skip in std::mem::take(&mut plan.skips) {
        match skip.candidate.clone() {
            Some(c)
                if skip.reason != "excluded by .forge/ci.toml"
                    && selected(&settings.include, &c.key, &c.name) =>
            {
                plan.steps.push(c)
            }
            _ => remaining.push(skip),
        }
    }
    plan.skips = remaining;
}

#[cfg(test)]
mod tests;
