//! The existing-tests-are-spec guard: which requests arm it, what it counts as a rewritten test,
//! and that a test edit the model already committed is still caught and restored.

use super::*;
use crate::test_guard::{
    assess, guidance, is_assertion_line, is_skip_marker_line, level, parse_name_status,
    prompt_asks_to_change_tests, prompt_expects_green_existing_tests,
    prompt_requires_pristine_existing_tests, push_back, restore_originals, sentences,
    session_requires_pristine_existing_tests, warning, Level, TestEdits, Why, KEEP_TESTS_GUIDANCE,
    PRISTINE_TEST_GUIDANCE,
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
fn prompts_that_name_green_tests_arm_the_guard() {
    for prompt in [
        "Add a new limiter under `tests/`. The existing tests must keep passing.",
        "Make lookups skip stale entries? Keep the existing tests green.",
        // the `./...` must not split the sentence between the command and "green"
        "Make `cache.Sweep` follow the docs, and keep `go test -race ./...` green. Please \
         don't change its signature, other packages call it.",
        "Get the build working again and `cargo test` green. CI builds offline.",
        "Update the README, add tests for the new flag, and make sure `cargo test` and \
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
        // the tests are renamed along with the code
        "Please rename the `Customer` type to `Client` across the repo. Everything should \
         still build and `go test ./...` should pass.",
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

/// "Don't touch `test/helpers.ts`" read as a ban on the tests (a word `test` after a "don't
/// touch"), so Forge told the model the tracked tests were immutable while the same request said to
/// update them; the model obeyed Forge, left `npm test` red and said so.
const SIGNATURE_REQUEST: &str = "`send(to, subject, body, isHtml, retries)` takes too many \
positional arguments. Please change it to take a single options object instead:\n\n    \
send({ to, subject, body, html?, retries? })\n\n`isHtml` becomes `html`, the defaults stay as \
they are, and old-style calls must be a compile error.\n\nUpdate every caller, the tests, the \
README and `docs/`. `npm run typecheck` and `npm test` should both be clean at the end. Don't \
touch `package.json`, `tsconfig.json` or `test/helpers.ts`, and keep the existing tests (updated \
for the new signature, not deleted or skipped).";

#[test]
fn a_request_to_update_the_tests_does_not_make_them_immutable() {
    assert!(prompt_asks_to_change_tests(SIGNATURE_REQUEST));
    assert!(!prompt_requires_pristine_existing_tests(SIGNATURE_REQUEST));
    assert_eq!(guidance(SIGNATURE_REQUEST, &[]), None);
    assert_eq!(
        level(false, &[Message::user(SIGNATURE_REQUEST)], false),
        Level::Off
    );
    assert_eq!(
        level(false, &[Message::user(SIGNATURE_REQUEST)], true),
        Level::Off
    );
    let with_green = format!("{SIGNATURE_REQUEST} Make sure the tests pass.");
    assert_eq!(
        level(false, &[Message::user(&with_green)], false),
        Level::Concealing,
        "a request to update the tests never freezes them, even when it also wants them green"
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
    assert_eq!(level(true, &lifted, true), Level::Concealing);

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
    assert_eq!(level(true, &user("hi"), false), Level::Guarded);
    assert_eq!(
        level(false, &user("Don't edit the existing tests."), true),
        Level::Frozen
    );
    assert_eq!(
        level(true, &user("Don't edit the existing tests."), false),
        Level::Frozen,
        "a person's explicit instruction beats the run's own guard"
    );
    assert_eq!(
        level(true, &user("Update the tests for the new default."), false),
        Level::Concealing,
        "a code-change run that is asked to update the tests only keeps failures from being hidden"
    );
    assert_eq!(
        level(false, &user("Keep the existing tests green."), false),
        Level::Guarded
    );
    assert_eq!(
        level(false, &user("Keep the existing tests green."), true),
        Level::Off,
        "a person in a chat is not given an extra round for saying the tests must pass"
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
    assert_eq!(level(false, &sticky, false), Level::Guarded);
    let updated_later = vec![
        Message::user("Keep the existing tests green."),
        Message::assistant("ok"),
        Message::user("Now rename the helper and update the tests to match."),
    ];
    assert_eq!(level(false, &updated_later, false), Level::Concealing);
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
    // The push-back after a restore says "fix the production code rather than the tests", and
    // the one for a hiding edit says to re-apply a changed expectation.
    for (level_pushed, why) in [
        (Level::Frozen, Why::Requested),
        (Level::Guarded, Why::AlwaysTrue),
    ] {
        let edits = TestEdits {
            restore: vec![crate::test_guard::Restore {
                path: "tests/test_a.py".to_string(),
                why: vec![why],
            }],
            ..TestEdits::default()
        };
        let pushed_back = push_back(
            level_pushed,
            &edits,
            Some(std::path::Path::new("/tmp/edits.patch")),
            None,
        );
        let transcript = vec![
            Message::user("Fix the parser. Leave the existing tests alone."),
            Message::assistant("ok"),
            nudge(pushed_back),
        ];
        assert!(session_requires_pristine_existing_tests(&transcript));
        assert_eq!(level(false, &transcript, true), Level::Frozen);
    }

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
    assert_eq!(level(false, &transcript, false), Level::Guarded);
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

fn paths(edits: &TestEdits) -> Vec<&str> {
    edits.restore.iter().map(|r| r.path.as_str()).collect()
}

fn why_of(edits: &TestEdits, path: &str) -> Vec<Why> {
    edits
        .restore
        .iter()
        .find(|r| r.path == path)
        .map(|r| r.why.clone())
        .unwrap_or_default()
}

/// Rewrite `tests/test_foo.py` and, when `code` is set, `src/lib.py` too.
fn edit_test(dir: &std::path::Path, from: &str, to: &str, code: bool) {
    let original = std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap();
    std::fs::write(dir.join("tests/test_foo.py"), original.replace(from, to)).unwrap();
    if code {
        std::fs::write(dir.join("src/lib.py"), "def fix():\n    return 2\n").unwrap();
    }
}

#[test]
fn an_appended_test_is_fine_unless_the_tests_are_frozen() {
    let dir = repo_with_committed_test();
    let mut text = std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap();
    text.push_str("\ndef test_c():\n    assert fix() == 3\n");
    std::fs::write(dir.join("tests/test_foo.py"), text).unwrap();
    for lvl in [Level::Guarded, Level::Concealing] {
        assert_eq!(
            assess(Some(&dir), None, lvl),
            TestEdits::default(),
            "{lvl:?}"
        );
    }
    assert_eq!(
        paths(&assess(Some(&dir), None, Level::Frozen)),
        vec!["tests/test_foo.py"]
    );
    assert_eq!(assess(Some(&dir), None, Level::Off), TestEdits::default());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_test_inserted_before_existing_ones_or_reordered_is_not_a_change_of_what_they_check() {
    let dir = repo_with_committed_test();
    let original = std::fs::read_to_string(dir.join("tests/test_foo.py")).unwrap();
    let inserted = original.replace(
        "def test_b():",
        "def test_new():\n    assert fix() == 9\n\ndef test_b():",
    );
    std::fs::write(dir.join("tests/test_foo.py"), inserted).unwrap();
    assert_eq!(
        assess(Some(&dir), None, Level::Guarded),
        TestEdits::default()
    );
    let reordered =
        "def test_b():\n    assert other() == 2\n\ndef test_a():\n    assert fix() == 1\n";
    std::fs::write(dir.join("tests/test_foo.py"), reordered).unwrap();
    assert_eq!(
        assess(Some(&dir), None, Level::Guarded),
        TestEdits::default()
    );
    std::fs::write(
        dir.join("tests/test_foo.py"),
        original.replace("    assert", "        assert"),
    )
    .unwrap();
    assert_eq!(
        assess(Some(&dir), None, Level::Guarded),
        TestEdits::default(),
        "re-indenting is not a change"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_changed_expectation_with_a_code_change_behind_it_stays_and_is_questioned() {
    let dir = repo_with_committed_test();
    edit_test(&dir, "== 1", "== 2", true);
    let edits = assess(Some(&dir), None, Level::Guarded);
    assert!(edits.restore.is_empty(), "{edits:?}");
    assert_eq!(edits.revised, vec!["tests/test_foo.py".to_string()]);
    let ask = push_back(Level::Guarded, &edits, None, Some("0123456789abcdef"));
    assert!(
        ask.contains("assert fix() == 1") && ask.contains("assert fix() == 2"),
        "{ask}"
    );
    assert!(
        ask.contains("Does the request change the behavior"),
        "{ask}"
    );
    assert!(
        ask.contains("git checkout 0123456789ab -- tests/test_foo.py"),
        "{ask}"
    );
    assert!(!ask.contains("Restored test files"), "{ask}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_changed_expectation_with_no_code_change_behind_it_is_put_back() {
    let dir = repo_with_committed_test();
    edit_test(&dir, "== 1", "== 0", false);
    let edits = assess(Some(&dir), None, Level::Guarded);
    assert_eq!(paths(&edits), vec!["tests/test_foo.py"]);
    assert_eq!(
        why_of(&edits, "tests/test_foo.py"),
        vec![Why::CodeUnchanged]
    );
    assert!(edits.revised.is_empty());
    std::fs::write(dir.join("README.md"), "docs only\n").unwrap();
    assert_eq!(
        assess(Some(&dir), None, Level::Guarded),
        edits,
        "an edited README is not a code change"
    );
    std::fs::write(
        dir.join("src/new_module.py"),
        "def helper():\n    return 1\n",
    )
    .unwrap();
    let with_new_code = assess(Some(&dir), None, Level::Guarded);
    assert!(
        with_new_code.restore.is_empty() && with_new_code.revised.len() == 1,
        "an untracked new source file is a code change: {with_new_code:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_request_to_update_the_tests_lets_a_changed_expectation_through() {
    let dir = repo_with_committed_test();
    edit_test(&dir, "== 1", "== 0", false);
    assert_eq!(
        assess(Some(&dir), None, Level::Concealing),
        TestEdits::default()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_a_test_file_is_always_put_back() {
    let dir = repo_with_committed_test();
    std::fs::remove_file(dir.join("tests/test_foo.py")).unwrap();
    for lvl in [Level::Guarded, Level::Concealing] {
        let edits = assess(Some(&dir), None, lvl);
        assert_eq!(
            why_of(&edits, "tests/test_foo.py"),
            vec![Why::Deleted],
            "{lvl:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn removing_assertions_is_put_back_even_when_code_changed() {
    let dir = repo_with_committed_test();
    std::fs::write(dir.join("src/lib.py"), "def fix():\n    return 2\n").unwrap();
    std::fs::write(
        dir.join("tests/test_foo.py"),
        "def test_a():\n    assert fix() == 1\n\ndef test_b():\n    pass\n",
    )
    .unwrap();
    for lvl in [Level::Guarded, Level::Concealing] {
        let edits = assess(Some(&dir), None, lvl);
        assert_eq!(
            why_of(&edits, "tests/test_foo.py"),
            vec![Why::AssertionsRemoved(1)],
            "{lvl:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_skip_marker_inserted_above_an_existing_test_is_put_back() {
    let dir = repo_with_committed_test();
    std::fs::write(dir.join("src/lib.py"), "def fix():\n    return 2\n").unwrap();
    edit_test(
        &dir,
        "def test_a():",
        "@pytest.mark.skip(reason=\"later\")\ndef test_a():",
        false,
    );
    let edits = assess(Some(&dir), None, Level::Concealing);
    assert_eq!(why_of(&edits, "tests/test_foo.py"), vec![Why::SkipAdded(1)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_assertion_replaced_by_one_that_cannot_fail_is_put_back() {
    let dir = repo_with_committed_test();
    std::fs::write(dir.join("src/lib.py"), "def fix():\n    return 2\n").unwrap();
    edit_test(&dir, "assert fix() == 1", "assert True", false);
    let edits = assess(Some(&dir), None, Level::Guarded);
    assert_eq!(why_of(&edits, "tests/test_foo.py"), vec![Why::AlwaysTrue]);
    edit_test(&dir, "assert True", "assert fix() == 1 or True", false);
    let edits = assess(Some(&dir), None, Level::Guarded);
    assert_eq!(why_of(&edits, "tests/test_foo.py"), vec![Why::AlwaysTrue]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn skip_and_assertion_patterns_cover_the_common_frameworks() {
    for line in [
        "@pytest.mark.xfail(strict=False)",
        "self.skipTest('flaky')",
        "#[ignore]",
        "it.skip('does a thing', () => {})",
        "xit('does a thing', () => {})",
        "t.Skip(\"later\")",
    ] {
        assert!(is_skip_marker_line(line), "{line}");
    }
    for line in [
        "assert x == 1",
        "self.assertEqual(a, b)",
        "assert_eq!(a, b);",
        "expect(a).toBe(b)",
        "with pytest.raises(ValueError):",
        "t.Errorf(\"got %v\", got)",
    ] {
        assert!(is_assertion_line(line), "{line}");
    }
    assert!(!is_assertion_line("x = compute()"));
    assert!(!is_skip_marker_line("def test_skipping_stale_entries():"));
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
        paths(&assess(Some(&dir), None, Level::Guarded)),
        vec!["tests/test_foo.py"],
        "the original path was removed; the added file and the source edit are not tests that \
         were changed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_committed_test_edit_is_only_visible_against_the_turn_start_commit() {
    let dir = repo_with_committed_test();
    let start = git(&dir, &["rev-parse", "HEAD"]);
    edit_test(&dir, "== 1", "== 0", false);
    git(&dir, &["commit", "-qam", "make the tests agree"]);
    assert!(
        git(&dir, &["status", "--porcelain"]).is_empty(),
        "the tree is clean after the model's own commit, which is what hid the edit from `git status`"
    );
    assert!(assess(Some(&dir), None, Level::Guarded).is_empty());
    assert_eq!(
        paths(&assess(Some(&dir), Some(&start), Level::Guarded)),
        vec!["tests/test_foo.py"]
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

/// Every message the guard can send, for each way it can send it.
fn every_push_back() -> Vec<String> {
    let patch = std::path::Path::new("/state/test-edits/1.patch");
    let restored = |why: Vec<Why>| TestEdits {
        restore: vec![crate::test_guard::Restore {
            path: "tests/test_foo.py".to_string(),
            why,
        }],
        ..TestEdits::default()
    };
    let mut out = Vec::new();
    for why in [
        vec![Why::Deleted],
        vec![Why::AssertionsRemoved(2), Why::SkipAdded(1)],
        vec![Why::AlwaysTrue],
        vec![Why::CodeUnchanged],
    ] {
        out.push(push_back(
            Level::Guarded,
            &restored(why),
            Some(patch),
            Some("abc"),
        ));
    }
    out.push(push_back(
        Level::Frozen,
        &restored(vec![Why::Requested]),
        Some(patch),
        None,
    ));
    out
}

#[test]
fn the_push_back_says_how_to_get_the_edits_back() {
    let messages = every_push_back();
    for message in &messages {
        assert!(
            message.contains("`git apply /state/test-edits/1.patch`")
                && message.contains("tests/test_foo.py"),
            "{message}"
        );
        assert!(!message.contains("stash"), "{message}");
    }
    assert!(
        messages[1].contains("2 assertion(s) were removed"),
        "{}",
        messages[1]
    );
    assert!(
        messages[1].contains("1 skip/xfail marker(s) were added"),
        "{}",
        messages[1]
    );
    assert!(messages[4].contains("left as they are"), "{}", messages[4]);
}

/// The guard cannot know how a piece of work will be judged. A claim that the tests which count are
/// "the original ones" sent a model back to an old behaviour that its own correct change had just
/// replaced.
#[test]
fn no_text_the_model_sees_claims_how_the_work_is_judged() {
    let edits = TestEdits {
        revised: vec!["tests/test_foo.py".to_string()],
        ..TestEdits::default()
    };
    let mut texts = every_push_back();
    texts.push(push_back(Level::Guarded, &edits, None, None));
    texts.push(warning(Level::Guarded, &edits));
    texts.push(KEEP_TESTS_GUIDANCE.to_string());
    texts.push(PRISTINE_TEST_GUIDANCE.to_string());
    texts.push(crate::MINIMAL_DIFF_BIAS.to_string());
    texts.push(crate::completeness::DIRECT_COMPLETENESS_PROMPT.to_string());
    texts.push(crate::completeness::DIRECT_COMPLETENESS_EMPTY_SEARCH_RETRY_PROMPT.to_string());
    texts.push(crate::completeness::DIRECT_COMPLETENESS_MISSING_SEARCH_RETRY_PROMPT.to_string());
    texts.push(crate::completeness::DIRECT_COMPLETENESS_UNHANDLED_PATH_PROMPT.to_string());
    texts.push(crate::completeness::DIRECT_COMPLETENESS_UNRESOLVED_RETRY_PROMPT.to_string());
    for text in texts {
        let lowered = text.to_lowercase();
        for banned in ["hidden", "evaluation", "original tests", "pristine tests"] {
            assert!(!lowered.contains(banned), "`{banned}` in: {text}");
        }
    }
}

#[test]
fn a_kept_edit_is_asked_about_not_ordered_back() {
    let edits = TestEdits {
        revised: vec!["tests/test_foo.py".to_string()],
        ..TestEdits::default()
    };
    let ask = push_back(Level::Guarded, &edits, None, None);
    assert!(ask.contains('?'), "{ask}");
    assert!(ask.contains("keep the edit"), "{ask}");
    assert!(!ask.contains("shrink"), "{ask}");
    assert!(
        !ask.contains("git apply"),
        "nothing was restored, so nothing to re-apply: {ask}"
    );
}

#[test]
fn the_guidance_lets_a_changed_behavior_update_its_test_but_never_hide_one() {
    for text in [KEEP_TESTS_GUIDANCE, PRISTINE_TEST_GUIDANCE] {
        assert!(text.contains("which test and why"), "{text}");
    }
    assert!(KEEP_TESTS_GUIDANCE.contains("update that test's expected value"));
    assert!(KEEP_TESTS_GUIDANCE.contains("Never delete a test or an assertion, skip or xfail"));
    assert!(!KEEP_TESTS_GUIDANCE.contains("do not rewrite"));
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

/// Runs one shell command with the first completion, then answers; every later completion is a
/// short reply.
struct ShellThenAnswer {
    command: String,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl Provider for ShellThenAnswer {
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
                args: serde_json::json!({ "command": &self.command }),
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

/// A code-change session (what `bench swe` and a worktree build session are) over `dir`.
fn code_change_session(
    dir: &std::path::Path,
    provider: Arc<ShellThenAnswer>,
) -> (Session, Arc<std::sync::Mutex<Vec<PresenterEvent>>>) {
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
        ToolRegistry::with_core_tools_in(dir),
        Box::new(capture),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap();
    session.set_expect_code_change(true);
    (session, events)
}

const COMMIT: &str = "git add -A && git -c user.name=t -c user.email=t@t commit -qm change";

/// A repo with a version check in `src/check.py` and a test whose expected message names the same
/// version, so raising the version has to change both.
fn repo_with_version_check() -> std::path::PathBuf {
    let dir = clean_git_repo();
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/check.py"),
        "MINIMUM = '3.8'\n\ndef message():\n    return f'needs {MINIMUM}'\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("tests/test_check.py"),
        "from src.check import message\n\ndef test_message():\n    msg = 'needs 3.8'\n    assert message() == msg\n",
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "add check"]);
    dir
}

/// The shape that cost a correct fix: the request changes the behavior an existing test pins down,
/// the model changes the code and that test's expected message, and the guard used to put the old
/// test back and tell the model the original tests are what count, so the model reverted its own
/// fix. The edit must stay, and the model is asked about it.
#[cfg(unix)]
#[tokio::test]
async fn an_updated_expectation_that_follows_the_requested_change_survives_the_guard() {
    let dir = repo_with_version_check();
    let provider = Arc::new(ShellThenAnswer {
        command: format!("sed -i 's/3\\.8/3.9/' src/check.py tests/test_check.py && {COMMIT}"),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let calls = Arc::clone(&provider);
    let (mut session, events) = code_change_session(&dir, provider);

    session
        .run_turn("Raise the minimum supported version to 3.9 and say so in the message.")
        .await
        .unwrap();

    assert_eq!(
        calls.calls.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "tool call, the answer, then exactly ONE question"
    );
    let test = std::fs::read_to_string(dir.join("tests/test_check.py")).unwrap();
    assert!(
        test.contains("needs 3.9"),
        "the updated expectation stays: {test}"
    );
    let code = std::fs::read_to_string(dir.join("src/check.py")).unwrap();
    assert!(code.contains("'3.9'"), "{code}");
    let ask = session
        .transcript
        .iter()
        .find(|m| m.content.contains("Does the request change the behavior"))
        .expect("the model is asked whether the request changes what the test pins down");
    assert!(ask.content.contains("msg = 'needs 3.8'") && ask.content.contains("msg = 'needs 3.9'"));
    assert!(
        !ask.content.to_lowercase().contains("hidden"),
        "{}",
        ask.content
    );
    assert!(
        !ask.content.contains("git apply"),
        "nothing was restored: {}",
        ask.content
    );
    let warned = events.lock().unwrap().iter().any(
        |e| matches!(e, PresenterEvent::Warning(w) if w.contains("asking whether the request justifies")),
    );
    assert!(warned);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the guard exists for: a failing test that disappears, or is made to not run, is put back
/// and the model is told once, even though code changed in the same turn.
#[cfg(unix)]
#[tokio::test]
async fn a_deleted_or_skipped_test_is_put_back_and_pushed_back_once() {
    for (label, command, reason) in [
        (
            "delete",
            "git rm -q tests/test_check.py",
            "the file was deleted or replaced",
        ),
        (
            "skip",
            "sed -i 's/^def test_message/@pytest.mark.skip\\ndef test_message/' tests/test_check.py",
            "1 skip/xfail marker(s) were added",
        ),
    ] {
        let dir = repo_with_version_check();
        let provider = Arc::new(ShellThenAnswer {
            command: format!("sed -i 's/3\\.8/3.9/' src/check.py && {command} && {COMMIT}"),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let calls = Arc::clone(&provider);
        let (mut session, _events) = code_change_session(&dir, provider);

        session
            .run_turn("Raise the minimum supported version to 3.9.")
            .await
            .unwrap();

        assert_eq!(
            calls.calls.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "{label}: tool call, the answer, then exactly ONE guard re-drive"
        );
        let test = std::fs::read_to_string(dir.join("tests/test_check.py")).unwrap();
        assert!(
            test.contains("msg = 'needs 3.8'") && !test.contains("skip"),
            "{label}: the original test is back: {test}"
        );
        let code = std::fs::read_to_string(dir.join("src/check.py")).unwrap();
        assert!(code.contains("'3.9'"), "{label}: the code change stays: {code}");
        let nudge = session
            .transcript
            .iter()
            .find(|m| m.content.contains("git apply"))
            .unwrap_or_else(|| panic!("{label}: the model is told how to re-apply"));
        assert!(nudge.content.contains(reason), "{label}: {}", nudge.content);
        crate::test_state::assert_test_edits_kept_as_patch(&dir);
        let _ = std::fs::remove_dir_all(&dir);
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
            |e| matches!(e, PresenterEvent::Warning(w) if w.contains("restored 1 existing test file(s)")),
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
        !session
            .transcript
            .iter()
            .any(|m| m.content.contains(KEEP_TESTS_GUIDANCE) || m.content.contains("git apply")),
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
