//! Assay — the critic crew (docs/features/analysis-mode.md). A read-only, multi-agent quality
//! analysis: specialized critics scan the scope in parallel (each mesh-routed by its lens's
//! tier), every candidate finding is checked by an independent adversarial verifier, and the
//! survivors are synthesized into a ranked [`AssayReport`]. Assay never writes — fixing is a
//! separate, opt-in agent turn.

use std::sync::Arc;

use forge_mesh::pricing::Pricing;
use forge_provider::{Provider, StreamEvent};
use forge_types::{
    new_id, AssayReport, AssayScope, Confidence, Effort, Finding, FindingCategory, Message,
    Severity, TaskTier,
};
use serde::Deserialize;

/// Candidate models per Mesh tier, best-first (resolved by the caller from the health-filtered
/// catalog). A critic tries them in order, benching any that rate-limit / go down and failing over
/// to the next — so one dead model no longer wipes out a whole tier's critics.
#[derive(Debug, Clone)]
pub struct TierModels {
    pub trivial: Vec<String>,
    pub complex: Vec<String>,
}

impl TierModels {
    fn chain_for(&self, c: FindingCategory) -> &[String] {
        match c.tier() {
            TaskTier::Trivial => &self.trivial,
            _ => &self.complex,
        }
    }
}

