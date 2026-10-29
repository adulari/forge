use super::*;

async fn test_session() -> Arc<tokio::sync::Mutex<Session>> {
    super::super::driver::tests::isolate_store();
    let session = build_session_with(
        Box::new(forge_tui::HeadlessPresenter::default()),
        true,
        None,
        None,
        None,
        true,
    )
    .await
    .expect("build mock session");
    Arc::new(tokio::sync::Mutex::new(session))
}

fn running(app: &mut forge_tui::App) -> GoalState {
    GoalState::begin(
        app,
        1,
        "ship it".into(),
        vec!["true".into()],
        Some(1_000),
        Some(30),
    )
}

#[tokio::test]
async fn pause_is_applied_when_the_turn_ends_and_resume_keeps_the_counters() {
    let session = test_session().await;
    let mut app = forge_tui::App::default();
    let mut gs = running(&mut app);
    gs.iter = 3;
    gs.budget.tokens_used = 400;

    goal_control_outcome(&session, &mut app, GoalControl::Pause)
        .await
        .unwrap();
    assert_eq!(
        app.goal.as_ref().unwrap().phase,
        GoalPhase::Active,
        "waits for the turn"
    );
    assert!(matches!(
        apply_goal_requests(&mut gs, &session, &mut app).await,
        Some(GoalNext::Stop)
    ));
    let paused = app.goal.clone().unwrap();
    assert_eq!(paused.phase, GoalPhase::Paused);
    assert_eq!((paused.iteration, paused.tokens_used), (3, 400));

    let outcome = goal_control_outcome(&session, &mut app, GoalControl::Resume)
        .await
        .unwrap();
    let DispatchOutcome::StartGoal {
        goal,
        gates,
        max_tokens,
        max_minutes,
        ..
    } = outcome
    else {
        panic!("resume restarts the goal");
    };
    assert_eq!(
        (goal.as_str(), gates, max_tokens, max_minutes),
        ("ship it", vec!["true".to_string()], Some(1_000), Some(30))
    );
    let resumed = GoalState::begin(
        &mut app,
        2,
        goal,
        vec!["true".into()],
        Some(1_000),
        Some(30),
    );
    assert_eq!((resumed.iter, resumed.budget.tokens_used), (4, 400));
    assert_eq!(app.goal.as_ref().unwrap().phase, GoalPhase::Active);
}

#[tokio::test]
async fn clear_stops_a_live_goal_and_drops_a_paused_one() {
    let session = test_session().await;
    let mut app = forge_tui::App::default();
    let mut gs = running(&mut app);
    goal_control_outcome(&session, &mut app, GoalControl::Clear)
        .await
        .unwrap();
    assert!(matches!(
        apply_goal_requests(&mut gs, &session, &mut app).await,
        Some(GoalNext::Stop)
    ));
    assert!(app.goal.is_none());

    let mut gs = running(&mut app);
    goal_control_outcome(&session, &mut app, GoalControl::Pause)
        .await
        .unwrap();
    apply_goal_requests(&mut gs, &session, &mut app).await;
    goal_control_outcome(&session, &mut app, GoalControl::Clear)
        .await
        .unwrap();
    assert!(app.goal.is_none());
}

#[tokio::test]
async fn edit_rewords_the_running_goal_at_the_turn_boundary() {
    let session = test_session().await;
    let mut app = forge_tui::App::default();
    let mut gs = running(&mut app);
    goal_control_outcome(
        &session,
        &mut app,
        GoalControl::Edit("ship it faster".into()),
    )
    .await
    .unwrap();
    assert_eq!(gs.goal, "ship it");
    assert!(apply_goal_requests(&mut gs, &session, &mut app)
        .await
        .is_none());
    assert_eq!(gs.goal, "ship it faster");
    assert_eq!(app.goal.as_ref().unwrap().objective, "ship it faster");
}

#[tokio::test]
async fn edit_without_a_goal_starts_one_named_edit() {
    let session = test_session().await;
    let mut app = forge_tui::App::default();
    let outcome = goal_control_outcome(&session, &mut app, GoalControl::Edit("the readme".into()))
        .await
        .unwrap();
    assert!(
        matches!(outcome, DispatchOutcome::StartGoal { goal, .. } if goal == "edit the readme")
    );
}

#[tokio::test]
async fn pause_and_resume_without_a_goal_only_say_so() {
    let session = test_session().await;
    let mut app = forge_tui::App::default();
    for control in [GoalControl::Pause, GoalControl::Resume, GoalControl::Clear] {
        let outcome = goal_control_outcome(&session, &mut app, control)
            .await
            .unwrap();
        assert!(matches!(outcome, DispatchOutcome::Handled));
    }
    assert!(app.goal.is_none());
}

#[test]
fn an_interrupted_run_is_cancelled_not_paused() {
    let mut app = forge_tui::App::default();
    let gs = running(&mut app);
    let mut state = Some(gs);
    abandon_goal(&mut state, &mut app);
    assert!(state.is_none());
    assert_eq!(app.goal.as_ref().unwrap().phase, GoalPhase::Cancelled);
}
