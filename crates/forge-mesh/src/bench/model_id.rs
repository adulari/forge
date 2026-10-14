//! How a Forge id is reduced to the model it names, and the vocabulary the matcher in
//! [`super::BenchmarkScores`] uses to decide whether a feed row names the same model.

use super::{tokens, BenchEffort};

/// Role words describe a task capability that many unrelated families share, so they never count
/// as the shared family word: without this, "coder" let deepseek-coder-v2 take Qwen2.5-Coder's row.
pub(super) const ROLE_WORDS: &[&str] = &["coder", "chat", "code", "instruct", "vision", "embed"];

/// Words that name a different tier of a family — a different model with a different score. A row
/// carrying one must be asked for by it, and an id carrying one must find it in the row.
pub(super) const TIER_WORDS: &[&str] = &[
    "mini",
    "nano",
    "lite",
    "next",
    "omni",
    "pro",
    "max",
    "plus",
    "flash",
    "flashx",
    "turbo",
    "air",
    "ultra",
    "super",
    "small",
    "medium",
    "large",
    "spark",
    "codex",
    "image",
    "full",
    "lightning",
    "glimmer",
    "wonder",
];

/// Serving tags: the same weights served differently (a faster or backup pool, an explicit
/// thinking toggle, a free or contributor access tier, a provider's speed/variant label). An id may
/// carry one that the benchmark row does not; it does not make a different model.
pub(super) const SERVING_WORDS: &[&str] = &[
    "fast",
    "highspeed",
    "backup",
    "thinking",
    "build",
    "free",
    "contributor",
    "latest",
    "instant",
    "versatile",
    "customtools",
    "vision",
    "chat",
    "it",
];

/// Reseller tags a gateway prefixes onto an upstream model name with a DASH (InferHub's
/// `kiro-opus-5`). Slash-prefixed routes (`ag/…`, `cmc/meta/…`) need no list: only the last path
/// segment is ever tokenised.
const ROUTE_WORDS: &[&str] = &["kiro"];

/// Effort suffixes an id may carry to name the rung it is run at (`gemini-3.8-flash-high`). `max`
/// is deliberately absent: it is a TIER in most ids (`qwen3.8-max`), not a rung.
const RUNG_SUFFIXES: &[(&str, BenchEffort)] = &[
    ("low", BenchEffort::Low),
    ("medium", BenchEffort::Medium),
    ("high", BenchEffort::High),
    ("xhigh", BenchEffort::XHigh),
];

/// A Forge id reduced to what identifies the measured model, plus the rung its name pins.
pub(super) struct ModelId {
    pub(super) tokens: Vec<String>,
    pub(super) rung: Option<BenchEffort>,
}

impl ModelId {
    /// Normalise a Forge id: provider-derived family words (so the bare CLI bridges match), the
    /// last path segment's tokens, minus gateway decoration — a dash route tag (`kiro-`), a
    /// context-size variant (`-1m`, `-256k`), a trailing effort suffix (kept as `rung`) — and with
    /// "claude" supplied for a bare Anthropic tier name (`kiro-opus-5` → Claude Opus 5).
    pub(super) fn parse(id: &str) -> Self {
        let (provider, model) = id.split_once("::").unwrap_or(("", id));
        let mut model = model.rsplit('/').next().unwrap_or(model);
        // Bedrock spells routes with dots: `global.moonshotai.kimi-k3`. Only a dot between two
        // letters is a route separator — `kimi-k2.7` keeps its version.
        if provider == "bedrock" {
            let b = model.as_bytes();
            if let Some(i) = (1..b.len().saturating_sub(1)).rev().find(|&i| {
                b[i] == b'.' && b[i - 1].is_ascii_alphabetic() && b[i + 1].is_ascii_alphabetic()
            }) {
                model = &model[i + 1..];
            }
        }
        let mut toks = tokens(kimi_code_benchmark_name(&model.to_lowercase()));
        // A bridge alias that names only a tier (`agy-cli::flash`) gets its provider's family
        // word. One that names its own family (`agy-cli::claude-opus-5-5-high`) must not: the
        // bridge serves other vendors' models too, and a stray "gemini" would read as a different
        // model.
        let names_family = toks.iter().any(|t| {
            is_family_word(t)
                && !SERVING_WORDS.contains(&t.as_str())
                && !RUNG_SUFFIXES.iter().any(|(w, _)| w == t)
        });
        let provider_family = match provider {
            "claude-cli" | "anthropic" => Some("claude"),
            "codex-cli" => Some("gpt"),
            "agy-cli" => Some("gemini"),
            _ => None,
        };
        if let Some(family) = provider_family.filter(|_| !names_family) {
            toks.insert(0, family.to_string());
        }
        if toks.len() > 1 && ROUTE_WORDS.contains(&toks[0].as_str()) {
            toks.remove(0);
        }
        // `opus-4.7-1m` / `k3-256k`: a number followed by a k/m unit at the end of a run is a
        // context-window variant, not a version. ("minimax-m3" is a unit BEFORE a number.)
        let mut i = 0;
        while i + 1 < toks.len() {
            // Millions only as a 1-2 digit window (`1m`): `270m` is a parameter count.
            let unit = toks[i + 1] == "k" || (toks[i + 1] == "m" && toks[i].len() == 1);
            let ends_run = toks.get(i + 2).is_none_or(|t| !is_num(t));
            if is_num(&toks[i]) && unit && ends_run && i > 0 {
                toks.drain(i..i + 2);
            } else {
                i += 1;
            }
        }
        let mut rung = None;
        if toks.len() > 2 {
            if let Some((_, effort)) = RUNG_SUFFIXES
                .iter()
                .find(|(word, _)| toks.last().is_some_and(|t| t == word))
            {
                rung = Some(*effort);
                toks.pop();
            }
        }
        if !toks.iter().any(|t| t == "claude")
            && toks
                .iter()
                .any(|t| matches!(t.as_str(), "opus" | "sonnet" | "haiku" | "fable"))
        {
            toks.insert(0, "claude".to_string());
        }
        Self { tokens: toks, rung }
    }
}

