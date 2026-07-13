//! Optional cheap-LLM task classifier (ADR-0006). Asks a small model to
//! label the tier before routing, then reuses the heuristic router's pin/budget/cost-aware
//! selection. Any failure — error, timeout, or an unparseable reply — silently falls back to
//! the deterministic heuristic, so enabling it can never break a turn.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use forge_mesh::{BudgetState, HeuristicRouter, RouteHints, Router, RoutingDecision};
use forge_provider::Provider;
use forge_types::{EffortLevel, Message, ModelHealth, ProjectContext, SubscriptionQuota, TaskTier};

/// Hard ceiling on the classification call so a slow/hung model degrades to the heuristic.
const CLASSIFY_TIMEOUT: Duration = Duration::from_secs(15);
const CANDIDATE_TIMEOUT: Duration = Duration::from_secs(5);
const CACHE_CAPACITY: usize = 64;

/// Prompt that drives the LLM classification call.
///
/// Key design choices:
/// - Three tiers with concrete examples, not vague descriptions.
/// - Explicit "LENGTH IS NOT THE SIGNAL" rule — the single most important insight: a 6-word
///   prompt can be deeply complex; a 200-word prompt can be mechanical.
/// - Phrased as "what does this REQUIRE" not "how does it read".
/// - One word reply format, tolerant parser handles any stray prose.
const CLASSIFY_SYSTEM: &str = "You classify a software-engineering task by what it REQUIRES, \
not how many words describe it. Reply with EXACTLY ONE lowercase word: trivial, standard, \
or complex. No explanation, no punctuation, just the word.

trivial — mechanical edit, zero reasoning needed: fix a typo, rename a symbol, reformat \
or reorder code, bump a version number, delete or add a single line or comment, change a \
string literal, add whitespace.

standard — routine engineering with a clear scope: implement a self-contained function or \
endpoint, write or update tests, fix a clearly-described bug, add a small feature, \
convert/port code between similar languages, straightforward refactoring of one module.

complex — requires deep analysis, broad context, or subtle reasoning: architecture or \
system design decisions, debugging an intermittent or non-obvious bug, security audits, \
performance profiling and optimisation, algorithm design or correctness proofs, \
understanding how a non-trivial system works, reviewing an entire module or codebase area, \
evaluating trade-offs between approaches, multi-module refactoring.

CRITICAL: prompt length is irrelevant. Examples — \
'Fix the race condition in the scheduler' is COMPLEX (subtle concurrency, needs deep analysis). \
'Investigate why the cache warms slowly' is COMPLEX (open-ended investigation). \
'Audit the permission checks' is COMPLEX (security analysis). \
'Add a newline to the README' is TRIVIAL despite being in a long message. \
'Rename foo to bar in utils.rs' is TRIVIAL. \
'Implement a rate-limiter with token-bucket' is STANDARD (clear, self-contained). \
Classify by what thinking the task demands, not its surface length.";

/// A [`Router`] that labels the tier with a cheap model call, falling back to `fallback`.
///
/// Two modes:
/// - `hybrid = false` (Llm): always calls the LLM, every turn.
/// - `hybrid = true` (Hybrid): checks heuristic confidence first; only calls the LLM when
///   the heuristic score is near a tier boundary (the uncertain middle zone). Clear Trivial
///   or strongly-signalled Complex tasks skip the LLM entirely — zero added latency for them.
pub struct LlmRouter {
    provider: Arc<dyn Provider>,
    candidates: Vec<String>,
    fallback: HeuristicRouter,
    hybrid: bool,
    cache: Mutex<VecDeque<(u64, TaskTier)>>,
}

impl LlmRouter {
    pub fn new(
        provider: Arc<dyn Provider>,
        candidates: Vec<String>,
        fallback: HeuristicRouter,
    ) -> Self {
        Self {
            provider,
            candidates,
            fallback,
            hybrid: false,
            cache: Mutex::new(VecDeque::with_capacity(CACHE_CAPACITY)),
        }
    }

    /// Enable hybrid mode: skip the LLM when the heuristic is already confident.
    pub fn with_hybrid(mut self, hybrid: bool) -> Self {
        self.hybrid = hybrid;
        self
    }
}

/// Find the first tier word anywhere in the reply (tolerant of "Standard.", "I think complex",
/// leading whitespace, etc.). `None` if no tier word appears.
fn parse_tier(text: &str) -> Option<TaskTier> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .find_map(|w| match w {
            "trivial" => Some(TaskTier::Trivial),
            "standard" => Some(TaskTier::Standard),
            "complex" => Some(TaskTier::Complex),
            _ => None,
        })
}

