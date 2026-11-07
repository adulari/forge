use super::*;

fn started() -> App {
    let mut app = App::default();
    app.goal_begin(
        "ship it",
        &["cargo test".to_string()],
        Some(50_000),
        Some(30),
    );
    app
}

#[test]
fn the_chip_reports_progress_state_and_budget() {
    let mut app = started();
    app.goal_sync(2, 12_500, 90);
    let goal = app.goal.clone().unwrap();
    assert_eq!(goal.chip(0, 0), "iteration 2 · 12.5k/50k tok");
    assert_eq!(goal.chip(2, 5), "2/5 · 12.5k/50k tok");
    app.goal_request(GoalRequest::Pause);
    assert_eq!(
        app.goal.as_ref().unwrap().chip(2, 5),
        "2/5 · pausing · 12.5k/50k tok"
    );
    app.goal_paused();
    assert_eq!(
        app.goal.as_ref().unwrap().chip(2, 5),
        "paused · 2/5 · 12.5k/50k tok"
    );
}

#[test]
fn a_goal_without_a_token_budget_has_a_bare_chip() {
    let mut app = App::default();
    app.goal_begin("g", &[], None, None);
    assert_eq!(app.goal.as_ref().unwrap().chip(1, 4), "1/4");
}

#[test]
fn requests_only_land_on_a_live_goal_and_are_taken_once() {
    let mut app = App::default();
    assert!(!app.goal_request(GoalRequest::Pause));
    let mut app = started();
    assert!(app.goal_request(GoalRequest::Clear));
    assert_eq!(app.goal_take_request(), Some(GoalRequest::Clear));
    assert_eq!(app.goal_take_request(), None);
    app.goal_paused();
    assert!(
        !app.goal_request(GoalRequest::Pause),
        "a paused goal has nothing to pause"
    );
}

#[test]
fn an_edit_waits_for_a_live_goal_and_applies_at_once_to_a_paused_one() {
    let mut app = started();
    assert!(app.goal_request_edit("ship it twice".into()));
    assert_eq!(app.goal.as_ref().unwrap().objective, "ship it");
    assert_eq!(app.goal_take_edit().as_deref(), Some("ship it twice"));
    assert_eq!(app.goal.as_ref().unwrap().objective, "ship it twice");
    app.goal_paused();
    assert!(app.goal_request_edit("ship it thrice".into()));
    assert_eq!(app.goal.as_ref().unwrap().objective, "ship it thrice");
    app.goal_clear();
    assert!(!app.goal_request_edit("nothing".into()));
}

#[test]
fn only_a_paused_goal_carries_its_counters_into_the_next_run() {
    let mut app = started();
    app.goal_sync(4, 900, 120);
    assert!(
        app.goal_prepare_resume().is_none(),
        "a live goal is not resumable"
    );
    app.goal_paused();
    let paused = app.goal_prepare_resume().unwrap();
    assert_eq!(paused.objective, "ship it");
    let carry = app.goal_begin("ship it", &[], None, None).unwrap();
    assert_eq!(
        (carry.iteration, carry.tokens_used, carry.elapsed_secs),
        (5, 900, 120)
    );
    assert!(
        app.goal_begin("ship it", &[], None, None).is_none(),
        "the carry is spent"
    );
}

#[test]
fn a_different_objective_starts_fresh_even_after_a_resume_was_prepared() {
    let mut app = started();
    app.goal_paused();
    app.goal_prepare_resume();
    assert!(app.goal_begin("something else", &[], None, None).is_none());
    assert_eq!(app.goal.as_ref().unwrap().iteration, 1);
}

#[test]
fn how_a_goal_ended_decides_its_phase() {
    let mut app = started();
    app.goal_ended(true, "");
    assert_eq!(app.goal.as_ref().unwrap().phase, GoalPhase::Complete);
    let mut app = started();
    app.goal_ended(false, "🎯 goal stopped — token budget exhausted");
    assert_eq!(app.goal.as_ref().unwrap().phase, GoalPhase::BudgetLimit);
    let mut app = started();
    app.goal_ended(false, "🎯 goal wedged — no progress");
    let goal = app.goal.as_ref().unwrap();
    assert_eq!(goal.phase, GoalPhase::Failed);
    assert!(goal.reason.as_deref().unwrap().contains("wedged"));
    let mut app = started();
    app.goal_interrupted();
    assert_eq!(app.goal.as_ref().unwrap().phase, GoalPhase::Cancelled);
}

