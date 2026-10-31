//! Every loop pattern the ledger recognises, and the long productive turns it must leave alone.

use super::*;
use crate::progress_report::Lane;

pub(super) const REPEATED: &str =
    "You're right — I looped. Answering the question from evidence, then finishing the revert.";

/// Narration with nothing in common from one step to the next, so wording never counts as a
/// repeat.
pub(super) fn distinct(i: usize) -> String {
    format!("alpha{i} beta{i} gamma{i} delta{i} epsilon{i} zeta{i}")
}

pub(super) fn obs(sig: u64, ok: bool, mutating: bool, result: &str) -> CallObservation {
    CallObservation::new(sig, ok, mutating, result)
}

/// A successful read whose result shows lines nothing else printed.
pub(super) fn novel(n: usize) -> CallObservation {
    obs(
        1_000_000 + n as u64,
        true,
        false,
        &format!("first line of {n}\nsecond line of {n}\nthird line of {n}"),
    )
}

/// A read of something already seen: a different call (so no cycle) with a result already shown.
pub(super) fn stale(n: usize) -> CallObservation {
    let text: String = (0..20).map(|i| format!("old line {i}\n")).collect();
    obs(2_000_000 + n as u64, true, false, &text)
}

/// A read that shows `n` lines of a file, the first `n - 1` of which an earlier read already
/// showed (`limit` creeping up by one).
pub(super) fn creeping_read(n: usize) -> CallObservation {
    let text: String = (0..n).map(|i| format!("line {i} of the file\n")).collect();
    obs(3_000_000 + n as u64, true, false, &text)
}

pub(super) fn edit(n: usize) -> CallObservation {
    obs(4_000_000 + n as u64, true, true, "edited src/lib.rs")
}

pub(super) fn failing(n: usize, tool: &str) -> CallObservation {
    obs(5_000_000 + n as u64, false, false, "error: nothing matched").named(tool)
}

pub(super) fn wait(n: usize) -> CallObservation {
    obs(
        6_000_000 + n as u64,
        true,
        false,
        "shell: not ready after 100s (last exit 1)",
    )
    .waiting_for_job()
}

pub(super) fn feed(
    ledger: &mut ProgressLedger,
    text: &str,
    calls: Vec<CallObservation>,
) -> Verdict {
    ledger.observe_step(text, &calls, false, &mut || None)
}

/// A scripted working tree: each sample returns the next state, and the last one again after
/// that.
pub(super) fn tree_of(states: &[u64]) -> impl FnMut() -> Option<u64> + '_ {
    let mut next = 0;
    move || {
        let state = states[next.min(states.len() - 1)];
        next += 1;
        Some(state)
    }
}

/// 1-based index of the first verdict satisfying `pred`.
pub(super) fn first(verdicts: &[Verdict], pred: impl Fn(&Verdict) -> bool) -> Option<usize> {
    verdicts.iter().position(pred).map(|i| i + 1)
}

pub(super) fn nudge_of(lane: Lane) -> impl Fn(&Verdict) -> bool {
    move |v| matches!(v, Verdict::Nudge(r) if r.lane == lane)
}

pub(super) fn stop_of(lane: Lane) -> impl Fn(&Verdict) -> bool {
    move |v| matches!(v, Verdict::Stop(r) if r.lane == lane)
}

pub(super) fn any_event(verdicts: &[Verdict]) -> bool {
    verdicts.iter().any(|v| *v != Verdict::Fine)
}

pub(super) fn run(steps: Vec<(String, Vec<CallObservation>)>) -> Vec<Verdict> {
    let mut ledger = ProgressLedger::default();
    steps
        .into_iter()
        .map(|(text, calls)| feed(&mut ledger, &text, calls))
        .collect()
}

#[test]
fn a_long_productive_turn_is_never_touched() {
    // 300 steps that edit a new place, run a check, read something new: nothing to say.
    let steps: Vec<_> = (0..300)
        .map(|i| {
            let calls = match i % 3 {
                0 => vec![edit(i)],
                1 => vec![novel(i)],
                _ => vec![obs(
                    7_000_000 + i as u64,
                    true,
                    false,
                    &format!("test {i} ... ok"),
                )],
            };
            (
                "Now let me update the parser to cover the next case".to_string(),
                calls,
            )
        })
        .collect();
    assert!(!any_event(&run(steps)));
}

#[test]
fn an_edit_every_other_step_with_one_opening_never_nudges() {
    let steps: Vec<_> = (0..300)
        .map(|i| {
            let calls = if i % 2 == 0 {
                vec![edit(i)]
            } else {
                vec![obs(
                    7_000_000 + i as u64,
                    true,
                    false,
                    &format!("test case_{i} ... ok"),
                )]
            };
            (
                "Now let me update the parser to cover the next case".to_string(),
                calls,
            )
        })
        .collect();
    assert!(!any_event(&run(steps)));
}

