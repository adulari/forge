use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::*;
use crate::serve_pr_watch::{poll_once, Mode, Outcome, Phase};

/// One poll, and when it started a background review, wait for that review and poll again.
async fn poll(gh: &Gh, root: &Path, watch: &mut Watch, free: bool, now: i64) -> Outcome {
    let first = poll_once(gh, root, watch, free, now).await;
    if !watch
        .review
        .as_ref()
        .is_some_and(|r| r.verdict == Verdict::Running)
    {
        return first;
    }
    settle().await;
    poll_once(gh, root, watch, free, now).await
}

async fn settle() {
    for _ in 0..500 {
        if jobs().running() == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("the background review never finished");
}

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
        dismissed: None,
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
    let first = poll(&gh, dir.path(), &mut watch, true, 10).await;
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

    let waiting = poll(&gh, dir.path(), &mut watch, true, 50).await;
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
    let after_push = poll(&gh, dir.path(), &mut watch, true, 90).await;
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
    let out = poll(&gh, dir.path(), &mut watch, true, 10).await;
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
    poll(&gh, dir.path(), &mut watch, true, 10).await;
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
    let out = poll(&gh, dir.path(), &mut watch, true, 10).await;
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
    poll(&gh, dir.path(), &mut watch, true, 10).await;
    assert_eq!(reviewer.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_failing_reviewer_blocks_and_retries_later_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let reviewer = install_hook(vec![Err("model timed out".into()), Ok(vec![])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    let first = poll(&gh, dir.path(), &mut watch, true, 10).await;
    assert!(!first.merged);
    let why = watch.blocked_reason.clone().unwrap();
    assert!(
        why.contains("model timed out") && why.contains("not merging"),
        "{why}"
    );
    poll(&gh, dir.path(), &mut watch, true, 60).await;
    assert_eq!(
        reviewer.calls.load(Ordering::SeqCst),
        1,
        "no retry inside the back-off"
    );
    let later = poll(&gh, dir.path(), &mut watch, true, 400).await;
    assert!(later.merged, "the retry found it clean");
    assert_eq!(reviewer.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_review_that_ran_into_its_timeout_is_not_restarted_when_its_result_arrives() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let reviewer = install_hook(vec![Err("review timed out after 600s".into())], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    settle().await;
    poll_once(&gh, dir.path(), &mut watch, true, 700).await;
    poll_once(&gh, dir.path(), &mut watch, true, 730).await;
    settle().await;
    assert_eq!(
        reviewer.calls.load(Ordering::SeqCst),
        1,
        "the back-off counts from the failure, not from the start"
    );
    poll(&gh, dir.path(), &mut watch, true, 1000).await;
    assert_eq!(reviewer.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_reviewer_that_keeps_failing_stops_spending_and_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let reviewer = install_hook(vec![Err("model timed out".into())], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    for now in [10, 400, 800, 1200, 1600] {
        poll(&gh, dir.path(), &mut watch, true, now).await;
    }
    assert_eq!(reviewer.calls.load(Ordering::SeqCst), MAX_ERRORS as usize);
    assert_eq!(watch.state, Phase::Blocked);
    let why = watch.blocked_reason.clone().unwrap();
    assert!(
        why.contains("review failed 3 times") && why.contains("model timed out"),
        "{why}"
    );
    assert!(!calls(dir.path()).contains("pr merge"));

    view(dir.path(), "bbbb2222");
    let mut rearmed = Watch::new("s1", Mode::Merge, "squash", 0);
    rearmed.review = watch.review.clone();
    poll(&gh, dir.path(), &mut rearmed, true, 2000).await;
    assert_eq!(
        reviewer.calls.load(Ordering::SeqCst),
        MAX_ERRORS as usize + 1,
        "a new head gets a fresh set of tries"
    );
}

#[tokio::test]
async fn the_error_policy_can_let_the_merge_through() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Err("no model".into())], |s| s.merge_on_error = true);
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    assert!(poll(&gh, dir.path(), &mut watch, true, 10).await.merged);
}

#[tokio::test]
async fn findings_that_survive_every_round_leave_the_pr_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Ok(vec![finding("high", 1)])], |s| s.max_rounds = 2);
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    for (i, sha) in ["a1", "b2", "c3"].iter().enumerate() {
        view(dir.path(), sha);
        poll(&gh, dir.path(), &mut watch, true, 10 + 40 * i as i64).await;
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
    let busy = poll(&gh, dir.path(), &mut watch, false, 10).await;
    assert!(busy.prompt.is_none());
    assert_eq!(watch.review.as_ref().unwrap().findings.len(), 2);
    assert_eq!(
        watch.review.as_ref().unwrap().findings[0].severity,
        "critical"
    );
    let free = poll(&gh, dir.path(), &mut watch, true, 50).await;
    assert!(free.prompt.is_some());
}

#[tokio::test]
async fn blocking_findings_push_once_per_head() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Ok(vec![finding("high", 1)])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    poll(&gh, dir.path(), &mut watch, true, 10).await;
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
        merge_anyway: None,
        errors: 0,
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
    poll(&gh, dir.path(), &mut watch, true, 10).await;
    let queues = crate::serve_merge_queue::snapshot(&[watch.clone()]);
    let json = serde_json::to_value(&queues).unwrap();
    assert_eq!(json[0]["rows"][0]["review"], "review: 1 confirmed, fixing");
    let stored = serde_json::to_value(&watch).unwrap();
    assert_eq!(stored["review"]["findings"][0]["line"], 12);
    assert_eq!(stored["review"]["findings"][0]["blocking"], true);
}

/// A reviewer that holds each review until the test lets it go, and counts how many are inside
/// the model call at once.
struct Held {
    started: AtomicUsize,
    inside: AtomicUsize,
    peak: AtomicUsize,
    release: tokio::sync::Semaphore,
}

impl Held {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            started: AtomicUsize::new(0),
            inside: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            release: tokio::sync::Semaphore::new(0),
        })
    }
}

