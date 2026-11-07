use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::*;
use crate::serve_pr_watch::{poll_once, Mode, Phase};

struct Scripted {
    calls: AtomicUsize,
    replies: Mutex<Vec<Result<Vec<ReviewFinding>, String>>>,
}

#[async_trait::async_trait]
impl Reviewer for Scripted {
    async fn review(&self, _: &ReviewRequest<'_>) -> Result<Reviewed, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut replies = self.replies.lock().unwrap();
        let next = if replies.len() > 1 {
            replies.remove(0)
        } else {
            replies[0].clone()
        };
        next.map(|findings| Reviewed {
            findings,
            model: Some("mock::critic".into()),
        })
    }
}

fn finding(severity: &str, line: u32) -> ReviewFinding {
    ReviewFinding {
        severity: severity.into(),
        file: "src/lib.rs".into(),
        line: Some(line),
        title: "unchecked unwrap on user input".into(),
        why: "panics the daemon on an empty body".into(),
        fix: "return an error".into(),
        blocking: false,
    }
}

fn install_hook(
    replies: Vec<Result<Vec<ReviewFinding>, String>>,
    tune: impl FnOnce(&mut Settings),
) -> Arc<Scripted> {
    let reviewer = Arc::new(Scripted {
        calls: AtomicUsize::new(0),
        replies: Mutex::new(replies),
    });
    let mut settings = Settings {
        enabled: true,
        max_findings: 5,
        threshold: forge_types::Severity::High,
        merge_on_error: false,
        max_rounds: 3,
    };
    tune(&mut settings);
    TEST_HOOK.with(|h| *h.borrow_mut() = Some((reviewer.clone(), settings)));
    reviewer
}

fn fake_gh(dir: &Path) -> Gh {
    let path = dir.join("gh");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> {log}\ncase \"$2\" in\n  view) cat {view} ;;\n  diff) echo '+let x = input.unwrap();' ;;\n  merge) exit 0 ;;\nesac\n",
        log = dir.join("calls.log").display(),
        view = dir.join("view.json").display(),
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    Gh::with_program(path)
}

fn view(dir: &Path, sha: &str) {
    let body = format!(
        r#"{{"number":7,"url":"https://example.invalid/pr/7","title":"t","state":"OPEN","isDraft":false,
        "headRefName":"feat","baseRefName":"main","headRefOid":"{sha}","mergeable":"MERGEABLE",
        "mergeStateStatus":"CLEAN","reviewDecision":"","statusCheckRollup":[{{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}}],
        "comments":[],"reviews":[]}}"#
    );
    std::fs::write(dir.join("view.json"), body).unwrap();
}

fn calls(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default()
}

#[tokio::test]
async fn a_planted_defect_blocks_the_merge_goes_back_to_the_session_and_merges_once_clean() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let reviewer = install_hook(
        vec![
            Ok(vec![finding("high", 12), finding("low", 40)]),
            Ok(vec![]),
        ],
        |_| {},
    );
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);

    view(dir.path(), "aaaa1111");
    let first = poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    assert!(!first.merged, "a confirmed defect must hold the merge");
    let prompt = first.prompt.expect("the findings go back as a fix turn");
    assert!(
        prompt.contains("[high] src/lib.rs:12: unchecked unwrap"),
        "{prompt}"
    );
    assert!(prompt.contains("why: panics the daemon"), "{prompt}");
    assert!(
        !prompt.contains("src/lib.rs:40"),
        "below-threshold findings are not sent"
    );
    assert!(prompt.contains("Never force-push"));
    assert!(!calls(dir.path()).contains("pr merge"));
    let review = watch.review.clone().unwrap();
    assert_eq!((review.confirmed(), review.findings.len()), (1, 2));
    assert_eq!(review.summary(), "review: 1 confirmed, fixing");

    let waiting = poll_once(&gh, dir.path(), &mut watch, true, 50).await;
    assert!(waiting.prompt.is_none(), "one fix turn per head");
    assert_eq!(
        reviewer.calls.load(Ordering::SeqCst),
        1,
        "one review per head"
    );
    assert!(watch
        .blocked_reason
        .as_deref()
        .unwrap()
        .contains("waiting for the session to push"));

    view(dir.path(), "bbbb2222");
    let after_push = poll_once(&gh, dir.path(), &mut watch, true, 90).await;
    assert!(
        after_push.merged,
        "a clean re-review on the new head merges"
    );
    assert_eq!(watch.state, Phase::Merged);
    assert_eq!(reviewer.calls.load(Ordering::SeqCst), 2);
    assert_eq!(watch.review.unwrap().summary(), "review clean");
    assert!(calls(dir.path()).contains("pr merge 7 --squash --match-head-commit bbbb2222"));
}

#[tokio::test]
async fn auto_merge_is_gated_before_github_is_told_to_merge() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Ok(vec![finding("critical", 3)])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "auto", 0);
    view(dir.path(), "aaaa1111");
    let out = poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    assert!(out.prompt.is_some());
    assert!(
        !calls(dir.path()).contains("--auto"),
        "{}",
        calls(dir.path())
    );
}