fn prompt_hash(prompt: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    prompt.hash(&mut hasher);
    hasher.finish()
}

fn guard_tier(llm: TaskTier, heuristic: TaskTier, confident: bool) -> TaskTier {
    if confident && heuristic == TaskTier::Complex {
        TaskTier::Complex
    } else {
        llm
    }
}

impl LlmRouter {
    fn cached(&self, key: u64) -> Option<TaskTier> {
        self.cache
            .lock()
            .ok()?
            .iter()
            .find(|(cached, _)| *cached == key)
            .map(|(_, tier)| *tier)
    }

    fn store(&self, key: u64, tier: TaskTier) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.retain(|(cached, _)| *cached != key);
            if cache.len() == CACHE_CAPACITY {
                cache.pop_front();
            }
            cache.push_back((key, tier));
        }
    }
}

#[async_trait]
impl Router for LlmRouter {
    async fn route(
        &self,
        prompt: &str,
        has_images: bool,
        budget: BudgetState,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        project: &ProjectContext,
    ) -> RoutingDecision {
        let hints = RouteHints::from_prompt(prompt);

        // Hybrid fast-path: if the heuristic is already confident, skip the LLM call. This
        // keeps zero added latency for obvious Trivial tasks (typo, rename) and strongly-
        // signalled Complex ones (multiple reasoning terms). Only the uncertain middle — score
        // −3…7 — triggers the extra round-trip.
        if self.hybrid {
            let (tier, confident, reason) = HeuristicRouter::classify_confident(prompt, project);
            if confident {
                return self.fallback.decide(
                    tier,
                    format!("{reason} (hybrid: heuristic confident)"),
                    budget,
                    health,
                    hints,
                    quota,
                    effort,
                    has_images,
                );
            }
        }

        let (heuristic_tier, heuristic_confident, heuristic_reason) =
            HeuristicRouter::classify_confident(prompt, project);
        let key = prompt_hash(prompt);
        if let Some(tier) = self.cached(key) {
            let tier = guard_tier(tier, heuristic_tier, heuristic_confident);
            return self.fallback.decide(
                tier,
                format!("cached classifier result: {}", tier.as_str()),
                budget,
                health,
                hints,
                quota,
                effort,
                has_images,
            );
        }

        let messages = [
            Message::system(CLASSIFY_SYSTEM),
            Message::user(format!("TASK TO CLASSIFY:\n{prompt}")),
        ];
        let started = Instant::now();
        let mut answered = None;
        for model in self.candidates.iter().filter(|m| !health.is_benched(m)) {
            let remaining = CLASSIFY_TIMEOUT.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break;
            }
            let timeout = remaining.min(CANDIDATE_TIMEOUT);
            let mut sink = |_: forge_provider::StreamEvent| {};
            let result = tokio::time::timeout(
                timeout,
                self.provider.complete(model, &messages, &[], &mut sink),
            )
            .await;
            if let Ok(Ok(resp)) = result {
                if let Some(tier) = parse_tier(&resp.content) {
                    answered = Some((model.as_str(), tier));
                    break;
                }
            }
        }

