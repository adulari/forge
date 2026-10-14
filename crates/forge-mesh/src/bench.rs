//! Measured model performance scores (ADR-0011) used to rank models on REAL data rather than the
//! family-name heuristic in [`capability`]. Scores come from the Artificial Analysis Data API
//! (a 0–100 composite `intelligence` index + a `coding` index, covering closed + open models);
//! the binary fetches + caches them and attaches a [`BenchmarkScores`] to the catalog. This module
//! is pure data + the id↔model-name matching; the async fetch lives in the binary.
//!
//! Matching is the hard part: Artificial Analysis names a model "Claude 4.5 Sonnet" while Forge's
//! id is `anthropic::claude-sonnet-4-5` (and the bridges are bare, `claude-cli::opus`). We reduce
//! both to a token *set* (split on separators and letter↔digit boundaries, lowercased) so word
//! order doesn't matter, try an exact set match, then fall back to best token-overlap that shares a
//! family word. Unmatched models just fall back to the heuristic — no wrong guess is forced.

use std::collections::HashMap;

use forge_types::EffortLevel;
use serde::{Deserialize, Serialize};

mod model_id;
use model_id::{is_family_word, is_num, ModelId, Shape, SERVING_WORDS, TIER_WORDS};

/// One model's measured performance — Artificial Analysis indices, each roughly 0–70 today.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BenchScore {
    /// Composite general-intelligence index (reasoning/knowledge/science/agentic/coding blend).
    pub intelligence: f64,
    /// Coding-specific index (LiveCodeBench/SciCode/terminal-style benches).
    pub coding: f64,
}

/// One rung on a model's measured effort ladder, as the source publishes it: the effort
/// parenthetical on a row name ("GPT-6 Astra (medium)"). Kept distinct from
/// [`forge_types::EffortLevel`] because the source also rates a *non-reasoning* rung that Forge
/// has no pin for, and because a rated rung is not necessarily one a given provider can be asked
/// for. Ordered weakest-first so a ladder sorts naturally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BenchEffort {
    NonReasoning,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl BenchEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            BenchEffort::NonReasoning => "non-reasoning",
            BenchEffort::Low => "low",
            BenchEffort::Medium => "medium",
            BenchEffort::High => "high",
            BenchEffort::XHigh => "xhigh",
            BenchEffort::Max => "max",
        }
    }

    /// The rung a Forge pin asks for. `WhiteHot` maps to `Max` because white-hot's extra lift is
    /// orchestration guidance in forge-core, not a provider setting — the reasoning rung it asks
    /// the provider for is the top one, the same rung the source labels "(max)".
    pub fn from_level(level: EffortLevel) -> Self {
        match level {
            EffortLevel::Low => BenchEffort::Low,
            EffortLevel::Medium => BenchEffort::Medium,
            EffortLevel::High => BenchEffort::High,
            EffortLevel::XHigh => BenchEffort::XHigh,
            EffortLevel::WhiteHot => BenchEffort::Max,
        }
    }

    /// The Forge pin that requests this rung, or `None` for the non-reasoning rung — Forge has no
    /// level meaning "turn reasoning off", so that row is scoring evidence only, never a routing
    /// choice.
    pub fn to_level(self) -> Option<EffortLevel> {
        match self {
            BenchEffort::NonReasoning => None,
            BenchEffort::Low => Some(EffortLevel::Low),
            BenchEffort::Medium => Some(EffortLevel::Medium),
            BenchEffort::High => Some(EffortLevel::High),
            BenchEffort::XHigh => Some(EffortLevel::XHigh),
            BenchEffort::Max => Some(EffortLevel::WhiteHot),
        }
    }

    /// The effort rung a source row name declares, if any.
    ///
    /// Vendors spell the rung two different ways, and reading only one of them silently drops a
    /// whole vendor's ladder:
    ///
    /// - OpenAI-style, the whole parenthetical is the rung — "GPT-6 Astra (high)".
    /// - Anthropic-style, the parenthetical is a comma-separated config in which one segment names
    ///   the rung — "Claude Opus 5 (Adaptive Reasoning, Max Effort)", "Claude Fable 5.1 (Adaptive
    ///   Reasoning, High Effort, Default Fallback)".
    ///
    /// The feed decorates names with far more than efforts, so each form is matched precisely:
    /// "(June 2026)" is a release date, "(Vision)" a modality, "(Reasoning)" carries no rung, and
    /// "Quasar 438B (max, based on GLM-5.2)" describes a derived model whose "max" belongs to the
    /// base it was built from. That last one is why a BARE effort word only counts as the entire
    /// parenthetical: as one segment among several it is not a rung anyone can request.
    ///
    /// A "Non-reasoning" segment wins over the rung beside it ("Claude Sonnet 5 (Non-reasoning,
    /// High Effort)" is a measurement of the thinking-off configuration), so the row stays scoring
    /// evidence and never becomes a routing choice.
    fn from_source_name(name: &str) -> Option<Self> {
        let mut depth = 0u32;
        let mut group = String::new();
        let mut found: Option<Self> = None;
        for c in name.chars() {
            match c {
                '(' | '[' => {
                    depth += 1;
                    group.clear();
                }
                ')' | ']' => {
                    depth = depth.saturating_sub(1);
                    if let Some(effort) = Self::parse_group(&group) {
                        if effort == BenchEffort::NonReasoning {
                            return Some(effort);
                        }
                        found = found.or(Some(effort));
                    }
                    group.clear();
                }
                _ if depth > 0 => group.push(c),
                _ => {}
            }
        }
        found
    }

    /// The rung declared by one parenthetical group's contents.
    fn parse_group(group: &str) -> Option<Self> {
        // Whole-content form: "(high)", "(Non-reasoning)".
        if let Some(effort) = Self::parse_exact(group) {
            return Some(effort);
        }
        // Segmented form: one segment is "<rung> Effort", and a "Non-reasoning" segment anywhere
        // outranks it.
        let mut rung = None;
        let mut bare = None;
        let mut only_config_beside_bare = true;
        for segment in group.split(',') {
            let segment = segment.trim();
            if Self::parse_exact(segment) == Some(BenchEffort::NonReasoning) {
                return Some(BenchEffort::NonReasoning);
            }
            if let Some(word) = segment
                .strip_suffix(" Effort")
                .or_else(|| segment.strip_suffix(" effort"))
            {
                rung = rung.or_else(|| Self::parse_exact(word));
            } else if let Some(effort) = Self::parse_exact(segment) {
                bare = bare.or(Some(effort));
            } else if !Self::is_config_segment(segment) {
                only_config_beside_bare = false;
            }
        }
        // The feed's current Anthropic spelling drops the "Effort" word: "Claude Opus 5.5 (Max,
        // Default Fallback)". A bare rung is accepted ONLY beside configuration segments (a
        // fallback note, "Adaptive Reasoning"), never beside a "Based on …" lineage note, so the
        // derived-model guard ("Quasar 438B (Max, Based on GLM-5.2)") still holds.
        rung.or(bare.filter(|_| only_config_beside_bare))
    }

    /// A parenthetical segment that describes how a rung was run rather than which model ran it.
    fn is_config_segment(segment: &str) -> bool {
        let s = segment.trim().to_lowercase();
        s.ends_with(" fallback") || s == "adaptive reasoning" || s == "reasoning"
    }

    fn parse_exact(text: &str) -> Option<Self> {
        match text
            .trim()
            .to_lowercase()
            .replace(['-', '_', ' '], "")
            .as_str()
        {
            "nonreasoning" => Some(BenchEffort::NonReasoning),
            "low" => Some(BenchEffort::Low),
            "medium" | "med" => Some(BenchEffort::Medium),
            "high" => Some(BenchEffort::High),
            "xhigh" | "extrahigh" => Some(BenchEffort::XHigh),
            "max" => Some(BenchEffort::Max),
            _ => None,
        }
    }
}

