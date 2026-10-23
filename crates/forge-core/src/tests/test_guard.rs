//! The existing-tests-are-spec guard: which requests arm it, what it counts as a rewritten test,
//! and that a test edit the model already committed is still caught and restored.

use super::*;
use crate::test_guard::{
    guidance, level, modified_existing_tests, parse_name_status, prompt_asks_to_change_tests,
    prompt_expects_green_existing_tests, prompt_requires_pristine_existing_tests, push_back,
    restore_originals, sentences, session_requires_pristine_existing_tests, Level,
    KEEP_TESTS_GUIDANCE, PRISTINE_TEST_GUIDANCE,
};

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repo whose HEAD holds `tests/test_foo.py` (two tests) and `src/lib.py`.
fn repo_with_committed_test() -> std::path::PathBuf {
    let dir = clean_git_repo();
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("tests/test_foo.py"),
        "def test_a():\n    assert fix() == 1\n\ndef test_b():\n    assert other() == 2\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.py"), "def fix():\n    return 0\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "add tests"]);
    dir
}

#[test]
fn the_bench_prompts_that_name_green_tests_arm_the_rewrite_guard() {
    for prompt in [
        // a02
        "Add tests for it under `tests/`. The existing tests must keep passing.",
        // b02
        "Can you fix it so `Get` never returns an expired entry? Keep the existing tests green.",
        // b04: the `./...` must not split the sentence between the command and "green"
        "Can you make `batch.Run` do what README.md says, and keep `go test -race ./...` green? \
         Please don't change its signature, other packages call it.",
        // b05
        "Can you get main building again and `cargo test` green? CI builds offline.",
        // b01
        "Update the README, add tests for the new behaviour, and make sure `cargo test` and \
         `cargo fmt --check` both pass.",
        "The test suite has to stay green.",
        "keep the tests passing",
    ] {
        assert!(prompt_expects_green_existing_tests(prompt), "{prompt}");
    }
}

#[test]
fn requests_that_expect_tests_to_change_do_not_arm_it() {
    for prompt in [
        // b03: the tests are renamed along with the code
        "Please rename the `User` concept to `Account`. Everything should still build and `go \
         test ./...` should pass.",
        "Refactor the pricing module and keep the tests green.",
        "Update the tests to match the new output and make sure they pass.",
        "Fix the failing test so that it passes.",
        "The db keyword is deprecated; use database instead. Tests should pass.",
        "Implement pagination.",
        "Add a --verbose flag to the CLI.",
        "Review the latest changes and attest to them.",
    ] {
        assert!(!prompt_expects_green_existing_tests(prompt), "{prompt}");
    }
}

/// A review round is not a promise that the tests stay green. A last-look prompt with "pass" as a
/// noun used to arm the guard, which restored a test file the model had repaired and cost a second
/// full round.
#[test]
fn a_pass_over_the_work_is_not_a_green_tests_request() {
    for prompt in [
        "Final pass: rerun everything and confirm each change is covered by a test of its own.",
        "Do one more pass over the tests and tell me what is missing.",
        "Take a quick pass through the test directory.",
        "A second pass on the tests, please, with notes per file.",
        "Two passes over the unit tests would be enough.",
        "Review pass: confirm each change is covered by a test.",
    ] {
        assert!(!prompt_expects_green_existing_tests(prompt), "{prompt}");
        assert_eq!(guidance(prompt, &[]), None, "{prompt}");
        assert_eq!(level(false, &[Message::user(prompt)], false), Level::Off);
    }
}

#[test]
fn the_verb_pass_still_asks_for_green_tests() {
    for prompt in [
        "Make the tests pass.",
        "All the tests must pass when you are done.",
        "The tests that pass today have to keep passing.",
        "Every test should still pass after the change.",
        "Run the suite once more; it passes right now and must stay that way, tests included.",
        "Final pass: the tests must keep passing.",
    ] {
        assert!(prompt_expects_green_existing_tests(prompt), "{prompt}");
    }
}

