use super::*;
use forge_store::DispatchItemRow;

fn item(idx: i64, status: &str, session: Option<&str>, deps: &[i64]) -> DispatchItemRow {
    DispatchItemRow {
        dispatch_id: "d1".into(),
        idx,
        title: format!("Item {idx}"),
        prompt: format!("do item {idx}"),
        depends_on: deps.to_vec(),
        status: status.into(),
        session_id: session.map(str::to_string),
        outcome: None,
        started_at: None,
        finished_at: None,
    }
}

fn row(status: &str, items: Vec<DispatchItemRow>) -> DispatchRow {
    DispatchRow {
        id: "d1".into(),
        coordinator_session_id: "coord".into(),
        cwd: "/repo".into(),
        prompt: "split it".into(),
        summary: "the plan".into(),
        status: status.into(),
        worktree: true,
        permission_mode: Some("accept-edits".into()),
        max_running: 4,
        max_items: 8,
        created_at: 0,
        updated_at: 0,
        items,
    }
}

fn obs(handle_key: usize, turns_finished: u64, busy: bool) -> Observation {
    Observation {
        handle_key,
        turns_finished,
        busy,
        waiting: false,
        outcome: Some("success".into()),
        stop_reason: Some("final_answer".into()),
        final_reply: "all done".into(),
    }
}

fn live(entries: &[(&str, Observation)]) -> HashMap<String, Observation> {
    entries
        .iter()
        .map(|(id, o)| (id.to_string(), o.clone()))
        .collect()
}

fn running_one() -> DispatchRow {
    row(
        dispatch_status::RUNNING,
        vec![item(1, item_status::RUNNING, Some("s1"), &[])],
    )
}

#[test]
fn a_turn_counted_past_the_baseline_finishes_an_idle_item() {
    let mut trackers = Trackers::new();
    let now = Instant::now();
    let d = running_one();
    assert!(observe(&d, &live(&[("s1", obs(7, 0, true))]), &mut trackers, now).is_empty());
    let changes = observe(&d, &live(&[("s1", obs(7, 1, false))]), &mut trackers, now);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].status, item_status::SUCCEEDED);
    assert_eq!(changes[0].outcome, "final_answer");
    assert_eq!(changes[0].final_reply, "all done");
    assert_eq!(changes[0].session_id, "s1");
}

#[test]
fn a_turn_shorter_than_a_poll_is_still_seen_through_the_counter() {
    let mut trackers = Trackers::new();
    // The supervisor never saw the session busy: the whole turn fell between two passes.
    let changes = observe(
        &running_one(),
        &live(&[("s1", obs(7, 1, false))]),
        &mut trackers,
        Instant::now(),
    );
    assert_eq!(changes.len(), 1);
}

#[test]
fn a_busy_session_is_not_finished_even_when_its_count_grew() {
    let mut trackers = Trackers::new();
    let changes = observe(
        &running_one(),
        &live(&[("s1", obs(7, 1, true))]),
        &mut trackers,
        Instant::now(),
    );
    assert!(
        changes.is_empty(),
        "a queued prompt already started the next turn"
    );
}

#[test]
fn a_session_waiting_on_a_person_is_not_finished() {
    let mut trackers = Trackers::new();
    let mut waiting = obs(7, 1, false);
    waiting.waiting = true;
    let changes = observe(
        &running_one(),
        &live(&[("s1", waiting)]),
        &mut trackers,
        Instant::now(),
    );
    assert!(changes.is_empty());
}

#[test]
fn the_same_count_is_reported_only_once() {
    let mut trackers = Trackers::new();
    let now = Instant::now();
    let seen = live(&[("s1", obs(7, 1, false))]);
    assert_eq!(observe(&running_one(), &seen, &mut trackers, now).len(), 1);
    let succeeded = row(
        dispatch_status::RUNNING,
        vec![item(1, item_status::SUCCEEDED, Some("s1"), &[])],
    );
    assert!(observe(&succeeded, &seen, &mut trackers, now).is_empty());
}