#[test]
fn twelve_stale_steps_are_nudged_at_five_and_stopped_at_ten() {
    let steps: Vec<_> = (0..12).map(|i| (distinct(i), vec![stale(i)])).collect();
    let verdicts = run(steps);
    // The first read is informative (first sight), so the stale run starts at step 2.
    assert_eq!(
        first(&verdicts, nudge_of(Lane::Futile)),
        Some(6),
        "{verdicts:?}"
    );
    assert_eq!(
        first(&verdicts, stop_of(Lane::Futile)),
        Some(11),
        "{verdicts:?}"
    );
}

#[test]
fn six_failed_steps_on_one_tool_are_nudged_at_three_and_stopped_at_six() {
    let steps: Vec<_> = (0..8)
        .map(|i| (distinct(i), vec![failing(i, "edit_file")]))
        .collect();
    let verdicts = run(steps);
    assert_eq!(
        first(&verdicts, nudge_of(Lane::Failure)),
        Some(3),
        "{verdicts:?}"
    );
    assert_eq!(
        first(&verdicts, stop_of(Lane::Failure)),
        Some(6),
        "{verdicts:?}"
    );
    let Verdict::Nudge(report) = &verdicts[2] else {
        panic!("{verdicts:?}")
    };
    let failing = report
        .failing
        .as_ref()
        .expect("the nudge names the failure");
    assert_eq!(failing.0, "edit_file");
    assert_eq!(failing.1, "error: nothing matched");
}

#[test]
fn a_success_between_failures_breaks_the_failure_streak() {
    let mut steps = Vec::new();
    for i in 0..10 {
        steps.push((distinct(2 * i), vec![failing(i, "edit_file")]));
        steps.push((distinct(2 * i + 1), vec![novel(i)]));
    }
    let verdicts = run(steps);
    assert!(
        first(&verdicts, nudge_of(Lane::Failure)).is_none(),
        "{verdicts:?}"
    );
}

#[test]
fn the_same_call_with_the_same_result_is_nudged_at_three_and_stopped_at_five() {
    let same = || {
        obs(
            9,
            true,
            false,
            "contents of a.rs\nfn a() {}\nfn b() {}\nfn c() {}",
        )
    };
    let steps: Vec<_> = (0..7).map(|i| (distinct(i), vec![same()])).collect();
    let verdicts = run(steps);
    assert_eq!(
        first(&verdicts, nudge_of(Lane::Cycle)),
        Some(3),
        "{verdicts:?}"
    );
    assert_eq!(
        first(&verdicts, stop_of(Lane::Cycle)),
        Some(5),
        "{verdicts:?}"
    );
}

#[test]
fn polling_with_a_changing_result_is_not_a_cycle() {
    let steps: Vec<_> = (0..40)
        .map(|i| {
            (
                distinct(i),
                vec![obs(
                    9,
                    true,
                    false,
                    &format!("job state\nprogress {i} of 40\nstill running {i}"),
                )],
            )
        })
        .collect();
    let verdicts = run(steps);
    assert!(
        first(&verdicts, nudge_of(Lane::Cycle)).is_none(),
        "{verdicts:?}"
    );
}

#[test]
fn alternating_between_two_calls_with_identical_results_is_a_cycle() {
    let a = "contents of a.rs\nfn a() {}\nfn b() {}\nfn c() {}";
    let b = "contents of b.rs\nfn x() {}\nfn y() {}\nfn z() {}";
    let steps: Vec<_> = (0..10)
        .map(|i| {
            let call = if i % 2 == 0 {
                obs(1, true, false, a)
            } else {
                obs(2, true, false, b)
            };
            (distinct(i), vec![call])
        })
        .collect();
    let verdicts = run(steps);
    assert_eq!(
        first(&verdicts, nudge_of(Lane::Cycle)),
        Some(5),
        "{verdicts:?}"
    );
    assert_eq!(
        first(&verdicts, stop_of(Lane::Cycle)),
        Some(7),
        "{verdicts:?}"
    );
}

#[test]
fn device_polling_with_changing_results_is_never_touched() {
    let steps: Vec<_> = (0..40)
        .map(|i| {
            let calls = vec![
                obs(10_000 + i as u64, true, false, "tapped"),
                obs(
                    1,
                    true,
                    false,
                    &format!("screen {i}\nbutton Next\nlabel Step {i} of 40\nprogress {i}%"),
                ),
            ];
            (
                "Tapping the next control and checking the screen".to_string(),
                calls,
            )
        })
        .collect();
    assert!(!any_event(&run(steps)));
}

#[test]
fn device_polling_whose_screen_never_changes_is_still_a_loop() {
    let steps: Vec<_> = (0..20)
        .map(|i| {
            let calls = vec![
                obs(10_000 + i as u64, true, false, "tapped"),
                obs(1, true, false, "screen home\nbutton Next\nlabel Welcome"),
            ];
            (distinct(i), calls)
        })
        .collect();
    let verdicts = run(steps);
    assert!(first(&verdicts, |v| matches!(v, Verdict::Stop(_))).is_some_and(|s| s <= 12));
}

#[test]
fn rereading_with_a_creeping_limit_and_one_opening_is_stopped_by_step_six() {
    let steps: Vec<_> = (20..40)
        .map(|n| (REPEATED.to_string(), vec![creeping_read(n)]))
        .collect();
    let verdicts = run(steps);
    assert_eq!(
        first(&verdicts, nudge_of(Lane::Narration)),
        Some(4),
        "{verdicts:?}"
    );
    assert_eq!(
        first(&verdicts, stop_of(Lane::Narration)),
        Some(6),
        "{verdicts:?}"
    );
}