#[async_trait::async_trait]
impl Reviewer for Held {
    async fn review(&self, _: &ReviewRequest<'_>) -> Result<Reviewed, String> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let now = self.inside.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        self.release.acquire().await.unwrap().forget();
        self.inside.fetch_sub(1, Ordering::SeqCst);
        Ok(Reviewed {
            findings: vec![],
            model: None,
        })
    }
}

fn install_held(held: &Arc<Held>) {
    let settings = Settings {
        enabled: true,
        max_findings: 5,
        threshold: forge_types::Severity::High,
        merge_on_error: false,
        max_rounds: 3,
    };
    TEST_HOOK.with(|h| *h.borrow_mut() = Some((held.clone(), settings)));
}

async fn pause() {
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
}

#[tokio::test]
async fn a_slow_review_runs_in_the_background_and_the_poll_returns_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let held = Held::new();
    install_held(&held);
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");

    let first = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        poll_once(&gh, dir.path(), &mut watch, true, 10),
    )
    .await
    .expect("an inline review would hang the tick until the reviewer answered");
    assert!(!first.merged);
    assert_eq!(watch.blocked_reason.as_deref(), Some("review running"));
    assert_eq!(watch.review.as_ref().unwrap().summary(), "review running");
    pause().await;
    assert_eq!(held.started.load(Ordering::SeqCst), 1);

    poll_once(&gh, dir.path(), &mut watch, true, 40).await;
    pause().await;
    assert_eq!(
        held.started.load(Ordering::SeqCst),
        1,
        "a running review is not started twice"
    );
    assert!(!calls(dir.path()).contains("pr merge"));

    held.release.add_permits(1);
    settle().await;
    let done = poll_once(&gh, dir.path(), &mut watch, true, 70).await;
    assert!(done.merged, "the decision is re-evaluated with the result");
}

