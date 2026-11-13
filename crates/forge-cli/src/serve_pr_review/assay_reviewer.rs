//! The production reviewer: Assay's critic crew (correctness and unsafe lenses) over the PR diff,
//! every candidate checked by an independent adversarial verifier, only survivors kept.

use std::sync::{Arc, Mutex};

use forge_types::goal_envelope::{EnvelopeCheck, EnvelopeKind, EnvelopeSpec, EnvelopeState};

use super::{ReviewFinding, ReviewRequest, Reviewed, Reviewer};

pub(super) struct AssayReviewer {
    store: Arc<forge_store::Store>,
    /// What a window-slice budget (`codex:5%w`) has used up so far; see [`check_window`].
    window: Mutex<Option<EnvelopeState>>,
}

impl AssayReviewer {
    pub(super) fn new(store: Arc<forge_store::Store>) -> Self {
        Self {
            store,
            window: Mutex::new(None),
        }
    }
}

fn budget_of(config: &forge_config::Config) -> &str {
    let c = &config.pr_watch;
    if c.review_budget.trim().is_empty() {
        config.harness.expert.budget.trim()
    } else {
        c.review_budget.trim()
    }
}

pub(super) fn reviewer_models(
    config: &forge_config::Config,
) -> Result<forge_core::assay::TierModels, String> {
    let c = &config.pr_watch;
    let pinned = if c.review_model.trim().is_empty() {
        config.harness.expert.model.trim().to_string()
    } else {
        c.review_model.trim().to_string()
    };
    if pinned.is_empty() {
        let mut dropped = Vec::new();
        let mut usable = |candidates: Vec<String>| -> Vec<String> {
            candidates
                .into_iter()
                .filter(|m| match window_spec(config, m) {
                    Ok(_) => true,
                    Err(_) if !is_subscription(m) => true,
                    Err(why) => {
                        dropped.push(why);
                        false
                    }
                })
                .collect()
        };
        let tiers = forge_core::assay::TierModels {
            trivial: usable(config.candidates_for(forge_types::TaskTier::Trivial)),
            complex: usable(config.candidates_for(forge_types::TaskTier::Complex)),
        };
        if tiers.trivial.is_empty() && tiers.complex.is_empty() {
            return Err(dropped.into_iter().next().unwrap_or_else(|| {
                "no reviewer model is configured; set [pr_watch] review_model".into()
            }));
        }
        return Ok(tiers);
    }
    let provider = forge_config::provider_of(&pinned);
    if !provider.is_empty() && !forge_config::is_known_provider(provider) {
        return Err(format!(
            "'{provider}' is not a known provider in reviewer model {pinned}"
        ));
    }
    window_spec(config, &pinned)?;
    Ok(forge_core::assay::TierModels {
        trivial: vec![pinned.clone()],
        complex: vec![pinned],
    })
}

fn is_subscription(model: &str) -> bool {
    forge_provider::is_cli_bridge(model) || model.starts_with("codex-oauth::")
}

/// The window slice the budget names for `model`, or an explanation of why it cannot be used.
/// `Ok(None)` is no budget, or a dollar cap (enforced against Assay's cost estimate).
fn window_spec(config: &forge_config::Config, model: &str) -> Result<Option<EnvelopeSpec>, String> {
    let budget = budget_of(config);
    let subscription = is_subscription(model);
    if subscription && budget.is_empty() {
        return Err(format!(
            "{model} is a subscription model and needs [pr_watch] review_budget"
        ));
    }
    if budget.is_empty() {
        return Ok(None);
    }
    let family = forge_core::expert::subscription_family(model);
    if subscription && family.is_none() {
        return Err(format!(
            "{model} is a subscription Forge cannot meter, so it cannot be the reviewer; use \
             codex-oauth, codex-cli or claude-cli, or an API model"
        ));
    }
    let spec = EnvelopeSpec::parse(budget, family)
        .map_err(|e| format!("[pr_watch] review_budget: {e}"))?;
    match (&spec.kind, family) {
        (EnvelopeKind::Usd, _) => Ok(None),
        (EnvelopeKind::Quota { provider, .. }, Some(f)) if provider == f => Ok(Some(spec)),
        (EnvelopeKind::Quota { .. }, Some(f)) => Err(format!(
            "the review budget for {model} must be a {f} slice such as \"{f}:5%w\""
        )),
        (EnvelopeKind::Quota { .. }, None) => Err(format!(
            "[pr_watch] review_budget '{budget}' is a subscription window slice, but {model} is \
             billed per token; use a dollar budget such as \"$2\" or leave it empty"
        )),
    }
}

/// A dollar cap (`$2`) from a budget string.
fn usd_cap(config: &forge_config::Config) -> Option<f64> {
    budget_of(config)
        .strip_prefix('$')
        .and_then(|v| v.trim().parse().ok())
}