#[test]
fn varying_only_the_quoted_query_of_an_empty_search_is_not_new() {
    let steps: Vec<_> = (0..12)
        .map(|i| {
            (
                REPEATED.to_string(),
                vec![obs(
                    i,
                    true,
                    false,
                    &format!("no matches for 'pattern_{i}'"),
                )],
            )
        })
        .collect();
    let verdicts = run(steps);
    assert!(first(&verdicts, |v| matches!(v, Verdict::Stop(_))).is_some());
}

#[test]
fn failed_calls_and_repeated_mutations_are_not_progress() {
    let mut ledger = ProgressLedger::default();
    assert_eq!(
        feed(
            &mut ledger,
            REPEATED,
            vec![obs(7, true, true, "edited src/lib.rs")]
        ),
        Verdict::Fine
    );
    for _ in 0..2 {
        feed(
            &mut ledger,
            REPEATED,
            vec![obs(7, true, true, "edited src/lib.rs")],
        );
        feed(
            &mut ledger,
            REPEATED,
            vec![obs(8, false, true, "error: no match")],
        );
    }
    // A short error is a cause, not information, so it never breaks the run.
    assert_eq!(ledger.futile_run, 4, "{}", ledger.futile_run);
    assert_eq!(ledger.idle_steps, 4);
}

#[test]
fn a_gate_redrive_makes_the_next_step_neutral() {
    let mut ledger = ProgressLedger::default();
    feed(&mut ledger, &distinct(0), vec![novel(0)]);
    ledger.note_gate();
    for i in 1..=5 {
        let verdict = feed(&mut ledger, &distinct(i), vec![stale(i)]);
        assert_eq!(verdict, Verdict::Fine, "step {i}");
    }
    assert_eq!(ledger.futile_run, 4, "the gated step did not count");
    assert!(matches!(
        feed(&mut ledger, &distinct(6), vec![stale(6)]),
        Verdict::Nudge(_)
    ));
}

#[test]
fn a_gate_does_not_hide_real_progress() {
    let mut ledger = ProgressLedger::default();
    for i in 0..4 {
        feed(&mut ledger, &distinct(i), vec![stale(i)]);
    }
    ledger.note_gate();
    feed(&mut ledger, &distinct(9), vec![edit(9)]);
    assert_eq!(ledger.futile_run, 0);
    assert_eq!(ledger.last_progress.as_ref().map(|p| p.0), Some(5));
}

#[test]
fn poll_waits_are_neutral_and_only_the_waiting_lane_counts_them() {
    let steps: Vec<_> = (0..200)
        .map(|i| (REPEATED.to_string(), vec![wait(i)]))
        .collect();
    assert_eq!(events(&run(steps)), vec![(24, "nudge", "waiting")]);
    let steps: Vec<_> = (0..23)
        .map(|i| (REPEATED.to_string(), vec![wait(i)]))
        .collect();
    assert!(!any_event(&run(steps)));
}

#[test]
fn a_wait_batched_with_another_call_is_still_a_wait() {
    let mut ledger = ProgressLedger::default();
    for i in 0..8 {
        feed(&mut ledger, &distinct(i), vec![wait(i), stale(i)]);
    }
    assert_eq!((ledger.futile_run, ledger.wait_steps), (0, 8));
}

#[test]
fn a_hundred_and_fifty_informative_steps_are_nudged_at_sixty_and_never_stopped_attended() {
    let steps: Vec<_> = (0..150).map(|i| (distinct(i), vec![novel(i)])).collect();
    let verdicts = run(steps);
    assert_eq!(
        first(&verdicts, nudge_of(Lane::Idle)),
        Some(60),
        "{verdicts:?}"
    );
    assert!(first(&verdicts, |v| matches!(v, Verdict::Stop(_))).is_none());
    assert_eq!(
        verdicts.iter().filter(|v| **v != Verdict::Fine).count(),
        1,
        "one nudge per episode, however long it runs"
    );
}

#[test]
fn an_unattended_read_only_turn_is_stopped_at_two_hundred() {
    let mut ledger = ProgressLedger::default();
    ledger.configure(60, 200, true);
    let verdicts: Vec<Verdict> = (0..250)
        .map(|i| feed(&mut ledger, &distinct(i), vec![novel(i)]))
        .collect();
    assert_eq!(first(&verdicts, nudge_of(Lane::Idle)), Some(60));
    assert_eq!(
        first(&verdicts, stop_of(Lane::Idle)),
        Some(200),
        "{verdicts:?}"
    );
}

#[test]
fn the_idle_thresholds_are_configurable_and_zero_disables_them() {
    let mut ledger = ProgressLedger::default();
    ledger.configure(5, 8, true);
    let verdicts: Vec<Verdict> = (0..12)
        .map(|i| feed(&mut ledger, &distinct(i), vec![novel(i)]))
        .collect();
    assert_eq!(first(&verdicts, nudge_of(Lane::Idle)), Some(5));
    assert_eq!(first(&verdicts, stop_of(Lane::Idle)), Some(8));
    let mut off = ProgressLedger::default();
    off.configure(0, 0, true);
    let verdicts: Vec<Verdict> = (0..400)
        .map(|i| feed(&mut off, &distinct(i), vec![novel(i)]))
        .collect();
    assert!(!any_event(&verdicts));
}

