//! `[mesh] harness_profile`: how much advisory steering the harness adds on top of the model's
//! own loop (docs/features/harness-profile.md).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `guarded` keeps every advisory nudge; `minimal` drops them and leaves only the safety and
/// integrity guards (permission broker, loop halts, phantom-edit gate, step cap, spend guards).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HarnessProfile {
    #[default]
    Guarded,
    Minimal,
}

impl HarnessProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            HarnessProfile::Guarded => "guarded",
            HarnessProfile::Minimal => "minimal",
        }
    }

    pub fn advisory_nudges(self) -> bool {
        self == HarnessProfile::Guarded
    }
}

/// The profile for `model`: the most specific matching `[mesh.harness_profiles]` glob wins (the
/// pattern with the most literal characters, so `bedrock::*kimi*` beats `bedrock::*`), else the
/// `[mesh] harness_profile` default.
pub fn resolve_harness_profile(
    default: HarnessProfile,
    overrides: &BTreeMap<String, HarnessProfile>,
    model: &str,
) -> HarnessProfile {
    overrides
        .iter()
        .filter(|(pattern, _)| glob_matches(pattern, model))
        .max_by_key(|(pattern, _)| pattern.chars().filter(|c| *c != '*').count())
        .map_or(default, |(_, profile)| *profile)
}

/// `*` matches any run of characters (including `::`); everything else is literal.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if text.len() < first.len() + last.len() || !text.starts_with(first) || !text.ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for middle in &parts[1..parts.len() - 1] {
        match rest.find(middle) {
            Some(at) => rest = &rest[at + middle.len()..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, MeshConfig};

    #[test]
    fn globs_match_whole_ids() {
        assert!(glob_matches(
            "bedrock::*",
            "bedrock::global.moonshotai.kimi-k3"
        ));
        assert!(glob_matches("*kimi*", "bedrock::global.moonshotai.kimi-k3"));
        assert!(glob_matches(
            "codex-oauth::gpt-5.6-luna",
            "codex-oauth::gpt-5.6-luna"
        ));
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("a*b*c", "abc"));
        assert!(!glob_matches("a*b*c", "acb"));
        assert!(!glob_matches("bedrock::*", "openrouter::bedrock::x"));
        assert!(!glob_matches("ab*ba", "aba"));
        assert!(!glob_matches("claude-*", "claude"));
    }

    #[test]
    fn defaults_to_guarded_and_parses_per_model_overrides() {
        let cfg = Config::default();
        assert_eq!(cfg.mesh.harness_profile, HarnessProfile::Guarded);
        assert!(cfg.mesh.harness_profiles.is_empty());
        assert_eq!(
            cfg.mesh.harness_profile_for("bedrock::x"),
            HarnessProfile::Guarded
        );

        let mesh = layered(
            r#"
            [mesh]
            harness_profile = "minimal"
            [mesh.harness_profiles]
            "ollama::*" = "guarded"
            "bedrock::global.moonshotai.*" = "guarded"
            "#,
        )
        .unwrap();
        assert_eq!(
            mesh.harness_profile_for("bedrock::x"),
            HarnessProfile::Minimal
        );
        assert_eq!(
            mesh.harness_profile_for("ollama::qwen"),
            HarnessProfile::Guarded
        );
        assert_eq!(
            mesh.harness_profile_for("bedrock::global.moonshotai.kimi-k3"),
            HarnessProfile::Guarded
        );
        assert!(layered("[mesh]\nharness_profile = \"lean\"").is_err());
    }

    fn layered(toml: &str) -> Result<MeshConfig, Box<figment::Error>> {
        use figment::providers::{Format, Serialized, Toml};
        figment::Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::string(toml))
            .extract::<Config>()
            .map(|c| c.mesh)
            .map_err(Box::new)
    }

    #[test]
    fn the_most_specific_glob_wins() {
        let overrides = BTreeMap::from([
            ("bedrock::*".to_string(), HarnessProfile::Minimal),
            ("bedrock::*kimi*".to_string(), HarnessProfile::Guarded),
        ]);
        let pick = |m| resolve_harness_profile(HarnessProfile::Guarded, &overrides, m);
        assert_eq!(pick("bedrock::claude-opus"), HarnessProfile::Minimal);
        assert_eq!(pick("bedrock::moonshotai.kimi-k3"), HarnessProfile::Guarded);
        assert_eq!(pick("groq::llama"), HarnessProfile::Guarded);
    }
}