/// Measured performance for the models a data source knew about, matchable to Forge ids.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchmarkScores {
    /// Exact lookup by sorted-token canonical key (fast path).
    by_canon: HashMap<String, BenchScore>,
    /// All rows as (token set, score) for the overlap fallback.
    entries: Vec<(Vec<String>, BenchScore)>,
    /// Every rated effort rung, keyed by the same canonical key as `by_canon`. `tokens` strips the
    /// effort parenthetical before tokenising, so all of a model's rungs share one key. Only
    /// models the source rated at more than one rung get an entry worth routing on.
    #[serde(default)]
    by_effort: HashMap<String, Vec<(BenchEffort, BenchScore)>>,
}

impl BenchmarkScores {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Record one source row under `name` (the source's model name or slug, e.g. "Claude 4.5
    /// Sonnet" or "gpt-5-2"). Source names often carry an effort/variant parenthetical
    /// ("GPT-5.5 (xhigh)", "… (low)") that collapses to the same canonical key — when that
    /// happens we keep the HIGHER-intelligence row, i.e. the model's best effort, as its
    /// representative capability.
    ///
    /// The collapse is lossy, so the rung is ALSO kept on the model's effort ladder
    /// ([`Self::efforts_for`]). Which rung a model is actually run at moves its measured quality
    /// substantially, and not always upward: GPT-6 Astra's coding index peaks at `high` (77.1) and
    /// falls at `xhigh` (75.9). Routing on the collapsed row alone scores every model as though it
    /// ran at its best-rated effort, whatever effort it is really being sent.
    pub fn insert(&mut self, name: &str, intelligence: f64, coding: f64) {
        let score = BenchScore {
            intelligence,
            coding,
        };
        let toks = tokens(name);
        if toks.is_empty() {
            return;
        }
        let key = canon(&toks);
        match self.by_canon.get(&key) {
            Some(prev) if prev.intelligence >= intelligence => {}
            _ => {
                self.by_canon.insert(key.clone(), score);
            }
        }
        // entries: one row per canonical key for the overlap fallback. Effort variants of the
        // same model ("GPT-5.5 (xhigh)" / "GPT-5.5 (low)") collapse to the same key after
        // strip_parens; keeping only the best avoids a bloated O(n) scan with redundant candidates.
        match self.entries.iter_mut().find(|(t, _)| canon(t) == key) {
            Some((_, s)) if s.intelligence < intelligence => *s = score,
            Some(_) => {}
            None => self.entries.push((toks, score)),
        }
        // Keep the rung itself. A feed may rate the same rung twice (a re-run, or two spellings
        // that canonicalise together); the higher-intelligence measurement wins, matching how the
        // collapsed row above is chosen.
        if let Some(effort) = BenchEffort::from_source_name(name) {
            let rungs = self.by_effort.entry(key).or_default();
            match rungs.iter_mut().find(|(e, _)| *e == effort) {
                Some((_, s)) if s.intelligence < intelligence => *s = score,
                Some(_) => {}
                None => {
                    rungs.push((effort, score));
                    rungs.sort_by_key(|(e, _)| *e);
                }
            }
        }
    }

    /// The score for a Forge id using ONLY an exact token-set match — no fuzzy family fallback.
    /// For precisely-named ids (e.g. local Ollama tags `ollama::qwen2.5-coder:14b`) this avoids the
    /// fallback cross-matching different sizes/families that merely share a word like "coder".
    pub fn exact_score_for(&self, id: &str) -> Option<BenchScore> {
        let want = ModelId::parse(id);
        if want.tokens.is_empty() {
            return None;
        }
        let key = canon(&want.tokens);
        self.rung_or_row(&key, want.rung)
    }

    /// The effective score for a Forge `provider::model` id, including explicitly approved
    /// predecessor inheritance for a newly released model that has no source row yet.
    ///
    /// A directly published source score always wins. [`Self::source_score_for`] intentionally
    /// excludes inheritance so the benchmark cache can keep checking for the new model's own row.
    /// Documented in docs/features/mesh-routing.md.
    pub fn score_for(&self, id: &str) -> Option<BenchScore> {
        let want = ModelId::parse(id);
        self.resolve_key(id)
            .and_then(|key| self.rung_or_row(&key, want.rung))
    }

    /// The score matched directly from a benchmark-source row, without predecessor inheritance.
    /// Cache refresh logic uses this to distinguish measured data from a temporary routing prior.
    pub fn source_score_for(&self, id: &str) -> Option<BenchScore> {
        let want = ModelId::parse(id);
        self.resolve(&want)
            .and_then(|key| self.rung_or_row(&key, want.rung))
    }

    /// Every measured effort rung for a Forge id, weakest first.
    ///
    /// Empty when the source rated the model at a single undecorated effort — there is then no
    /// ladder to choose from and callers must fall back to [`Self::score_for`]. Resolution follows
    /// [`Self::score_for`] exactly, predecessor inheritance included, so a model routed on an
    /// inherited score reads its rungs from the same row it was scored by.
    ///
    /// Also empty for an id that NAMES its rung (`agy-cli::gemini-3.8-flash-low`,
    /// `inferhub::ag/gemini-3.8-flash-high`): that model is always run at the rung in its name, so
    /// there is no ladder to route along — [`Self::score_for`] already returns that rung's score.
    pub fn efforts_for(&self, id: &str) -> Vec<(BenchEffort, BenchScore)> {
        if ModelId::parse(id).rung.is_some() {
            return Vec::new();
        }
        self.resolve_key(id)
            .and_then(|key| self.by_effort.get(&key))
            .cloned()
            .unwrap_or_default()
    }

    /// The measured score for one exact rung, or `None` when the source did not rate it. A caller
    /// must not substitute a neighbouring rung: the whole point of the ladder is that rungs differ,
    /// and on some models they do not even increase monotonically.
    pub fn score_for_effort(&self, id: &str, effort: BenchEffort) -> Option<BenchScore> {
        self.efforts_for(id)
            .into_iter()
            .find(|(rung, _)| *rung == effort)
            .map(|(_, score)| score)
    }

    /// The row's score, or — for an id that names its rung — that rung's measured score when the
    /// source rated it. An unrated named rung falls back to the row rather than to a neighbour.
    fn rung_or_row(&self, key: &str, rung: Option<BenchEffort>) -> Option<BenchScore> {
        let named = rung.and_then(|rung| {
            self.by_effort
                .get(key)?
                .iter()
                .find(|(r, _)| *r == rung)
                .map(|(_, s)| *s)
        });
        named.or_else(|| self.by_canon.get(key).copied())
    }

    /// The canonical key of the row that represents `id` under [`Self::score_for`] semantics: the
    /// model's own row (exact or matched), else a reviewed predecessor's row.
    fn resolve_key(&self, id: &str) -> Option<String> {
        let want = ModelId::parse(id);
        self.resolve(&want).or_else(|| {
            let predecessor = Self::predecessor_canon(&want)?;
            self.by_canon
                .contains_key(&predecessor)
                .then_some(predecessor)
        })
    }

    /// Match a normalised Forge id to a source row, returning that row's canonical key. The key
    /// is what ties a model to its effort ladder, which is stored under the same key.
    ///
    /// An identical token set wins outright. Otherwise a row is a candidate only when it names the
    /// SAME model, which is decided word by word and number by number rather than by overlap:
    ///
    /// - The id's family (its first word) must be in the row, and they must share a family word
    ///   that is not a role word ("coder", "chat", …), so `deepseek-coder-v2` never takes
    ///   Qwen-Coder's row and `seedance-1.0-pro` never takes "Gemini 1.0 Pro".
    /// - Every other word of the id must be in the row, unless it is a serving tag
    ///   ([`SERVING_WORDS`]) — so `gemini-omni-flash` is not Gemini Flash and `gpt-5.4-pro` is not
    ///   GPT-5.4, while `deepseek-v4.1-flash-fast` still is DeepSeek V4.1 Flash.
    /// - A row word the id lacks is fine unless it is a tier word ([`TIER_WORDS`]): "Kimi K2.7
    ///   Code" may stand for `kimi-k2.7`, but "Qwen3.8-Flash-Next" never stands for `qwen3.8-flash`.
    /// - Version numbers must be EQUAL when the id has any. Comparing them positionally let a
    ///   shorter id inherit a point release: `kiro-opus-5` ([5]) zipped against "Opus 5.5" ([5, 5])
    ///   found no conflict and took 5.5's score. Release dates and parameter sizes are not
    ///   versions: a date only has to agree when both carry one, a size whenever the row has one.
    /// - A version-less id (`claude-cli::opus`) is an alias for the family's best row.
    fn resolve(&self, want: &ModelId) -> Option<String> {
        if want.tokens.is_empty() || self.entries.is_empty() {
            return None;
        }
        let key = canon(&want.tokens);
        if self.by_canon.contains_key(&key) {
            return Some(key);
        }
        let full = Shape::of(&want.tokens);
        // A leading vendor word (`Meta-Llama-3.3-70B`) is not part of the model's name in the
        // feed. It is dropped only on a second pass and only when the next word can itself be a
        // family, so `seedance-1.0-pro` never becomes a bare "pro".
        let tail = (full.words.len() > 1
            && want.tokens[0] == full.words[0]
            && is_family_word(&full.words[1])
            && want.tokens.get(1).is_some_and(|t| !is_num(t)))
        .then(|| Shape::of(&want.tokens[1..]));
        std::iter::once(full)
            .chain(tail)
            .find_map(|w| self.best_row(&w))
    }

    /// The best row for an id shape under the rules described on [`Self::resolve`].
    fn best_row(&self, w: &Shape) -> Option<String> {
        let family = w.words.first()?;
        // (row words the id did not name, intelligence, canonical key)
        let mut best: Option<(Vec<String>, f64, String)> = None;
        let mut ambiguous = false;
        for (toks, score) in &self.entries {
            let c = Shape::of(toks);
            if !c.words.contains(family) {
                continue;
            }
            let shares_family_word = w
                .words
                .iter()
                .any(|t| is_family_word(t) && c.words.contains(t));
            if !shares_family_word {
                continue;
            }
            if w.words
                .iter()
                .any(|t| !c.words.contains(t) && !SERVING_WORDS.contains(&t.as_str()))
            {
                continue;
            }
            if c.words
                .iter()
                .any(|t| !w.words.contains(t) && TIER_WORDS.contains(&t.as_str()))
            {
                continue;
            }
            if !c.version_fits(w) {
                continue;
            }
            if !c.sizes.is_empty() && w.sizes != c.sizes {
                continue;
            }
            if !w.dates.is_empty() && !c.dates.is_empty() && w.dates != c.dates {
                continue;
            }
            let extra: Vec<String> = c
                .words
                .into_iter()
                .filter(|t| !w.words.contains(t))
                .collect();
            let rival =
                matches!(&best, Some((be, _, _)) if be.len() == extra.len() && *be != extra);
            let better = match &best {
                None => true,
                Some((be, bi, _)) => {
                    extra.len() < be.len() || (extra.len() == be.len() && score.intelligence > *bi)
                }
            };
            if better {
                best = Some((extra, score.intelligence, canon(toks)));
                ambiguous = rival;
            } else if rival {
                ambiguous = true;
            }
        }
        // `kiro-gpt-5.6` fits "GPT-5.6 Sol", "Terra" and "Luna" equally — each adds a different
        // word the id never named. Picking the best-scored sibling would be a guess dressed up as
        // a measurement, so an id that only matches by borrowing different words matches nothing.
        match best {
            Some((extra, _, key)) if extra.is_empty() || !ambiguous => Some(key),
            _ => None,
        }
    }

    /// Resolve narrowly scoped, product-reviewed benchmark inheritance to the predecessor's exact
    /// canonical key. Consulted only when the successor has no row of its own — exact or matched
    /// — so a published row always wins, and the version rule in [`Self::resolve`] keeps a
    /// successor from matching an older sibling on its own.
    fn predecessor_canon(want: &ModelId) -> Option<String> {
        const PREDECESSORS: &[(&str, &str)] = &[
            ("qwen3.8-max-preview", "qwen3.7-max"),
            // Muse Spark 1.3 shipped 2026-09-02 and is free on OpenCode Zen. It sat in the
            // feed with no row for weeks while Artificial Analysis's site already published it;
            // inheriting 1.2's measured score was the conservative prior. Kept for feeds (and
            // caches) that predate the 1.3 row — the row wins wherever it exists.
            ("muse-spark-1.3", "muse-spark-1.2"),
            ("muse-spark-1.3-contributor", "muse-spark-1.2"),
            ("muse-spark-1.3-contributor-free", "muse-spark-1.2"),
        ];

        let key = canon(&want.tokens);
        PREDECESSORS.iter().find_map(|(successor, predecessor)| {
            (key == canon(&tokens(successor))).then(|| canon(&tokens(predecessor)))
        })
    }
}

/// How far below the best available rung a cheaper rung may measure and still be taken.
///
/// Matches the routing band used to choose between models, for the same reason: a difference this
/// small is not reliably a difference at all, and paying several times the reasoning tokens to
/// chase it is a bad trade.
pub const RUNG_QUALITY_BAND: f64 = 1.0;

/// Choose the rung to run a model at, given its measured ladder.
///
/// Returns the CHEAPEST rung whose measured quality is within [`RUNG_QUALITY_BAND`] of the best
/// rung available under `ceiling`. Where a model's ladder is flat this drops effort a long way for
/// free; where it is steep it changes nothing, because no cheaper rung is in band.
///
/// Quality is read from the metric the turn is actually judged on — the coding index for code-heavy
/// work, the general index otherwise. That distinction is not cosmetic: on GPT-6 Astra the coding
/// ladder is nearly flat and non-monotonic (medium 76.7, high 77.1, xhigh 75.9) while the
/// intelligence ladder climbs steadily (52.2 → 53.4 → 54.3). The same model therefore warrants
/// medium for coding and xhigh for reasoning, and a single blended score would get both wrong.
///
/// `None` when the source rated no rung Forge can request — the caller then sends no rung rather
/// than inventing one.
///
/// Deliberately NOT a cost-weighted optimum: what an extra rung costs is reasoning tokens burned,
/// which the benchmark ladder does not carry. Rather than model that with a number we have not
/// measured, this takes the cheapest rung that is not measurably worse.
pub fn select_rung(
    ladder: &[(BenchEffort, BenchScore)],
    ceiling: Option<EffortLevel>,
    code_heavy: bool,
) -> Option<EffortLevel> {
    let metric = |score: &BenchScore| {
        if code_heavy {
            score.coding
        } else {
            score.intelligence
        }
    };
    let cap = ceiling.map(BenchEffort::from_level);
    // The non-reasoning row is scoring evidence, never a routing choice: Forge has no pin that
    // means "turn reasoning off", so a model must not be routed onto it.
    let mut usable: Vec<(BenchEffort, f64)> = ladder
        .iter()
        .filter(|(rung, _)| rung.to_level().is_some())
        .filter(|(rung, _)| cap.is_none_or(|cap| *rung <= cap))
        .map(|(rung, score)| (*rung, metric(score)))
        .collect();
    if usable.is_empty() {
        return None;
    }
    usable.sort_by_key(|(rung, _)| *rung);
    let best = usable
        .iter()
        .map(|(_, quality)| *quality)
        .fold(f64::NEG_INFINITY, f64::max);
    usable
        .into_iter()
        .find(|(_, quality)| best - *quality <= RUNG_QUALITY_BAND)
        .and_then(|(rung, _)| rung.to_level())
}

/// Lowercased alphanumeric tokens, split on separators AND letter↔digit boundaries, so
/// "claude-opus-4-8", "Claude 4.8 Opus" and "llama3.2" all tokenise comparably. A leading
/// gateway path (`anthropic/claude-...`) is dropped to its last segment first, and any
/// parenthetical decoration ("GPT-5.5 (xhigh)", "… (Opus 4.8 Fallback)") is stripped — that
/// trailing junk would otherwise pollute the token set and cross-match unrelated models.
/// Public because the context-window derivation matches bridge aliases to canonical fetched
/// models with the same token vocabulary this module uses for benchmark rows.
/// Documented in docs/features/mesh-routing.md.
pub fn tokens(s: &str) -> Vec<String> {
    let s = strip_parens(s);
    let s = s.rsplit('/').next().unwrap_or(&s).to_lowercase();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_digit = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            let d = c.is_ascii_digit();
            if !cur.is_empty() && d != cur_digit {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
            cur_digit = d;
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    // Drop noise tokens that don't help identify a model: release qualifiers and effort/variant
    // decoration. Tier words that DO disambiguate (mini/nano/flash/max/pro/air) are kept.
    out.retain(|t| {
        !matches!(
            t.as_str(),
            "latest"
                | "preview"
                | "exp"
                | "instruct"
                | "it"
                | "reasoning"
                | "nonreasoning"
                | "non"
                | "effort"
                | "adaptive"
                | "fallback"
        )
    });
    out
}

/// Remove parenthetical segments (and any trailing dangling open paren) from a source name.
/// Documented in docs/features/mesh-routing.md.
///
/// A group that is nothing but a snapshot number is KEPT: "Qwen3.8 Max (0902)" is a different
/// model from "Qwen3.8 Max", and dropping the "(0902)" merged the two rows — the plain Max then
/// scored as the 0902 snapshot (45.4 instead of its own 40.2).
fn strip_parens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0u32;
    let mut group = String::new();
    for c in s.chars() {
        match c {
            '(' | '[' => {
                depth += 1;
                group.clear();
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                let snapshot = group.trim();
                if depth == 0 && snapshot.len() >= 3 && snapshot.chars().all(|c| c.is_ascii_digit())
                {
                    out.push(' ');
                    out.push_str(snapshot);
                }
                group.clear();
            }
            _ if depth == 0 => out.push(c),
            _ => group.push(c),
        }
    }
    out
}

/// A stable key for a token list: the words sorted and deduped (so "Claude 4.8 Opus" and
/// "claude-opus-4-8" agree, and a provider-derived family word may repeat the model's own), then
/// the version numbers IN ORDER and with repeats. Deduping the numbers too made Opus 5.5 and
/// Opus 5 one key — Opus 5 was scored as 5.5 and the two effort ladders merged — and sorting them
/// made 5.4 and 4.5 one key.
fn canon(toks: &[String]) -> String {
    let is_num = |t: &&str| t.chars().all(|c| c.is_ascii_digit());
    let mut words: Vec<&str> = toks
        .iter()
        .map(String::as_str)
        .filter(|t| !is_num(t))
        .collect();
    words.sort_unstable();
    words.dedup();
    let nums: Vec<&str> = toks.iter().map(String::as_str).filter(is_num).collect();
    format!("{}#{}", words.join("-"), nums.join("."))
}

#[cfg(test)]
mod gateway_tests;

#[cfg(test)]
mod tests {
    /// The six rows Artificial Analysis publishes for GPT-6 Astra, verbatim from the live feed
    /// cached at `~/.local/share/forge/benchmarks.json` (2026-09-05). Real numbers on purpose: the
    /// ladder's shape is the thing under test, and a made-up ladder would be monotonic.
    fn astra_feed() -> BenchmarkScores {
        let mut b = BenchmarkScores::new();
        b.insert("GPT-6 Astra (Non-reasoning)", 47.8, 76.2);
        b.insert("GPT-6 Astra (low)", 49.3, 75.7);
        b.insert("GPT-6 Astra (medium)", 52.2, 76.7);
        b.insert("GPT-6 Astra (high)", 53.4, 77.1);
        b.insert("GPT-6 Astra (xhigh)", 54.3, 75.9);
        b.insert("GPT-6 Astra (max)", 54.7, 76.9);
        b
    }

