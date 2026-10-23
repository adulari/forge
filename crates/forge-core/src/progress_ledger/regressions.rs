//! Streams that an independent verification of the ledger found it judged wrongly, as tests: each
//! one is a turn that is working or waiting and must not be touched, or a loop that must still be
//! caught. Where the cause was a signal the ledger lacked (the working tree, a check's verdict,
//! the difference between waiting and polling) the stream is fed with that signal.

use super::tests::*;
use super::*;

const WORDS: [&str; 8] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta",
];

fn shell_call(sig: u64, ok: bool, body: &str) -> CallObservation {
    let code = if ok { 0 } else { 1 };
    let result = if body.is_empty() {
        format!("shell: exit {code} in 3ms")
    } else {
        format!("shell: exit {code} in 3ms\n\n{body}")
    };
    obs(sig, ok, false, &result).named("shell").opaquely()
}

fn silent_shell(sig: u64) -> CallObservation {
    shell_call(sig, true, "").silently()
}

const FAILING_TEST: &str =
    "FAILED tests::parses_the_header\nassertion `left == right` failed\n  left: 1\n right: 2";

fn check_run(ok: bool, extra: &str) -> CallObservation {
    let body = if ok {
        "all tests passed".to_string()
    } else {
        format!("{FAILING_TEST}\n{extra}")
    };
    shell_call(500, ok, &body).running_check("test")
}

fn read_of(sig: u64, path: &str, body: &str) -> CallObservation {
    obs(sig, true, false, body).named("read_file").reading(path)
}

fn job_poll() -> CallObservation {
    obs(
        77,
        true,
        false,
        "job 4242 is running\n  command: cargo build --release\n  log: /tmp/job.log",
    )
    .named("shell_job")
    .waiting_for_job()
}

fn stops(verdicts: &[Verdict]) -> Vec<usize> {
    verdicts
        .iter()
        .enumerate()
        .filter(|(_, v)| matches!(v, Verdict::Stop(_)))
        .map(|(i, _)| i + 1)
        .collect()
}

// ---- the working tree --------------------------------------------------------------------------

/// A file that moves through `period` states and comes back is working through states it already
/// visited; the tree is only progress while it reaches one it never had.
fn flipping_file(period: u64, steps: u64) -> Vec<Verdict> {
    let mut ledger = ProgressLedger::default();
    ledger.rebase_tree(Some(0));
    (1..=steps)
        .map(|i| {
            let state = i % period;
            ledger.observe_step(
                &distinct(i as usize),
                &[silent_shell(i % period)],
                false,
                &mut || Some(state),
            )
        })
        .collect()
}

#[test]
fn a_file_that_flips_between_known_states_is_nudged_and_stopped_whatever_the_period() {
    for period in 2..=6 {
        let verdicts = flipping_file(period, 60);
        let nudge = first(&verdicts, |v| matches!(v, Verdict::Nudge(_)));
        let stop = first(&verdicts, |v| matches!(v, Verdict::Stop(_)));
        assert!(
            nudge.is_some() && stop.is_some_and(|s| s <= 40),
            "period {period}: {:?}",
            events(&verdicts)
        );
    }
}

#[test]
fn shell_steps_that_keep_reaching_new_tree_states_are_never_judged() {
    let mut ledger = ProgressLedger::default();
    ledger.rebase_tree(Some(0));
    for i in 1..=150u64 {
        let verdict = ledger.observe_step(
            "Appending to the file again.",
            &[silent_shell(1)],
            false,
            &mut || Some(i),
        );
        assert_eq!(verdict, Verdict::Fine, "step {i}");
    }
    assert_eq!(
        ledger.last_progress.as_ref().map(|p| p.1.as_str()),
        Some("the working tree changed")
    );
}

#[test]
fn a_tree_change_the_model_made_with_a_write_is_not_credited_to_the_next_shell_step() {
    let mut ledger = ProgressLedger::default();
    ledger.rebase_tree(Some(0));
    feed(&mut ledger, "editing", vec![edit(1)]);
    let mut probe = tree_of(&[1]);
    ledger.observe_step(
        "running the tests",
        &[shell_call(9, true, "ok 1\nok 2")],
        false,
        &mut probe,
    );
    assert_eq!(
        ledger.last_progress.as_ref().map(|p| p.0),
        Some(1),
        "the shell step only moved the baseline"
    );
}

