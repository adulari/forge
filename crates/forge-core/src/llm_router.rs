//! Cheap-LLM task classifier (ADR-0006). Asks a small model to label the tier before routing,
//! then reuses the deterministic router only for pin/budget/cost-aware model selection. The
//! heuristic tier classifier is an availability fallback: it runs only after every bounded LLM
//! attempt errors, times out, or returns an unparseable reply.

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
'Reply exactly: ok. Do not use tools.' is TRIVIAL. \
'Implement a rate-limiter with token-bucket' is STANDARD (clear, self-contained). \
Classify by what thinking the task demands, not its surface length.";

/// A [`Router`] that labels every unhinted tier with a cheap model call, falling back to
/// `fallback` only when no bounded LLM attempt produces a parseable tier.
pub struct LlmRouter {
    provider: Arc<dyn Provider>,
    candidates: Vec<String>,
    fallback: HeuristicRouter,
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
            cache: Mutex::new(VecDeque::with_capacity(CACHE_CAPACITY)),
        }
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

fn guard_tier(llm: TaskTier, code_heavy: bool) -> TaskTier {
    let tier = llm;
    // Hard floor: a code-editing task must NEVER route to the Trivial tier. Trivial-tier models
    // (the cheapest free models) cannot reliably write correct code, however "mechanical" the
    // classifier judged the task — and the LLM classifier itself runs on those same weak models,
    // so it frequently under-labels real code work as trivial. This guardrail is independent of
    // the classifier's verdict: if the turn touches code, the floor is Standard.
    if code_heavy && tier == TaskTier::Trivial {
        TaskTier::Standard
    } else {
        tier
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
        let key = prompt_hash(prompt);
        if let Some(tier) = self.cached(key) {
            let tier = guard_tier(tier, hints.code_heavy);
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
                let tier = guard_tier(tier, hints.code_heavy);
                self.fallback.decide(
                    tier,
                    format!("classified by {model} as {}", tier.as_str()),
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
    async fn llm_label_is_not_overridden_by_a_confident_heuristic() {
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
        assert_eq!(
            d.tier,
            TaskTier::Trivial,
            "a parseable LLM decision must not be silently replaced by the heuristic"
        );
        assert!(d.rationale.contains("classified by"), "{}", d.rationale);
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
    async fn code_editing_task_never_routes_trivial() {
        // The LLM classifier itself runs on weak trivial-tier models and frequently under-labels
        // real code work as "trivial" — which would then route the edit to a model too weak to
        // write correct code. A code-editing turn must floor at Standard regardless of the label.
        let d = llm_router(Ok("trivial"))
            .route(
                "fix the padding in ForgeSessionActivity.swift so the content stops clipping",
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
            "a code-editing task labeled trivial must floor to Standard: {}",
            d.rationale
        );
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
}