        match answered {
            Some((model, tier)) => {
                self.store(key, tier);
                let tier = guard_tier(tier, heuristic_tier, heuristic_confident);
                let guard = if tier != heuristic_tier && heuristic_confident {
                    format!("; never-downgrade from {heuristic_reason}")
                } else {
                    String::new()
                };
                self.fallback.decide(
                    tier,
                    format!("classified by {model} as {}{guard}", tier.as_str()),
                    budget,
                    health,
                    hints,
                    quota,
                    effort,
                    has_images,
                )
            }
            None => {
                let mut d = self
                    .fallback
                    .route(prompt, has_images, budget, health, quota, effort, project)
                    .await;
                d.rationale
                    .push_str(" (llm classify unavailable → heuristic)");
                d
            }
        }
    }

    async fn route_hinted(
        &self,
        prompt: &str,
        has_images: bool,
        budget: BudgetState,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        tier_override: Option<TaskTier>,
        effort: Option<EffortLevel>,
        project: &ProjectContext,
    ) -> RoutingDecision {
        match tier_override {
            // An explicit command/skill tier hint skips the classifier model call entirely.
            Some(tier) => self.fallback.decide(
                tier,
                format!("tier hint: {}", tier.as_str()),
                budget,
                health,
                RouteHints::from_prompt(prompt),
                quota,
                effort,
                has_images,
            ),
            None => {
                self.route(prompt, has_images, budget, health, quota, effort, project)
                    .await
            }
        }
    }

    fn trivial_candidates(&self) -> Vec<String> {
        self.fallback.trivial_candidates()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use forge_provider::{EventSink, ModelResponse, ProviderError, ToolSpec};

    /// A provider that returns a fixed classification reply, or an error.
    struct FakeProvider(Result<String, ()>);

    #[async_trait]
    impl Provider for FakeProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut EventSink<'_>,
        ) -> Result<ModelResponse, ProviderError> {
            match &self.0 {
                Ok(text) => Ok(ModelResponse {
                    content: text.clone(),
                    tool_calls: Vec::new(),
                    usage: Default::default(),
                    quotas: Vec::new(),
                }),
                Err(()) => Err(ProviderError::Request("boom".into())),
            }
        }
    }

    struct SequenceProvider {
        responses: Mutex<Vec<Result<String, ()>>>,
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Provider for SequenceProvider {
        async fn complete(
            &self,
            model: &str,
            _messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut EventSink<'_>,
        ) -> Result<ModelResponse, ProviderError> {
            self.calls.lock().unwrap().push(model.to_string());
            match self.responses.lock().unwrap().remove(0) {
                Ok(content) => Ok(ModelResponse {
                    content,
                    tool_calls: Vec::new(),
                    usage: Default::default(),
                    quotas: Vec::new(),
                }),
                Err(()) => Err(ProviderError::Request("boom".into())),
            }
        }
    }

    fn llm_router(reply: Result<&str, ()>) -> LlmRouter {
        let provider = Arc::new(FakeProvider(reply.map(String::from)));
        let fallback = HeuristicRouter::new(forge_config::Config::default());
        LlmRouter::new(provider, vec!["ollama::tiny".into()], fallback)
    }

    #[tokio::test]
    async fn first_candidate_error_uses_second_candidate() {
        let provider = Arc::new(SequenceProvider {
            responses: Mutex::new(vec![Err(()), Ok("standard".into())]),
            calls: Mutex::new(Vec::new()),
        });
        let router = LlmRouter::new(
            provider.clone(),
            vec!["first::model".into(), "second::model".into()],
            HeuristicRouter::new(forge_config::Config::default()),
        );
        let d = router
            .route(
                "implement a small utility",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Standard);
        assert!(d.rationale.contains("second::model"));
        assert_eq!(
            provider.calls.lock().unwrap().as_slice(),
            ["first::model", "second::model"]
        );
    }

    #[tokio::test]
    async fn benched_first_candidate_is_skipped() {
        let provider = Arc::new(SequenceProvider {
            responses: Mutex::new(vec![Ok("complex".into())]),
            calls: Mutex::new(Vec::new()),
        });
        let router = LlmRouter::new(
            provider.clone(),
            vec!["first::model".into(), "second::model".into()],
            HeuristicRouter::new(forge_config::Config::default()),
        );
        let health = ModelHealth::new(["first::model".to_string()].into_iter().collect());
        let _ = router
            .route(
                "tweak it",
                false,
                BudgetState::default(),
                &health,
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(provider.calls.lock().unwrap().as_slice(), ["second::model"]);
    }

    #[tokio::test]
    async fn never_downgrades_certain_complex() {
        let router = llm_router(Ok("trivial"));
        let d = router
            .route(
                "analyze the performance bottleneck in the authentication service",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Complex);
    }

    #[tokio::test]
    async fn identical_prompt_uses_classifier_cache() {
        let provider = Arc::new(SequenceProvider {
            responses: Mutex::new(vec![Ok("standard".into())]),
            calls: Mutex::new(Vec::new()),
        });
        let router = LlmRouter::new(
            provider.clone(),
            vec!["cache::model".into()],
            HeuristicRouter::new(forge_config::Config::default()),
        );
        for _ in 0..2 {
            let _ = router
                .route(
                    "implement a small utility",
                    false,
                    BudgetState::default(),
                    &ModelHealth::default(),
                    &SubscriptionQuota::default(),
                    None,
                    &ProjectContext::default(),
                )
                .await;
        }
        assert_eq!(provider.calls.lock().unwrap().len(), 1);
    }
    #[test]
    fn parses_tier_words_tolerantly() {
        assert_eq!(parse_tier("complex"), Some(TaskTier::Complex));
        assert_eq!(parse_tier("Standard."), Some(TaskTier::Standard));
        assert_eq!(parse_tier("  trivial\n"), Some(TaskTier::Trivial));
        assert_eq!(
            parse_tier("I think this is standard"),
            Some(TaskTier::Standard)
        );
        assert_eq!(parse_tier("banana"), None);
    }

    #[tokio::test]
    async fn uses_the_llm_label() {
        let d = llm_router(Ok("complex"))
            .route(
                "tweak it",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Complex); // AC-B1
        assert!(d.rationale.contains("classified by"), "{}", d.rationale);
    }

    #[tokio::test]
    async fn falls_back_on_gibberish() {
        let d = llm_router(Ok("banana"))
            .route(
                "design a lock-free queue",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        // heuristic catches the hard prompt
        assert_eq!(d.tier, TaskTier::Complex);
        assert!(d.rationale.contains("heuristic"), "{}", d.rationale); // AC-B2
    }

    #[tokio::test]
    async fn falls_back_on_provider_error() {
        let d = llm_router(Err(()))
            .route(
                "fix typo",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(d.tier, TaskTier::Trivial);
        assert!(d.rationale.contains("heuristic"), "{}", d.rationale); // AC-B2
    }

    fn hybrid_router(reply: Result<&str, ()>) -> LlmRouter {
        let provider = Arc::new(FakeProvider(reply.map(String::from)));
        let fallback = HeuristicRouter::new(forge_config::Config::default());
        LlmRouter::new(provider, vec!["ollama::tiny".into()], fallback).with_hybrid(true)
    }

    #[tokio::test]
    async fn hybrid_skips_llm_for_confident_trivial() {
        // "typo" hits TRIVIAL_PATTERNS → score −4 → confident → LLM must NOT be called.
        // The FakeProvider would return "complex" if called, revealing whether the skip worked.
        let d = hybrid_router(Ok("complex"))
            .route(
                "fix the typo in the readme",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(
            d.tier,
            TaskTier::Trivial,
            "hybrid must not call LLM for confident Trivial: {}",
            d.rationale
        );
        assert!(
            d.rationale.contains("confident"),
            "rationale should mention confident fast-path: {}",
            d.rationale
        );
    }

    #[tokio::test]
    async fn hybrid_skips_llm_for_confident_complex() {
        // REASONING_TERM (+5) + two ANALYSIS_TERMS (+3 each) → score 11 ≥ 8 → confident.
        // FakeProvider returns "trivial" — if called, tier would flip; it must be skipped.
        let d = hybrid_router(Ok("trivial"))
            .route(
                "analyze the performance bottleneck in the authentication service",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(
            d.tier,
            TaskTier::Complex,
            "hybrid must not call LLM for confident Complex: {}",
            d.rationale
        );
        assert!(
            d.rationale.contains("confident"),
            "rationale should mention confident fast-path: {}",
            d.rationale
        );
    }

    #[tokio::test]
    async fn hybrid_calls_llm_for_uncertain_standard() {
        // "add a function" → score ~2 (Standard, uncertain) → LLM IS called and overrides.
        // FakeProvider returns "complex" → tier should become Complex.
        let d = hybrid_router(Ok("complex"))
            .route(
                "add a function that validates emails",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(
            d.tier,
            TaskTier::Complex,
            "hybrid must use LLM for uncertain Standard: {}",
            d.rationale
        );
        assert!(
            d.rationale.contains("classified by"),
            "rationale should show llm result: {}",
            d.rationale
        );
    }

    #[tokio::test]
    async fn hybrid_calls_llm_for_barely_complex_prompt() {
        // Single REASONING_TERM → score 5 (barely Complex, uncertain) → LLM IS called.
        // FakeProvider returns "standard" → tier becomes Standard.
        let d = hybrid_router(Ok("standard"))
            .route(
                "refactor this helper",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        assert_eq!(
            d.tier,
            TaskTier::Standard,
            "hybrid must use LLM for barely-Complex uncertain prompt: {}",
            d.rationale
        );
    }

    #[tokio::test]
    async fn hybrid_falls_back_gracefully_when_llm_fails() {
        // Uncertain prompt + provider error → heuristic tier used.
        let d = hybrid_router(Err(()))
            .route(
                "implement a small utility function",
                false,
                BudgetState::default(),
                &ModelHealth::default(),
                &SubscriptionQuota::default(),
                None,
                &ProjectContext::default(),
            )
            .await;
        // Heuristic gives Standard for this prompt.
        assert_eq!(d.tier, TaskTier::Standard);
        assert!(d.rationale.contains("heuristic"), "{}", d.rationale);
    }
}