#[test]
fn only_a_step_that_could_have_changed_files_looks_at_the_tree() {
    let mut ledger = ProgressLedger::default();
    let mut probes = 0;
    for i in 0..30 {
        ledger.observe_step(&distinct(i), &[novel(i)], false, &mut || {
            probes += 1;
            Some(i as u64)
        });
    }
    assert_eq!(probes, 0, "reads cannot change the tree");
    ledger.observe_step(
        "running it",
        &[shell_call(5, true, "out")],
        false,
        &mut || {
            probes += 1;
            Some(99)
        },
    );
    assert_eq!(probes, 1);
}

#[test]
fn a_tree_that_cannot_be_read_is_no_evidence_either_way() {
    let mut ledger = ProgressLedger::default();
    let verdicts: Vec<Verdict> = (0..12)
        .map(|i| ledger.observe_step(&distinct(i), &[stale(i).opaquely()], false, &mut || None))
        .collect();
    assert!(
        first(&verdicts, stop_of(Lane::Futile)).is_some(),
        "{verdicts:?}"
    );
}

// ---- checks ------------------------------------------------------------------------------------

/// `runs` rounds of: a different edit, then the same check. The check passes in round `pass_at`.
fn edit_test_rounds(
    runs: usize,
    pass_at: Option<usize>,
    vary: impl Fn(usize) -> String,
) -> Vec<Verdict> {
    let mut ledger = ProgressLedger::default();
    let mut verdicts = Vec::new();
    for i in 0..runs {
        verdicts.push(feed(&mut ledger, &distinct(2 * i), vec![edit(i)]));
        let call = check_run(pass_at == Some(i), &vary(i));
        verdicts.push(feed(&mut ledger, &distinct(2 * i + 1), vec![call]));
    }
    verdicts
}

#[test]
fn different_edits_between_the_same_failing_check_are_nudged_at_six_runs_and_stopped_at_twelve() {
    // The run's duration is in the output and differs every time; it is not a different failure.
    let verdicts = edit_test_rounds(14, None, |i| format!("finished in 0.{}s", 40 + i));
    assert_eq!(
        first(&verdicts, nudge_of(Lane::Check)),
        Some(12),
        "{:?}",
        events(&verdicts)
    );
    assert_eq!(
        first(&verdicts, stop_of(Lane::Check)),
        Some(24),
        "{:?}",
        events(&verdicts)
    );
    let Verdict::Nudge(report) = &verdicts[11] else {
        panic!("{verdicts:?}")
    };
    assert_eq!(report.check, Some(("test", 6)));
    assert!(report.nudge_text(None, 0).contains("failed 6 times"));
}

#[test]
fn a_check_that_passes_on_the_sixth_try_is_never_touched() {
    let verdicts = edit_test_rounds(8, Some(5), |_| String::new());
    assert!(!any_event(&verdicts), "{:?}", events(&verdicts));
}

#[test]
fn a_check_whose_failure_keeps_changing_is_working_through_something() {
    let verdicts = edit_test_rounds(40, None, |i| {
        format!("also failing: case_{}", WORDS[i % 8].repeat(1 + i / 8))
    });
    assert!(!any_event(&verdicts), "{:?}", events(&verdicts));
}

#[test]
fn breaking_and_repairing_the_same_thing_is_progress_only_the_first_time() {
    let mut ledger = ProgressLedger::default();
    let mut verdicts = Vec::new();
    for i in 0..30 {
        let steps = [
            obs(1, true, true, "edited src/lib.rs"),
            check_run(false, ""),
            obs(2, true, true, "edited src/lib.rs"),
            check_run(true, ""),
        ];
        for (k, call) in steps.into_iter().enumerate() {
            verdicts.push(feed(&mut ledger, &distinct(4 * i + k), vec![call]));
        }
    }
    assert!(
        verdicts[..4].iter().all(|v| *v == Verdict::Fine),
        "the first round fixed something: {:?}",
        events(&verdicts)
    );
    let stop = first(&verdicts, |v| matches!(v, Verdict::Stop(_)));
    assert!(stop.is_some_and(|s| s <= 40), "{:?}", events(&verdicts));
}

// ---- after a stop ------------------------------------------------------------------------------

#[test]
fn a_continue_whose_repeated_call_now_succeeds_is_not_stopped() {
    let mut ledger = ProgressLedger::default();
    let probe = || shell_call(9, false, "probe failed\nconnection refused\nretry later");
    let verdicts: Vec<Verdict> = (0..8)
        .map(|i| feed(&mut ledger, &distinct(i), vec![probe()]))
        .collect();
    assert!(first(&verdicts, |v| matches!(v, Verdict::Stop(_))).is_some());
    ledger.stopped();
    ledger.begin_turn();
    let now_up = shell_call(9, true, "service is up\nlistening on 8080");
    assert_eq!(
        feed(&mut ledger, &distinct(100), vec![now_up]),
        Verdict::Fine
    );
    assert!(
        stop_of(Lane::Resumed)(&feed(&mut ledger, &distinct(101), vec![probe()])),
        "the call that fails the same way is still stopped"
    );
}

