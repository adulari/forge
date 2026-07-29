//! Deterministic task classification policy for Model Mesh.
//!
//! Execution and route inspection consume the same weighted scorer so the
//! recorded rationale cannot diverge from the routed tier.

use forge_types::{ProjectContext, TaskTier};

use crate::context::normalized_turn;
use crate::{catalog, RoutingContext};

// --- Classification signals (weighted scoring; see `classify`). Capability over length. ---

/// Explicit user hint that forces Complex regardless of anything else (ADR-0006: user hints).
const COMPLEX_HINTS: &[&str] = &[
    "think hard",
    "think deeply",
    "ultrathink",
    "think carefully",
    "step by step",
    // Hyphenated form — a plain-string list can't normalize "step-by-step" to "step by step"
    // without extra machinery, so both spellings are listed explicitly (matches the existing
    // "in depth"/"in-depth" pair below).
    "step-by-step",
    "in depth",
    "in-depth",
    "deep dive",
    "comprehensive",
    "thorough",
    "think it through",
];
/// Explicit "this is easy" hints — a strong pull toward Trivial (-5 pts).
const TRIVIAL_HINTS: &[&str] = &[
    "quick",
    "simple",
    "one-liner",
    "one liner",
    "minor",
    "briefly",
    "small fix",
    "small change",
];
/// Reasoning / algorithmic / architectural terms — cognitive load, not length. A single one
/// carries a short prompt to Complex (+5 pts, threshold is 5).
const REASONING_TERMS: &[&str] = &[
    "architect",
    "architecture",
    "refactor",
    "design",
    "debug",
    "why",
    "explain",
    "optimi",
    "concurren",
    "lock-free",
    "lockless",
    "race condition",
    "deadlock",
    "thread-safe",
    "prove",
    "proof",
    "complexity",
    "invariant",
    "distributed",
    "analyze",
    "analyse",
    "trade-off",
    "tradeoff",
    "algorithm",
    "investigate",
    "audit",
    "diagnose",
    "evaluate",
    "vulnerabilit",
    "memory leak",
    // Planning/proposal work is inherently a reasoning task — producing a good plan REQUIRES
    // weighing approaches, not mechanically executing one. Catches the reported failure
    // ("produce a step-by-step plan...") even without any other strong signal present.
    "plan",
    "propose",
    "restructure",
];
/// Core-infrastructure vocabulary — only a complexity signal when [`ProjectContext::is_self_hosting`]
/// is true. In any OTHER project these are just ordinary words with no special stakes (a project
/// with its own unrelated "router" module shouldn't get an unearned complexity bump); but when the
/// agent is genuinely working on its own source, a task touching its own routing/classification
/// logic carries real, wide-blast-radius stakes the raw prompt text alone can't convey.
const SELF_HOSTING_INFRA_TERMS: &[&str] = &[
    "mesh",
    "router",
    "routing",
    "classifier",
    "classification",
    "task tier",
    "model selection",
    "provider adapter",
    "harness",
];
/// Medium-weight analytical signals (+3 pts each). A single term lifts to Standard; two
/// or one + another signal reach Complex.
const ANALYSIS_TERMS: &[&str] = &[
    "performance",
    "security",
    "compare",
    "review",
    "bottleneck",
    "scalab",
    "understand",
];
/// Dev-action verbs that imply real (non-trivial) work. Phrases ("add a"/"write a") avoid
/// matching trivial requests like "add a comment" (handled by TRIVIAL_PATTERNS first).
const ACTION_VERBS: &[&str] = &[
    "implement",
    "migrate",
    "integrate",
    "benchmark",
    "profile",
    "parallelize",
    "deploy",
    "improve",
    "valida",
    "wire ",
    "port ",
    "convert ",
    "add a ",
    "write a ",
    "create a ",
    "build a ",
];
/// Trivial-edit patterns — a strong pull toward Trivial (-8 pts) regardless of length.
pub(super) const TRIVIAL_PATTERNS: &[&str] = &[
    "typo",
    "rename",
    "bump version",
    "bump the version",
    "update the version",
    "change the version",
    "reformat",
    "add a comment",
    "fix import",
    "fix the import",
    "whitespace",
    "one-liner",
    "one liner",
    "delete this line",
    "remove this line",
];
/// Code-vs-prose markers (besides a fenced ```code block```). Symbol-based on purpose —
/// natural-language words like "function"/"class"/"import" appear in prose and would false-
/// positive ("write a function that…" is not code).
const CODE_TOKENS: &[&str] = &["fn ", "});", "() =>", "();", "{\n", "=> {"];
/// Error / stack-trace markers (a concrete failure usually means real debugging).
const ERROR_MARKERS: &[&str] = &[
    "panic",
    "traceback",
    "stack trace",
    "error[",
    "exception",
    "segfault",
    " at line ",
];