#[test]
fn an_unsuccessful_turn_fails_the_item_with_its_stop_reason() {
    let mut trackers = Trackers::new();
    let mut failed = obs(7, 1, false);
    failed.outcome = Some("failed".into());
    failed.stop_reason = Some("max_steps".into());
    let changes = observe(
        &running_one(),
        &live(&[("s1", failed)]),
        &mut trackers,
        Instant::now(),
    );
    assert_eq!(changes[0].status, item_status::FAILED);
    assert_eq!(changes[0].outcome, "max_steps");
    assert_eq!(changes[0].stop_reason.as_deref(), Some("max_steps"));
}

#[test]
fn a_turn_without_a_stop_reason_records_the_outcome() {
    let mut trackers = Trackers::new();
    let mut bare = obs(7, 1, false);
    bare.stop_reason = None;
    let changes = observe(
        &running_one(),
        &live(&[("s1", bare)]),
        &mut trackers,
        Instant::now(),
    );
    assert_eq!(changes[0].outcome, "success");
}

#[test]
fn a_running_item_without_a_handle_stops_only_after_the_grace_period() {
    let mut trackers = Trackers::new();
    let start = Instant::now();
    let d = running_one();
    let none = HashMap::new();
    assert!(observe(&d, &none, &mut trackers, start).is_empty());
    assert!(observe(&d, &none, &mut trackers, start + Duration::from_secs(9)).is_empty());
    let changes = observe(&d, &none, &mut trackers, start + MISSING_GRACE);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].status, item_status::STOPPED);
    assert_eq!(changes[0].outcome, "session_ended");
}

#[test]
fn a_handle_back_inside_the_grace_period_keeps_the_item_running() {
    let mut trackers = Trackers::new();
    let start = Instant::now();
    let d = running_one();
    let none = HashMap::new();
    // A merge respawn: gone for a moment, then a new driver, then gone again briefly.
    assert!(observe(&d, &none, &mut trackers, start).is_empty());
    let back = live(&[("s1", obs(9, 0, false))]);
    assert!(observe(&d, &back, &mut trackers, start + Duration::from_secs(5)).is_empty());
    assert!(observe(&d, &none, &mut trackers, start + Duration::from_secs(12)).is_empty());
    assert!(
        observe(&d, &none, &mut trackers, start + Duration::from_secs(21)).is_empty(),
        "the grace period restarts from the latest disappearance"
    );
}

#[test]
fn a_respawned_driver_whose_counter_restarted_still_reports_its_next_turn() {
    let mut trackers = Trackers::new();
    let now = Instant::now();
    // The turn is reported while the item runs; the store then records it as succeeded.
    assert_eq!(
        observe(
            &running_one(),
            &live(&[("s1", obs(7, 2, false))]),
            &mut trackers,
            now
        )
        .len(),
        1
    );
    let d = row(
        dispatch_status::RUNNING,
        vec![item(1, item_status::SUCCEEDED, Some("s1"), &[])],
    );
    // A merge-conflict respawn: a new driver whose counter restarted.
    assert!(observe(&d, &live(&[("s1", obs(8, 0, false))]), &mut trackers, now).is_empty());
    let changes = observe(&d, &live(&[("s1", obs(8, 1, false))]), &mut trackers, now);
    assert_eq!(changes.len(), 1, "the new driver's first turn counts");
}

#[test]
fn a_lower_count_on_the_same_handle_resets_the_baseline() {
    let mut trackers = Trackers::new();
    let now = Instant::now();
    let d = running_one();
    assert_eq!(
        observe(&d, &live(&[("s1", obs(7, 3, false))]), &mut trackers, now).len(),
        1
    );
    assert!(observe(&d, &live(&[("s1", obs(7, 0, true))]), &mut trackers, now).is_empty());
    assert_eq!(
        observe(&d, &live(&[("s1", obs(7, 1, false))]), &mut trackers, now).len(),
        1
    );
}

