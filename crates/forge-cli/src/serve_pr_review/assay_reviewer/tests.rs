//! The production reviewer end to end: the real Assay pipeline (lens prompts, candidate parsing,
//! adversarial verification, finding conversion) over a scripted model provider.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex as StdMutex;

use forge_provider::{EventSink, ModelResponse, ProviderError, ToolSpec};
use forge_types::{Message, Usage};

use super::*;
use crate::serve_pr::Gh;
use crate::serve_pr_review::TEST_HOOK;
use crate::serve_pr_watch::{poll_once, Mode, Watch};

const DIFF: &str = "diff --git a/src/api.rs b/src/api.rs\n+++ b/src/api.rs\n@@ -10,3 +10,4 @@\n+    let n: usize = body.parse().unwrap();\n+    counter += n;\n";

/// A model that plays whatever role the Assay prompt gives it. As a critic it reports the
/// unwrap on user input under the correctness lens, and (when `overflow` is set) a second claim
/// the verifier will refute; as a verifier it refutes the claim titled "possible overflow".
struct Scripted {
    overflow: bool,
    systems: StdMutex<Vec<String>>,
    sources: StdMutex<Vec<String>>,
    verifier_calls: AtomicUsize,
    /// Reported on every reply as the subscription window's consumed fraction.
    window_fraction: StdMutex<Option<f64>>,
}

impl Scripted {
    fn new(overflow: bool) -> Arc<Self> {
        Arc::new(Self {
            overflow,
            systems: StdMutex::new(Vec::new()),
            sources: StdMutex::new(Vec::new()),
            verifier_calls: AtomicUsize::new(0),
            window_fraction: StdMutex::new(None),
        })
    }
}

#[async_trait::async_trait]
impl forge_provider::Provider for Scripted {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        let system = messages[0].content.clone();
        let user = messages.last().unwrap().content.clone();
        self.systems.lock().unwrap().push(system.clone());
        let content = if system.contains("ASSAY-VERIFIER") {
            self.verifier_calls.fetch_add(1, Ordering::SeqCst);
            if user.contains("possible overflow") {
                r#"{"verdict":"refute","confidence":"high"}"#
            } else {
                r#"{"verdict":"uphold","confidence":"high"}"#
            }
            .to_string()
        } else if system.contains("ASSAY-CRITIC") {
            self.sources.lock().unwrap().push(user.clone());
            if system.contains("'correctness'") && user.contains(".parse().unwrap()") {
                let mut found = vec![
                    r#"{"severity":"high","file":"src/api.rs","line":11,"title":"unwrap on user input","why":"an empty or non-numeric body panics the handler","fix":"return a 400 on a parse error","effort":"small"}"#,
                ];
                if self.overflow {
                    found.push(
                        r#"{"severity":"high","file":"src/api.rs","line":12,"title":"possible overflow in counter","why":"counter += n can wrap","fix":"use checked_add","effort":"small"}"#,
                    );
                }
                format!("[{}]", found.join(","))
            } else {
                "[]".to_string()
            }
        } else {
            "[]".to_string()
        };
        let quotas = self
            .window_fraction
            .lock()
            .unwrap()
            .map(|fraction| {
                vec![forge_types::QuotaHint {
                    provider: "codex-cli".into(),
                    window: "weekly".into(),
                    status: forge_types::QuotaStatus::Ok,
                    resets_at: Some(chrono::Utc::now().timestamp() + 100_000),
                    fraction_used: Some(fraction),
                }]
            })
            .unwrap_or_default();
        Ok(ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls: vec![],
            usage: Usage::default(),
            quotas,
        })
    }
}

fn config(model: &str, budget: &str) -> forge_config::Config {
    let mut config = forge_config::Config::default();
    config.pr_watch.review_model = model.into();
    config.pr_watch.review_budget = budget.into();
    config
}

fn request(diff: &str) -> ReviewRequest<'_> {
    ReviewRequest {
        session_id: "s1",
        pr_number: 7,
        head_sha: "aaaa1111",
        diff,
    }
}