/// A finding as a critic emits it (category is implied by the critic's lens).
#[derive(Debug, Clone, Deserialize)]
struct Candidate {
    severity: String,
    file: String,
    #[serde(default)]
    line: Option<u32>,
    title: String,
    #[serde(default)]
    why: String,
    #[serde(default)]
    fix: String,
    #[serde(default)]
    effort: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Verdict {
    verdict: String,
    #[serde(default)]
    confidence: String,
}

/// Run the critic crew over `source` (the bundled scope content) and return a ranked report.
/// `provider`/`pricing`/`store` are shared; critics + verifiers run concurrently, each failing
/// over down its tier's model chain (benching dead models to `store`) so a rate-limited model
/// doesn't skip the whole tier.
#[allow(clippy::too_many_arguments)]
pub async fn run_assay(
    scope: AssayScope,
    source: Arc<str>,
    lenses: Vec<FindingCategory>,
    models: TierModels,
    provider: Arc<dyn Provider>,
    pricing: Arc<Pricing>,
    store: Arc<forge_store::Store>,
    cooldown: std::time::Duration,
) -> AssayReport {
    let models = Arc::new(models);
    let mut cost = 0.0;
    let mut skipped: Vec<(String, String)> = Vec::new();

    // 1. Critics — one per lens, concurrently, read-only. Each carries its lens so results stay
    //    attributable regardless of completion order.
    let mut critic_handles = Vec::new();
    for lens in lenses {
        let (provider, source, pricing, models, store) = (
            provider.clone(),
            source.clone(),
            pricing.clone(),
            models.clone(),
            store.clone(),
        );
        critic_handles.push(tokio::spawn(async move {
            let msgs = critic_messages(lens, &source);
            match complete_with_failover(
                &provider,
                &pricing,
                &store,
                models.chain_for(lens),
                cooldown,
                &msgs,
            )
            .await
            {
                Ok((text, c)) => (lens, Ok(parse_candidates(&text)), c),
                Err(e) => (lens, Err(e), 0.0),
            }
        }));
    }

    let mut candidates: Vec<(FindingCategory, Candidate)> = Vec::new();
    for h in critic_handles {
        match h.await {
            Ok((lens, Ok(cands), c)) => {
                cost += c;
                candidates.extend(cands.into_iter().map(|cand| (lens, cand)));
            }
            Ok((lens, Err(reason), _)) => skipped.push((lens.as_str().to_string(), reason)),
            Err(_) => skipped.push(("(critic)".into(), "task panicked".into())),
        }
    }

    // 2. Adversarial verification — an independent verifier per candidate, concurrently. Refuted
    //    candidates are dropped; survivors keep the verifier's confidence.
    let mut verify_handles = Vec::new();
    for (lens, cand) in candidates {
        let (provider, pricing, models, store) = (
            provider.clone(),
            pricing.clone(),
            models.clone(),
            store.clone(),
        );
        verify_handles.push(tokio::spawn(async move {
            let msgs = verifier_messages(lens, &cand);
            let (verdict, c) = match complete_with_failover(
                &provider,
                &pricing,
                &store,
                models.chain_for(lens),
                cooldown,
                &msgs,
            )
            .await
            {
                Ok((text, c)) => (parse_verdict(&text), c),
                Err(_) => (None, 0.0),
            };
            (lens, cand, verdict, c)
        }));
    }

    let mut findings = Vec::new();
    for h in verify_handles {
        let Ok((lens, cand, verdict, c)) = h.await else {
            continue;
        };
        cost += c;
        match verdict {
            // Explicit refutation drops the finding (the noise-cut mechanism).
            Some(v) if v.verdict.trim().eq_ignore_ascii_case("refute") => continue,
            // Upheld → keep at the verifier's confidence.
            Some(v) => {
                let conf = Confidence::parse(&v.confidence).unwrap_or(Confidence::Medium);
                findings.push(build_finding(lens, cand, conf));
            }
            // Unparseable verifier → keep but flag low-confidence rather than silently drop a
            // possibly-real finding.
            None => findings.push(build_finding(lens, cand, Confidence::Low)),
        }
    }

    let mut report = AssayReport {
        run_id: String::new(),
        scope,
        findings,
        cost_usd: cost,
        skipped_lenses: skipped,
    };
    report.rank();
    report
}

/// Try each model in `chain` (best-first) until one answers; returns its text + priced cost.
/// A retryable failure (rate-limit / unavailable / auth) benches that model in `store` and falls
/// over to the next — the same model-health failover the agent loop uses (model-health-failover).
/// `Err` only when the whole chain is exhausted (carries the last failure reason).
async fn complete_with_failover(
    provider: &Arc<dyn Provider>,
    pricing: &Pricing,
    store: &forge_store::Store,
    chain: &[String],
    cooldown: std::time::Duration,
    messages: &[Message],
) -> Result<(String, f64), String> {
    if chain.is_empty() {
        return Err("no usable model for this tier".to_string());
    }
    let mut sink = |_ev: StreamEvent| {};
    let mut last = String::from("no usable model");
    for model in chain {
        match provider.complete(model, messages, &[], &mut sink).await {
            Ok(r) => {
                let cost = pricing.cost_for(model, r.usage.input_tokens, r.usage.output_tokens);
                return Ok((r.content, cost));
            }
            Err(e) => {
                last = e.reason().to_string();
                // Bench retryable failures so other critics + later runs route around them.
                if e.is_retryable() {
                    let _ = store.bench_for(model, e.cooldown(cooldown), e.reason());
                }
                // Try the next candidate in the chain.
            }
        }
    }
    Err(last)
}

const CRITIC_MARKER: &str = "ASSAY-CRITIC";
const VERIFIER_MARKER: &str = "ASSAY-VERIFIER";

fn lens_brief(c: FindingCategory) -> &'static str {
    match c {
        FindingCategory::DeadWeight => "unused/unreachable/dead code, duplicated logic",
        FindingCategory::Correctness => "bugs, wrong logic, panics on real fallible paths",
        FindingCategory::Unsafe => {
            "unsafe blocks, unchecked unwrap/expect on fallible paths, races"
        }
        FindingCategory::TestCoverage => {
            "untested branches, missing tests (one baseline if no tests)"
        }
        FindingCategory::Design => "SRP violations, complexity, coupling, leaky abstractions",
        FindingCategory::Architecture => {
            "layering, module boundaries, inverted dependency direction"
        }
        FindingCategory::DocumentationRot => "docs/comments that disagree with the code",
        FindingCategory::OverEngineering => {
            "needless abstraction, AI-slop patterns, premature generality"
        }
    }
}

fn critic_messages(lens: FindingCategory, source: &str) -> Vec<Message> {
    let sys = format!(
        "You are an {CRITIC_MARKER} with the '{}' lens. Critically review the code below for: {}. \
         Be precise and skeptical — only real problems. Output ONLY a JSON array of findings, \
         each: {{\"severity\":\"critical|high|medium|low\",\"file\":\"path\",\"line\":<int|null>,\
         \"title\":\"one line\",\"why\":\"reasoning\",\"fix\":\"suggested fix\",\
         \"effort\":\"trivial|small|medium|large\"}}. Empty array [] if nothing.",
        lens.as_str(),
        lens_brief(lens),
    );
    vec![Message::system(&sys), Message::user(source)]
}

fn verifier_messages(lens: FindingCategory, c: &Candidate) -> Vec<Message> {
    let sys = format!(
        "You are an {VERIFIER_MARKER}. A '{}' critic raised the finding below. Try hard to REFUTE \
         it — is it actually wrong, already handled, or a false positive? Output ONLY JSON: \
         {{\"verdict\":\"uphold|refute\",\"confidence\":\"high|medium|low\"}}.",
        lens.as_str()
    );
    let body = format!(
        "severity: {}\nfile: {}\nline: {:?}\ntitle: {}\nwhy: {}",
        c.severity, c.file, c.line, c.title, c.why
    );
    vec![Message::system(&sys), Message::user(&body)]
}