#[tokio::test]
async fn other_watches_keep_polling_while_a_review_runs() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let held = Held::new();
    install_held(&held);
    view(dir.path(), "aaaa1111");
    let mut slow = Watch::new("slow", Mode::Merge, "squash", 0);
    poll_once(&gh, dir.path(), &mut slow, true, 10).await;
    pause().await;
    assert_eq!(held.inside.load(Ordering::SeqCst), 1);

    let mut other = Watch::new("fixer", Mode::Fix, "squash", 0);
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        poll_once(&gh, dir.path(), &mut other, true, 11),
    )
    .await
    .expect("another watch's poll waited for the review");
    assert!(
        out.green,
        "the fix-mode watch was polled while the review ran"
    );
    held.release.add_permits(1);
    settle().await;
}

#[tokio::test]
async fn at_most_two_reviews_call_a_model_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let held = Held::new();
    install_held(&held);
    view(dir.path(), "aaaa1111");
    let mut watches: Vec<Watch> = ["w1", "w2", "w3"]
        .iter()
        .map(|id| Watch::new(id, Mode::Merge, "squash", 0))
        .collect();
    for w in &mut watches {
        poll_once(&gh, dir.path(), w, true, 10).await;
    }
    pause().await;
    assert_eq!(held.started.load(Ordering::SeqCst), 2, "the third queues");
    assert!(watches
        .iter()
        .all(|w| w.review.as_ref().unwrap().verdict == Verdict::Running));
    held.release.add_permits(3);
    settle().await;
    assert_eq!(held.started.load(Ordering::SeqCst), 3);
    assert_eq!(held.peak.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_review_lost_in_a_restart_starts_again_on_the_next_tick() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let reviewer = install_hook(vec![Ok(vec![])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    watch.review = Some(ReviewState::started("aaaa1111", 0, 5));
    assert_eq!(jobs().running(), 0, "the new daemon has no job for it");
    view(dir.path(), "aaaa1111");
    let out = poll(&gh, dir.path(), &mut watch, true, 10).await;
    assert_eq!(reviewer.calls.load(Ordering::SeqCst), 1);
    assert!(out.merged);
}

#[tokio::test]
async fn a_push_during_a_review_reviews_the_new_head_and_drops_the_old_result() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let held = Held::new();
    install_held(&held);
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    view(dir.path(), "bbbb2222");
    poll_once(&gh, dir.path(), &mut watch, true, 20).await;
    assert_eq!(watch.review.as_ref().unwrap().head_sha, "bbbb2222");
    held.release.add_permits(2);
    settle().await;
    let out = poll_once(&gh, dir.path(), &mut watch, true, 30).await;
    assert!(out.merged);
    assert!(calls(dir.path()).contains("--match-head-commit bbbb2222"));
}

#[tokio::test]
async fn a_superseded_head_s_review_is_aborted_and_gives_its_slot_back() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let held = Held::new();
    install_held(&held);
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    for (sha, now) in [("aaaa1111", 10), ("bbbb2222", 20), ("cccc3333", 30)] {
        view(dir.path(), sha);
        poll_once(&gh, dir.path(), &mut watch, true, now).await;
        pause().await;
    }
    assert_eq!(
        jobs().running(),
        1,
        "only the current head is still reviewed"
    );
    assert_eq!(
        held.started.load(Ordering::SeqCst),
        3,
        "the newest head's review got a permit the superseded ones gave back"
    );
    held.release.add_permits(1);
    settle().await;
    let out = poll_once(&gh, dir.path(), &mut watch, true, 40).await;
    assert!(out.merged);
    assert!(calls(dir.path()).contains("--match-head-commit cccc3333"));
}

#[tokio::test]
async fn a_relay_mid_review_hands_the_running_review_to_the_successor() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let held = Held::new();
    install_held(&held);
    let (from, to) = ("review-relay-pred", "review-relay-succ");
    let mut watch = registry().arm(from, Mode::Merge, "squash", 0).unwrap();
    view(dir.path(), "aaaa1111");
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    pause().await;
    assert_eq!(held.started.load(Ordering::SeqCst), 1);
    registry().store(from, watch);
    assert!(registry().rekey(from, to));
    let mut watch = registry().get(to).expect("the watch followed the goal");
    assert_eq!(watch.review.as_ref().unwrap().verdict, Verdict::Running);

    poll_once(&gh, dir.path(), &mut watch, true, 20).await;
    pause().await;
    assert_eq!(
        held.started.load(Ordering::SeqCst),
        1,
        "the successor reads the predecessor's review instead of paying for a second one"
    );
    held.release.add_permits(1);
    settle().await;
    let out = poll_once(&gh, dir.path(), &mut watch, true, 30).await;
    assert!(
        out.merged,
        "the moved review's result is read under the successor"
    );
    assert_eq!(jobs().running(), 0);
    registry().disarm(to);
}