#[test]
fn a_continue_that_opens_the_same_way_over_new_findings_is_not_stopped() {
    let mut ledger = ProgressLedger::default();
    let verdicts: Vec<Verdict> = (20..40)
        .map(|n| feed(&mut ledger, REPEATED, vec![creeping_read(n)]))
        .collect();
    assert!(first(&verdicts, |v| matches!(v, Verdict::Stop(_))).is_some());
    ledger.stopped();
    ledger.begin_turn();
    assert_eq!(feed(&mut ledger, REPEATED, vec![novel(500)]), Verdict::Fine);
}

// ---- failures ----------------------------------------------------------------------------------

#[test]
fn eight_different_failing_commands_that_explain_themselves_are_exploring_not_one_failure() {
    let steps: Vec<_> = (0..8)
        .map(|i| {
            (
                distinct(i),
                vec![shell_call(
                    i as u64,
                    false,
                    &format!(
                        "error: {} is wrong\nhint: look at {}\nnote: {} was last changed by {}\nnote: see {}",
                        WORDS[i],
                        WORDS[(i + 3) % 8],
                        WORDS[(i + 5) % 8],
                        WORDS[(i + 1) % 8],
                        WORDS[(i + 2) % 8]
                    ),
                )],
            )
        })
        .collect();
    let verdicts = run(steps);
    assert!(!any_event(&verdicts), "{:?}", events(&verdicts));
}

#[test]
fn a_dozen_different_one_line_errors_in_a_row_are_futile_not_one_failure() {
    let steps: Vec<_> = (0..12)
        .map(|i| {
            (
                distinct(i),
                vec![shell_call(
                    i as u64,
                    false,
                    &format!("error: {} is wrong", WORDS[i % 8].repeat(1 + i / 8)),
                )],
            )
        })
        .collect();
    let verdicts = run(steps);
    assert!(
        !events(&verdicts)
            .iter()
            .any(|(_, _, lane)| *lane == "failure"),
        "{:?}",
        events(&verdicts)
    );
    assert_eq!(
        first(&verdicts, nudge_of(Lane::Futile)),
        Some(5),
        "{:?}",
        events(&verdicts)
    );
    assert!(first(&verdicts, stop_of(Lane::Futile)).is_some_and(|s| s >= 10));
}

#[test]
fn reads_of_paths_that_do_not_exist_are_one_failure_whatever_the_path() {
    let steps: Vec<_> = (0..8)
        .map(|i| {
            (
                distinct(i),
                vec![obs(
                    i as u64,
                    false,
                    false,
                    &format!("error: no such file or directory: does-not-exist-{i}.rs"),
                )
                .named("read_file")],
            )
        })
        .collect();
    let verdicts = run(steps);
    assert_eq!(
        first(&verdicts, nudge_of(Lane::Failure)),
        Some(3),
        "{:?}",
        events(&verdicts)
    );
    assert_eq!(
        first(&verdicts, stop_of(Lane::Failure)),
        Some(6),
        "{:?}",
        events(&verdicts)
    );
}

#[test]
fn six_failing_scripts_that_open_alike_but_print_different_things_are_not_one_failure() {
    let steps: Vec<_> = (0..6)
        .map(|i| {
            // Numbered cases, as test suites print them: only the numbers tell them apart.
            let body: String = (0..25)
                .map(|k| format!("suite {i} case {k}: fixture_{k} is missing\n"))
                .collect();
            (
                distinct(i),
                vec![shell_call(
                    i as u64,
                    false,
                    &format!("Running suite\n{body}"),
                )],
            )
        })
        .collect();
    let verdicts = run(steps);
    assert!(!any_event(&verdicts), "{:?}", events(&verdicts));
}

#[test]
fn searches_that_find_nothing_are_idle_not_failing() {
    let steps: Vec<_> = (0..12)
        .map(|i| (distinct(i), vec![shell_call(i as u64, false, "")]))
        .collect();
    let verdicts = run(steps);
    assert!(
        !events(&verdicts)
            .iter()
            .any(|(_, _, lane)| *lane == "failure"),
        "{:?}",
        events(&verdicts)
    );
    assert!(first(&verdicts, nudge_of(Lane::Futile)).is_some());
    assert!(first(&verdicts, stop_of(Lane::Futile)).is_some_and(|s| s >= 10));
}

