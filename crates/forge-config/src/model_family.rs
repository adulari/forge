//! Per-model-family harness profiles (docs/features/harness-profile.md, "Family profiles").
//!
//! One table, [`profile_for`], maps a model family to tuned defaults. The whole layer sits behind
//! `[harness] family_profiles` (default off) so a benchmark round can A/B it. An explicit
//! `[harness]` flag always wins over a family default, whichever way it is set.

/// Model family, detected from the model id's bare name (the part after `namespace::`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFamily {
    Deepseek,
    Kimi,
    Qwen,
    Glm,
    Claude,
    Gpt,
    Gemini,
    Generic,
}

impl ModelFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelFamily::Deepseek => "deepseek",
            ModelFamily::Kimi => "kimi",
            ModelFamily::Qwen => "qwen",
            ModelFamily::Glm => "glm",
            ModelFamily::Claude => "claude",
            ModelFamily::Gpt => "gpt",
            ModelFamily::Gemini => "gemini",
            ModelFamily::Generic => "generic",
        }
    }
}

/// Which edit tool shape a family is steered to. Forge ships only `StrReplace` (exact
/// `old_string`/`new_string`, the same shape as oh-my-pi's "replace" mode) today; `Hashline` and
/// `WholeFile` are reserved for the per-family edit A/B (frontier-harness Phase 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    StrReplace,
}

/// Tool-description verbosity. Only `Standard` exists today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDocs {
    Standard,
}

/// How a family's reasoning is carried across tool-call turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningHandling {
    /// Provider-native handling; nothing extra.
    Default,
    /// Thinking-only API: the assistant's `reasoning_content` goes back on every assistant message
    /// (an empty string when the reply streamed none), including behind custom OpenAI-compatible
    /// gateways that forward the upstream's check.
    EchoOnToolTurns,
}

/// A `[harness]` flag a family turns on. `None` leaves the flag at the user's value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FamilyFlags {
    pub completion_contract: Option<bool>,
    pub stop_retries: Option<bool>,
    pub thinking_loop_breaker: Option<bool>,
    pub repeat_interrupt: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FamilyProfile {
    pub edit_mode: EditMode,
    pub tool_docs: ToolDocs,
    pub flags: FamilyFlags,
    /// Appended to the end of the base system prompt (static text, cache-stable per model).
    pub prompt_delta: Option<&'static str>,
    pub reasoning: ReasoningHandling,
}

const NONE: FamilyFlags = FamilyFlags {
    completion_contract: None,
    stop_retries: None,
    thinking_loop_breaker: None,
    repeat_interrupt: None,
};

const BASE: FamilyProfile = FamilyProfile {
    edit_mode: EditMode::StrReplace,
    tool_docs: ToolDocs::Standard,
    flags: NONE,
    prompt_delta: None,
    reasoning: ReasoningHandling::Default,
};

/// Kimi announces a tool and then ends the turn (oh-my-pi ships a Kimi-only dialect prompt for
/// exactly this: "NEVER announce a tool then stop").
pub const KIMI_PROMPT_DELTA: &str = "\n\n<critical>\n\
- Never announce a tool call and then end the turn. If you say you will run something, call the \
tool in the same reply.\n\
</critical>";

/// THE TABLE. Every row starts from `BASE` (today's behaviour) and lists only evidence-backed
/// differences. Add a family default here and nowhere else.
pub fn profile_for(family: ModelFamily) -> FamilyProfile {
    match family {
        // Kimi: 75 blank replies in 60 days of live use were mostly Kimi, its API ends degenerate
        // generations with `finish_reason: "repeat"`, and announce-then-stop was fixed by
        // `tool_choice: required` (12/12 vs 2/6). Retrying a blank/announced reply is cheap.
        ModelFamily::Kimi => FamilyProfile {
            flags: FamilyFlags {
                stop_retries: Some(true),
                ..NONE
            },
            prompt_delta: Some(KIMI_PROMPT_DELTA),
            reasoning: ReasoningHandling::EchoOnToolTurns,
            ..BASE
        },
        // DeepSeek V4 is thinking-only and rejects a follow-up that omits `reasoning_content`.
        ModelFamily::Deepseek => FamilyProfile {
            reasoning: ReasoningHandling::EchoOnToolTurns,
            ..BASE
        },
        // No measured difference yet for the rest; an A/B round decides.
        ModelFamily::Qwen
        | ModelFamily::Glm
        | ModelFamily::Claude
        | ModelFamily::Gpt
        | ModelFamily::Gemini
        | ModelFamily::Generic => BASE,
    }
}

