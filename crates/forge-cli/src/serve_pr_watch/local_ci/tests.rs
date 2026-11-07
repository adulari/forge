use super::*;

fn entry(phase: Stage, head: &str, failures: u32) -> Entry {
    Entry {
        head: head.into(),
        phase,
        failures,
    }
}

#[test]
fn a_watch_with_no_fix_turn_or_no_note_polls_as_usual() {
    assert_eq!(verdict(None, 1, "h1", true), Verdict::Proceed);
    let sent = entry(Stage::Sent, "h0", 0);
    assert_eq!(verdict(Some(&sent), 0, "h1", true), Verdict::Proceed);
}

#[test]
fn only_a_commit_after_the_fix_prompt_is_checked_and_only_when_the_session_is_idle() {
    let sent = entry(Stage::Sent, "h0", 0);
    assert_eq!(verdict(Some(&sent), 1, "h0", true), Verdict::Proceed);
    assert_eq!(verdict(Some(&sent), 1, "h1", false), Verdict::Hold);
    assert_eq!(
        verdict(Some(&sent), 1, "h1", true),
        Verdict::Check("h1".into())
    );
}

#[test]
fn a_running_check_holds_and_a_failed_head_holds_until_the_session_pushes_another() {
    assert_eq!(
        verdict(Some(&entry(Stage::Running, "h1", 0)), 1, "h1", true),
        Verdict::Hold
    );
    let failed = entry(Stage::Failed, "h1", 1);
    assert_eq!(verdict(Some(&failed), 1, "h1", true), Verdict::Hold);
    assert_eq!(
        verdict(Some(&failed), 1, "h2", true),
        Verdict::Check("h2".into())
    );
    let hopeless = entry(Stage::Failed, "h2", MAX_LOCAL_FAILURES);
    assert_eq!(verdict(Some(&hopeless), 1, "h2", true), Verdict::GiveUp);
    assert_eq!(
        verdict(Some(&entry(Stage::Passed, "h1", 0)), 1, "h1", true),
        Verdict::Proceed
    );
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}

fn repo(step: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".github/workflows");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(
        wf.join("ci.yml"),
        format!(
            "on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - name: lint\n        run: {step}\n"
        ),
    )
    .unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    dir
}

fn commit(dir: &Path, name: &str) {
    std::fs::write(dir.join(name), name).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", name]);
}

async fn prompt(rx: &mut mpsc::Receiver<RemoteInput>) -> String {
    match tokio::time::timeout(std::time::Duration::from_secs(60), rx.recv())
        .await
        .expect("a prompt arrives")
    {
        Some(RemoteInput::Prompt { text, .. }) => text,
        other => panic!("expected a prompt, got {other:?}"),
    }
}

async fn settled(session: &str, head: &str) {
    for _ in 0..600 {
        let done = table()
            .lock()
            .unwrap()
            .get(session)
            .is_some_and(|e| e.head == head && e.phase != Stage::Running);
        if done {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the check never settled");
}

#[tokio::test]
async fn a_fix_that_fails_the_plan_goes_back_to_the_session_and_is_never_merged_over() {
    let dir = repo("echo boom-output >&2; exit 3");
    let (tx, mut rx) = mpsc::channel(4);
    note_prompt("ci-fail", dir.path());

    assert!(matches!(
        holds("ci-fail", dir.path(), 1, true, &tx).await,
        Hold::No
    ));
    commit(dir.path(), "fix1");
    assert!(
        matches!(holds("ci-fail", dir.path(), 1, false, &tx).await, Hold::Yes),
        "a busy session is left to finish its turn"
    );
    assert!(matches!(
        holds("ci-fail", dir.path(), 1, true, &tx).await,
        Hold::Yes
    ));
    let text = prompt(&mut rx).await;
    assert!(text.contains("Local CI step failed: lint"), "{text}");
    assert!(text.contains("boom-output") && text.contains("normal `git push`"));
    let head = head_of(dir.path()).unwrap();
    settled("ci-fail", &head).await;
    assert!(
        matches!(holds("ci-fail", dir.path(), 1, true, &tx).await, Hold::Yes),
        "the same failing head holds the merge"
    );

    commit(dir.path(), "fix2");
    assert!(matches!(
        holds("ci-fail", dir.path(), 1, true, &tx).await,
        Hold::Yes
    ));
    prompt(&mut rx).await;
    settled("ci-fail", &head_of(dir.path()).unwrap()).await;
    assert!(matches!(
        holds("ci-fail", dir.path(), 1, true, &tx).await,
        Hold::GiveUp(2)
    ));
}

#[tokio::test]
async fn a_fix_that_passes_the_plan_lets_the_watch_carry_on() {
    let dir = repo("echo fine");
    let (tx, mut rx) = mpsc::channel(4);
    note_prompt("ci-pass", dir.path());
    commit(dir.path(), "fix1");
    assert!(matches!(
        holds("ci-pass", dir.path(), 1, true, &tx).await,
        Hold::Yes
    ));
    settled("ci-pass", &head_of(dir.path()).unwrap()).await;
    assert!(matches!(
        holds("ci-pass", dir.path(), 1, true, &tx).await,
        Hold::No
    ));
    assert!(rx.try_recv().is_err(), "a passing plan sends nothing");
}