/// Tier classification with the human-readable signals that drove it.
pub(crate) struct Classification {
    pub(crate) tier: TaskTier,
    /// Raw weighted score. ≤0 → Trivial, ≥5 → Complex, else Standard. Exposed so callers
    /// can measure confidence: a score far from both boundaries is a high-confidence call;
    /// a near-boundary score means an LLM classifier should be consulted.
    pub(crate) score: i32,
    pub(crate) reasons: Vec<&'static str>,
}

/// Prompt-derived context for model selection (beyond the tier): whether the task is code-heavy
/// (mild coding-provider prior) and a stable per-prompt seed (so genuine ties spread across
/// equally-good providers instead of always the alphabetically-first one). `Default` = a neutral
/// context for callers that have no prompt.
#[derive(Debug, Clone, Copy, Default)]
pub struct RouteHints {
    pub code_heavy: bool,
    pub seed: u64,
    /// Whether this turn depends on an already-established task.
    pub continuation: bool,
    /// Whether the user explicitly requested adversarial or skeptical review. These turns use a
    /// tighter affinity quality band because a small measured quality edge matters more near the
    /// acceptance boundary than it does during ordinary implementation work.
    pub quality_critical: bool,
}

impl RouteHints {
    /// Documented in docs/features/mesh-routing.md.
    pub fn from_prompt(prompt: &str) -> Self {
        Self {
            code_heavy: is_code_heavy(prompt),
            seed: catalog::stable_hash(prompt),
            continuation: false,
            quality_critical: is_quality_critical(prompt),
        }
    }

    /// Derive hints from the active task when the current turn is referential (for example,
    /// "continue"). Standalone turns retain the prompt-only behavior.
    pub fn from_context(prompt: &str, context: &RoutingContext) -> Self {
        let Some(active_task) = context
            .is_dependent_turn(prompt)
            .then(|| context.active_task_material())
            .flatten()
        else {
            return Self::from_prompt(prompt);
        };
        let seeded = format!("{active_task}\nCURRENT TURN:\n{prompt}");
        Self {
            code_heavy: is_code_heavy(&active_task) || is_code_heavy(prompt),
            seed: catalog::stable_hash(&seeded),
            continuation: true,
            quality_critical: is_quality_critical(prompt),
        }
    }
}