#[test]
fn fifty_nine_steps_a_strong_step_and_fifty_nine_more_say_nothing() {
    let mut steps: Vec<_> = (0..59).map(|i| (distinct(i), vec![novel(i)])).collect();
    steps.push((distinct(59), vec![edit(59)]));
    steps.extend((60..119).map(|i| (distinct(i), vec![novel(i)])));
    assert!(!any_event(&run(steps)));
}

#[test]
fn informative_steps_break_a_futile_run_before_it_reaches_the_bar() {
    let mut steps = Vec::new();
    for round in 0..11 {
        for k in 0..4 {
            steps.push((distinct(round * 5 + k), vec![stale(round * 5 + k)]));
        }
        steps.push((distinct(round * 5 + 4), vec![novel(round)]));
    }
    let verdicts = run(steps);
    assert!(!any_event(&verdicts), "{verdicts:?}");
}

#[test]
fn a_passing_check_after_a_failing_one_is_strong_progress() {
    let mut ledger = ProgressLedger::default();
    for i in 0..4 {
        feed(&mut ledger, &distinct(i), vec![stale(i)]);
    }
    let verdict = feed(
        &mut ledger,
        &distinct(10),
        vec![stale(10).flipping_a_check()],
    );
    assert_eq!(verdict, Verdict::Fine);
    assert_eq!(ledger.futile_run, 0);
    assert_eq!(ledger.idle_steps, 0);
}

#[test]
fn a_closed_task_and_a_steer_are_strong_progress() {
    let mut ledger = ProgressLedger::default();
    for i in 0..4 {
        feed(&mut ledger, &distinct(i), vec![stale(i)]);
    }
    ledger.observe_step(&distinct(5), &[stale(5)], true, &mut || None);
    assert_eq!((ledger.futile_run, ledger.idle_steps), (0, 0));
    for i in 6..9 {
        feed(&mut ledger, &distinct(i), vec![stale(i)]);
    }
    ledger.note_steer();
    feed(&mut ledger, &distinct(10), vec![stale(10)]);
    assert_eq!((ledger.futile_run, ledger.idle_steps), (0, 0));
}

#[test]
fn a_novel_step_after_the_nudge_clears_it_so_a_later_loop_gets_its_own() {
    let mut ledger = ProgressLedger::default();
    let mut verdicts = Vec::new();
    for n in 20..25 {
        verdicts.push(feed(&mut ledger, REPEATED, vec![creeping_read(n)]));
    }
    assert!(nudge_of(Lane::Narration)(&verdicts[3]), "{verdicts:?}");
    assert_eq!(feed(&mut ledger, "Found it", vec![edit(99)]), Verdict::Fine);
    let again: Vec<Verdict> = (30..36)
        .map(|n| feed(&mut ledger, REPEATED, vec![creeping_read(n)]))
        .collect();
    assert!(
        again.iter().any(nudge_of(Lane::Narration)),
        "second loop was not nudged again: {again:?}"
    );
}

#[test]
fn a_nudge_is_sent_once_per_episode_whichever_lane_fires_first() {
    let mut ledger = ProgressLedger::default();
    let verdicts: Vec<Verdict> = (0..9)
        .map(|i| feed(&mut ledger, REPEATED, vec![failing(i, "edit_file")]))
        .collect();
    let nudges = verdicts
        .iter()
        .filter(|v| matches!(v, Verdict::Nudge(_)))
        .count();
    assert_eq!(nudges, 1, "{verdicts:?}");
}

#[test]
fn a_compaction_forgets_what_was_read_but_not_the_streak() {
    let mut ledger = ProgressLedger::default();
    for n in 20..24 {
        feed(&mut ledger, REPEATED, vec![creeping_read(n)]);
    }
    assert!(matches!(ledger.episode, Episode::Nudged { .. }));
    ledger.note_compaction();
    // The same file read again after the compaction is new information for a model that lost it
    // (so not stale), but the pending nudge survives it.
    assert_eq!(
        feed(&mut ledger, REPEATED, vec![creeping_read(23)]),
        Verdict::Fine
    );
    assert!(matches!(ledger.episode, Episode::Nudged { .. }));
    let verdicts: Vec<Verdict> = (24..34)
        .map(|n| feed(&mut ledger, REPEATED, vec![creeping_read(n)]))
        .collect();
    assert!(
        verdicts.iter().any(|v| matches!(v, Verdict::Stop(_))),
        "{verdicts:?}"
    );
}