fn store() -> Arc<forge_store::Store> {
    Arc::new(forge_store::Store::open_in_memory().unwrap())
}

#[tokio::test]
async fn the_real_pipeline_confirms_a_planted_defect() {
    let model = Scripted::new(false);
    let reviewer = AssayReviewer::new(store());
    let reviewed = reviewer
        .review_with(&config("ollama::critic", ""), model.clone(), &request(DIFF))
        .await
        .unwrap();
    assert_eq!(reviewed.model.as_deref(), Some("ollama::critic"));
    assert_eq!(reviewed.findings.len(), 1, "{:?}", reviewed.findings);
    let f = &reviewed.findings[0];
    assert_eq!(
        (f.severity.as_str(), f.file.as_str(), f.line),
        ("high", "src/api.rs", Some(11))
    );
    assert_eq!(f.title, "unwrap on user input");
    assert!(f.why.contains("panics the handler") && f.fix.contains("400"));

    let systems = model.systems.lock().unwrap();
    assert!(systems.iter().any(|s| s.contains("'correctness'")));
    assert!(systems.iter().any(|s| s.contains("'unsafe'")));
    assert!(model
        .sources
        .lock()
        .unwrap()
        .iter()
        .all(|s| s.contains("Pull request #7 at aaaa1111") && s.contains(".parse().unwrap()")));
    assert_eq!(
        model.verifier_calls.load(Ordering::SeqCst),
        1,
        "every candidate is checked by a verifier"
    );
}

struct Real {
    reviewer: AssayReviewer,
    config: forge_config::Config,
    model: Arc<Scripted>,
}

#[async_trait::async_trait]
impl Reviewer for Real {
    async fn review(&self, request: &ReviewRequest<'_>) -> Result<Reviewed, String> {
        self.reviewer
            .review_with(&self.config, self.model.clone(), request)
            .await
    }
}

fn fake_gh(dir: &Path) -> Gh {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("gh");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> {log}\ncase \"$2\" in\n  view) cat {view} ;;\n  diff) cat {diff} ;;\n  merge) exit 0 ;;\nesac\n",
        log = dir.join("calls.log").display(),
        view = dir.join("view.json").display(),
        diff = dir.join("diff.txt").display(),
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("diff.txt"), DIFF).unwrap();
    std::fs::write(
        dir.join("view.json"),
        r#"{"number":7,"url":"https://example.invalid/pr/7","title":"t","state":"OPEN","isDraft":false,
        "headRefName":"feat","baseRefName":"main","headRefOid":"aaaa1111","mergeable":"MERGEABLE",
        "mergeStateStatus":"CLEAN","reviewDecision":"","statusCheckRollup":[{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}],
        "comments":[],"reviews":[]}"#,
    )
    .unwrap();
    Gh::with_program(path)
}