    #[test]
    fn an_effort_ladder_survives_the_collapse_to_one_representative_row() {
        let b = astra_feed();
        // The collapsed row is unchanged: still the highest-intelligence rung.
        let collapsed = b
            .score_for("codex-oauth::gpt-6-astra")
            .expect("astra is rated");
        assert_eq!(
            collapsed.intelligence, 54.7,
            "the (max) row still represents the model"
        );

        let ladder = b.efforts_for("codex-oauth::gpt-6-astra");
        assert_eq!(
            ladder.iter().map(|(e, _)| *e).collect::<Vec<_>>(),
            [
                BenchEffort::NonReasoning,
                BenchEffort::Low,
                BenchEffort::Medium,
                BenchEffort::High,
                BenchEffort::XHigh,
                BenchEffort::Max,
            ],
            "every rated rung is kept, weakest first"
        );
    }

    #[test]
    fn coding_quality_does_not_rise_monotonically_with_effort() {
        // The reason routing may not simply ask for the top rung. Astra codes BETTER at high than
        // at xhigh, and medium is within 0.4 of the best rung it has — at a fraction of the burn.
        // An implementation that assumes "more effort is more quality" passes every synthetic
        // fixture and gets this backwards on the real feed.
        let b = astra_feed();
        let coding = |rung| {
            b.score_for_effort("codex-oauth::gpt-6-astra", rung)
                .expect("rung is rated")
                .coding
        };
        assert!(
            coding(BenchEffort::High) > coding(BenchEffort::XHigh),
            "astra codes better at high ({}) than at xhigh ({})",
            coding(BenchEffort::High),
            coding(BenchEffort::XHigh)
        );
        assert!(
            coding(BenchEffort::Max) - coding(BenchEffort::Medium) < 0.5,
            "medium is within half a point of the top rung on coding"
        );
        assert!(
            coding(BenchEffort::Low) < coding(BenchEffort::Medium),
            "the ladder is not inverted either — low really is worse"
        );
    }