#[test]
fn leave_the_existing_tests_alone_is_an_explicit_prohibition() {
    for prompt in [
        "Fix the parser. Leave the existing tests alone.",
        "Fix the parser and leave existing tests untouched.",
        "Fix the parser without changing the tests.",
        "Fix the parser, existing tests as is.",
        "Don't edit the existing tests.",
    ] {
        assert!(prompt_requires_pristine_existing_tests(prompt), "{prompt}");
    }
    for prompt in [
        "The existing tests must keep passing.",
        "Fix the parser without changing its public API.",
        "Leave the logging alone and update the tests.",
    ] {
        assert!(!prompt_requires_pristine_existing_tests(prompt), "{prompt}");
    }
}

/// The bench task a05, verbatim. "Don't touch `package.json`, `tsconfig.json` or `test/helpers.ts`"
/// read as a ban on the tests (a word `test` after a "don't touch"), so Forge told the model the
/// tracked tests were immutable while the same request said to update them; the model obeyed Forge,
/// left `npm test` red and said so.
const MAILER_REQUEST: &str = "`sendMail(to, subject, body, isHtml, retries, cc)` has too many \
positional arguments. Please change it to take a single options object instead:\n\n    \
sendMail({ to, subject, body, html?, retries?, cc? })\n\n`isHtml` becomes `html`; the defaults \
stay exactly as they are today. There should be no compatibility overload for the old positional \
form: I want old-style calls to be a compile error.\n\nUpdate every caller, the tests, the README \
and `docs/`, and anything else in the repo that sends mail. `npm run typecheck` and `npm test` \
should both be clean at the end (`tsc` is on the PATH). Don't touch `package.json`, \
`tsconfig.json` or `test/helpers.ts`, and keep the existing tests (updated for the new signature, \
not deleted or skipped).";

#[test]
fn a_request_to_update_the_tests_does_not_make_them_immutable() {
    assert!(prompt_asks_to_change_tests(MAILER_REQUEST));
    assert!(!prompt_requires_pristine_existing_tests(MAILER_REQUEST));
    assert_eq!(guidance(MAILER_REQUEST, &[]), None);
    assert_eq!(
        level(false, &[Message::user(MAILER_REQUEST)], false),
        Level::Off
    );
    assert_eq!(
        level(false, &[Message::user(MAILER_REQUEST)], true),
        Level::Off
    );
}

#[test]
fn a_file_under_a_test_folder_is_not_the_tests() {
    for prompt in [
        "Don't touch `test/helpers.ts`.",
        "Do not modify package.json or test/helpers.ts.",
        "Never edit `tests/fixtures/big.json` or `tsconfig.json`.",
        "Don't change the schema, and keep the existing tests green.",
        "Don't modify the public API (the tests exercise it).",
    ] {
        assert!(!prompt_requires_pristine_existing_tests(prompt), "{prompt}");
    }
    for prompt in [
        "Don't touch the tests.",
        "Never edit the existing tests or the fixtures.",
        "Do not modify package.json or the tests.",
        "Don't change `tsconfig.json`, and don't modify the unit-tests either.",
    ] {
        assert!(prompt_requires_pristine_existing_tests(prompt), "{prompt}");
    }
}

#[test]
fn only_a_change_verb_aimed_at_the_tests_asks_to_change_them() {
    for prompt in [
        "Update every caller, the tests, the README and docs.",
        "Adjust the tests to match the new output.",
        "Fix the failing test.",
        "Rename it everywhere, including the tests.",
        "Keep the existing tests (updated for the new signature, not deleted or skipped).",
        "Migrate the call sites and the tests to the new API.",
    ] {
        assert!(prompt_asks_to_change_tests(prompt), "{prompt}");
    }
    for prompt in [
        "Update the README, add tests for the new flag.",
        "Fix the parser and leave existing tests untouched.",
        "Fix the parser, existing tests as is.",
        "Don't update the tests.",
        "Fix the parser without changing the tests.",
        "Add tests for the parser.",
        "Make sure the tests pass.",
    ] {
        assert!(!prompt_asks_to_change_tests(prompt), "{prompt}");
    }
}