#[test]
fn the_same_error_with_a_changing_attempt_number_is_one_failure() {
    let steps: Vec<_> = (0..8)
        .map(|i| {
            (
                distinct(i),
                vec![shell_call(
                    i as u64,
                    false,
                    &format!("attempt {i} of 20: connection refused (os error 111)"),
                )],
            )
        })
        .collect();
    let verdicts = run(steps);
    assert_eq!(
        first(&verdicts, nudge_of(Lane::Failure)),
        Some(3),
        "{:?}",
        events(&verdicts)
    );
    assert_eq!(
        first(&verdicts, stop_of(Lane::Failure)),
        Some(6),
        "{:?}",
        events(&verdicts)
    );
}

// ---- waiting -----------------------------------------------------------------------------------

#[test]
fn polling_a_running_job_is_not_a_loop_but_a_wait_that_never_ends_is_nudged_then_stopped_headless()
{
    let steps: Vec<_> = (0..200)
        .map(|_| (REPEATED.to_string(), vec![job_poll()]))
        .collect();
    let attended = run(steps);
    assert_eq!(events(&attended), vec![(24, "nudge", "waiting")]);

    let mut ledger = ProgressLedger::default();
    ledger.configure(60, 200, true);
    // The turn ends at the first stop.
    let headless: Vec<Verdict> = (0..96)
        .map(|_| feed(&mut ledger, REPEATED, vec![job_poll()]))
        .collect();
    assert_eq!(
        events(&headless),
        vec![(24, "nudge", "waiting"), (96, "stop", "waiting")]
    );
}

#[test]
fn a_zero_idle_stop_means_no_headless_stop_for_waiting_either() {
    let mut ledger = ProgressLedger::default();
    ledger.configure(60, 0, true);
    let verdicts: Vec<Verdict> = (0..300)
        .map(|_| feed(&mut ledger, REPEATED, vec![job_poll()]))
        .collect();
    assert_eq!(events(&verdicts), vec![(24, "nudge", "waiting")]);
}

#[test]
fn a_wait_batched_with_a_quiet_peek_at_the_log_is_still_a_wait() {
    let peek = || shell_call(5, true, "compiling...").opaquely();
    let steps: Vec<_> = (0..40)
        .map(|i| (distinct(i), vec![job_poll(), peek()]))
        .collect();
    let verdicts = run(steps);
    assert_eq!(events(&verdicts), vec![(24, "nudge", "waiting")]);
}

#[test]
fn a_sleep_that_alternates_with_a_log_that_keeps_growing_is_not_a_cycle() {
    let sleep = || obs(1, true, false, "shell: exit 0 in 2012ms").named("shell");
    let steps: Vec<_> = (0..40)
        .map(|i| {
            let log: String = (0..15)
                .map(|k| format!("line {} of the build\n", i * 15 + k))
                .collect();
            let call = if i % 2 == 0 {
                sleep()
            } else {
                obs(2, true, false, &format!("job 1 is running\n\n{log}")).named("shell_job")
            };
            (distinct(i), vec![call])
        })
        .collect();
    let verdicts = run(steps);
    assert!(!any_event(&verdicts), "{:?}", events(&verdicts));
}

#[test]
fn real_progress_starts_the_wait_count_over() {
    let mut steps: Vec<_> = (0..23).map(|i| (distinct(i), vec![job_poll()])).collect();
    steps.push((distinct(23), vec![edit(1)]));
    steps.extend((24..47).map(|i| (distinct(i), vec![job_poll()])));
    assert!(!any_event(&run(steps)));
}

// ---- narration and reads -----------------------------------------------------------------------

#[test]
fn reads_that_turn_something_up_after_a_narration_nudge_are_not_relapses() {
    let mut ledger = ProgressLedger::default();
    let mut verdicts: Vec<Verdict> = (20..24)
        .map(|n| feed(&mut ledger, REPEATED, vec![creeping_read(n)]))
        .collect();
    assert!(
        nudge_of(Lane::Narration)(&verdicts[3]),
        "{:?}",
        events(&verdicts)
    );
    // Files nobody had read: the model keeps its opening, but it is finding things out.
    verdicts.extend((0..8u64).map(|k| {
        let body = format!("fn module_{k}() {{}}\npub struct Thing{k};");
        feed(
            &mut ledger,
            REPEATED,
            vec![read_of(900 + k, &format!("src/module_{k}.rs"), &body)],
        )
    }));
    assert!(stops(&verdicts).is_empty(), "{:?}", events(&verdicts));
}