    #[test]
    fn a_flat_coding_ladder_routes_far_below_the_top_rung() {
        // The case that motivated all of this. Astra codes within 0.4 of its best rung at medium,
        // so a coding turn takes medium and stops paying for reasoning tokens that buy nothing.
        let ladder = astra_feed().efforts_for("codex-oauth::gpt-6-astra");
        assert_eq!(
            select_rung(&ladder, None, true),
            Some(EffortLevel::Medium),
            "coding ladder is flat above medium"
        );
    }

    #[test]
    fn a_steep_reasoning_ladder_on_the_same_model_still_routes_high() {
        // Same model, same ladder, different metric — and the answer must differ, or the selector
        // is just a constant. Astra's intelligence index climbs where its coding index does not.
        let ladder = astra_feed().efforts_for("codex-oauth::gpt-6-astra");
        assert_eq!(select_rung(&ladder, None, false), Some(EffortLevel::XHigh));
    }

    #[test]
    fn the_ceiling_caps_the_choice_and_is_never_exceeded() {
        let ladder = astra_feed().efforts_for("codex-oauth::gpt-6-astra");
        for ceiling in [
            EffortLevel::Low,
            EffortLevel::Medium,
            EffortLevel::High,
            EffortLevel::XHigh,
        ] {
            let picked = select_rung(&ladder, Some(ceiling), false).expect("a rung is rated");
            assert!(
                BenchEffort::from_level(picked) <= BenchEffort::from_level(ceiling),
                "picked {picked:?} above the ceiling {ceiling:?}"
            );
        }
        // A low ceiling leaves exactly one rung, and it is taken rather than refused.
        assert_eq!(
            select_rung(&ladder, Some(EffortLevel::Low), false),
            Some(EffortLevel::Low)
        );
    }

