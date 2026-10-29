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