#[test]
fn a_finished_item_that_finishes_again_is_updated() {
    let mut trackers = Trackers::new();
    let now = Instant::now();
    let d = row(
        dispatch_status::DONE,
        vec![item(1, item_status::FAILED, Some("s1"), &[])],
    );
    // The turn that failed it is already accounted for.
    assert!(observe(&d, &live(&[("s1", obs(7, 1, false))]), &mut trackers, now).is_empty());
    // The user prompts the worker again; it works, then finishes.
    assert!(observe(&d, &live(&[("s1", obs(7, 1, true))]), &mut trackers, now).is_empty());
    let changes = observe(&d, &live(&[("s1", obs(7, 2, false))]), &mut trackers, now);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].status, item_status::SUCCEEDED);
}

#[test]
fn a_finished_item_whose_session_is_gone_is_left_alone() {
    let mut trackers = Trackers::new();
    let start = Instant::now();
    let d = row(
        dispatch_status::RUNNING,
        vec![item(1, item_status::SUCCEEDED, Some("s1"), &[])],
    );
    assert!(observe(&d, &HashMap::new(), &mut trackers, start).is_empty());
    assert!(observe(
        &d,
        &HashMap::new(),
        &mut trackers,
        start + MISSING_GRACE * 2
    )
    .is_empty());
}

#[test]
fn items_without_a_session_or_in_a_settled_state_are_not_watched() {
    let mut trackers = Trackers::new();
    let d = row(
        dispatch_status::RUNNING,
        vec![
            item(1, item_status::QUEUED, None, &[]),
            item(2, item_status::MERGED, Some("s2"), &[]),
            item(3, item_status::DISCARDED, Some("s3"), &[]),
        ],
    );
    let seen = live(&[("s2", obs(1, 5, false)), ("s3", obs(2, 5, false))]);
    assert!(observe(&d, &seen, &mut trackers, Instant::now() + MISSING_GRACE).is_empty());
    assert!(trackers.is_empty());
}

#[test]
fn an_item_on_a_new_session_starts_from_a_fresh_baseline() {
    let mut trackers = Trackers::new();
    let now = Instant::now();
    assert_eq!(
        observe(
            &running_one(),
            &live(&[("s1", obs(7, 4, false))]),
            &mut trackers,
            now
        )
        .len(),
        1
    );
    let moved = row(
        dispatch_status::RUNNING,
        vec![item(1, item_status::RUNNING, Some("s9"), &[])],
    );
    assert_eq!(
        observe(
            &moved,
            &live(&[("s9", obs(7, 1, false))]),
            &mut trackers,
            now
        )
        .len(),
        1
    );
}

#[test]
fn two_items_finishing_in_one_pass_each_get_their_own_report() {
    let d = row(
        dispatch_status::RUNNING,
        vec![
            item(1, item_status::RUNNING, Some("s1"), &[]),
            item(2, item_status::RUNNING, Some("s2"), &[]),
            item(3, item_status::QUEUED, None, &[1]),
        ],
    );
    let mut trackers = Trackers::new();
    let changes = observe(
        &d,
        &live(&[("s1", obs(1, 1, false)), ("s2", obs(2, 1, false))]),
        &mut trackers,
        Instant::now(),
    );
    assert_eq!(changes.len(), 2);
    let after = row(
        dispatch_status::RUNNING,
        vec![
            item(1, item_status::SUCCEEDED, Some("s1"), &[]),
            item(2, item_status::SUCCEEDED, Some("s2"), &[]),
            item(3, item_status::RUNNING, Some("s3"), &[1]),
        ],
    );
    let advanced = Advance {
        started: vec![(3, "s3".into())],
        ..Advance::default()
    };
    let messages = compose_messages(&after, &changes, &advanced);
    assert_eq!(messages.len(), 2);
    assert!(
        messages[0].contains("Session 1/3 \"Item 1\""),
        "{}",
        messages[0]
    );
    assert!(
        messages[1].contains("Session 2/3 \"Item 2\""),
        "{}",
        messages[1]
    );
    for m in &messages {
        assert!(m.contains("Still running: 1. Waiting to start: 0."), "{m}");
        assert!(m.contains("all done"), "{m}");
    }
}