#[test]
fn the_same_opening_over_output_that_only_looked_new_is_still_a_relapse() {
    let mut ledger = ProgressLedger::default();
    let mut verdicts: Vec<Verdict> = (20..24)
        .map(|n| feed(&mut ledger, REPEATED, vec![creeping_read(n)]))
        .collect();
    assert!(
        nudge_of(Lane::Narration)(&verdicts[3]),
        "{:?}",
        events(&verdicts)
    );
    verdicts.extend((0..8).map(|k| feed(&mut ledger, REPEATED, vec![novel(500 + k)])));
    assert!(!stops(&verdicts).is_empty(), "{:?}", events(&verdicts));
}

const MANIFEST: &str = "[package]\nedition = \"2021\"\nversion.workspace = true\nlicense.workspace = true\nrust-version.workspace = true\n[dependencies]\nserde.workspace = true";

#[test]
fn the_first_read_of_each_file_is_information_however_alike_the_files_are() {
    let steps: Vec<_> = (0..25)
        .map(|i| {
            let body = format!("{MANIFEST}\nname = \"crate{i}\"");
            (
                "Let me read the manifest of the next crate to see its dependencies.".to_string(),
                vec![read_of(i as u64, &format!("crates/c{i}/Cargo.toml"), &body)],
            )
        })
        .collect();
    let verdicts = run(steps);
    assert!(!any_event(&verdicts), "{:?}", events(&verdicts));
}

#[test]
fn reading_the_same_files_again_is_not_a_first_read() {
    let mut ledger = ProgressLedger::default();
    let mut verdicts = Vec::new();
    for pass in 0..2u64 {
        for i in 0..8u64 {
            let body = format!("{MANIFEST}\nname = \"crate{i}\"");
            verdicts.push(feed(
                &mut ledger,
                "Let me read the manifest of the next crate to see its dependencies.",
                vec![read_of(
                    pass * 100 + i,
                    &format!("crates/c{i}/Cargo.toml"),
                    &body,
                )],
            ));
        }
    }
    assert!(
        first(&verdicts, |v| matches!(v, Verdict::Nudge(_))).is_some(),
        "{:?}",
        events(&verdicts)
    );
}

// ---- silent shell work and writes --------------------------------------------------------------

#[test]
fn distinct_silent_commands_are_actions_but_the_same_one_again_is_a_loop() {
    let steps: Vec<_> = (0..40)
        .map(|i| (distinct(i), vec![silent_shell(i as u64)]))
        .collect();
    assert!(!any_event(&run(steps)));
    let steps: Vec<_> = (0..8)
        .map(|i| (distinct(i), vec![silent_shell(1)]))
        .collect();
    assert!(first(&run(steps), nudge_of(Lane::Cycle)).is_some());
}

#[test]
fn a_failed_write_does_not_make_its_arguments_old() {
    let mut ledger = ProgressLedger::default();
    feed(
        &mut ledger,
        "editing",
        vec![obs(7, false, true, "error: no match for old_string")],
    );
    assert!(ledger.last_progress.is_none());
    feed(
        &mut ledger,
        "editing again",
        vec![obs(7, true, true, "edited src/lib.rs")],
    );
    assert_eq!(ledger.last_progress.as_ref().map(|p| p.0), Some(2));
}

// ---- harness notes -----------------------------------------------------------------------------

#[test]
fn a_persistent_hint_comes_back_after_a_while_and_after_a_compaction() {
    let mut ledger = ProgressLedger::default();
    for _ in 0..3 {
        assert!(ledger.admit_note(MessageKind::Hint, "lsp: 1 error in lib.rs"));
    }
    assert!(!ledger.admit_note(MessageKind::Hint, "lsp: 1 error in lib.rs"));
    for i in 0..NOTE_REMIND_AFTER_STEPS as usize {
        feed(&mut ledger, &distinct(i), vec![novel(i)]);
    }
    assert!(
        ledger.admit_note(MessageKind::Hint, "lsp: 1 error in lib.rs"),
        "the error is still there: say so again"
    );
    let mut other = ProgressLedger::default();
    for _ in 0..4 {
        other.admit_note(MessageKind::Hint, "same");
    }
    other.note_compaction();
    assert!(
        other.admit_note(MessageKind::Hint, "same"),
        "the model lost the earlier copies"
    );
}