#[tokio::test]
async fn a_clean_review_lets_auto_merge_arm() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Ok(vec![])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "auto", 0);
    view(dir.path(), "aaaa1111");
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    assert!(calls(dir.path()).contains("--auto --squash"));
}

#[tokio::test]
async fn review_off_merges_exactly_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let reviewer = install_hook(vec![Ok(vec![finding("critical", 3)])], |s| {
        s.enabled = false
    });
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    let out = poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    assert!(out.merged && watch.review.is_none());
    assert_eq!(reviewer.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn fix_mode_is_never_reviewed() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let reviewer = install_hook(vec![Ok(vec![])], |_| {});
    let mut watch = Watch::new("s1", Mode::Fix, "squash", 0);
    view(dir.path(), "aaaa1111");
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    assert_eq!(reviewer.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_failing_reviewer_blocks_and_retries_later_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let reviewer = install_hook(vec![Err("model timed out".into()), Ok(vec![])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    let first = poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    assert!(!first.merged);
    let why = watch.blocked_reason.clone().unwrap();
    assert!(
        why.contains("model timed out") && why.contains("not merging"),
        "{why}"
    );
    poll_once(&gh, dir.path(), &mut watch, true, 60).await;
    assert_eq!(
        reviewer.calls.load(Ordering::SeqCst),
        1,
        "no retry inside the back-off"
    );
    let later = poll_once(&gh, dir.path(), &mut watch, true, 400).await;
    assert!(later.merged, "the retry found it clean");
    assert_eq!(reviewer.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_error_policy_can_let_the_merge_through() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Err("no model".into())], |s| s.merge_on_error = true);
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    assert!(
        poll_once(&gh, dir.path(), &mut watch, true, 10)
            .await
            .merged
    );
}

#[tokio::test]
async fn findings_that_survive_every_round_leave_the_pr_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Ok(vec![finding("high", 1)])], |s| s.max_rounds = 2);
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    for (i, sha) in ["a1", "b2", "c3"].iter().enumerate() {
        view(dir.path(), sha);
        poll_once(&gh, dir.path(), &mut watch, true, 10 + 40 * i as i64).await;
    }
    assert_eq!(watch.state, Phase::Blocked);
    assert!(watch
        .blocked_reason
        .as_deref()
        .unwrap()
        .contains("unresolved after 2 fix rounds"));
}

#[tokio::test]
async fn a_busy_session_defers_the_fix_turn_and_the_cap_keeps_the_worst() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(
        vec![Ok(vec![
            finding("high", 1),
            finding("critical", 2),
            finding("high", 3),
        ])],
        |s| s.max_findings = 2,
    );
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    let busy = poll_once(&gh, dir.path(), &mut watch, false, 10).await;
    assert!(busy.prompt.is_none());
    assert_eq!(watch.review.as_ref().unwrap().findings.len(), 2);
    assert_eq!(
        watch.review.as_ref().unwrap().findings[0].severity,
        "critical"
    );
    let free = poll_once(&gh, dir.path(), &mut watch, true, 50).await;
    assert!(free.prompt.is_some());
}

#[tokio::test]
async fn blocking_findings_push_once_per_head() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Ok(vec![finding("high", 1)])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    let first = crate::serve_pr_watch::settle_push(&mut watch, false).expect("a push is owed");
    assert!(
        first.message.contains("blocked by review: 1 confirmed"),
        "{}",
        first.message
    );
    assert!(first.high);
    assert!(crate::serve_pr_watch::settle_push(&mut watch, false).is_none());
}

#[test]
fn the_row_text_names_the_state() {
    let mut state = ReviewState {
        head_sha: "a".into(),
        verdict: Verdict::Findings,
        findings: vec![
            ReviewFinding {
                blocking: true,
                ..finding("high", 1)
            };
            2
        ],
        model: None,
        error: None,
        reviewed_at: 0,
        fix_sent: false,
        rounds: 0,
        announced: false,
    };
    assert_eq!(state.summary(), "review: 2 confirmed");
    state.fix_sent = true;
    assert_eq!(state.summary(), "review: 2 confirmed, fixing");
}

#[tokio::test]
async fn the_queue_row_and_the_review_route_carry_the_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Ok(vec![finding("high", 12)])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    watch.queue = Some(crate::serve_merge_queue::QueueInfo::for_test("r @ main"));
    view(dir.path(), "aaaa1111");
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    let queues = crate::serve_merge_queue::snapshot(&[watch.clone()]);
    let json = serde_json::to_value(&queues).unwrap();
    assert_eq!(json[0]["rows"][0]["review"], "review: 1 confirmed, fixing");
    let stored = serde_json::to_value(&watch).unwrap();
    assert_eq!(stored["review"]["findings"][0]["line"], 12);
    assert_eq!(stored["review"]["findings"][0]["blocking"], true);
}