#[test]
fn a_new_user_turn_gets_a_fresh_budget_but_a_loop_is_still_caught() {
    let mut ledger = ProgressLedger::default();
    for n in 20..24 {
        feed(&mut ledger, REPEATED, vec![creeping_read(n)]);
    }
    ledger.begin_turn();
    assert_eq!(
        feed(&mut ledger, REPEATED, vec![creeping_read(20)]),
        Verdict::Fine,
        "the first step of a new turn must never stop on the previous turn's evidence"
    );
    let verdicts: Vec<Verdict> = (24..40)
        .map(|n| feed(&mut ledger, REPEATED, vec![creeping_read(n)]))
        .collect();
    assert!(
        verdicts.iter().any(|v| matches!(v, Verdict::Stop(_))),
        "{verdicts:?}"
    );
}

#[test]
fn rereading_old_output_in_a_new_turn_does_not_stop_it() {
    let mut ledger = ProgressLedger::default();
    for n in 0..WINDOW_STEPS + 4 {
        feed(
            &mut ledger,
            "",
            vec![obs(
                n as u64,
                true,
                false,
                "the same old file contents\nand more",
            )],
        );
    }
    ledger.begin_turn();
    for n in 0..FUTILE_NUDGE_AT - 1 {
        assert_eq!(
            feed(
                &mut ledger,
                "",
                vec![obs(
                    1000 + n as u64,
                    true,
                    false,
                    "the same old file contents\nand more"
                )]
            ),
            Verdict::Fine
        );
    }
}

#[test]
fn a_continue_that_repeats_the_stopped_call_is_stopped_at_once() {
    let mut ledger = ProgressLedger::default();
    let same = || obs(9, true, false, "contents\nof the file\nthat never\nchanges");
    let verdicts: Vec<Verdict> = (0..6)
        .map(|i| feed(&mut ledger, &distinct(i), vec![same()]))
        .collect();
    assert!(first(&verdicts, |v| matches!(v, Verdict::Stop(_))).is_some());
    ledger.stopped();
    ledger.begin_turn();
    let resumed = feed(&mut ledger, &distinct(100), vec![same()]);
    assert!(stop_of(Lane::Resumed)(&resumed), "{resumed:?}");
}

#[test]
fn a_continue_that_changes_course_is_left_alone_and_the_watch_expires() {
    let mut ledger = ProgressLedger::default();
    let same = || obs(9, true, false, "contents\nof the file\nthat never\nchanges");
    for i in 0..6 {
        feed(&mut ledger, &distinct(i), vec![same()]);
    }
    ledger.stopped();
    ledger.begin_turn();
    for i in 0..RESUME_WATCH_STEPS as usize {
        assert_eq!(
            feed(&mut ledger, &distinct(100 + i), vec![novel(i)]),
            Verdict::Fine
        );
    }
    // Past the watch window the same call is just a call again.
    assert_eq!(
        feed(&mut ledger, &distinct(200), vec![same()]),
        Verdict::Fine
    );
}

#[test]
fn a_continue_that_repeats_the_stopped_opening_is_stopped_at_once() {
    let mut ledger = ProgressLedger::default();
    let verdicts: Vec<Verdict> = (20..40)
        .map(|n| feed(&mut ledger, REPEATED, vec![creeping_read(n)]))
        .collect();
    assert!(first(&verdicts, |v| matches!(v, Verdict::Stop(_))).is_some());
    ledger.stopped();
    ledger.begin_turn();
    // The same words over lines it had already read.
    let resumed = feed(&mut ledger, REPEATED, vec![creeping_read(30)]);
    assert!(stop_of(Lane::Resumed)(&resumed), "{resumed:?}");
}

#[test]
fn real_progress_after_a_stop_clears_the_resume_watch() {
    let mut ledger = ProgressLedger::default();
    let same = || obs(9, true, false, "contents\nof the file\nthat never\nchanges");
    for i in 0..6 {
        feed(&mut ledger, &distinct(i), vec![same()]);
    }
    ledger.stopped();
    ledger.begin_turn();
    feed(&mut ledger, &distinct(100), vec![edit(1)]);
    assert_eq!(
        feed(&mut ledger, &distinct(101), vec![same()]),
        Verdict::Fine
    );
}

#[test]
fn a_stop_that_is_only_reported_is_not_repeated_in_the_episode() {
    let mut ledger = ProgressLedger::default();
    let mut stops = 0;
    for i in 0..30 {
        if let Verdict::Stop(_) = feed(&mut ledger, &distinct(i), vec![stale(i)]) {
            stops += 1;
            ledger.suppress_stop();
        }
    }
    assert_eq!(stops, 1, "warn mode reports a stop once per episode");
}

#[test]
fn identical_harness_notes_are_dropped_after_the_third_copy() {
    let mut ledger = ProgressLedger::default();
    for copy in 1..=3 {
        assert!(
            ledger.admit_note(MessageKind::Hint, "same hint"),
            "copy {copy}"
        );
    }
    assert!(!ledger.admit_note(MessageKind::Hint, "same hint"));
    assert!(ledger.admit_note(MessageKind::Hint, "another hint"));
    assert!(
        ledger.admit_note(MessageKind::LoopGuard, "same hint"),
        "counted per kind"
    );
    for _ in 0..10 {
        assert!(
            ledger.admit_note(MessageKind::Clock, "tick"),
            "only the harness-talking-to-itself kinds are limited"
        );
    }
    ledger.begin_turn();
    assert!(ledger.admit_note(MessageKind::Hint, "same hint"));
}