/// The family of a model id (`namespace::name`, or a bare name).
pub fn family_of(model: &str) -> ModelFamily {
    let name = model
        .rsplit_once("::")
        .map_or(model, |(_, n)| n)
        .to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| name.contains(n));
    if has(&["deepseek"]) {
        ModelFamily::Deepseek
    } else if has(&["kimi", "moonshot"]) {
        ModelFamily::Kimi
    } else if has(&["qwen", "qwq"]) {
        ModelFamily::Qwen
    } else if has(&["glm", "zai-"]) {
        ModelFamily::Glm
    } else if has(&["claude"]) {
        ModelFamily::Claude
    } else if has(&["gpt", "codex"]) || name.starts_with("o3") || name.starts_with("o4") {
        ModelFamily::Gpt
    } else if has(&["gemini", "gemma"]) {
        ModelFamily::Gemini
    } else {
        ModelFamily::Generic
    }
}

/// The four `[harness]` loop flags as the loop sees them for one model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HarnessFlags {
    pub completion_contract: bool,
    pub stop_retries: bool,
    pub thinking_loop_breaker: bool,
    pub repeat_interrupt: bool,
}

/// One-line description of the active profile for status surfaces (`/mesh`, model detail).
pub fn describe(model: &str, family_profiles: bool) -> String {
    let family = family_of(model);
    if !family_profiles {
        return format!("family {} (profiles off)", family.as_str());
    }
    let p = profile_for(family);
    let mut on: Vec<&str> = Vec::new();
    if p.flags.completion_contract == Some(true) {
        on.push("completion_contract");
    }
    if p.flags.stop_retries == Some(true) {
        on.push("stop_retries");
    }
    if p.flags.thinking_loop_breaker == Some(true) {
        on.push("thinking_loop_breaker");
    }
    if p.flags.repeat_interrupt == Some(true) {
        on.push("repeat_interrupt");
    }
    let flags = if on.is_empty() {
        "defaults".to_string()
    } else {
        on.join("+")
    };
    let mut extra = String::new();
    if p.prompt_delta.is_some() {
        extra.push_str(", prompt delta");
    }
    if p.reasoning == ReasoningHandling::EchoOnToolTurns {
        extra.push_str(", reasoning echo");
    }
    format!(
        "family {}: edit str_replace, flags {flags}{extra}",
        family.as_str()
    )
}

use crate::HarnessConfig;

impl HarnessConfig {
    /// The loop flags for `model`: the user's value, with the family default filling in only the
    /// flags the user left unset, and only when `family_profiles` is on.
    pub fn flags_for(&self, model: &str) -> HarnessFlags {
        let family = if self.family_profiles {
            profile_for(family_of(model)).flags
        } else {
            FamilyFlags::default()
        };
        let pick = |name: &str, user: bool, fam: Option<bool>| {
            if self.explicit_flags.iter().any(|f| f == name) {
                user
            } else {
                fam.unwrap_or(user)
            }
        };
        HarnessFlags {
            completion_contract: pick(
                "completion_contract",
                self.completion_contract,
                family.completion_contract,
            ),
            stop_retries: pick("stop_retries", self.stop_retries, family.stop_retries),
            thinking_loop_breaker: pick(
                "thinking_loop_breaker",
                self.thinking_loop_breaker,
                family.thinking_loop_breaker,
            ),
            repeat_interrupt: pick(
                "repeat_interrupt",
                self.repeat_interrupt,
                family.repeat_interrupt,
            ),
        }
    }

    /// Whether `model` is a thinking-only family whose `reasoning_content` must ride back on
    /// every assistant message, even behind a custom gateway.
    pub fn family_echoes_reasoning(&self, model: &str) -> bool {
        self.family_profiles
            && profile_for(family_of(model)).reasoning == ReasoningHandling::EchoOnToolTurns
    }

    /// The family prompt delta for `model`, when the layer is on.
    pub fn family_prompt_delta(&self, model: &str) -> Option<&'static str> {
        self.family_profiles
            .then(|| profile_for(family_of(model)).prompt_delta)
            .flatten()
    }
}

