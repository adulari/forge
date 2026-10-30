use std::os::unix::fs::PermissionsExt;

use super::*;

fn fake_gh(dir: &Path) -> Gh {
    let path = dir.join("gh");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> {log}\ncase \"$2\" in\n  view) cat {view} ;;\n  merge) exit 0 ;;\nesac\n",
        log = dir.join("calls.log").display(),
        view = dir.join("view.json").display(),
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    Gh::with_program(path)
}

fn view(dir: &Path, state: &str, checks: &str, merge_state: &str, extra: &str) {
    let body = format!(
        r#"{{"number":7,"url":"https://example.invalid/pr/7","title":"t","state":"{state}","isDraft":false,
        "headRefName":"feat","baseRefName":"main","headRefOid":"abcdef1234567","mergeable":"MERGEABLE",
        "mergeStateStatus":"{merge_state}","reviewDecision":"","statusCheckRollup":{checks},
        "comments":[],"reviews":[]{extra}}}"#
    );
    std::fs::write(dir.join("view.json"), body).unwrap();
}

const FAILING: &str = r#"[{"name":"build","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"https://example.invalid/run/1"}]"#;
const GREEN: &str = r#"[{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}]"#;
const PENDING: &str = r#"[{"name":"build","status":"IN_PROGRESS","conclusion":""}]"#;

fn calls(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default()
}

#[tokio::test]
async fn failing_check_enqueues_one_fix_turn_per_issue_key() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    view(dir.path(), "OPEN", FAILING, "BLOCKED", "");
    let mut watch = Watch::new("s1", Mode::Fix, "squash", 0);

    let first = poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    let prompt = first.prompt.expect("a fix turn is enqueued");
    assert!(prompt.contains("build (failure)"));
    assert!(prompt.contains("Never force-push"));
    assert_eq!(watch.fix_turns, 1);

    let again = poll_once(&gh, dir.path(), &mut watch, true, 50).await;
    assert!(
        again.prompt.is_none(),
        "same state must not burn another turn"
    );
    assert_eq!(watch.fix_turns, 1);
    assert!(watch
        .blocked_reason
        .as_deref()
        .unwrap()
        .contains("waiting for a push"));
}

#[tokio::test]
async fn busy_session_defers_the_fix_without_consuming_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    view(dir.path(), "OPEN", FAILING, "BLOCKED", "");
    let mut watch = Watch::new("s1", Mode::Fix, "squash", 0);
    assert!(poll_once(&gh, dir.path(), &mut watch, false, 10)
        .await
        .prompt
        .is_none());
    assert!(poll_once(&gh, dir.path(), &mut watch, true, 50)
        .await
        .prompt
        .is_some());
}

#[tokio::test]
async fn green_pr_merges_once_and_marks_done() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    view(dir.path(), "OPEN", GREEN, "CLEAN", "");
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    let outcome = poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    assert!(outcome.merged);
    assert_eq!(watch.state, Phase::Merged);
    let log = calls(dir.path());
    assert!(
        log.contains("pr merge 7 --squash --match-head-commit abcdef1234567"),
        "{log}"
    );
    assert!(!log.contains("--force"));
}

#[tokio::test]
async fn fix_mode_never_merges() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    view(dir.path(), "OPEN", GREEN, "CLEAN", "");
    let mut watch = Watch::new("s1", Mode::Fix, "squash", 0);
    let outcome = poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    assert!(!outcome.merged && outcome.prompt.is_none());
    assert!(!calls(dir.path()).contains("pr merge"));
}

#[tokio::test]
async fn pending_checks_and_blocked_merges_wait_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    view(dir.path(), "OPEN", PENDING, "BLOCKED", "");
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    assert!(watch.active() && watch.blocked_reason.is_none());
    view(dir.path(), "OPEN", GREEN, "BLOCKED", "");
    poll_once(&gh, dir.path(), &mut watch, true, 50).await;
    assert!(watch.active());
    assert!(watch
        .blocked_reason
        .as_deref()
        .unwrap()
        .contains("required review"));
    assert!(!calls(dir.path()).contains("pr merge"));
}

#[tokio::test]
async fn auto_method_enables_github_auto_merge_once() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    view(dir.path(), "OPEN", PENDING, "BLOCKED", "");
    let mut watch = Watch::new("s1", Mode::Merge, "auto", 0);
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    poll_once(&gh, dir.path(), &mut watch, true, 50).await;
    assert_eq!(calls(dir.path()).matches("--auto --squash").count(), 1);
    view(dir.path(), "MERGED", GREEN, "CLEAN", "");
    let done = poll_once(&gh, dir.path(), &mut watch, true, 90).await;
    assert!(done.merged && watch.state == Phase::Merged);
}