#[test]
fn cancelled_dependents_are_listed_under_the_item_that_blocked_them() {
    let after = row(
        dispatch_status::RUNNING,
        vec![
            item(1, item_status::SUCCEEDED, Some("s1"), &[]),
            item(2, item_status::FAILED, Some("s2"), &[]),
            item(3, item_status::CANCELLED, None, &[2]),
            item(4, item_status::CANCELLED, None, &[3]),
            item(5, item_status::RUNNING, Some("s5"), &[]),
        ],
    );
    let change = |idx: i64, status: &'static str| ItemChange {
        idx,
        status,
        outcome: "x".into(),
        stop_reason: Some("max_steps".into()),
        final_reply: String::new(),
        session_id: format!("s{idx}"),
    };
    let changes = vec![
        change(2, item_status::FAILED),
        change(1, item_status::SUCCEEDED),
    ];
    let advanced = Advance {
        cancelled: vec![3, 4],
        ..Advance::default()
    };
    let messages = compose_messages(&after, &changes, &advanced);
    assert!(messages[0].contains("stopped without finishing (max steps)"));
    assert!(
        messages[0].contains("3. Item 3; 4. Item 4"),
        "transitive dependents belong to item 2: {}",
        messages[0]
    );
    assert!(!messages[1].contains("Not starting"), "{}", messages[1]);
    assert_eq!(messages.len(), 2, "item 5 still runs, so no summary yet");
}

#[test]
fn a_failed_start_is_reported_to_the_coordinator() {
    let after = row(
        dispatch_status::RUNNING,
        vec![
            item(1, item_status::FAILED, None, &[]),
            item(2, item_status::RUNNING, Some("s2"), &[]),
        ],
    );
    let advanced = Advance {
        start_failed: vec![(1, "could not start its session: disk full".into())],
        ..Advance::default()
    };
    let messages = compose_messages(&after, &[], &advanced);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("could not start its session: disk full"));
}

#[test]
fn the_last_item_finishing_adds_the_summary_request() {
    let after = row(
        dispatch_status::RUNNING,
        vec![
            item(1, item_status::SUCCEEDED, Some("s1"), &[]),
            item(2, item_status::SKIPPED, None, &[]),
        ],
    );
    let changes = vec![ItemChange {
        idx: 1,
        status: item_status::SUCCEEDED,
        outcome: "final_answer".into(),
        stop_reason: Some("final_answer".into()),
        final_reply: "ok".into(),
        session_id: "s1".into(),
    }];
    assert!(is_done(&after));
    let messages = compose_messages(&after, &changes, &Advance::default());
    assert_eq!(messages.len(), 2);
    assert!(messages[1].starts_with("[dispatch] All sessions have finished:"));
    assert!(messages[1].contains("2. Item 2: not selected"));
}

#[test]
fn a_dispatch_that_is_not_running_is_never_done_again() {
    let finished = vec![item(1, item_status::SUCCEEDED, Some("s1"), &[])];
    assert!(!is_done(&row(dispatch_status::DONE, finished.clone())));
    assert!(!is_done(&row(dispatch_status::CANCELLED, finished)));
    assert!(compose_messages(
        &row(
            dispatch_status::DONE,
            vec![item(1, item_status::SUCCEEDED, Some("s1"), &[])]
        ),
        &[],
        &Advance::default()
    )
    .is_empty());
}

fn transcript(rows: &[(&str, &str)]) -> Vec<SnapTranscriptRow> {
    rows.iter()
        .map(|(kind, text)| SnapTranscriptRow {
            kind: kind.to_string(),
            text: text.to_string(),
            tool: None,
            meta: None,
        })
        .collect()
}

#[test]
fn the_final_reply_is_the_trailing_run_of_assistant_rows() {
    let rows = transcript(&[
        ("user", "do it"),
        ("assistant", "looking"),
        ("tool", "read_file"),
        ("assistant", "Done."),
        ("assistant", "Verified with cargo test."),
        ("assistant", ""),
    ]);
    assert_eq!(final_reply(&rows), "Done.\nVerified with cargo test.");
}