const HARNESS_FLAG_NAMES: [&str; 4] = [
    "completion_contract",
    "stop_retries",
    "thinking_loop_breaker",
    "repeat_interrupt",
];

/// The `[harness]` loop flags a user set explicitly, in either config file or the environment.
pub(crate) fn explicit_harness_flags(
    user: Option<&std::path::Path>,
    project: Option<&std::path::Path>,
) -> Vec<String> {
    let mut set = Vec::new();
    for path in [user, project].into_iter().flatten() {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(table) = toml::from_str::<toml::Table>(&text) else {
            continue;
        };
        let Some(harness) = table.get("harness").and_then(toml::Value::as_table) else {
            continue;
        };
        for name in HARNESS_FLAG_NAMES {
            if harness.contains_key(name) {
                set.push(name.to_string());
            }
        }
    }
    for name in HARNESS_FLAG_NAMES {
        if std::env::var_os(format!("FORGE_HARNESS__{}", name.to_uppercase())).is_some() {
            set.push(name.to_string());
        }
    }
    set.sort();
    set.dedup();
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_families_across_namespaces() {
        for (id, want) in [
            ("modelgateway::deepseek-v4.1-flash", ModelFamily::Deepseek),
            ("inferhub::cb/deepseek-v4.1-flash", ModelFamily::Deepseek),
            ("bedrock::global.moonshotai.kimi-k3", ModelFamily::Kimi),
            ("kimi::k3-preview-kimi", ModelFamily::Kimi),
            ("nvidia::qwen/qwen3-coder", ModelFamily::Qwen),
            ("inferhub::glm-5.3-flash", ModelFamily::Glm),
            (
                "bedrock::global.anthropic.claude-sonnet-5",
                ModelFamily::Claude,
            ),
            ("codex-oauth::gpt-5.6-luna", ModelFamily::Gpt),
            ("gemini::gemini-3-flash", ModelFamily::Gemini),
            ("ollama::llama3", ModelFamily::Generic),
        ] {
            assert_eq!(family_of(id), want, "{id}");
        }
    }

    #[test]
    fn only_evidence_backed_rows_differ_from_base() {
        for f in [
            ModelFamily::Qwen,
            ModelFamily::Glm,
            ModelFamily::Claude,
            ModelFamily::Gpt,
            ModelFamily::Gemini,
            ModelFamily::Generic,
        ] {
            assert_eq!(profile_for(f), BASE, "{f:?}");
        }
        assert_eq!(
            profile_for(ModelFamily::Kimi).flags.stop_retries,
            Some(true)
        );
        assert_eq!(
            profile_for(ModelFamily::Kimi).edit_mode,
            EditMode::StrReplace
        );
        assert_eq!(profile_for(ModelFamily::Deepseek).flags, NONE);
    }

    #[test]
    fn describe_names_the_active_profile() {
        assert!(describe("ollama::x", false).contains("profiles off"));
        let kimi = describe("bedrock::kimi-k3", true);
        assert!(
            kimi.contains("stop_retries") && kimi.contains("reasoning echo"),
            "{kimi}"
        );
    }
}

#[cfg(test)]
mod flag_tests {
    use crate::HarnessConfig;

    const KIMI: &str = "bedrock::global.moonshotai.kimi-k3";

    #[test]
    fn flags_follow_the_family_only_when_the_layer_is_on_and_the_user_is_silent() {
        let mut h = HarnessConfig::default();
        assert!(!h.flags_for(KIMI).stop_retries, "layer off");
        h.family_profiles = true;
        assert!(h.flags_for(KIMI).stop_retries);
        assert!(!h.flags_for("ollama::llama3").stop_retries);
        h.explicit_flags = vec!["stop_retries".into()];
        assert!(!h.flags_for(KIMI).stop_retries, "explicit false wins");
        assert!(h.family_echoes_reasoning("modelgateway::deepseek-v4.1-flash"));
        assert!(!h.family_echoes_reasoning("ollama::llama3"));
    }

    #[test]
    fn explicit_flags_are_read_from_the_config_file() {
        let dir = std::env::temp_dir().join(format!("fam-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[harness]\nstop_retries = false\nfamily_profiles = true\n",
        )
        .unwrap();
        let set = super::explicit_harness_flags(Some(&path), None);
        assert_eq!(set, vec!["stop_retries".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