fn act(
    watch: &mut Watch,
    action: owner::ReviewAction,
    now: i64,
) -> Result<String, crate::serve_merge_queue::ActionError> {
    let mut all = std::collections::HashMap::from([(watch.session_id.clone(), watch.clone())]);
    let result = owner::apply(
        &mut all,
        &watch.session_id,
        None,
        action,
        "floris (cli)",
        now,
    );
    *watch = all.remove(&watch.session_id).unwrap();
    result
}

#[tokio::test]
async fn dismissing_findings_one_by_one_lets_the_merge_through_and_records_who() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(
        vec![Ok(vec![finding("critical", 2), finding("high", 9)])],
        |_| {},
    );
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    poll(&gh, dir.path(), &mut watch, false, 10).await;
    assert_eq!(watch.review.as_ref().unwrap().confirmed(), 2);

    let said = act(&mut watch, owner::ReviewAction::Dismiss(Some(1)), 20).unwrap();
    assert!(said.contains("dismissed finding 1"), "{said}");
    let review = watch.review.as_ref().unwrap();
    let done = review.findings[0].dismissed.as_ref().unwrap();
    assert_eq!((done.by.as_str(), done.at), ("floris (cli)", 20));
    assert!(!review.findings[0].blocking && review.findings[1].blocking);
    assert!(!poll(&gh, dir.path(), &mut watch, false, 30).await.merged);

    act(&mut watch, owner::ReviewAction::Dismiss(None), 40).unwrap();
    assert_eq!(watch.review.as_ref().unwrap().confirmed(), 0);
    assert!(poll(&gh, dir.path(), &mut watch, false, 50).await.merged);
    assert_eq!(
        watch.review.unwrap().summary(),
        "review clean (2 dismissed)"
    );
}

#[tokio::test]
async fn a_dismissal_is_for_one_head_and_the_next_head_is_reviewed_from_scratch() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let reviewer = install_hook(vec![Ok(vec![finding("high", 2)])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    poll(&gh, dir.path(), &mut watch, false, 10).await;
    act(&mut watch, owner::ReviewAction::Dismiss(Some(1)), 20).unwrap();

    view(dir.path(), "bbbb2222");
    let out = poll(&gh, dir.path(), &mut watch, true, 60).await;
    assert!(
        !out.merged,
        "the dismissal did not follow the PR to a new head"
    );
    assert_eq!(reviewer.calls.load(Ordering::SeqCst), 2);
    let review = watch.review.unwrap();
    assert_eq!(review.head_sha, "bbbb2222");
    assert!(review.findings[0].dismissed.is_none() && review.findings[0].blocking);
}

#[tokio::test]
async fn merge_anyway_merges_the_current_head_once_and_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let reviewer = install_hook(vec![Ok(vec![finding("critical", 2)])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    poll(&gh, dir.path(), &mut watch, true, 10).await;
    assert!(!calls(dir.path()).contains("pr merge"));

    let said = act(&mut watch, owner::ReviewAction::MergeAnyway, 20).unwrap();
    assert!(said.contains("will merge without waiting"), "{said}");
    let mark = watch.review.as_ref().unwrap().merge_anyway.clone().unwrap();
    assert_eq!((mark.by.as_str(), mark.at), ("floris (cli)", 20));
    assert_eq!(
        watch.review.as_ref().unwrap().summary(),
        "review overridden by floris (cli)"
    );
    assert!(poll(&gh, dir.path(), &mut watch, true, 30).await.merged);
    assert_eq!(reviewer.calls.load(Ordering::SeqCst), 1);
    assert!(calls(dir.path()).contains("pr merge 7 --squash --match-head-commit aaaa1111"));
}

#[tokio::test]
async fn a_review_action_naming_another_pr_than_the_sessions_watch_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Ok(vec![finding("critical", 2)])], |_| {});
    let mut watch = Watch::new("s_b", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    poll(&gh, dir.path(), &mut watch, false, 10).await;
    assert_eq!(watch.pr_number, Some(7));
    let reg = crate::serve_pr_watch::Registry::new(None);
    reg.lock().insert(watch.session_id.clone(), watch);

    let wrong = reg
        .review_action(
            Some("s_b"),
            Some(10),
            owner::ReviewAction::MergeAnyway,
            "app",
            20,
        )
        .unwrap_err();
    assert!(format!("{wrong:?}").contains("not PR #10"), "{wrong:?}");
    reg.review_action(
        Some("s_b"),
        Some(10),
        owner::ReviewAction::Dismiss(Some(1)),
        "app",
        21,
    )
    .unwrap_err();
    let held = reg.lock()["s_b"].review.clone().unwrap();
    assert!(held.merge_anyway.is_none() && held.findings[0].blocking);

    reg.review_action(
        Some("s_b"),
        Some(7),
        owner::ReviewAction::MergeAnyway,
        "app",
        22,
    )
    .unwrap();
    assert!(reg.lock()["s_b"]
        .review
        .as_ref()
        .unwrap()
        .merge_anyway
        .is_some());
}