    #[test]
    fn a_model_is_never_routed_onto_the_non_reasoning_rung() {
        // Astra's non-reasoning row (76.2 coding) is within a point of its best coding rung, so a
        // band-only rule WOULD select it. Forge has no pin meaning "reasoning off", so a routing
        // choice it cannot express must never be made.
        let ladder = astra_feed().efforts_for("codex-oauth::gpt-6-astra");
        let picked = select_rung(&ladder, None, true).expect("a rung is rated");
        assert_ne!(picked, EffortLevel::Low, "low codes measurably worse");
        assert!(
            ladder
                .iter()
                .any(|(rung, _)| *rung == BenchEffort::NonReasoning),
            "the fixture really does carry the rung this test guards against"
        );
    }

    #[test]
    fn a_model_with_no_rated_ladder_selects_nothing() {
        // No ladder means no evidence. The caller must then send no rung rather than invent one.
        assert_eq!(select_rung(&[], None, true), None);
    }

    #[test]
    fn score_for_effort_never_substitutes_a_neighbouring_rung() {
        let mut b = BenchmarkScores::new();
        b.insert("GPT-6 Astra (high)", 53.4, 77.1);
        assert!(b
            .score_for_effort("codex-oauth::gpt-6-astra", BenchEffort::High)
            .is_some());
        assert!(
            b.score_for_effort("codex-oauth::gpt-6-astra", BenchEffort::Medium)
                .is_none(),
            "an unrated rung is unknown, not approximated by the rung next to it"
        );
    }

    #[test]
    fn a_parenthetical_that_is_not_an_effort_is_not_read_as_one() {
        // Three real decorations from the feed that must not become rungs: a release date, a
        // modality, and a derived model whose "max" belongs to the base it was built from.
        let mut b = BenchmarkScores::new();
        b.insert("GPT-5.5 Instant (June 2026)", 21.6, 39.4);
        b.insert("Llama 3.2 Instruct 90B (Vision)", 1.0, 1.0);
        b.insert("Quasar 438B (max, based on GLM-5.2)", 33.9, 61.2);
        assert!(b.efforts_for("openai::gpt-5.5-instant").is_empty());
        assert!(b.efforts_for("meta::llama-3.2-90b").is_empty());
        assert!(
            b.efforts_for("openrouter::quasar-438b").is_empty(),
            "a derived model's base-model parenthetical is not a rung Forge can request"
        );
    }

    #[test]
    fn an_anthropic_style_config_parenthetical_still_yields_its_rung() {
        // Anthropic does not spell rungs the way OpenAI does. Reading only the OpenAI form left
        // EVERY Claude model — the whole `claude-cli` provider — with no ladder at all, silently,
        // while the astra fixtures all passed.
        let mut b = BenchmarkScores::new();
        b.insert("Claude Opus 5 (Adaptive Reasoning, Max Effort)", 60.0, 80.0);
        b.insert(
            "Claude Opus 5 (Adaptive Reasoning, Xhigh Effort)",
            59.0,
            79.5,
        );
        b.insert("Claude Opus 5 (Adaptive Reasoning, Low Effort)", 50.0, 70.0);
        let ladder = b.efforts_for("claude-cli::opus");
        assert_eq!(
            ladder.iter().map(|(e, _)| *e).collect::<Vec<_>>(),
            [BenchEffort::Low, BenchEffort::XHigh, BenchEffort::Max]
        );
    }