/// The parts of a token list that decide whether two names are the same model.
pub(super) struct Shape {
    /// Alphabetic tokens in order (first = family), plus parameter-size units.
    pub(super) words: Vec<String>,
    /// Release version numbers, trailing zeros trimmed (`3.0` is `3`).
    pub(super) versions: Vec<String>,
    /// Parameter counts (`70b` → 70, `a3b` → 3).
    pub(super) sizes: Vec<String>,
    /// Snapshot / release dates (`0731`, `2604`).
    pub(super) dates: Vec<String>,
}

impl Shape {
    pub(super) fn of(toks: &[String]) -> Self {
        let mut s = Shape {
            words: Vec::new(),
            versions: Vec::new(),
            sizes: Vec::new(),
            dates: Vec::new(),
        };
        let at = |i: usize| toks.get(i).map(String::as_str);
        let is_size_num =
            |i: usize| at(i).is_some_and(is_num) && matches!(at(i + 1), Some("b" | "m" | "x"));
        for (i, t) in toks.iter().enumerate() {
            if !is_num(t) {
                let unit_after_num =
                    i > 0 && is_num(&toks[i - 1]) && matches!(t.as_str(), "b" | "x" | "k" | "m");
                // `a3b` (active params) and `e2b` (effective params) mark a size, not a word.
                let size_marker = matches!(t.as_str(), "a" | "e") && is_size_num(i + 1);
                if !unit_after_num && !size_marker && !s.words.contains(t) {
                    s.words.push(t.clone());
                }
                continue;
            }
            if is_size_num(i) {
                let marker = i
                    .checked_sub(1)
                    .and_then(at)
                    .filter(|m| matches!(*m, "a" | "e"))
                    .unwrap_or_default();
                // Millions keep their unit: "Gemma 3 270M" is not a 270B model.
                let unit = if at(i + 1) == Some("m") { "m" } else { "" };
                s.sizes
                    .push(format!("{marker}{}{unit}", t.trim_start_matches('0')));
            } else if at(i + 1) == Some("k") {
                // A context window ("MiniMax M1 80k") identifies a serving variant, not a model.
            } else if t.len() >= 3 {
                s.dates.push(t.clone());
            } else {
                s.versions.push(t.trim_start_matches('0').to_string());
            }
        }
        // `3.0` is `3`.
        while s.versions.len() > 1 && s.versions.last().is_some_and(|v| v.is_empty()) {
            s.versions.pop();
        }
        s
    }

    /// Whether this row's version is the one `want` asks for. A version-less id is an alias for
    /// any release; a version-less ROW is the family's first release ("Muse Spark" is 1.0).
    pub(super) fn version_fits(&self, want: &Shape) -> bool {
        if want.versions.is_empty() {
            return true;
        }
        if self.versions.is_empty() {
            // A row named by its parameter count alone ("Mistral 7B Instruct") is pinned by that
            // size; the id's trailing revision tag (`-v0.3`) does not make it another model.
            return want.versions == ["1"] || (!self.sizes.is_empty() && self.sizes == want.sizes);
        }
        want.versions == self.versions
    }
}

/// A word that can name a model family: alphabetic, at least three letters, and neither a role
/// nor a tier word.
pub(super) fn is_family_word(t: &str) -> bool {
    t.len() >= 3
        && t.chars().all(char::is_alphabetic)
        && !ROLE_WORDS.contains(&t)
        && !TIER_WORDS.contains(&t)
}

pub(super) fn is_num(t: &str) -> bool {
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
}

/// The benchmark row a Kimi Code plan-slot model name is measured by.
///
/// The endpoint names its models by plan slot, not by model: `k3` alone tokenises to a one-letter
/// family that the matcher (correctly) refuses, so without this the whole provider read as
/// unscored and routed on heuristics. The mapping follows each id's own `/models` `display_name`
/// (read 2026-09-14). `kimi-for-coding` is "K2.8 Preview", which has no published row, so it takes
/// its predecessor's K2.7 Code score as the conservative prior — the same rule
/// [`super::BenchmarkScores`]'s predecessor table applies to other unrated successors. Applied to the bare
/// model name whatever the provider, because gateways resell the same slots
/// (`inferhub::kc/kimi-for-coding`).
fn kimi_code_benchmark_name(model: &str) -> &str {
    match model {
        "k3" | "k3-256k" => "kimi-k3",
        "kimi-for-coding" | "kimi-for-coding-highspeed" => "kimi-k2.7-code",
        other => other,
    }
}