#[tokio::test]
async fn merge_anyway_does_not_carry_to_a_new_head() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Ok(vec![finding("critical", 2)])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    poll(&gh, dir.path(), &mut watch, false, 10).await;
    act(&mut watch, owner::ReviewAction::MergeAnyway, 20).unwrap();
    view(dir.path(), "bbbb2222");
    let out = poll(&gh, dir.path(), &mut watch, false, 60).await;
    assert!(!out.merged);
    assert!(watch.review.unwrap().merge_anyway.is_none());
}

#[tokio::test]
async fn merge_anyway_also_overrides_a_reviewer_that_failed() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Err("no model".into())], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "aaaa1111");
    assert!(!poll(&gh, dir.path(), &mut watch, true, 10).await.merged);
    act(&mut watch, owner::ReviewAction::MergeAnyway, 20).unwrap();
    assert!(poll(&gh, dir.path(), &mut watch, true, 30).await.merged);
}

#[tokio::test]
async fn owner_actions_are_refused_with_a_reason_when_they_do_not_apply() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    install_hook(vec![Ok(vec![])], |_| {});
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    let none = act(&mut watch, owner::ReviewAction::MergeAnyway, 1).unwrap_err();
    assert!(format!("{none:?}").contains("no review"), "{none:?}");
    view(dir.path(), "aaaa1111");
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    let running = act(&mut watch, owner::ReviewAction::Dismiss(None), 11).unwrap_err();
    assert!(
        format!("{running:?}").contains("still running"),
        "{running:?}"
    );
    settle().await;
    poll_once(&gh, dir.path(), &mut watch, true, 12).await;
}

#[test]
fn an_override_made_while_a_poll_ran_survives_the_poll_being_stored() {
    let mut live = Watch::new("s1", Mode::Merge, "squash", 0);
    let mut next = live.clone();
    let mut review = ReviewState::started("aaaa1111", 0, 1);
    review.verdict = Verdict::Findings;
    review.findings = vec![ReviewFinding {
        blocking: true,
        ..finding("high", 1)
    }];
    next.review = Some(review.clone());
    let mut owned = review;
    owned.merge_anyway = Some(Owner {
        by: "tui".into(),
        at: 5,
    });
    owned.findings[0].dismissed = Some(Owner {
        by: "tui".into(),
        at: 6,
    });
    owned.findings[0].blocking = false;
    live.review = Some(owned);
    carry_owner_edits(&live, &mut next);
    let kept = next.review.unwrap();
    assert!(kept.merge_anyway.is_some() && kept.findings[0].dismissed.is_some());
    assert!(!kept.findings[0].blocking);
}