#[test]
fn the_latest_request_about_the_tests_decides_whether_they_are_protected() {
    let protected = vec![
        Message::user("Fix the parser. Don't edit the existing tests."),
        Message::assistant("ok"),
        Message::user("Now handle the empty case."),
    ];
    assert!(session_requires_pristine_existing_tests(&protected));
    assert!(guidance("Now handle the empty case.", &protected).is_some());

    let lifted = vec![
        Message::user("Fix the parser. Don't edit the existing tests."),
        Message::assistant("ok"),
        Message::user("Now rename the helper and update the tests to match."),
    ];
    assert!(!session_requires_pristine_existing_tests(&lifted));
    assert_eq!(
        guidance(
            "Now rename the helper and update the tests to match.",
            &protected
        ),
        None,
        "the request being sent lifts the earlier ban before it is in the transcript"
    );
    assert_eq!(level(false, &lifted, true), Level::Off);

    let restored = [
        lifted.clone(),
        vec![Message::user(
            "Leave the existing tests alone from here on.",
        )],
    ]
    .concat();
    assert!(session_requires_pristine_existing_tests(&restored));
}

#[test]
fn a_dot_inside_a_token_does_not_end_a_sentence() {
    assert_eq!(
        sentences("Keep `go test ./...` green. Done"),
        vec!["keep `go test ./...` green", " done"]
    );
    assert_eq!(
        sentences("edit main.rs\nthen stop"),
        vec!["edit main.rs", "then stop"]
    );
}

#[test]
fn the_level_follows_the_strongest_signal() {
    let user = |text: &str| vec![Message::user(text)];
    assert_eq!(level(true, &user("hi"), false), Level::AnyEdit);
    assert_eq!(
        level(false, &user("Don't edit the existing tests."), true),
        Level::AnyEdit
    );
    assert_eq!(
        level(false, &user("Keep the existing tests green."), false),
        Level::Rewrites
    );
    assert_eq!(
        level(false, &user("Keep the existing tests green."), true),
        Level::Off,
        "a person in a chat is not given an extra restore round for saying the tests must pass"
    );
    assert_eq!(
        level(false, &user("Implement pagination."), false),
        Level::Off
    );
    // A later turn that says nothing about tests keeps the session's protection.
    let sticky = vec![
        Message::user("Keep the existing tests green."),
        Message::assistant("ok"),
        Message::user("Now handle the empty case."),
    ];
    assert_eq!(level(false, &sticky, false), Level::Rewrites);
}

#[test]
fn guidance_matches_the_strength_of_the_request() {
    assert_eq!(
        guidance("Don't edit the existing tests.", &[]).map(|g| g.1),
        Some(PRISTINE_TEST_GUIDANCE)
    );
    assert_eq!(
        guidance("Keep the existing tests green.", &[]).map(|g| g.1),
        Some(KEEP_TESTS_GUIDANCE)
    );
    assert_eq!(guidance("Implement pagination.", &[]), None);
}

#[test]
fn a_harness_nudge_does_not_speak_for_the_person() {
    let nudge = |text: String| Message::user(text).with_kind(forge_types::MessageKind::Nudge);
    // The push-back after a restore says "fix the production code rather than the tests".
    let pushed_back = push_back(
        false,
        &["tests/test_a.py".to_string()],
        std::path::Path::new("/tmp/edits.patch"),
    );
    let transcript = vec![
        Message::user("Fix the parser. Leave the existing tests alone."),
        Message::assistant("ok"),
        nudge(pushed_back),
    ];
    assert!(session_requires_pristine_existing_tests(&transcript));
    assert_eq!(level(false, &transcript, true), Level::AnyEdit);

    // A nudge that talks about passing tests does not arm the keep-it-green guard.
    let transcript = vec![
        Message::user("Implement pagination."),
        Message::assistant("done"),
        nudge("Run the tests again and make sure they pass.".to_string()),
    ];
    assert_eq!(level(false, &transcript, false), Level::Off);
    assert_eq!(guidance("Implement pagination.", &transcript), None);

    // The same words typed by the person still count.
    let transcript = vec![Message::user(
        "Run the tests again and make sure they pass.",
    )];
    assert_eq!(level(false, &transcript, false), Level::Rewrites);
}