#[test]
fn the_summary_names_state_budget_and_gates() {
    let mut app = started();
    app.goal_sync(3, 1_200, 75);
    let text = app.goal.as_ref().unwrap().summary();
    assert!(text.contains("goal active — ship it"), "{text}");
    assert!(
        text.contains("iteration 3 · 1.2k tokens of 50k · 75s of 30m"),
        "{text}"
    );
    assert!(text.contains("gates: cargo test"), "{text}");
}

fn enveloped() -> App {
    use forge_types::goal_envelope::{EnvelopeSpec, EnvelopeState};
    let mut app = App::default();
    app.pending_envelope = Some(EnvelopeState::new(
        EnvelopeSpec::parse("claude:20%w", None).unwrap(),
        40.0,
        None,
    ));
    app.goal_begin("harden the importer", &[], None, None);
    app
}

#[test]
fn the_envelope_meter_rides_the_chip_and_the_summary() {
    let mut app = enveloped();
    let mut envelope = app.goal.as_ref().unwrap().envelope.clone().unwrap();
    envelope.used = 12.0;
    app.goal_envelope_sync(&envelope);
    let goal = app.goal.clone().unwrap();
    assert_eq!(
        goal.chip(0, 0),
        "iteration 1 · envelope 12/20% claude weekly"
    );
    assert!(goal.summary().contains("envelope 12/20% claude weekly"));
    assert!(app.pending_envelope.is_none(), "the run adopted it");
}

#[test]
fn a_goal_paused_at_its_cap_keeps_the_reason_and_resume_keeps_the_envelope() {
    let mut app = enveloped();
    app.goal_paused_by_envelope("Envelope spent (20% of Claude weekly).");
    let goal = app.goal.clone().unwrap();
    assert_eq!(goal.phase, GoalPhase::Paused);
    assert_eq!(
        goal.reason.as_deref(),
        Some("Envelope spent (20% of Claude weekly).")
    );
    app.goal_prepare_resume().unwrap();
    app.goal_begin("harden the importer", &[], None, None);
    assert!(app.goal.as_ref().unwrap().envelope.is_some());
}

#[test]
fn raising_a_paused_envelope_takes_at_once_and_a_live_one_waits_for_the_loop() {
    use forge_types::goal_envelope::EnvelopeSpec;
    let raise = EnvelopeSpec::parse("30%w", Some("claude")).unwrap();
    let mut app = enveloped();
    assert!(app.goal_request_envelope(raise.clone()));
    assert_eq!(app.goal_take_envelope_edit(), Some(raise.clone()));
    app.goal_paused();
    assert!(app.goal_request_envelope(raise));
    assert_eq!(
        app.goal
            .as_ref()
            .unwrap()
            .envelope
            .as_ref()
            .unwrap()
            .spec
            .slice,
        30.0
    );
    let mut bare = started();
    assert!(!bare.goal_request_envelope(EnvelopeSpec::parse("$5", None).unwrap()));
}

#[test]
fn a_goal_waiting_for_its_window_says_when_it_resumes() {
    let mut app = enveloped();
    let mut envelope = app.goal.as_ref().unwrap().envelope.clone().unwrap();
    envelope.used = 21.0;
    envelope.spent = true;
    envelope.resets_at = Some(5000);
    envelope.resume_label = Some("04:00".into());
    app.goal_envelope_sync(&envelope);
    app.goal_paused_by_envelope("spent");
    let goal = app.goal.clone().unwrap();
    assert!(
        goal.chip(0, 0).ends_with(" · resumes at 04:00"),
        "{}",
        goal.chip(0, 0)
    );
    assert!(goal
        .summary()
        .contains("resumes at 04:00 when the window resets"));
    envelope.held = true;
    app.goal_envelope_sync(&envelope);
    let held = app.goal.clone().unwrap();
    assert!(!held.chip(0, 0).contains("resumes at"));
    assert!(!held.summary().contains("resumes at"));
}