#[test]
fn the_report_names_what_repeated_and_what_failed() {
    let mut ledger = ProgressLedger::default();
    let same = || obs(9, true, false, "contents\nof the file\nthat never\nchanges");
    let mut report = None;
    for i in 0..4 {
        if let Verdict::Nudge(r) = feed(&mut ledger, &distinct(i), vec![same()]) {
            report = Some(r);
        }
    }
    let report = report.expect("a cycle nudge");
    assert_eq!(report.lane, Lane::Cycle);
    let (label, times) = report.repeated.expect("the repeated call is named");
    assert!(label.starts_with("call "), "{label}");
    assert!(times >= 3);
}

#[test]
fn explicit_model_call_counting_spans_the_turn() {
    let mut ledger = ProgressLedger::default();
    assert_eq!(ledger.begin_model_call(), 0);
    assert_eq!(ledger.begin_model_call(), 1);
    assert_eq!(ledger.model_calls(), 2);
    ledger.begin_turn();
    assert_eq!(ledger.model_calls(), 0);
}

#[test]
fn pressure_tracks_stale_repetition_only() {
    let mut ledger = ProgressLedger::default();
    for i in 0..6 {
        feed(
            &mut ledger,
            "Now let me update the parser to cover the next case",
            vec![edit(i)],
        );
    }
    assert_eq!(
        ledger.pressure(),
        0,
        "productive repetition never runs hotter"
    );
    feed(&mut ledger, REPEATED, vec![creeping_read(20)]);
    feed(&mut ledger, REPEATED, vec![creeping_read(21)]);
    assert_eq!(ledger.pressure(), 1);
    feed(&mut ledger, REPEATED, vec![creeping_read(22)]);
    feed(&mut ledger, REPEATED, vec![creeping_read(23)]);
    assert_eq!(ledger.pressure(), 2);
}

#[test]
fn an_idle_nudge_does_not_run_the_sampling_hot() {
    let mut ledger = ProgressLedger::default();
    for i in 0..80 {
        feed(&mut ledger, &distinct(i), vec![novel(i)]);
    }
    assert!(matches!(
        ledger.episode,
        Episode::Nudged {
            lane: Lane::Idle,
            ..
        }
    ));
    assert_eq!(ledger.pressure(), 0);
}

/// Real looping stretches from the live store (2026-09-08 and 2026-09-17), reduced to the shape
/// the ledger sees: narration openings, call signatures, success flags, and result lines interned
/// to ids (one blob per distinct result). Anonymised: no file contents, paths or credentials
/// survive.
fn replay(name: &str) -> (Vec<Verdict>, usize) {
    replay_in(include_str!("../tests/fixtures/loop_replays.json"), name)
}

/// Whole bench sessions reduced the same way (`tests/fixtures/bench_replays.json`): hash-only,
/// narration words and result lines replaced by opaque ids, with the harness gates, poll waits and
/// task closures the live loop would have told the ledger about.
fn bench_replay(name: &str) -> (Vec<Verdict>, usize) {
    replay_in(include_str!("../tests/fixtures/bench_replays.json"), name)
}

/// An id as letters: the ledger masks digits in failures, so ids made of digits would all read
/// as one line.
fn letters(mut id: u64) -> String {
    let mut out = String::new();
    loop {
        out.push((b'a' + (id % 26) as u8) as char);
        id /= 26;
        if id == 0 {
            return out;
        }
    }
}

fn replay_in(fixture: &str, name: &str) -> (Vec<Verdict>, usize) {
    let all: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let seg = all
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("no replay named {name}"));
    let blobs: Vec<String> = seg["blobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|runs| {
            let mut text = String::new();
            for run in runs.as_array().unwrap() {
                let (start, count) = (run[0].as_u64().unwrap(), run[1].as_u64().unwrap());
                for id in start..start + count {
                    text.push_str(&format!("L{}\n", letters(id)));
                }
            }
            text
        })
        .collect();
    let steps = seg["steps"].as_array().unwrap();
    let mut ledger = ProgressLedger::default();
    let verdicts = steps
        .iter()
        .map(|step| {
            let calls: Vec<CallObservation> = step["calls"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    let call = obs(
                        c["sig"].as_u64().unwrap(),
                        c["ok"].as_bool().unwrap(),
                        c["mut"].as_bool().unwrap(),
                        &blobs[c["res"].as_u64().unwrap() as usize],
                    );
                    if c["wait"].as_bool().unwrap_or(false) {
                        call.waiting_for_job()
                    } else {
                        call
                    }
                })
                .collect();
            if step["gate"].as_bool().unwrap_or(false) {
                ledger.note_gate();
            }
            let closed = step["closed"].as_bool().unwrap_or(false);
            ledger.observe_step(step["text"].as_str().unwrap(), &calls, closed, &mut || None)
        })
        .collect();
    (verdicts, steps.len())
}

