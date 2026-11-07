//! The review gate in front of PR Watch's merge (docs/features/pr-review-gate.md).
//!
//! When a merge-mode watch is about to merge (`Decision::Merge`, which includes GitHub's
//! auto-merge method and the merge queue's head), the PR's diff goes to an independent reviewer
//! once per head SHA. Confirmed findings at or above the configured severity block the merge and
//! return to the PR's session as one fix turn; the review runs again on the head the session
//! pushes. A clean review lets the merge proceed. The verdict lives on the watch record
//! (`pr-watch.json`), so every surface that shows a watch shows the review too.
//!
//! Routes: `GET /api/pr/review?pr=|session=` the stored review with every finding.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};

use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_pr::{Gh, PrSummary};
use crate::serve_pr_watch::{registry, Announcement, Watch};

const MAX_DIFF_BYTES: usize = 150_000;
/// A failed review is not retried sooner than this.
const RETRY_SECS: i64 = 300;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Verdict {
    Clean,
    Findings,
    Error,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReviewFinding {
    pub(crate) severity: String,
    pub(crate) file: String,
    pub(crate) line: Option<u32>,
    pub(crate) title: String,
    pub(crate) why: String,
    #[serde(default)]
    pub(crate) fix: String,
    /// At or above the configured severity: this one holds the merge.
    pub(crate) blocking: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReviewState {
    pub(crate) head_sha: String,
    pub(crate) verdict: Verdict,
    pub(crate) findings: Vec<ReviewFinding>,
    pub(crate) model: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) reviewed_at: i64,
    /// The fix turn for this head's findings has been sent.
    pub(crate) fix_sent: bool,
    /// Fix rounds sent so far, across heads.
    pub(crate) rounds: u32,
    /// The push for this head's findings has been delivered.
    #[serde(default)]
    pub(crate) announced: bool,
}

impl ReviewState {
    pub(crate) fn confirmed(&self) -> usize {
        self.findings.iter().filter(|f| f.blocking).count()
    }

    /// The row text: `review clean`, `review: 2 confirmed, fixing`.
    pub(crate) fn summary(&self) -> String {
        match self.verdict {
            Verdict::Clean => "review clean".into(),
            Verdict::Error => "review failed, will retry".into(),
            Verdict::Findings if self.fix_sent => {
                format!("review: {} confirmed, fixing", self.confirmed())
            }
            Verdict::Findings => format!("review: {} confirmed", self.confirmed()),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Settings {
    pub(crate) enabled: bool,
    pub(crate) max_findings: usize,
    pub(crate) threshold: forge_types::Severity,
    pub(crate) merge_on_error: bool,
    pub(crate) max_rounds: u32,
}

impl Settings {
    #[cfg_attr(test, allow(dead_code))]
    fn from_config(config: &forge_config::Config) -> Self {
        let c = &config.pr_watch;
        Self {
            enabled: c.review_enabled(),
            max_findings: c.review_max_findings.max(1) as usize,
            threshold: forge_types::Severity::parse(&c.review_severity)
                .unwrap_or(forge_types::Severity::High),
            merge_on_error: c.merge_on_review_error(),
            max_rounds: c.review_max_rounds,
        }
    }
}

pub(crate) struct ReviewRequest<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) pr_number: u64,
    pub(crate) head_sha: &'a str,
    pub(crate) diff: &'a str,
}

/// An independent review of one PR diff. Implementations return only findings that survived
/// verification; the gate applies the severity threshold and the cap.
#[async_trait::async_trait]
pub(crate) trait Reviewer: Send + Sync {
    async fn review(&self, request: &ReviewRequest<'_>) -> Result<Reviewed, String>;
}

pub(crate) struct Reviewed {
    pub(crate) findings: Vec<ReviewFinding>,
    pub(crate) model: Option<String>,
}

static REVIEWER: OnceLock<Arc<dyn Reviewer>> = OnceLock::new();

/// Called once by the daemon: PR reviews run through Assay with the daemon's store.
pub(crate) fn install(state: &Arc<DaemonState>) {
    let _ = REVIEWER.set(Arc::new(AssayReviewer {
        store: state.store.clone(),
    }));
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_HOOK: std::cell::RefCell<Option<(Arc<dyn Reviewer>, Settings)>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn active() -> Option<(Arc<dyn Reviewer>, Settings)> {
    TEST_HOOK.with(|h| h.borrow().clone())
}

#[cfg(not(test))]
fn active() -> Option<(Arc<dyn Reviewer>, Settings)> {
    let reviewer = REVIEWER.get()?.clone();
    let config = forge_config::load().ok()?;
    Some((reviewer, Settings::from_config(&config)))
}

/// What the gate tells PR Watch to do instead of merging.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    Pass,
    Wait(String),
    Block(String),
    /// One fix turn for the session.
    Fix(String),
}

pub(crate) async fn gate(
    gh: &Gh,
    root: &Path,
    watch: &mut Watch,
    pr: &PrSummary,
    free: bool,
    now: i64,
) -> Gate {
    let Some((reviewer, settings)) = active() else {
        return Gate::Pass;
    };
    if !settings.enabled {
        return Gate::Pass;
    }
    let rounds = watch.review.as_ref().map_or(0, |r| r.rounds);
    let current = watch
        .review
        .as_ref()
        .filter(|r| r.head_sha == pr.head_sha)
        .cloned();
    let state = match current {
        Some(r) if r.verdict == Verdict::Error && now - r.reviewed_at >= RETRY_SECS => {
            run(&*reviewer, &settings, gh, root, watch, pr, rounds, now).await
        }
        Some(r) => r,
        None => run(&*reviewer, &settings, gh, root, watch, pr, rounds, now).await,
    };
    let verdict = state.verdict;
    watch.review = Some(state);
    match verdict {
        Verdict::Clean => Gate::Pass,
        Verdict::Error => {
            let why = watch
                .review
                .as_ref()
                .and_then(|r| r.error.clone())
                .unwrap_or_default();
            if settings.merge_on_error {
                watch.last_action = Some(format!("review failed ({why}); merging anyway"));
                Gate::Pass
            } else {
                Gate::Wait(format!("review failed ({why}); not merging, will retry"))
            }
        }
        Verdict::Findings => findings_gate(watch, pr, &settings, free),
    }
}

fn findings_gate(watch: &mut Watch, pr: &PrSummary, settings: &Settings, free: bool) -> Gate {
    let review = watch.review.as_mut().expect("set by the caller");
    let confirmed = review.confirmed();
    if confirmed == 0 {
        review.verdict = Verdict::Clean;
        return Gate::Pass;
    }
    if review.fix_sent {
        return Gate::Wait(format!(
            "review: {confirmed} confirmed; waiting for the session to push"
        ));
    }
    if review.rounds >= settings.max_rounds {
        return Gate::Block(format!(
            "review findings unresolved after {} fix rounds ({confirmed} confirmed)",
            review.rounds
        ));
    }
    if !free {
        return Gate::Wait(format!("review: {confirmed} confirmed; session is busy"));
    }
    review.fix_sent = true;
    review.rounds += 1;
    watch.last_action = Some(format!("review fix turn {} sent", review.rounds));
    Gate::Fix(fix_prompt(pr, &review.findings))
}

#[allow(clippy::too_many_arguments)]
async fn run(
    reviewer: &dyn Reviewer,
    settings: &Settings,
    gh: &Gh,
    root: &Path,
    watch: &Watch,
    pr: &PrSummary,
    rounds: u32,
    now: i64,
) -> ReviewState {
    let mut state = ReviewState {
        head_sha: pr.head_sha.clone(),
        verdict: Verdict::Clean,
        findings: Vec::new(),
        model: None,
        error: None,
        reviewed_at: now,
        fix_sent: false,
        rounds,
        announced: false,
    };
    let number = pr.number.to_string();
    let diff = match gh.run(root, &["pr", "diff", &number], None).await {
        Ok(diff) => clip(diff),
        Err(e) => {
            return failed(
                state,
                format!("could not read the PR diff: {}", e.message()),
            )
        }
    };
    if diff.trim().is_empty() {
        return state;
    }
    let request = ReviewRequest {
        session_id: &watch.session_id,
        pr_number: pr.number,
        head_sha: &pr.head_sha,
        diff: &diff,
    };
    match reviewer.review(&request).await {
        Err(e) => failed(state, e),
        Ok(reviewed) => {
            let mut findings = reviewed.findings;
            findings.sort_by_key(|f| std::cmp::Reverse(rank(&f.severity)));
            findings.truncate(settings.max_findings);
            for f in &mut findings {
                f.blocking = rank(&f.severity) >= settings.threshold.weight();
            }
            state.model = reviewed.model;
            if findings.iter().any(|f| f.blocking) {
                state.verdict = Verdict::Findings;
            }
            state.findings = findings;
            state
        }
    }
}

fn failed(mut state: ReviewState, why: String) -> ReviewState {
    state.verdict = Verdict::Error;
    state.error = Some(why);
    state
}

fn rank(severity: &str) -> u8 {
    forge_types::Severity::parse(severity).map_or(0, |s| s.weight())
}

fn clip(mut diff: String) -> String {
    if diff.len() > MAX_DIFF_BYTES {
        let mut end = MAX_DIFF_BYTES;
        while !diff.is_char_boundary(end) {
            end -= 1;
        }
        diff.truncate(end);
        diff.push_str("\n[diff truncated]\n");
    }
    diff
}

fn fix_prompt(pr: &PrSummary, findings: &[ReviewFinding]) -> String {
    let mut text = format!(
        "PR Watch: an independent review of pull request #{} ({}) confirmed defects that must be \
         fixed before it can merge.\n\nConfirmed findings:\n",
        pr.number, pr.url
    );
    for f in findings.iter().filter(|f| f.blocking) {
        let place = f
            .line
            .map_or_else(|| f.file.clone(), |l| format!("{}:{l}", f.file));
        text.push_str(&format!(
            "- [{}] {place}: {}\n  why: {}\n",
            f.severity, f.title, f.why
        ));
        if !f.fix.is_empty() {
            text.push_str(&format!("  suggested fix: {}\n", f.fix));
        }
    }
    text.push_str(
        "\nCheck each one against the code first; if a finding is wrong, say why in the commit \
         message. Fix the real ones in this worktree, add a test that fails without the fix, run \
         the relevant checks locally, commit, and push with a normal `git push`. Never \
         force-push, never rewrite history, never delete the branch. The review runs again on \
         the new head.",
    );
    text
}

/// The push owed for a freshly confirmed set of findings; at most once per head.
pub(crate) fn review_push(watch: &mut Watch) -> Option<Announcement> {
    let pr = watch
        .pr_number
        .map_or_else(|| "PR".to_string(), |n| format!("PR #{n}"));
    let review = watch.review.as_mut()?;
    if review.verdict != Verdict::Findings || review.announced || review.confirmed() == 0 {
        return None;
    }
    review.announced = true;
    Some(Announcement::new(
        "review",
        format!(
            "{pr} blocked by review: {} confirmed defect(s)",
            review.confirmed()
        ),
        true,
    ))
}

struct AssayReviewer {
    store: Arc<forge_store::Store>,
}

fn reviewer_models(config: &forge_config::Config) -> Result<forge_core::assay::TierModels, String> {
    let c = &config.pr_watch;
    let pinned = if c.review_model.trim().is_empty() {
        config.harness.expert.model.trim().to_string()
    } else {
        c.review_model.trim().to_string()
    };
    if pinned.is_empty() {
        let tiers = forge_core::assay::TierModels {
            trivial: config.candidates_for(forge_types::TaskTier::Trivial),
            complex: config.candidates_for(forge_types::TaskTier::Complex),
        };
        if tiers.trivial.is_empty() && tiers.complex.is_empty() {
            return Err("no reviewer model is configured; set [pr_watch] review_model".into());
        }
        return Ok(tiers);
    }
    let provider = forge_config::provider_of(&pinned);
    if !provider.is_empty() && !forge_config::is_known_provider(provider) {
        return Err(format!(
            "'{provider}' is not a known provider in reviewer model {pinned}"
        ));
    }
    let subscription =
        forge_provider::is_cli_bridge(&pinned) || pinned.starts_with("codex-oauth::");
    let budget = if c.review_budget.trim().is_empty() {
        config.harness.expert.budget.trim()
    } else {
        c.review_budget.trim()
    };
    if subscription && budget.is_empty() {
        return Err(format!(
            "{pinned} is a subscription model and needs [pr_watch] review_budget"
        ));
    }
    Ok(forge_core::assay::TierModels {
        trivial: vec![pinned.clone()],
        complex: vec![pinned],
    })
}

/// A dollar cap (`$2`) from a budget string; slices of a subscription window are not metered here.
fn usd_cap(config: &forge_config::Config) -> Option<f64> {
    let c = &config.pr_watch;
    let budget = if c.review_budget.trim().is_empty() {
        config.harness.expert.budget.trim()
    } else {
        c.review_budget.trim()
    };
    budget.strip_prefix('$').and_then(|v| v.trim().parse().ok())
}

#[async_trait::async_trait]
impl Reviewer for AssayReviewer {
    async fn review(&self, request: &ReviewRequest<'_>) -> Result<Reviewed, String> {
        use forge_core::assay;
        let config = forge_config::load().map_err(|e| e.to_string())?;
        let models = reviewer_models(&config)?;
        let model = models.complex.first().or(models.trivial.first()).cloned();
        let lenses = vec![
            forge_types::FindingCategory::Correctness,
            forge_types::FindingCategory::Unsafe,
        ];
        let pricing = Arc::new(forge_mesh::pricing::Pricing::from_config(&config));
        let source = format!(
            "Pull request #{} at {}\n\n{}",
            request.pr_number, request.head_sha, request.diff
        );
        if let Some(cap) = usd_cap(&config) {
            let estimate = assay::estimate_assay_cost(&source, &lenses, &models, &pricing);
            if estimate.est_usd > cap {
                return Err(format!(
                    "estimated ${:.3} exceeds the review budget ${cap:.2}",
                    estimate.est_usd
                ));
            }
        }
        let harness = config.mesh.bridge_mode == forge_config::BridgeMode::Harness;
        let provider: Arc<dyn forge_provider::Provider> = forge_core::zero_retention::guard(
            Arc::new(
                forge_provider::DispatchProvider::new(harness)
                    .with_max_output_tokens(config.mesh.effective_max_output_tokens()),
            ),
            &config,
        );
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
            })
            .collect();
        Ok(Reviewed { findings, model })
    }
}

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new().route(&format!("{base}/api/pr/review"), get(review_get))
}

#[derive(Deserialize)]
struct ReviewQuery {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    pr: Option<u64>,
}

async fn review_get(State(_): State<Arc<DaemonState>>, Query(q): Query<ReviewQuery>) -> Response {
    let all = registry().all();
    let mut hits = all.iter().filter(|w| match (&q.session, q.pr) {
        (Some(s), _) => &w.session_id == s,
        (None, Some(n)) => w.pr_number == Some(n),
        (None, None) => false,
    });
    let Some(watch) = hits.next() else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "no PR watch matches");
    };
    if hits.next().is_some() {
        return err_response(
            axum::http::StatusCode::CONFLICT,
            "that PR is watched in more than one repository; name the session",
        );
    }
    json_response(&serde_json::json!({
        "session_id": watch.session_id,
        "pr_number": watch.pr_number,
        "pr_url": watch.pr_url,
        "summary": watch.review.as_ref().map(ReviewState::summary),
        "review": watch.review,
    }))
}

#[cfg(test)]
mod tests;