#[tokio::test]
async fn gives_up_after_the_fix_budget_and_records_why() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let mut watch = Watch::new("s1", Mode::Fix, "squash", 0);
    for round in 0..MAX_FIX_TURNS {
        let checks = FAILING.replace("abc", &format!("x{round}"));
        let body = format!(
            r#"{{"number":7,"url":"u","title":"t","state":"OPEN","isDraft":false,"headRefName":"f","baseRefName":"main","headRefOid":"{:0>8}","mergeable":"MERGEABLE","mergeStateStatus":"BLOCKED","reviewDecision":"","statusCheckRollup":{checks},"comments":[],"reviews":[]}}"#,
            round + 1
        );
        std::fs::write(dir.path().join("view.json"), body).unwrap();
        assert!(
            poll_once(&gh, dir.path(), &mut watch, true, i64::from(round) * 60)
                .await
                .prompt
                .is_some()
        );
    }
    let body = std::fs::read_to_string(dir.path().join("view.json"))
        .unwrap()
        .replace("00000004", "00000009");
    std::fs::write(dir.path().join("view.json"), body).unwrap();
    let last = poll_once(&gh, dir.path(), &mut watch, true, 999).await;
    assert!(last.prompt.is_none());
    assert_eq!(watch.state, Phase::Blocked);
    assert!(watch.blocked_reason.as_deref().unwrap().contains("gave up"));
}

#[tokio::test]
async fn new_comments_trigger_a_turn_but_existing_ones_do_not() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    let mut watch = Watch::new("s1", Mode::Fix, "squash", 0);
    view(dir.path(), "OPEN", GREEN, "CLEAN", "");
    let body = std::fs::read_to_string(dir.path().join("view.json"))
        .unwrap()
        .replace(r#""comments":[]"#, r#""comments":[{"id":"C1"}]"#);
    std::fs::write(dir.path().join("view.json"), &body).unwrap();
    assert!(poll_once(&gh, dir.path(), &mut watch, true, 10)
        .await
        .prompt
        .is_none());
    std::fs::write(
        dir.path().join("view.json"),
        body.replace(r#"[{"id":"C1"}]"#, r#"[{"id":"C1"},{"id":"C2"}]"#),
    )
    .unwrap();
    let outcome = poll_once(&gh, dir.path(), &mut watch, true, 50).await;
    assert!(outcome.prompt.unwrap().contains("1 new comment"));
}

#[tokio::test]
async fn closed_pr_and_missing_gh_are_recorded_not_looped() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path());
    view(dir.path(), "CLOSED", GREEN, "CLEAN", "");
    let mut watch = Watch::new("s1", Mode::Merge, "squash", 0);
    poll_once(&gh, dir.path(), &mut watch, true, 10).await;
    assert_eq!(watch.state, Phase::Blocked);

    let mut other = Watch::new("s2", Mode::Fix, "squash", 0);
    let missing = Gh::with_program(dir.path().join("nope"));
    poll_once(&missing, dir.path(), &mut other, true, 10).await;
    assert!(other.active());
    assert!(other.blocked_reason.unwrap().contains("not installed"));
}

#[test]
fn force_push_prompts_are_recognised() {
    assert!(is_force_push("shell: git push --force origin feat"));
    assert!(is_force_push("git push -f"));
    assert!(is_force_push("git push origin +feat"));
    assert!(is_force_push("git push --force-with-lease"));
    assert!(!is_force_push("git push origin feat"));
    assert!(!is_force_push("rm -rf build"));
}

#[test]
fn registry_persists_and_off_removes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.json");
    let reg = Registry::new(Some(path.clone()));
    reg.arm("s1", Mode::Merge, "rebase", 5);
    let again = Registry::new(Some(path));
    assert_eq!(again.get("s1").unwrap().method, "rebase");
    again.arm("s1", Mode::Off, "squash", 6);
    assert!(again.get("s1").is_none());
}

#[test]
fn auto_profile_pre_authorises_a_plain_push_and_flags_a_forced_one() {
    use forge_core::permission::{auto_risk, AutoVerdict};
    use forge_types::SideEffect;
    let verdict = |cmd: &str| {
        auto_risk(
            SideEffect::Shell,
            "shell",
            &serde_json::json!({ "command": cmd }),
            None,
        )
    };
    assert_eq!(verdict("git push origin feat/demo"), AutoVerdict::Safe);
    assert!(matches!(
        verdict("git push --force origin feat/demo"),
        AutoVerdict::Risky(_)
    ));
    assert!(matches!(
        verdict("git push origin +feat/demo"),
        AutoVerdict::Risky(_)
    ));
}