/// The Sep 8 apology loop: 35 steps, no edit, the same `You're right — I looped` opening from
/// step 6 on, alternating a re-read of a 1,349-line file with small `git`/`grep` queries.
#[test]
fn the_apology_loop_is_nudged_by_step_12_and_stopped_by_step_14() {
    let (v, n) = replay("s1_looped_apology");
    assert_eq!(n, 35);
    assert_eq!(first(&v, nudge_of(Lane::Narration)), Some(12), "{v:?}");
    assert_eq!(first(&v, stop_of(Lane::Narration)), Some(14), "{v:?}");
}

/// Two stretches the first guard nudged although the model was working: a sweep-and-push where
/// every shell command printed something new, and a clippy cleanup reading fresh compiler output
/// each time. Neither is a loop.
#[test]
fn working_stretches_the_old_guard_nudged_are_left_alone() {
    for name in ["s2_sweep_and_push", "s3_clippy_shell"] {
        let (v, _) = replay(name);
        assert!(v.iter().all(|v| *v == Verdict::Fine), "{name}: {v:?}");
    }
}

pub(super) fn events(verdicts: &[Verdict]) -> Vec<(usize, &'static str, &'static str)> {
    verdicts
        .iter()
        .enumerate()
        .filter_map(|(i, v)| match v {
            Verdict::Fine => None,
            Verdict::Nudge(r) => Some((i + 1, "nudge", r.lane.as_str())),
            Verdict::Stop(r) => Some((i + 1, "stop", r.lane.as_str())),
        })
        .collect()
}

/// The acceptance gate for replacing the old guards: across whole bench sessions that were solved,
/// no stop, and no nudge the old failure and doom guards did not also send.
#[test]
fn no_solved_bench_turn_is_ever_stopped() {
    for name in BENCH_NAMES {
        let (verdicts, steps) = bench_replay(name);
        assert!(steps >= 10, "{name}");
        assert!(
            !verdicts.iter().any(|v| matches!(v, Verdict::Stop(_))),
            "{name}: {:?}",
            events(&verdicts)
        );
    }
}

/// Four sessions the old failure guard nudged although the model was working: its count ran
/// across the turn (3 failures of one tool and kind, with successes in between), where the ledger
/// needs three failed steps in a row. The empty `commands` array behind most of them is fixed at
/// the source.
#[test]
fn the_failure_nudges_that_were_false_alarms_do_not_fire() {
    for name in &BENCH_NAMES[1..5] {
        let (verdicts, _) = bench_replay(name);
        assert_eq!(events(&verdicts), vec![], "{name}");
    }
}

/// A session that did loop: `shell` failed three times running with the same error. The ledger
/// nudges on the third failed step, as the old guard did, and the model recovered.
#[test]
fn a_true_failure_streak_is_nudged_on_the_third_failed_step() {
    let (verdicts, _) = bench_replay(BENCH_NAMES[5]);
    assert_eq!(events(&verdicts), vec![(5, "nudge", "failure")]);
}

/// 46 steps of real work with 12 failed ones scattered through it. Three failed steps do come in a
/// row once (22 to 24), but with two different errors, so it is not the same failure again: no
/// nudge and no stop.
#[test]
fn a_long_session_with_scattered_failures_says_nothing() {
    let (verdicts, steps) = bench_replay(BENCH_NAMES[9]);
    assert_eq!(steps, 46);
    assert_eq!(events(&verdicts), vec![]);
}

/// Long productive sessions say nothing at all.
#[test]
fn long_productive_bench_sessions_are_left_alone() {
    for name in [
        BENCH_NAMES[0],
        BENCH_NAMES[6],
        BENCH_NAMES[7],
        BENCH_NAMES[8],
    ] {
        let (verdicts, _) = bench_replay(name);
        assert_eq!(events(&verdicts), vec![], "{name}");
    }
}

const BENCH_NAMES: [&str; 10] = [
    "b1_gate_redrive_stale_steps",
    "b2_failure_nudge_false_positive_a",
    "b3_failure_nudge_false_positive_b",
    "b4_failure_nudge_false_positive_c",
    "b5_failure_nudge_false_positive_d",
    "b6_true_early_failure_catch",
    "b7_long_productive_deepseek_a",
    "b8_long_productive_deepseek_b",
    "b9_long_productive_codex_a",
    "b10_long_productive_codex_b",
];

/// Deterministic pseudo-random streams: the properties hold for whatever order of step kinds the
/// seed produces.
mod properties {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Kind {
        Edit,
        Novel,
        Stale,
        Failed,
        Wait,
    }