    #[test]
    fn a_trailing_fallback_segment_does_not_hide_the_rung() {
        // Real feed names: the rung is the MIDDLE segment, not the last.
        assert_eq!(
            BenchEffort::from_source_name(
                "Claude Fable 5.1 (Adaptive Reasoning, High Effort, Default Fallback)"
            ),
            Some(BenchEffort::High)
        );
        assert_eq!(
            BenchEffort::from_source_name(
                "Claude Fable 5 (Adaptive Reasoning, Max Effort, Opus 4.8 Fallback)"
            ),
            Some(BenchEffort::Max)
        );
    }

    #[test]
    fn a_non_reasoning_segment_outranks_the_effort_beside_it() {
        // "Claude Sonnet 5 (Non-reasoning, High Effort)" measures the thinking-OFF configuration.
        // Reading it as a plain "high" rung would let mesh route onto a configuration it cannot
        // ask for, and would do so under the label of a rung it can.
        assert_eq!(
            BenchEffort::from_source_name("Claude Sonnet 5 (Non-reasoning, High Effort)"),
            Some(BenchEffort::NonReasoning)
        );
    }

    #[test]
    fn a_bare_effort_word_among_other_segments_is_still_not_a_rung() {
        // The guard that the segmented form must not weaken: "max" here belongs to the base model
        // this one was derived from, and "(Reasoning)" names no rung at all.
        assert_eq!(
            BenchEffort::from_source_name("Quasar 438B (max, based on GLM-5.2)"),
            None
        );
        assert_eq!(
            BenchEffort::from_source_name("Claude 4.5 Haiku (Reasoning)"),
            None
        );
        assert_eq!(
            BenchEffort::from_source_name("GPT-5.5 Instant (June 2026)"),
            None
        );
    }

    #[test]
    fn a_model_rated_at_a_single_undecorated_effort_has_no_ladder() {
        let mut b = BenchmarkScores::new();
        b.insert("Claude 4.5 Sonnet", 50.0, 70.0);
        assert!(b.score_for("anthropic::claude-sonnet-4-5").is_some());
        assert!(
            b.efforts_for("anthropic::claude-sonnet-4-5").is_empty(),
            "no rungs means callers fall back to the single score, not to a guess"
        );
    }

    #[test]
    fn an_inherited_score_reads_the_ladder_of_the_row_it_inherited_from() {
        // muse-spark-1.3 has no row of its own; `score_for` falls back to 1.2. Its rungs must come
        // from that same row, or the model would be scored by one row and routed by another.
        let mut b = BenchmarkScores::new();
        b.insert("Muse Spark 1.2 (xhigh)", 46.8, 72.2);
        b.insert("Muse Spark 1.2 (low)", 30.0, 55.0);
        let ladder = b.efforts_for("opencode::muse-spark-1.3-contributor-free");
        assert_eq!(
            ladder.iter().map(|(e, _)| *e).collect::<Vec<_>>(),
            [BenchEffort::Low, BenchEffort::XHigh]
        );
    }

    #[test]
    fn the_top_rung_maps_to_the_pin_that_asks_for_it() {
        // White-hot asks the provider for the top reasoning rung; its extra lift is orchestration
        // guidance, not a higher provider setting. So it must score against "(max)", not fall off
        // the ladder into the collapsed row.
        assert_eq!(
            BenchEffort::from_level(EffortLevel::WhiteHot),
            BenchEffort::Max
        );
        assert_eq!(BenchEffort::Max.to_level(), Some(EffortLevel::WhiteHot));
        assert_eq!(
            BenchEffort::NonReasoning.to_level(),
            None,
            "Forge has no pin meaning 'reasoning off' — that row is evidence, never a choice"
        );
    }

    fn muse_feed() -> BenchmarkScores {
        let mut b = BenchmarkScores::new();
        b.insert("Muse Spark 1.2 (xhigh)", 56.8, 72.2);
        b.insert("Muse Spark 1.1 (xhigh)", 53.2, 71.3);
        b
    }

    #[test]
    fn a_shared_major_digit_does_not_license_a_cross_minor_match() {
        let b = muse_feed();
        assert!(
            b.source_score_for("opencode::muse-spark-1.4").is_none(),
            "1.4 must not inherit 1.2's measured row through the fuzzy fallback"
        );
    }

    #[test]
    fn muse_1_3_falls_back_to_the_reviewed_predecessor_score() {
        let b = muse_feed();
        let s = b
            .score_for("opencode::muse-spark-1.3-contributor-free")
            .expect("1.3 inherits 1.2 until Artificial Analysis publishes its row");
        assert!((s.intelligence - 56.8).abs() < 1e-9);
        assert!((s.coding - 72.2).abs() < 1e-9);
    }

    #[test]
    fn a_shorter_candidate_number_run_still_matches() {
        let mut b = BenchmarkScores::new();
        b.insert("DeepSeek V4 Flash", 40.0, 50.0);
        assert!(
            b.source_score_for("nvidia::deepseek-ai/deepseek-v4-flash-0731")
                .is_some(),
            "a date suffix the feed does not carry must not read as a version conflict"
        );
    }

    use super::*;

    fn scores() -> BenchmarkScores {
        let mut b = BenchmarkScores::new();
        b.insert("GPT-5.2", 58.0, 55.0);
        b.insert("Claude 4.5 Sonnet", 60.0, 62.0);
        b.insert("Claude 4.5 Opus", 64.0, 66.0);
        b.insert("Llama 3.3 70B", 41.0, 38.0);
        b.insert("Gemini 3 Pro", 62.0, 57.0);
        b
    }

    #[test]
    fn exact_token_set_matches_regardless_of_word_order() {
        let b = scores();
        // Forge id has the version after the family; the source put it before — same token set.
        let s = b.score_for("anthropic::claude-sonnet-4-5").unwrap();
        assert_eq!(s.intelligence, 60.0);
        assert_eq!(s.coding, 62.0);
    }

    #[test]
    fn version_dotted_id_matches_dashed_source_slug() {
        let b = scores();
        let s = b.score_for("openai::gpt-5.2").unwrap();
        assert_eq!(s.intelligence, 58.0);
    }

    #[test]
    fn bare_bridge_alias_maps_via_injected_family_token() {
        let b = scores();
        // `claude-cli::opus` has no version — must map to a Claude-Opus row (the higher one on tie).
        let s = b.score_for("claude-cli::opus").unwrap();
        assert_eq!(s.intelligence, 64.0, "bare opus → best Claude-Opus");
    }