/// Reviews share one envelope per budget window: a slice such as `codex:5%w` is how far the
/// subscription window may rise across all reviews, counted from the first review in it. It is
/// checked before each review (the same point the expert checks before each consult), against the
/// window readings the reviews themselves record, and starts over when the window resets.
fn check_window(
    envelope: &Mutex<Option<EnvelopeState>>,
    spec: &EnvelopeSpec,
    windows: &[forge_store::SubscriptionWindow],
    now: i64,
) -> Result<(), String> {
    let mut envelope = envelope
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if envelope
        .as_ref()
        .is_some_and(|e| e.resets_at.is_some_and(|reset| now >= reset))
    {
        *envelope = None;
    }
    if forge_core::expert::meter(&mut envelope, spec, windows, 0.0, now) != EnvelopeCheck::Spent {
        return Ok(());
    }
    let used = envelope.as_ref().map_or(0.0, |e| e.used);
    Err(format!(
        "the review budget ({}% of the {} window) is spent: the window rose {used:.1}% across \
         reviews; it counts again from the window's reset",
        spec.slice,
        spec.label()
    ))
}

#[async_trait::async_trait]
impl Reviewer for AssayReviewer {
    async fn review(&self, request: &ReviewRequest<'_>) -> Result<Reviewed, String> {
        let config = forge_config::load().map_err(|e| e.to_string())?;
        let harness = config.mesh.bridge_mode == forge_config::BridgeMode::Harness;
        let provider: Arc<dyn forge_provider::Provider> = forge_core::zero_retention::guard(
            Arc::new(
                forge_provider::DispatchProvider::new(harness)
                    .with_max_output_tokens(config.mesh.effective_max_output_tokens()),
            ),
            &config,
        );
        self.review_with(&config, provider, request).await
    }
}

impl AssayReviewer {
    pub(super) async fn review_with(
        &self,
        config: &forge_config::Config,
        provider: Arc<dyn forge_provider::Provider>,
        request: &ReviewRequest<'_>,
    ) -> Result<Reviewed, String> {
        use forge_core::assay;
        let models = reviewer_models(config)?;
        let model = models.complex.first().or(models.trivial.first()).cloned();
        let lenses = vec![
            forge_types::FindingCategory::Correctness,
            forge_types::FindingCategory::Unsafe,
        ];
        let pricing = Arc::new(forge_mesh::pricing::Pricing::from_config(config));
        let source = format!(
            "Pull request #{} at {}\n\n{}",
            request.pr_number, request.head_sha, request.diff
        );
        if let Some(cap) = usd_cap(config) {
            let estimate = assay::estimate_assay_cost(&source, &lenses, &models, &pricing);
            if estimate.est_usd > cap {
                return Err(format!(
                    "estimated ${:.3} exceeds the review budget ${cap:.2}",
                    estimate.est_usd
                ));
            }
        }
        if let Some(spec) = model
            .as_deref()
            .map(|m| window_spec(config, m))
            .transpose()?
            .flatten()
        {
            let windows = self.store.subscription_windows().unwrap_or_default();
            check_window(
                &self.window,
                &spec,
                &windows,
                chrono::Utc::now().timestamp(),
            )?;
        }
        let cooldown = std::time::Duration::from_secs(config.mesh.failover_cooldown_secs);
        let wanted = lenses.len();
        let mut ignore_progress = |_: assay::AssayProgress| {};
        let run = assay::run_assay(
            forge_types::AssayScope::Diff,
            Arc::from(source.as_str()),
            lenses,
            models,
            provider,
            pricing,
            self.store.clone(),
            cooldown,
            &mut ignore_progress,
        );
        let secs = config.pr_watch.review_timeout_secs.max(30);
        let report = tokio::time::timeout(std::time::Duration::from_secs(secs), run)
            .await
            .map_err(|_| format!("the reviewer did not finish within {secs}s"))?;
        if report.skipped_lenses.len() >= wanted {
            let why = report
                .skipped_lenses
                .first()
                .map(|(_, reason)| reason.clone())
                .unwrap_or_default();
            return Err(format!("every reviewer lens failed ({why})"));
        }
        tracing::info!(
            session = request.session_id,
            pr = request.pr_number,
            "PR review done"
        );
        let findings = report
            .findings
            .into_iter()
            .filter(|f| f.verified)
            .map(|f| ReviewFinding {
                severity: f.severity.as_str().to_string(),
                file: f.file,
                line: f.line,
                title: f.title,
                why: f.rationale,
                fix: f.suggested_fix,
                blocking: false,
                dismissed: None,
            })
            .collect();
        Ok(Reviewed { findings, model })
    }
}

#[cfg(test)]
mod tests;