    fn stream(seed: u64, len: usize) -> Vec<Kind> {
        let mut state = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                match state % 10 {
                    0 => Kind::Edit,
                    1 | 2 => Kind::Novel,
                    3..=6 => Kind::Stale,
                    7 | 8 => Kind::Failed,
                    _ => Kind::Wait,
                }
            })
            .collect()
    }

    fn step(kind: Kind, i: usize) -> Vec<CallObservation> {
        vec![match kind {
            Kind::Edit => edit(i),
            Kind::Novel => novel(i),
            Kind::Stale => stale(i),
            Kind::Failed => failing(i, "edit_file"),
            Kind::Wait => wait(i),
        }]
    }

    fn play(kinds: &[Kind], unattended: bool) -> Vec<Verdict> {
        let mut ledger = ProgressLedger::default();
        ledger.configure(60, 200, unattended);
        kinds
            .iter()
            .enumerate()
            .map(|(i, kind)| feed(&mut ledger, &distinct(i), step(*kind, i)))
            .collect()
    }

    fn shape(v: &Verdict) -> (u8, Option<&'static str>) {
        match v {
            Verdict::Fine => (0, None),
            Verdict::Nudge(r) => (1, Some(r.lane.as_str())),
            Verdict::Stop(r) => (2, Some(r.lane.as_str())),
        }
    }

    #[test]
    fn a_stop_always_follows_a_nudge_by_at_least_two_steps() {
        for seed in 1..200 {
            let kinds = stream(seed, 150);
            let verdicts = play(&kinds, true);
            let mut last_nudge: Option<usize> = None;
            for (i, v) in verdicts.iter().enumerate() {
                match v {
                    Verdict::Nudge(_) => last_nudge = Some(i),
                    Verdict::Stop(_) => {
                        let nudged_at = last_nudge.unwrap_or_else(|| {
                            panic!("seed {seed}: stop at {i} with no nudge before it")
                        });
                        assert!(
                            i - nudged_at >= 2,
                            "seed {seed}: nudge {nudged_at}, stop {i}"
                        );
                    }
                    Verdict::Fine => {}
                }
                if kinds[i] == Kind::Edit {
                    last_nudge = None;
                }
            }
        }
    }

    #[test]
    fn a_strong_step_resets_every_lane() {
        for seed in 1..100 {
            let mut kinds = stream(seed, 80);
            kinds.push(Kind::Edit);
            for tail in 0..4 {
                kinds.push(Kind::Stale);
                let _ = tail;
            }
            let verdicts = play(&kinds, true);
            let after = &verdicts[81..];
            assert!(
                after.iter().all(|v| *v == Verdict::Fine),
                "seed {seed}: events right after an edit: {after:?}"
            );
        }
    }

    #[test]
    fn neutral_steps_never_advance_a_counter() {
        for seed in 1..100 {
            let kinds = stream(seed, 100);
            let without: Vec<Kind> = kinds.iter().copied().filter(|k| *k != Kind::Wait).collect();
            let a: Vec<_> = play(&without, true).iter().map(|v| shape(v)).collect();
            let with_waits = play(&kinds, true);
            let b: Vec<_> = kinds
                .iter()
                .zip(&with_waits)
                .filter(|(k, _)| **k != Kind::Wait)
                .map(|(_, v)| shape(v))
                .collect();
            assert_eq!(a.len(), b.len());
            // The stream positions differ, so the two ledgers hold different per-step signatures;
            // compare event counts rather than per-step shapes.
            let count =
                |s: &[(u8, Option<&str>)], kind: u8| s.iter().filter(|(k, _)| *k == kind).count();
            assert_eq!(count(&a, 1) > 0, count(&b, 1) > 0, "seed {seed}");
            assert_eq!(count(&a, 2) > 0, count(&b, 2) > 0, "seed {seed}");
        }
    }

    #[test]
    fn a_strong_step_every_fifty_nine_never_reaches_the_idle_lane() {
        for seed in 1..100 {
            let mut kinds = stream(seed, 400);
            for (i, k) in kinds.iter_mut().enumerate() {
                if i % 59 == 58 {
                    *k = Kind::Edit;
                }
            }
            let verdicts = play(&kinds, true);
            for v in &verdicts {
                if let Verdict::Nudge(r) | Verdict::Stop(r) = v {
                    assert_ne!(r.lane, Lane::Idle, "seed {seed}");
                }
            }
        }
    }

    #[test]
    fn the_same_stream_always_gives_the_same_verdicts() {
        for seed in 1..50 {
            let kinds = stream(seed, 200);
            assert_eq!(play(&kinds, true), play(&kinds, true), "seed {seed}");
        }
    }

    #[test]
    fn a_stream_of_edits_and_novelty_only_never_fires() {
        for seed in 1..100 {
            let kinds: Vec<Kind> = stream(seed, 300)
                .into_iter()
                .map(|k| {
                    if matches!(k, Kind::Edit | Kind::Novel) {
                        k
                    } else {
                        Kind::Edit
                    }
                })
                .collect();
            assert!(!any_event(&play(&kinds, true)), "seed {seed}");
        }
    }
}

#[test]
fn the_repeat_interrupt_is_not_offered_again_by_the_waiting_step_after_it() {
    let mut ledger = ProgressLedger::default();
    ledger.set_repeat_interrupt(true);
    let text: String = (0..20).map(|i| format!("same line {i}\n")).collect();
    let mut at = None;
    for i in 0..12 {
        feed(&mut ledger, &distinct(i), vec![obs(7, true, false, &text)]);
        if ledger.cycle_run >= 3 && at.is_none() {
            at = Some(ledger.cycle_run);
            assert!(ledger.repeat_label(ledger.cycle_run).is_some());
            break;
        }
    }
    let at = at.expect("identical calls build a cycle run");
    feed(&mut ledger, &distinct(99), vec![wait(0)]);
    assert_eq!(
        ledger.cycle_run, at,
        "the waiting step leaves the count alone"
    );
    assert_eq!(ledger.repeat_label(at), None);
}