async fn watch_through(
    overflow: bool,
) -> (crate::serve_pr_watch::Outcome, Watch, String, Arc<Scripted>) {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let model = Scripted::new(overflow);
    let real = Real {
        reviewer: AssayReviewer::new(store()),
        config: config("ollama::critic", ""),
        model: model.clone(),
    };
    let settings = crate::serve_pr_review::Settings {
        enabled: true,
        max_findings: 5,
        threshold: forge_types::Severity::High,
        merge_on_error: false,
        max_rounds: 3,
    };
    TEST_HOOK.with(|h| *h.borrow_mut() = Some((Arc::new(real), settings)));
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    for _ in 0..500 {
        if crate::serve_pr_review::jobs::jobs().running() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let out = poll_once(&gh, dir.path(), &mut watch, true, 20).await;
    let calls = std::fs::read_to_string(dir.path().join("calls.log")).unwrap_or_default();
    (out, watch, calls, model)
}

#[tokio::test]
async fn a_confirmed_finding_from_the_real_reviewer_blocks_the_merge() {
    let (out, watch, calls, _) = watch_through(false).await;
    assert!(!out.merged && !calls.contains("pr merge"), "{calls}");
    let prompt = out
        .prompt
        .expect("the confirmed defect goes back to the session");
    assert!(
        prompt.contains("[high] src/api.rs:11: unwrap on user input"),
        "{prompt}"
    );
    assert_eq!(
        watch.review.unwrap().summary(),
        "review: 1 confirmed, fixing"
    );
}

#[tokio::test]
async fn a_claim_the_verifier_refutes_does_not_block() {
    let (out, watch, _, model) = watch_through(true).await;
    assert_eq!(
        model.verifier_calls.load(Ordering::SeqCst),
        2,
        "both claims were put to a verifier"
    );
    let review = watch.review.unwrap();
    assert_eq!(review.findings.len(), 1, "the refuted claim never gets in");
    assert!(review
        .findings
        .iter()
        .all(|f| !f.title.contains("overflow")));
    assert!(out.prompt.is_some(), "the upheld defect still blocks");
}

#[tokio::test]
async fn a_window_slice_stops_reviews_once_the_window_has_risen_by_it() {
    let reviewer = AssayReviewer::new(store());
    let model = Scripted::new(false);
    let cfg = config("codex-cli::gpt-5.6-luna", "codex:5%w");

    *model.window_fraction.lock().unwrap() = Some(0.40);
    reviewer
        .review_with(&cfg, model.clone(), &request(DIFF))
        .await
        .expect("the first review starts the count at 40%");

    *model.window_fraction.lock().unwrap() = Some(0.43);
    reviewer
        .review_with(&cfg, model.clone(), &request(DIFF))
        .await
        .expect("3% of 5% is spent; the second review is allowed");

    *model.window_fraction.lock().unwrap() = Some(0.46);
    reviewer
        .review_with(&cfg, model.clone(), &request(DIFF))
        .await
        .expect("checked before the run: this one still starts");

    let refused = reviewer
        .review_with(&cfg, model.clone(), &request(DIFF))
        .await
        .unwrap_err();
    assert!(
        refused.contains("review budget (5% of the codex weekly window) is spent"),
        "{refused}"
    );
}

#[test]
fn a_window_slice_needs_a_matching_subscription_reviewer() {
    let bad = |model: &str, budget: &str| {
        reviewer_models(&config(model, budget))
            .err()
            .unwrap_or_default()
    };
    assert!(bad("codex-cli::x", "").contains("needs [pr_watch] review_budget"));
    assert!(bad("codex-cli::x", "claude:5%w").contains("must be a codex slice"));
    assert!(bad("codex-cli::x", "banana").contains("review_budget"));
    assert!(bad("ollama::x", "codex:5%w").contains("billed per token"));
    assert!(reviewer_models(&config("codex-cli::x", "codex:5%w")).is_ok());
    assert!(reviewer_models(&config("ollama::x", "$2")).is_ok());
}

#[test]
fn an_unpinned_reviewer_gets_the_same_subscription_budget_check() {
    let unpinned = |models: &[&str], budget: &str| {
        let mut c = config("", budget);
        let list = forge_config::OneOrMany::Many(models.iter().map(|m| m.to_string()).collect());
        c.mesh.models.insert("standard".into(), list.clone());
        c.mesh.models.insert("complex".into(), list.clone());
        c.mesh.models.insert("trivial".into(), list);
        reviewer_models(&c)
    };
    let err = unpinned(&["claude-cli::opus"], "").err().unwrap();
    assert!(err.contains("needs [pr_watch] review_budget"), "{err}");
    let kept = unpinned(&["claude-cli::opus", "ollama::x"], "").unwrap();
    assert_eq!(kept.complex, vec!["ollama::x".to_string()]);
    let kept = unpinned(&["claude-cli::opus", "ollama::x"], "claude:5%w").unwrap();
    assert_eq!(kept.complex.len(), 2);
}