#[test]
fn a_turn_that_ended_on_a_tool_row_has_no_final_reply() {
    let rows = transcript(&[
        ("assistant", "working"),
        ("tool", "write_file"),
        ("system", ""),
    ]);
    assert_eq!(final_reply(&rows), "");
    assert_eq!(final_reply(&[]), "");
}

#[test]
fn a_lost_tracker_does_not_report_an_already_finished_item_again() {
    let mut trackers = Trackers::new();
    let now = Instant::now();
    let seen = live(&[("s1", obs(7, 1, false))]);
    assert_eq!(observe(&running_one(), &seen, &mut trackers, now).len(), 1);
    // The supervisor loses its memory (a failed store read, or the dispatch left the recent
    // window) while the worker is idle on the turn that was already reported.
    trackers.clear();
    let succeeded = row(
        dispatch_status::RUNNING,
        vec![item(1, item_status::SUCCEEDED, Some("s1"), &[])],
    );
    assert!(
        observe(&succeeded, &seen, &mut trackers, now).is_empty(),
        "the coordinator must not get the same report twice"
    );
    // A genuinely new turn on that worker is still reported.
    let next = live(&[("s1", obs(7, 2, false))]);
    assert_eq!(observe(&succeeded, &next, &mut trackers, now).len(), 1);
}

#[test]
fn a_lost_tracker_still_reports_a_running_item_that_finished_meanwhile() {
    let mut trackers = Trackers::new();
    let now = Instant::now();
    assert!(observe(
        &running_one(),
        &live(&[("s1", obs(7, 0, true))]),
        &mut trackers,
        now
    )
    .is_empty());
    trackers.clear();
    let changes = observe(
        &running_one(),
        &live(&[("s1", obs(7, 1, false))]),
        &mut trackers,
        now,
    );
    assert_eq!(
        changes.len(),
        1,
        "the item is still running in the store, so its turn is unreported"
    );
}

#[test]
fn a_failed_store_read_keeps_every_tracker() {
    let mut trackers = Trackers::new();
    let now = Instant::now();
    observe(
        &running_one(),
        &live(&[("s1", obs(7, 0, true))]),
        &mut trackers,
        now,
    );
    assert_eq!(trackers.len(), 1);
    prune_trackers(&mut trackers, &BTreeSet::new(), false);
    assert_eq!(
        trackers.len(),
        1,
        "a read error is not an empty dispatch list"
    );
    prune_trackers(&mut trackers, &BTreeSet::new(), true);
    assert!(
        trackers.is_empty(),
        "a successful read with no dispatches does prune"
    );
}

fn tr(kind: &str, text: &str) -> SnapTranscriptRow {
    SnapTranscriptRow {
        kind: kind.into(),
        text: text.into(),
        tool: None,
        meta: None,
    }
}

#[test]
fn the_final_reply_skips_the_recap_and_notices_after_it_and_the_header_before_it() {
    // The tail of a real finished worker's snapshot (captured from a live mock daemon).
    let rows = [
        tr("tool", "✓ write_file wrote 26 bytes to /tmp/x"),
        tr("assistant", "  ⚒ forge"),
        tr(
            "system",
            "  ⚠ completeness check — reviewing the change against every requirement",
        ),
        tr("assistant", "  ⚒ forge"),
        tr("assistant", "  Done — the file is written."),
        tr("assistant", "  It is at mock-note.txt."),
        tr("system", "  ※ recap  Writing the file now."),
    ];
    assert_eq!(
        final_reply(&rows),
        "Done — the file is written.\nIt is at mock-note.txt."
    );
}

#[test]
fn a_transcript_with_no_assistant_text_has_no_final_reply() {
    assert_eq!(final_reply(&[]), "");
    assert_eq!(
        final_reply(&[tr("assistant", "  ⚒ forge"), tr("system", "  ※ recap  x")]),
        ""
    );
}