    #[test]
    fn kimi_code_plan_slots_map_to_their_measured_models() {
        let mut b = BenchmarkScores::new();
        b.insert("Kimi K3", 43.8, 76.2);
        b.insert("Kimi K2.7 Code", 26.3, 60.8);
        for id in ["kimi::k3", "kimi::k3-256k"] {
            assert_eq!(b.score_for(id).unwrap().coding, 76.2, "{id}");
        }
        for id in ["kimi::kimi-for-coding", "kimi::kimi-for-coding-highspeed"] {
            assert_eq!(b.score_for(id).unwrap().coding, 60.8, "{id}");
        }
    }

    #[test]
    fn unknown_model_has_no_score() {
        let b = scores();
        assert!(b.score_for("groq::some-unlisted-9000").is_none());
    }

    #[test]
    fn does_not_match_on_a_stray_number_alone() {
        let b = scores();
        // Shares "3" with Llama 3.3 / Gemini 3 but no family word → no match.
        assert!(b.score_for("foo::random-3").is_none());
    }

    #[test]
    fn parenthetical_decoration_does_not_cross_match() {
        // Real-world shape: Fable's source name carries "(… Opus 4.8 Fallback)". Without stripping
        // the parenthetical, `claude-opus-4.8` cross-matches Fable's row. With it stripped, each
        // maps to its own row.
        let mut b = BenchmarkScores::new();
        b.insert(
            "Claude Fable 5 (Adaptive Reasoning, Max Effort, Opus 4.8 Fallback)",
            59.9,
            76.5,
        );
        b.insert(
            "Claude Opus 4.8 (Adaptive Reasoning, Max Effort)",
            55.7,
            56.7,
        );
        let opus = b.score_for("anthropic::claude-opus-4.8").unwrap();
        assert_eq!(
            opus.intelligence, 55.7,
            "opus matches its own row, not Fable's"
        );
        let fable = b.score_for("anthropic::claude-fable-5").unwrap();
        assert_eq!(fable.intelligence, 59.9);
        // The versionless bridge alias maps to the same Fable row (family words claude+fable,
        // no version conflict) — so `claude-cli::fable` ranks on real scores, not the heuristic.
        let bridged = b.score_for("claude-cli::fable").unwrap();
        assert_eq!(bridged.intelligence, 59.9);
        assert_eq!(bridged.coding, 76.5);
    }

    #[test]
    fn exact_score_for_does_not_cross_match_on_a_shared_role_word() {
        // "deepseek-coder-v2:16b" shares "coder" with "Qwen2.5-Coder 14B"; the fuzzy path would
        // match, but the exact path (for precise local tags) must not.
        let mut b = BenchmarkScores::new();
        b.insert("Qwen2.5-Coder 14B", 70.0, 82.0);
        assert!(b.exact_score_for("ollama::qwen2.5-coder:14b").is_some());
        assert!(b.exact_score_for("ollama::deepseek-coder-v2:16b").is_none());
        assert!(b.exact_score_for("ollama::qwen2.5-coder:7b").is_none()); // different size
    }

    #[test]
    fn fuzzy_score_for_does_not_cross_match_on_coder_role_word() {
        // "coder" is shared by deepseek-coder and qwen-coder but is a ROLE word, not a model
        // family identifier. Before the ROLE_WORDS exclusion, score_for() would fuzzy-match
        // ollama::deepseek-coder-v2:16b → Qwen2.5-Coder 14B because they share "coder" (≥3
        // chars, alphabetic) plus "b" — satisfying the old family-word + shared≥2 guard.
        let mut b = BenchmarkScores::new();
        b.insert("Qwen2.5-Coder 14B", 70.0, 82.0);
        // No deepseek-coder entry → if the role-word fix works, fuzzy gives None.
        assert!(
            b.score_for("ollama::deepseek-coder-v2:16b").is_none(),
            "fuzzy match must not assign Qwen-Coder's score to deepseek-coder via shared 'coder'"
        );
        // The exact Qwen model still resolves correctly via the fuzzy path.
        assert!(b.score_for("openrouter::qwen/qwen2.5-coder-14b").is_some());
    }

    #[test]
    fn fuzzy_score_for_does_not_cross_match_a_newer_unrated_version_to_an_older_row() {
        // Real-world case: AA has scores for Sonnet 4.6 but not yet for a brand-new Sonnet 5 —
        // before the version-conflict guard, score_for() fuzzy-matched on the shared
        // "claude"+"sonnet" family words (ignoring that "5" vs "4"/"6" don't overlap) and
        // silently handed Sonnet 5 the OLD model's score. That's wrong on its own, AND it
        // defeats `benchmarks::ensure`'s "model has no score yet → refetch" trigger, so the
        // real Sonnet 5 row would never get picked up once AA actually publishes it.
        let mut b = BenchmarkScores::new();
        b.insert(
            "Claude Sonnet 4.6 (Adaptive Reasoning, Max Effort)",
            47.2,
            63.0,
        );
        assert!(
            b.score_for("openrouter::anthropic/claude-sonnet-5")
                .is_none(),
            "an unrated newer version must not silently inherit an older version's score"
        );
        // A genuinely versionless alias is unaffected — still maps to the best Claude-Sonnet row.
        assert!(b.score_for("claude-cli::sonnet").is_some());
        // The exact version still resolves once AA actually publishes it (fast path, no fuzzy
        // matching involved).
        b.insert("Claude Sonnet 5", 65.0, 70.0);
        let s = b
            .score_for("openrouter::anthropic/claude-sonnet-5")
            .unwrap();
        assert_eq!(s.intelligence, 65.0);
    }

    #[test]
    fn gpt_5_6_family_siblings_match_their_own_row() {
        // Sol/Terra/Luna share "gpt"+"5"+"6" but each carries its own distinguishing family word,
        // so the exact-canon fast path (not the overlap fallback) must pick the right sibling —
        // never let a request for one tier silently inherit another tier's score.
        let mut b = BenchmarkScores::new();
        b.insert("GPT-5.6 Sol (high)", 68.0, 71.0);
        b.insert("GPT-5.6 Terra (high)", 59.0, 61.0);
        b.insert("GPT-5.6 Luna (max)", 45.0, 48.0);

        let sol = b.score_for("codex-oauth::gpt-5.6-sol").unwrap();
        assert_eq!(sol.intelligence, 68.0, "sol must not resolve to terra/luna");

        let terra = b.score_for("codex-cli::gpt-5.6-terra").unwrap();
        assert_eq!(
            terra.intelligence, 59.0,
            "terra must not resolve to sol/luna"
        );

        let luna = b.score_for("gpt-5.6-luna").unwrap();
        assert_eq!(
            luna.intelligence, 45.0,
            "luna must not resolve to sol/terra"
        );
    }

    #[test]
    fn effort_variants_collapse_to_best() {
        // Same model at several effort levels → one canonical row, the highest-intelligence one.
        let mut b = BenchmarkScores::new();
        b.insert("GPT-5.5 (low)", 41.7, 52.1);
        b.insert("GPT-5.5 (xhigh)", 54.8, 74.9);
        b.insert("GPT-5.5 (medium)", 47.1, 47.1);
        let s = b.score_for("openai::gpt-5.5").unwrap();
        assert_eq!(s.intelligence, 54.8, "best effort represents the model");
    }