/// Extract the JSON array from a critic reply (tolerant of prose / code fences) and parse it.
fn parse_candidates(text: &str) -> Vec<Candidate> {
    let Some(json) = slice_between(text, '[', ']') else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<Candidate>>(json).unwrap_or_default()
}

fn parse_verdict(text: &str) -> Option<Verdict> {
    let json = slice_between(text, '{', '}')?;
    serde_json::from_str::<Verdict>(json).ok()
}

/// The substring from the first `open` to the last `close`, inclusive — pulls a JSON value out of
/// a reply that may be wrapped in prose or ```json fences.
fn slice_between(text: &str, open: char, close: char) -> Option<&str> {
    let start = text.find(open)?;
    let end = text.rfind(close)?;
    (end >= start).then(|| &text[start..=end])
}

fn build_finding(lens: FindingCategory, c: Candidate, confidence: Confidence) -> Finding {
    Finding {
        id: new_id(),
        category: lens,
        severity: Severity::parse(&c.severity).unwrap_or(Severity::Medium),
        confidence,
        file: c.file,
        line: c.line,
        title: c.title,
        rationale: c.why,
        suggested_fix: c.fix,
        effort: Effort::parse(&c.effort).unwrap_or(Effort::Small),
        lens: lens.as_str().to_string(),
        verified: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_provider::{EventSink, ModelResponse, ProviderError, ToolSpec};
    use forge_types::Usage;

    /// A scripted critic/verifier: emits a per-lens finding (or none), then a per-finding verdict.
    /// `bad` lenses error; `refute` titles get refuted by the verifier.
    struct ScriptedProvider {
        bad: std::collections::HashSet<FindingCategory>,
    }

    #[async_trait::async_trait]
    impl Provider for ScriptedProvider {
        async fn complete(
            &self,
            _model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut EventSink<'_>,
        ) -> Result<ModelResponse, ProviderError> {
            let sys = messages
                .iter()
                .find(|m| m.role == forge_types::Role::System)
                .map(|m| m.content.as_str())
                .unwrap_or("");
            let usage = Usage {
                input_tokens: 10,
                output_tokens: 5,
                cost_usd: 0.0,
            };
            // Critic call: emit findings keyed by which lens (carried in the system prompt).
            if sys.contains(CRITIC_MARKER) {
                // Fail any "bad" lens to exercise graceful degradation.
                for lens in &self.bad {
                    if sys.contains(&format!("'{}'", lens.as_str())) {
                        return Err(ProviderError::Request("critic blew up".into()));
                    }
                }
                let content = if sys.contains("'correctness'") {
                    r#"[{"severity":"critical","file":"core/lib.rs","line":204,
                        "title":"unwrap panics the turn","why":"5xx aborts session",
                        "fix":"propagate via ?","effort":"small"}]"#
                } else if sys.contains("'dead-weight'") {
                    r#"prose before... [{"severity":"low","file":"x.rs","line":1,
                        "title":"REFUTE ME dead fn","why":"unused","fix":"delete","effort":"trivial"}] trailing"#
                } else {
                    "[]"
                };
                return Ok(ModelResponse {
                    content: content.into(),
                    tool_calls: vec![],
                    usage,
                });
            }
            // Verifier call: refute findings whose body contains "REFUTE ME", else uphold.
            if sys.contains(VERIFIER_MARKER) {
                let body = messages.last().map(|m| m.content.as_str()).unwrap_or("");
                let v = if body.contains("REFUTE ME") {
                    r#"{"verdict":"refute","confidence":"high"}"#
                } else {
                    r#"{"verdict":"uphold","confidence":"high"}"#
                };
                return Ok(ModelResponse {
                    content: v.into(),
                    tool_calls: vec![],
                    usage,
                });
            }
            Ok(ModelResponse {
                content: "[]".into(),
                tool_calls: vec![],
                usage,
            })
        }
    }

    fn pricing() -> Arc<Pricing> {
        Arc::new(Pricing::from_config(&forge_config::Config::default()))
    }

    fn store() -> Arc<forge_store::Store> {
        Arc::new(forge_store::Store::open_in_memory().unwrap())
    }

    fn models() -> TierModels {
        TierModels {
            trivial: vec!["mock::cheap".into()],
            complex: vec!["mock::frontier".into()],
        }
    }

    /// A provider that rate-limits any model in `bad`, and otherwise plays critic (a correctness
    /// finding) + verifier (uphold). Used to exercise per-critic model failover.
    struct FailoverProvider {
        bad: std::collections::HashSet<String>,
    }
    #[async_trait::async_trait]
    impl Provider for FailoverProvider {
        async fn complete(
            &self,
            model: &str,
            messages: &[Message],
            _tools: &[ToolSpec],
            _on_event: &mut EventSink<'_>,
        ) -> Result<ModelResponse, ProviderError> {
            if self.bad.contains(model) {
                return Err(ProviderError::RateLimited {
                    message: "429".into(),
                    retry_after: Some(std::time::Duration::from_secs(30)),
                });
            }
            let sys = messages
                .iter()
                .find(|m| m.role == forge_types::Role::System)
                .map(|m| m.content.as_str())
                .unwrap_or("");
            let content = if sys.contains(VERIFIER_MARKER) {
                r#"{"verdict":"uphold","confidence":"high"}"#
            } else if sys.contains(CRITIC_MARKER) {
                r#"[{"severity":"high","file":"a.rs","line":1,"title":"bug","why":"w","fix":"f","effort":"small"}]"#
            } else {
                "[]"
            };
            Ok(ModelResponse {
                content: content.into(),
                tool_calls: vec![],
                usage: Usage::default(),
            })
        }
    }

    #[tokio::test]
    async fn critic_fails_over_when_its_model_is_rate_limited() {
        // The first complex-tier model 429s; the critic must fall over to the next and still
        // produce a finding (the bug the user hit: one dead model skipped the whole tier).
        let provider = Arc::new(FailoverProvider {
            bad: ["bad::model".to_string()].into_iter().collect(),
        });
        let st = store();
        let report = run_assay(
            AssayScope::Repo,
            Arc::from("fn main() {}"),
            vec![FindingCategory::Correctness], // complex tier
            TierModels {
                trivial: vec![],
                complex: vec!["bad::model".into(), "good::model".into()],
            },
            provider,
            pricing(),
            st.clone(),
            std::time::Duration::from_secs(60),
        )
        .await;

        assert_eq!(
            report.findings.len(),
            1,
            "failed over to a live model instead of skipping the tier: {report:?}"
        );
        assert!(
            report.skipped_lenses.is_empty(),
            "nothing skipped after failover: {report:?}"
        );
        assert!(
            st.current_benched().unwrap().is_benched("bad::model"),
            "the rate-limited model was benched"
        );
    }

    #[tokio::test]
    async fn crew_verifies_keeps_upheld_drops_refuted_and_ranks() {
        let provider = Arc::new(ScriptedProvider {
            bad: Default::default(),
        });
        let report = run_assay(
            AssayScope::Repo,
            Arc::from("fn main() {}"),
            vec![
                FindingCategory::Correctness,
                FindingCategory::DeadWeight,
                FindingCategory::Design,
            ],
            models(),
            provider,
            pricing(),
            store(),
            std::time::Duration::from_secs(60),
        )
        .await;

        // The dead-weight candidate is refuted and dropped; the correctness one survives.
        assert_eq!(
            report.findings.len(),
            1,
            "refuted finding dropped: {report:?}"
        );
        let f = &report.findings[0];
        assert_eq!(f.category, FindingCategory::Correctness);
        assert_eq!(f.severity, Severity::Critical);
        assert_eq!(f.confidence, Confidence::High);
        assert!(f.verified);
        assert_eq!(f.line, Some(204));
    }

    #[tokio::test]
    async fn a_failing_critic_degrades_gracefully() {
        let provider = Arc::new(ScriptedProvider {
            bad: [FindingCategory::Correctness].into_iter().collect(),
        });
        let report = run_assay(
            AssayScope::Repo,
            Arc::from("src"),
            vec![FindingCategory::Correctness, FindingCategory::Design],
            models(),
            provider,
            pricing(),
            store(),
            std::time::Duration::from_secs(60),
        )
        .await;

        assert!(
            report
                .skipped_lenses
                .iter()
                .any(|(l, _)| l == "correctness"),
            "failed lens recorded as skipped: {report:?}"
        );
        // The run still completes (the other lens produced no findings, but didn't crash).
        assert!(report.findings.is_empty());
    }

    #[test]
    fn parse_candidates_tolerates_prose_and_fences() {
        let text = "Here are the issues:\n```json\n[{\"severity\":\"high\",\"file\":\"a.rs\",\
                    \"title\":\"t\"}]\n```\nthat's all";
        let cands = parse_candidates(text);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].file, "a.rs");
    }
}