#[test]
fn name_status_lists_every_changed_path_and_splits_renames() {
    let raw = b"M\0tests/test_a.py\0D\0tests/test_old.py\0A\0tests/test_new.py\0M\0src/lib.py\0";
    assert_eq!(
        parse_name_status(raw),
        vec![
            ('M', "tests/test_a.py".to_string()),
            ('D', "tests/test_old.py".to_string()),
            ('A', "tests/test_new.py".to_string()),
            ('M', "src/lib.py".to_string()),
        ]
    );
}

#[test]
fn an_appended_test_is_fine_under_rewrites_but_not_under_any_edit() {
    let dir = repo_with_committed_test();
    let mut text = std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap();
    text.push_str("\ndef test_c():\n    assert fix() == 3\n");
    std::fs::write(dir.join("tests/test_foo.py"), text).unwrap();
    assert!(modified_existing_tests(Some(&dir), None, Level::Rewrites).is_empty());
    assert_eq!(
        modified_existing_tests(Some(&dir), None, Level::AnyEdit),
        vec!["tests/test_foo.py".to_string()]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_test_inserted_before_existing_ones_or_a_changed_line_is_a_rewrite() {
    let dir = repo_with_committed_test();
    let original = std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap();
    // b02's shape: a new test slipped in ahead of an existing one.
    let inserted = original.replace(
        "def test_b():",
        "def test_new():\n    assert fix() == 9\n\ndef test_b():",
    );
    std::fs::write(dir.join("tests/test_foo.py"), inserted).unwrap();
    assert_eq!(
        modified_existing_tests(Some(&dir), None, Level::Rewrites),
        vec!["tests/test_foo.py".to_string()]
    );
    // a02's shape: an existing import line widened.
    std::fs::write(
        dir.join("tests/test_foo.py"),
        original.replace("== 1", "== 2"),
    )
    .unwrap();
    assert_eq!(
        modified_existing_tests(Some(&dir), None, Level::Rewrites),
        vec!["tests/test_foo.py".to_string()]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleted_and_moved_tests_count_while_new_files_and_sources_do_not() {
    let dir = repo_with_committed_test();
    std::fs::rename(
        dir.join("tests/test_foo.py"),
        dir.join("tests/test_moved.py"),
    )
    .unwrap();
    std::fs::write(dir.join("tests/test_new.py"), "def test_n():\n    pass\n").unwrap();
    std::fs::write(dir.join("src/lib.py"), "def fix():\n    return 1\n").unwrap();
    git(&dir, &["add", "-A"]);
    assert_eq!(
        modified_existing_tests(Some(&dir), None, Level::Rewrites),
        vec!["tests/test_foo.py".to_string()],
        "the original path was removed; the added file and the source edit are not tests that \
         were rewritten"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_committed_test_edit_is_only_visible_against_the_turn_start_commit() {
    let dir = repo_with_committed_test();
    let start = git(&dir, &["rev-parse", "HEAD"]);
    let original = std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap();
    std::fs::write(
        dir.join("tests/test_foo.py"),
        original.replace("== 1", "== 2"),
    )
    .unwrap();
    git(&dir, &["commit", "-qam", "make the tests agree"]);
    assert!(
        git(&dir, &["status", "--porcelain"]).is_empty(),
        "the tree is clean after the model's own commit, which is what hid the edit from `git status`"
    );
    assert!(modified_existing_tests(Some(&dir), None, Level::Rewrites).is_empty());
    assert_eq!(
        modified_existing_tests(Some(&dir), Some(&start), Level::Rewrites),
        vec!["tests/test_foo.py".to_string()]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The SWE-bench repos Forge worked in all ended with `refs/stash` (8 of 10): the guard stashed the
/// model's test edits. A stash is a ref in the user's repo and an entry on the user's own stash
/// stack, so the originals are restored from a patch kept outside the repository instead.
#[test]
fn uncommitted_test_edits_are_restored_and_leave_nothing_behind_in_the_repo() {
    let dir = repo_with_committed_test();
    let original = std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap();
    let rewritten = original.replace("== 1", "== 2");
    std::fs::write(dir.join("tests/test_foo.py"), &rewritten).unwrap();
    let paths = vec!["tests/test_foo.py".to_string()];
    let start = git(&dir, &["rev-parse", "HEAD"]);
    let refs_before = git(&dir, &["for-each-ref"]);
    let objects_before = git(&dir, &["count-objects", "-v"]);

    let patch = restore_originals(Some(&dir), Some(&start), &paths).expect("restored");

    assert_eq!(
        std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap(),
        original
    );
    assert!(git(&dir, &["stash", "list"]).is_empty(), "no stash entry");
    assert_eq!(git(&dir, &["for-each-ref"]), refs_before, "no new ref");
    assert_eq!(
        git(&dir, &["count-objects", "-v"]),
        objects_before,
        "no new object"
    );
    assert_eq!(git(&dir, &["status", "--porcelain"]), "", "a clean tree");
    assert!(
        !patch.starts_with(&dir),
        "the patch lives outside the repo: {patch:?}"
    );
    git(&dir, &["apply", patch.to_str().unwrap()]);
    assert_eq!(
        std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap(),
        rewritten,
        "applying the patch gives the model its edit back"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_turn_start_commit_the_edits_are_diffed_against_head() {
    let dir = repo_with_committed_test();
    let original = std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap();
    std::fs::write(
        dir.join("tests/test_foo.py"),
        original.replace("== 1", "== 2"),
    )
    .unwrap();
    let paths = vec!["tests/test_foo.py".to_string()];
    assert!(restore_originals(Some(&dir), None, &paths).is_some());
    assert_eq!(
        std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap(),
        original
    );
    assert!(git(&dir, &["stash", "list"]).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nothing_is_claimed_restored_when_nothing_differs() {
    let dir = repo_with_committed_test();
    let paths = vec!["tests/test_foo.py".to_string()];
    assert_eq!(restore_originals(Some(&dir), None, &paths), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn committed_test_edits_are_restored_and_kept_as_a_patch() {
    let dir = repo_with_committed_test();
    let start = git(&dir, &["rev-parse", "HEAD"]);
    let original = std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap();
    let rewritten = original.replace("== 1", "== 2");
    std::fs::write(dir.join("tests/test_foo.py"), &rewritten).unwrap();
    git(&dir, &["commit", "-qam", "make the tests agree"]);
    let paths = vec!["tests/test_foo.py".to_string()];

    let Some(patch) = restore_originals(Some(&dir), Some(&start), &paths) else {
        panic!("a committed edit must be restored through a patch");
    };
    assert_eq!(
        std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap(),
        original
    );
    assert!(patch.exists(), "the patch must be on disk: {patch:?}");
    git(&dir, &["apply", patch.to_str().unwrap()]);
    assert_eq!(
        std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap(),
        rewritten,
        "applying the patch gives the model its edit back"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_push_back_says_how_to_get_the_edits_back_and_matches_the_run() {
    let edits = vec!["tests/test_foo.py".to_string()];
    let patch = std::path::Path::new("/state/test-edits/1.patch");
    let spec = push_back(true, &edits, patch);
    assert!(spec.contains("Hidden evaluation uses the ORIGINAL tests"));
    assert!(
        spec.contains("`git apply /state/test-edits/1.patch`")
            && spec.contains("tests/test_foo.py")
    );
    assert!(!spec.contains("stash"), "{spec}");
    let requested = push_back(false, &edits, patch);
    assert!(!requested.contains("Hidden evaluation"), "{requested}");
    assert!(
        requested.contains("`git apply /state/test-edits/1.patch`"),
        "{requested}"
    );
}

/// Rewrites a test and COMMITS it with one shell call, then answers; every later completion is a
/// short reply. The harness asks models to commit each verified unit, so this is the common shape.
struct RewriteCommitThenAnswer {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl Provider for RewriteCommitThenAnswer {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tool_calls = if n == 0 {
            vec![forge_types::ToolCall {
                id: forge_types::new_id(),
                name: "shell".into(),
                args: serde_json::json!({
                    "command": "printf 'def test_a():\\n    assert fix() == 2\\n\\ndef test_b():\\n    \
                                assert other() == 2\\n' > tests/test_foo.py && git add -A && \
                                git -c user.name=t -c user.email=t@t commit -qm agree"
                }),
            }]
        } else {
            Vec::new()
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if n == 0 { String::new() } else { "done".into() },
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_committed_test_rewrite_in_a_keep_green_turn_is_restored_and_pushed_back_once() {
    let dir = repo_with_committed_test();
    let provider = Arc::new(RewriteCommitThenAnswer {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let calls = Arc::clone(&provider);
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let mut config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        provider,
        Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(&dir),
        Box::new(capture),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap();

    session
        .run_turn("Fix `fix()` so it returns 1. Keep the existing tests green.")
        .await
        .unwrap();

    assert_eq!(
        calls.calls.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "tool call, the answer, then exactly ONE guard re-drive"
    );
    assert!(
        std::fs::read_to_string(dir.join("tests/test_foo.py"))
            .unwrap()
            .contains("== 1"),
        "the committed rewrite must be undone in the tree"
    );
    let warned =
        events.lock().unwrap().iter().any(
            |e| matches!(e, PresenterEvent::Warning(w) if w.contains("restored the originals")),
        );
    assert!(warned, "the restore must be announced");
    let nudge = session
        .transcript
        .iter()
        .find(|m| m.content.contains("git apply"))
        .expect("the model is told how to re-apply its edits");
    assert!(
        !nudge.content.contains("Hidden evaluation"),
        "a person's request is not a hidden evaluation: {}",
        nudge.content
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_turn_that_never_asked_for_green_tests_leaves_edited_tests_alone() {
    let dir = repo_with_committed_test();
    let original = std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap();
    let rewritten = original.replace("== 1", "== 2");
    std::fs::write(dir.join("tests/test_foo.py"), &rewritten).unwrap();
    let provider = Arc::new(DescribeOnlyProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let (_store, mut session) = fixed_session(
        provider,
        Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        }),
    );
    session.config.recap.enabled = false;
    session.config.suggest.enabled = false;
    session.config.mesh.auto_memory = false;
    session.workspace = WorkspaceContext::new(&dir).unwrap();

    session
        .run_turn("Update the tests for the new default.")
        .await
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap(),
        rewritten,
        "nothing asked for the existing tests to be kept, so the edit stays"
    );
    assert!(git(&dir, &["stash", "list"]).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// An earlier turn widened an existing test; a closing "final pass" turn must not read as "keep
/// the tests green", restore that test to the version the turn started with and send the model
/// round again.
#[tokio::test]
async fn a_final_pass_request_does_not_restore_or_push_back() {
    let dir = repo_with_committed_test();
    let original = std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap();
    let widened = original.replace("== 1", "== 2");
    std::fs::write(dir.join("tests/test_foo.py"), &widened).unwrap();
    let provider = Arc::new(DescribeOnlyProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let (_store, mut session) = fixed_session(
        provider,
        Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        }),
    );
    session.config.recap.enabled = false;
    session.config.suggest.enabled = false;
    session.config.mesh.auto_memory = false;
    session.workspace = WorkspaceContext::new(&dir).unwrap();

    session
        .run_turn(
            "Final pass: rerun everything and tell me which of today's changes still lack a test.",
        )
        .await
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap(),
        widened
    );
    assert!(
        !session.transcript.iter().any(|m| m
            .content
            .contains("existing tests are the specification")
            || m.content.contains("git apply")),
        "a review round is given neither the keep-the-tests text nor a restore push-back"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn an_attended_chat_that_says_tests_must_pass_gets_no_restore_round() {
    let dir = repo_with_committed_test();
    let provider = Arc::new(RewriteCommitThenAnswer {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let calls = Arc::clone(&provider);
    let mut config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        provider,
        Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(&dir),
        Box::new(CapturePresenter {
            attended: true,
            ..CapturePresenter::default()
        }),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap();

    session
        .run_turn("Fix `fix()` so it returns 1. Keep the existing tests green.")
        .await
        .unwrap();

    assert_eq!(calls.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(std::fs::read_to_string(dir.join("tests/test_foo.py"))
        .unwrap()
        .contains("== 2"));
    let _ = std::fs::remove_dir_all(&dir);
}