    #[test]
    fn qwen_3_8_max_preview_inherits_3_7_max_until_its_own_row_exists() {
        let mut b = BenchmarkScores::new();
        b.insert("Qwen3.5 Max", 90.0, 90.0);
        b.insert("Qwen3.6 Max Preview", 40.0, 40.0);
        b.insert("Qwen3.7 Max", 46.0, 66.0);

        assert!(
            b.source_score_for("qwencloud::qwen3.8-max-preview")
                .is_none(),
            "inheritance must not hide the missing source row from cache refresh logic"
        );

        for id in [
            "qwencloud::qwen3.8-max-preview",
            "openrouter::qwen/qwen3.8-max-preview",
        ] {
            let inherited = b.score_for(id).expect("Qwen 3.8 should inherit a score");
            assert_eq!(
                inherited,
                BenchScore {
                    intelligence: 46.0,
                    coding: 66.0,
                },
                "{id} must inherit Qwen3.7 Max, not the fuzzier Qwen3.6 preview row"
            );
        }

        // Once the benchmark source publishes a Qwen 3.8 row, it wins even if its measured score
        // is lower than the temporary predecessor score. Published data always beats inheritance.
        b.insert("Qwen3.8 Max (Preview)", 39.0, 38.0);
        assert!(b
            .source_score_for("qwencloud::qwen3.8-max-preview")
            .is_some());
        assert_eq!(
            b.score_for("qwencloud::qwen3.8-max-preview"),
            Some(BenchScore {
                intelligence: 39.0,
                coding: 38.0,
            })
        );
    }

    /// Real rows from the 2026-09-27 feed. The canonical key used to dedupe version numbers, so
    /// "Opus 5.5" and "Opus 5" became one key: Opus 5 was scored as 5.5 and their effort ladders
    /// merged into one.
    #[test]
    fn a_point_release_never_shares_a_key_with_its_base_version() {
        let mut b = BenchmarkScores::new();
        b.insert("Claude Opus 5 (Adaptive Reasoning, Max Effort)", 50.8, 78.0);
        b.insert("Claude Opus 5 (Adaptive Reasoning, Low Effort)", 39.4, 66.9);
        b.insert(
            "Claude Opus 5.5 (Adaptive Reasoning, Max Effort, Default Fallback)",
            57.6,
            81.6,
        );
        b.insert(
            "Claude Opus 5.5 (Adaptive Reasoning, High Effort, Default Fallback)",
            53.6,
            80.0,
        );

        assert_eq!(
            b.score_for("anthropic::claude-opus-5")
                .unwrap()
                .intelligence,
            50.8
        );
        assert_eq!(
            b.score_for("opencode::claude-opus-5-5")
                .unwrap()
                .intelligence,
            57.6
        );
        let ladder = |id| {
            b.efforts_for(id)
                .into_iter()
                .map(|(rung, _)| rung)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ladder("anthropic::claude-opus-5"),
            [BenchEffort::Low, BenchEffort::Max]
        );
        assert_eq!(
            ladder("opencode::claude-opus-5-5"),
            [BenchEffort::High, BenchEffort::Max]
        );
        assert_eq!(
            b.score_for("claude-cli::opus").unwrap().intelligence,
            57.6,
            "the bare alias still follows the best Claude-Opus row"
        );
    }

    #[test]
    fn swapped_version_digits_are_different_models() {
        let mut b = BenchmarkScores::new();
        b.insert("GPT-5.4 (xhigh)", 40.0, 70.0);
        b.insert("GPT-4.5", 20.0, 30.0);
        assert_eq!(b.score_for("openai::gpt-5.4").unwrap().intelligence, 40.0);
        assert_eq!(b.score_for("openai::gpt-4.5").unwrap().intelligence, 20.0);
    }

    /// Verbatim Artificial Analysis rows for Opus 5, as actually returned by
    /// `/api/v2/data/llms/models` on 2026-07-25 — AA publishes one row PER EFFORT LEVEL, and all
    /// six collapse to the same canonical key. The effort spread is wide (50.6 → 60.7 intelligence),
    /// so picking the wrong variant would silently under-rank the flagship by ~10 points and hand
    /// complex work to a weaker model. `insert` must keep the model's BEST effort as its
    /// representative capability regardless of the order the API happens to list them in.
    #[test]
    fn opus_5_collapses_real_effort_variants_to_its_best_row() {
        let mut b = BenchmarkScores::new();
        // Deliberately inserted worst-first so a "last write wins" regression would fail here.
        b.insert("Claude Opus 5 (Adaptive Reasoning, Low Effort)", 50.6, 66.9);
        b.insert(
            "Claude Opus 5 (Adaptive Reasoning, Medium Effort)",
            56.3,
            74.3,
        );
        b.insert(
            "Claude Opus 5 (Adaptive Reasoning, High Effort)",
            58.9,
            76.5,
        );
        b.insert(
            "Claude Opus 5 (Adaptive Reasoning, Xhigh Effort)",
            60.1,
            77.0,
        );
        b.insert("Claude Opus 5 (Adaptive Reasoning, Max Effort)", 60.7, 78.0);
        b.insert(
            "Claude Opus 4.8 (Adaptive Reasoning, Max Effort)",
            55.7,
            74.3,
        );
        // Fable's real row name contains the literal text "Opus 4.8 Fallback".
        b.insert(
            "Claude Fable 5 (Adaptive Reasoning, Max Effort, Opus 4.8 Fallback)",
            59.9,
            76.5,
        );

        assert_eq!(
            b.score_for("anthropic::claude-opus-5"),
            Some(BenchScore {
                intelligence: 60.7,
                coding: 78.0,
            }),
            "Opus 5 must rank on its Max-Effort row, not a lesser effort variant"
        );
        // A published row must win outright — the predecessor entry is only a pre-publication
        // bridge and must never shadow real measured data.
        assert_eq!(
            b.source_score_for("anthropic::claude-opus-5"),
            b.score_for("anthropic::claude-opus-5"),
            "a measured row must not be overridden by predecessor inheritance"
        );
        // Measured data, not the name heuristic, is what puts Opus 5 above Opus 4.8.
        let opus_48 = b.score_for("anthropic::claude-opus-4-8").unwrap();
        assert!(
            b.score_for("anthropic::claude-opus-5")
                .unwrap()
                .intelligence
                > opus_48.intelligence,
            "Opus 5 must outrank Opus 4.8 on measured intelligence"
        );
        // "Opus 4.8 Fallback" inside Fable's parenthetical must not pull Fable into an Opus match.
        assert_eq!(
            opus_48.intelligence, 55.7,
            "Fable's row must not be read as Opus 4.8"
        );
    }
}