fn is_quality_critical(prompt: &str) -> bool {
    let normalized = normalized_turn(prompt);
    [
        "adversarial",
        "skeptical",
        "scrutinize",
        "code review",
        "final verification",
        "stress test",
        "audit",
        "review the whole",
        "hidden acceptance",
        "hidden invariant",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

/// Whether `needle` occurs in `haystack` starting at a word boundary — i.e. not immediately
/// preceded by an alphanumeric character. Plain `str::contains` lets short verbs like "port "
/// match inside unrelated words that happen to end the same way (e.g. "port " inside "report ",
/// "export "), so ACTION_VERBS and other short-verb checks must use this instead.
fn contains_word_boundary(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut search_start = 0usize;
    while let Some(rel_idx) = haystack[search_start..].find(needle) {
        let abs_idx = search_start + rel_idx;
        let preceded_by_alnum = haystack[..abs_idx]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric());
        if !preceded_by_alnum {
            return true;
        }
        search_start = abs_idx + needle.len();
    }
    false
}

/// Whole-word match: `needle` must not be immediately preceded OR followed by an alphanumeric
/// character. Stricter than `contains_word_boundary` (which only checks the leading side) —
/// needed for single ambiguous words like "rename" that legitimately appear as a substring of an
/// unrelated word ("a script that renames files" describes what the script DOES, not an
/// instruction to rename something — `contains_word_boundary` alone still matches it since
/// nothing precedes "rename" inside "renames" at that position other than a non-alnum boundary).
pub(super) fn contains_whole_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut search_start = 0usize;
    while let Some(rel_idx) = haystack[search_start..].find(needle) {
        let abs_idx = search_start + rel_idx;
        let before_ok = haystack[..abs_idx]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_idx = abs_idx + needle.len();
        let after_ok = haystack[after_idx..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        search_start = abs_idx + needle.len();
    }
    false
}

/// Whether a prompt reads as a coding task (code fences, code tokens, or a dev-action verb) — the
/// signal behind the mild coding-provider prior.
/// Source-file extensions — a task that names a source file is editing code even when it carries
/// no code snippet or dev-action verb (e.g. "fix the padding in ForgeSessionActivity.swift").
const SOURCE_FILE_EXTS: &[&str] = &[
    ".rs", ".ts", ".tsx", ".js", ".jsx", ".swift", ".py", ".go", ".java", ".kt", ".rb", ".cpp",
    ".css", ".html", ".sh", ".toml", ".yaml", ".yml", ".sql", ".vue", ".php",
];

pub(super) fn is_code_heavy(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    prompt.contains("```")
        || CODE_TOKENS.iter().any(|t| lower.contains(t))
        || ACTION_VERBS
            .iter()
            .any(|v| contains_word_boundary(&lower, v))
        || SOURCE_FILE_EXTS.iter().any(|e| lower.contains(e))
}

/// Score a prompt's difficulty from weighted local signals (deterministic, no I/O of its own —
/// `project` is computed once per session by the caller). Capability signals (reasoning terms,
/// code, errors) can lift a *short* prompt to Complex; trivial-edit patterns and "quick" hints
/// pull it down. Length is one capped signal, never the decider — this is the fix for the old
/// length-bucket classifier.
pub(crate) fn score_prompt(prompt: &str, project: &ProjectContext) -> Classification {
    let lower = prompt.to_lowercase();

    // An explicit "think hard" hint is a hard override — the user told us it's hard.
    // score=i32::MAX signals "certain Complex" so hybrid mode never second-guesses it.
    if COMPLEX_HINTS.iter().any(|h| lower.contains(h)) {
        return Classification {
            tier: TaskTier::Complex,
            score: i32::MAX,
            reasons: vec!["explicit 'think hard' hint"],
        };
    }

    let words = prompt.split_whitespace().count();
    let mut pts: i32 = 0;
    let mut reasons: Vec<&'static str> = Vec::new();

    // Length: a single capped nudge, not the decider.
    if words > 120 {
        pts += 3;
        reasons.push("very long prompt");
    } else if words > 40 {
        pts += 1;
        reasons.push("long prompt");
    }

    let has_code = prompt.contains("```") || CODE_TOKENS.iter().any(|t| lower.contains(t));
    if REASONING_TERMS.iter().any(|t| lower.contains(t)) {
        pts += 5;
        reasons.push("reasoning/algorithmic term");
    }
    if has_code {
        pts += 3;
        reasons.push("code present");
    }
    if ACTION_VERBS
        .iter()
        .any(|v| contains_word_boundary(&lower, v))
    {
        pts += 2;
        reasons.push("dev-action verb");
    }
    let multistep = is_multistep(&lower);
    if multistep {
        pts += 2;
        reasons.push("multi-step scope");
    }
    if contains_word_boundary(&lower, "test")
        || lower.contains("benchmark")
        || lower.contains("edge case")
    {
        pts += 1;
        reasons.push("tests/edge-cases");
    }
    if ERROR_MARKERS.iter().any(|m| lower.contains(m)) {
        pts += 1;
        reasons.push("error/stack trace");
    }
    let analysis_hits = ANALYSIS_TERMS.iter().filter(|t| lower.contains(*t)).count() as i32;
    if analysis_hits > 0 {
        pts += analysis_hits * 3;
        reasons.push("analytical signal");
    }
    if project.is_self_hosting && SELF_HOSTING_INFRA_TERMS.iter().any(|t| lower.contains(t)) {
        pts += 5;
        reasons.push("self-hosting: touches this agent's own core routing/infra");
    }

    // "Explain what HTTP 429 means" is a factual protocol-code lookup, not the deep system
    // reasoning implied by the generic "explain" signal. Keep this narrow: a 3-digit HTTP status
    // plus an explicit meaning/explanation request, with no broad prompt scope.
    if is_simple_http_status_explanation(&lower, words) {
        pts -= 8;
        reasons.push("simple HTTP status explanation");
    }

    // Trivial pulls are strong only for a genuinely single mechanical edit. A trivial phrase in
    // one item of a numbered/multi-step brief must not erase the rest of the requirements.
    if TRIVIAL_HINTS.iter().any(|h| lower.contains(h)) && !multistep {
        pts -= 5;
        reasons.push("explicit 'quick' hint");
    }
    if TRIVIAL_PATTERNS
        .iter()
        .any(|p| contains_whole_word(&lower, p))
        && !multistep
    {
        // -8, not -4: an explicit trivial-edit pattern is a strong, deliberate signal (the user
        // is describing a mechanical single-file edit) and should reliably win over ONE weak
        // REASONING_TERMS hit from a word that's ambiguous outside its own context — e.g. "add a
        // comment EXPLAINING this function" trips "explain" (+5, normally a strong Complex
        // signal) despite the task itself being exactly what TRIVIAL_PATTERNS's "add a comment"
        // describes. -4 left that case net-positive (Standard); -8 does not.
        pts -= 8;
        reasons.push("trivial-edit pattern");
    }

    // Thresholds: <=0 Trivial, >=5 Complex, else Standard.
    let tier = if pts <= 0 {
        TaskTier::Trivial
    } else if pts >= 5 {
        TaskTier::Complex
    } else {
        TaskTier::Standard
    };
    if reasons.is_empty() {
        reasons.push(match tier {
            TaskTier::Trivial => "short prompt, no strong signals",
            TaskTier::Standard => "moderate task",
            TaskTier::Complex => "complex task",
        });
    }
    Classification {
        tier,
        score: pts,
        reasons,
    }
}

fn is_simple_http_status_explanation(lower: &str, words: usize) -> bool {
    words <= 16
        && contains_whole_word(lower, "http")
        && ["explain", "mean", "means", "meaning"]
            .iter()
            .any(|term| contains_whole_word(lower, term))
        && lower
            .split(|character: char| !character.is_ascii_digit())
            .any(|token| {
                token.len() == 3
                    && token
                        .parse::<u16>()
                        .is_ok_and(|status| (100..=599).contains(&status))
            })
}

pub(super) fn is_multistep(lower: &str) -> bool {
    let numbered_requirements = lower
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed
                .find(|c: char| !c.is_ascii_digit())
                .is_some_and(|i| {
                    let rest = &trimmed[i..];
                    rest.starts_with('.') || rest.starts_with(')')
                })
        })
        .count();
    numbered_requirements >= 2
        || lower.contains(" then ")
        || lower.contains("\n- ")
        || lower.contains("\n* ")
        || (lower.contains("1)") && lower.contains("2)"))
        || lower.contains("after that")
}

fn tier_rank(tier: TaskTier) -> u8 {
    match tier {
        TaskTier::Trivial => 0,
        TaskTier::Standard => 1,
        TaskTier::Complex => 2,
    }
}

/// Return the more demanding of two task tiers.
pub fn max_tier(left: TaskTier, right: TaskTier) -> TaskTier {
    if tier_rank(left) >= tier_rank(right) {
        left
    } else {
        right
    }
}

#[cfg(test)]
mod tests;