mod contract {
    use super::*;
    use forge_types::goal_contract::{parse_contract, AssertionResult, GoalContract};

    fn contract() -> GoalContract {
        let text = r#"{"assertions":[
          {"assertion":"a exists","check":"test -f a"},
          {"assertion":"b exists","check":"test -f b"},
          {"assertion":"c exists","check":"test -f c"}]}"#;
        let mut c = GoalContract::new(parse_contract(text).unwrap());
        c.approved = true;
        c
    }

    fn result(id: u32, passed: bool) -> AssertionResult {
        AssertionResult {
            id,
            passed,
            proof: "out".into(),
            reason: if passed {
                String::new()
            } else {
                "exit 1".into()
            },
        }
    }

    #[test]
    fn status_shows_the_contract_table_and_the_chip_the_count() {
        let mut app = App::default();
        app.goal_begin("ship", &[], None, None);
        app.goal_set_contract(contract());
        assert!(!app.goal.as_ref().unwrap().chip(0, 0).contains("contract"));
        app.goal_contract_results(vec![result(1, true), result(2, false), result(3, true)]);
        let goal = app.goal.as_ref().unwrap();
        assert!(
            goal.chip(1, 3).contains("contract 2/3 pass"),
            "{}",
            goal.chip(1, 3)
        );
        let summary = goal.summary();
        assert!(summary.contains("📜 contract — 2/3 pass"), "{summary}");
        assert!(summary.contains("✗ 2. b exists [FAIL]"), "{summary}");
        assert!(summary.contains("why: exit 1"), "{summary}");
    }

    #[test]
    fn a_resumed_goal_keeps_its_contract() {
        let mut app = App::default();
        app.goal_begin("ship", &[], None, None);
        app.goal_set_contract(contract());
        app.goal_paused();
        app.goal_prepare_resume();
        app.goal_begin("ship", &[], None, None);
        assert!(app.goal.as_ref().unwrap().contract.is_some());
        app.goal_begin("something else", &[], None, None);
        assert!(app.goal.as_ref().unwrap().contract.is_none());
    }

    fn mission() -> GoalContract {
        let items = |t: &str| {
            format!(
                r#"{{"title":"{t}","assertions":[{{"assertion":"{t} a","check":"true"}},{{"assertion":"{t} b","check":"true"}}]}}"#
            )
        };
        let reply = format!(
            r#"{{"milestones":[{},{},{}]}}"#,
            items("lexer"),
            items("parser"),
            items("cli")
        );
        let mut c = forge_types::goal_contract::mission::parse_mission(&reply, 6)
            .unwrap()
            .into_contract()
            .unwrap();
        c.approved = true;
        c
    }

    #[test]
    fn a_mission_status_groups_the_table_by_milestone_and_the_chip_shows_progress() {
        let mut app = App::default();
        app.goal_begin("ship", &[], None, None);
        app.goal_set_contract(mission());
        app.goal_contract_results(vec![result(1, true), result(2, true)]);
        app.goal
            .as_mut()
            .unwrap()
            .contract
            .as_mut()
            .unwrap()
            .advance_milestone();
        app.goal_contract_results(vec![result(3, true), result(4, false)]);
        let goal = app.goal.as_ref().unwrap();
        let chip = goal.chip(1, 3);
        assert!(chip.contains("contract milestone 2/3: 1/2 pass"), "{chip}");
        let summary = goal.summary();
        assert!(
            summary.contains("📜 contract — milestone 2/3: 1/2 pass"),
            "{summary}"
        );
        assert!(
            summary.contains("milestone 1/3 - lexer [done 2/2]"),
            "{summary}"
        );
        assert!(
            summary.contains("milestone 2/3 - parser [current 1/2 pass]"),
            "{summary}"
        );
        assert!(
            summary.contains("milestone 3/3 - cli [upcoming, 2 assertions]"),
            "{summary}"
        );
        assert!(summary.contains("✗ 4. parser b [FAIL]"), "{summary}");
    }
}
