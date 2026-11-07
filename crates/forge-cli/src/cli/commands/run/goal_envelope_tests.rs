use super::*;
use forge_types::goal_envelope::EnvelopeSpec;

fn window(
    provider: &str,
    kind: &str,
    pct: f64,
    updated_at: i64,
) -> forge_store::SubscriptionWindow {
    forge_store::SubscriptionWindow {
        provider: provider.into(),
        window_kind: kind.into(),
        status: "ok".into(),
        resets_at: Some(updated_at + 3600),
        fraction: Some(pct / 100.0),
        updated_at,
    }
}

fn sources(windows: Vec<forge_store::SubscriptionWindow>, now: i64) -> EnvelopeSources {
    EnvelopeSources {
        windows,
        session_usd: 0.0,
        now,
    }
}

#[test]
fn the_reading_is_the_newest_row_of_the_envelopes_provider_and_window() {
    let spec = EnvelopeSpec::parse("claude:20%w", None).unwrap();
    let s = sources(
        vec![
            window("claude-cli", "weekly", 30.0, 100),
            window("claude-cli", "weekly", 33.0, 200),
            window("claude-cli", "five_hour", 90.0, 300),
            window("codex-cli", "weekly", 5.0, 300),
        ],
        210,
    );
    let r = s.quota(&spec).unwrap();
    assert_eq!((r.pct, r.observed_at), (33.0, 200));
    let codex = EnvelopeSpec::parse("codex:10%w", None).unwrap();
    assert_eq!(s.quota(&codex).unwrap().pct, 5.0);
    let five = EnvelopeSpec::parse("claude:10%5h", None).unwrap();
    assert_eq!(s.quota(&five).unwrap().pct, 90.0);
}

#[test]
fn a_missing_window_reads_as_no_reading() {
    let spec = EnvelopeSpec::parse("codex:10%5h", None).unwrap();
    assert!(sources(vec![window("claude-cli", "weekly", 1.0, 1)], 2)
        .quota(&spec)
        .is_none());
}

#[test]
fn the_cap_message_names_the_slice_the_reset_and_how_to_raise_it() {
    let spec = EnvelopeSpec::parse("claude:20%w", None).unwrap();
    let mut env = EnvelopeState::new(spec, 40.0, Some(1_800_000_000));
    let s = sources(vec![window("claude-cli", "weekly", 61.0, 1000)], 1000);
    assert_eq!(s.check(&mut env), EnvelopeCheck::Spent);
    let msg = pause_message(&env, EnvelopeCheck::Spent);
    assert!(
        msg.starts_with("Envelope spent (20% of Claude weekly)."),
        "{msg}"
    );
    assert!(msg.contains("/goal budget 30%w"), "{msg}");
    assert!(msg.contains("resumes at"), "{msg}");
    assert!(!msg.contains("then /goal resume"), "{msg}");
    assert!(msg.contains(&resume_clock(4600, 1000)), "{msg}");
}

#[test]
fn an_unverifiable_envelope_says_so() {
    let spec = EnvelopeSpec::parse("claude:20%w", None).unwrap();
    let mut env = EnvelopeState::new(spec, 40.0, None);
    assert_eq!(
        sources(Vec::new(), 1000).check(&mut env),
        EnvelopeCheck::Unverified
    );
    let msg = pause_message(&env, EnvelopeCheck::Unverified);
    assert!(
        msg.contains("can't be verified") && msg.contains("claude weekly"),
        "{msg}"
    );
}

#[test]
fn resume_waits_for_the_reset_unless_raised() {
    let spec = EnvelopeSpec::parse("claude:20%w", None).unwrap();
    let mut env = EnvelopeState::new(spec, 40.0, Some(5000));
    env.used = 21.0;
    env.spent = true;
    assert!(resume_gate(&mut env, 4000)
        .unwrap_err()
        .contains("/goal budget 30%w"));
    assert!(resume_gate(&mut env, 5001).is_ok());
    assert!(!env.spent && env.start.is_none());
}

#[test]
fn the_goal_record_round_trips_through_json() {
    let mut app = forge_tui::App::default();
    let spec = EnvelopeSpec::parse("$5", None).unwrap();
    app.pending_envelope = Some(EnvelopeState::new(spec, 1.0, None));
    app.goal_begin(
        "harden the importer",
        &["cargo test".to_string()],
        None,
        Some(30),
    );
    let json = GoalRecord::of(&app).expect("an envelope goal is recorded");
    let record: GoalRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(record.objective, "harden the importer");
    assert_eq!(record.max_minutes, Some(30));
    assert!(record.envelope.as_ref().unwrap().is_usd());
    app.goal_clear();
    assert!(GoalRecord::of(&app).is_none());
}

#[test]
fn a_successor_keeps_counting_the_dollars_its_predecessor_spent() {
    let mut app = forge_tui::App::default();
    let mut envelope = EnvelopeState::new(EnvelopeSpec::parse("$10", None).unwrap(), 2.0, None);
    assert_eq!(envelope.observe_usd(9.0), EnvelopeCheck::Within);
    app.pending_envelope = Some(envelope);
    app.goal_begin("harden the importer", &[], None, None);
    let json = GoalRecord::for_successor(app.goal.as_ref().unwrap(), "wedged");
    let record: GoalRecord = serde_json::from_str(&json).unwrap();
    let mut successor = record
        .envelope
        .expect("the envelope moves to the successor");
    assert_eq!(
        successor.observe_usd(3.5),
        EnvelopeCheck::Spent,
        "$7 spent by the predecessor plus $3.50 by the successor is past the $10 cap"
    );
}

#[test]
fn a_restart_restores_the_goal_paused_with_its_envelope_and_counters() {
    let store = forge_store::Store::open_in_memory().unwrap();
    let id = store.create_session("/tmp", "Default").unwrap();
    let mut app = forge_tui::App::default();
    let mut envelope = EnvelopeState::new(
        EnvelopeSpec::parse("claude:20%w", None).unwrap(),
        40.0,
        Some(9),
    );
    envelope.used = 20.0;
    envelope.spent = true;
    app.pending_envelope = Some(envelope);
    app.goal_begin(
        "harden the importer",
        &["cargo test".to_string()],
        None,
        Some(30),
    );
    app.goal_sync(4, 1234, 90);
    store
        .set_session_goal_state(&id, GoalRecord::of(&app).as_deref())
        .unwrap();

    let mut fresh = forge_tui::App::default();
    restore(&mut fresh, &store, &id);
    let goal = fresh.goal.as_ref().expect("restored");
    assert_eq!(goal.phase, forge_tui::goal_status::GoalPhase::Paused);
    assert_eq!(
        (goal.iteration, goal.tokens_used, goal.elapsed_secs),
        (4, 1234, 90)
    );
    assert_eq!(goal.gates, vec!["cargo test".to_string()]);
    assert!(goal.envelope.as_ref().unwrap().spent);
    assert!(goal.reason.as_deref().unwrap().contains("restart"));
}

fn contract_goal(app: &mut forge_tui::App) {
    use forge_types::goal_contract::{parse_contract, AssertionResult, GoalContract};
    let text = r#"{"assertions":[
      {"assertion":"a exists","check":"test -f a"},
      {"assertion":"b exists","check":"test -f b"},
      {"assertion":"c exists","check":"test -f c"}]}"#;
    let mut contract = GoalContract::new(parse_contract(text).unwrap());
    contract.approved = true;
    app.goal_begin("ship it", &[], None, None);
    app.goal_set_contract(contract);
    app.goal_contract_results(
        (1..=3)
            .map(|id| AssertionResult {
                id,
                passed: id != 3,
                proof: "test: c: no such file".into(),
                reason: if id == 3 {
                    "exit 1".into()
                } else {
                    String::new()
                },
                artifact: None,
            })
            .collect(),
    );
}

#[test]
fn a_contract_goal_without_an_envelope_is_recorded_and_survives_a_restart() {
    let store = forge_store::Store::open_in_memory().unwrap();
    let id = store.create_session("/tmp", "Default").unwrap();
    let mut app = forge_tui::App::default();
    contract_goal(&mut app);
    store
        .set_session_goal_state(&id, GoalRecord::of(&app).as_deref())
        .unwrap();

    let mut fresh = forge_tui::App::default();
    restore(&mut fresh, &store, &id);
    let goal = fresh.goal.as_ref().expect("restored");
    assert_eq!(goal.phase, forge_tui::goal_status::GoalPhase::Paused);
    assert!(goal.envelope.is_none());
    let contract = goal.contract.as_ref().expect("the contract came back");
    assert_eq!(contract.headline(), "2/3 pass");
    assert_eq!(contract.failing_ids(), vec![3]);
    assert!(contract.approved);
}

#[test]
fn a_finished_contract_goal_keeps_its_table_and_comes_back_finished() {
    let store = forge_store::Store::open_in_memory().unwrap();
    let id = store.create_session("/tmp", "Default").unwrap();
    let mut app = forge_tui::App::default();
    contract_goal(&mut app);
    app.goal_ended(
        false,
        "🎯 goal wedged — assertions 3 still fail after 3 validations",
    );
    store
        .set_session_goal_state(&id, GoalRecord::of(&app).as_deref())
        .unwrap();

    let mut fresh = forge_tui::App::default();
    restore(&mut fresh, &store, &id);
    let goal = fresh
        .goal
        .as_ref()
        .expect("the finished goal is still there");
    assert_eq!(goal.phase, forge_tui::goal_status::GoalPhase::Failed);
    assert!(goal.reason.as_deref().unwrap().contains("wedged"));
    assert_eq!(goal.contract.as_ref().unwrap().passed_count(), 2);
    assert!(
        notes_of(&mut fresh).is_empty(),
        "nothing is announced for a goal that already ended"
    );
}

#[test]
fn a_finished_goal_without_a_contract_is_not_recorded() {
    let mut app = forge_tui::App::default();
    app.goal_begin("ship it", &[], None, None);
    app.goal_ended(true, "");
    assert!(GoalRecord::of(&app).is_none());
}

fn notes_of(app: &mut forge_tui::App) -> Vec<String> {
    app.drain_flush().iter().map(|l| l.to_string()).collect()
}

async fn mock_session() -> Arc<tokio::sync::Mutex<Session>> {
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

/// A goal paused at its cap, the way the loop leaves it: spent, with the window's reset time.
fn paused_at_cap(spec: &str, resets_at: Option<i64>, now: i64) -> forge_tui::App {
    let mut app = forge_tui::App::default();
    let mut envelope =
        EnvelopeState::new(EnvelopeSpec::parse(spec, None).unwrap(), 40.0, resets_at);
    envelope.used = envelope.spec.slice + 1.0;
    envelope.spent = true;
    app.pending_envelope = Some(envelope);
    app.goal_begin("sleep through it", &["true".to_string()], None, None);
    app.goal_paused_by_envelope("spent");
    if let Some(e) = app.goal.as_mut().and_then(|g| g.envelope.as_mut()) {
        mark_waiting(e, now);
    }
    app
}

#[tokio::test]
async fn a_goal_paused_at_the_cap_resumes_once_after_the_reset() {
    let session = mock_session().await;
    let mut app = paused_at_cap("claude:20%w", Some(5000), 1000);
    assert!(
        auto_resume(&session, &mut app, 4999).await.is_none(),
        "not before"
    );
    let outcome = auto_resume(&session, &mut app, 5000).await;
    assert!(matches!(
        outcome,
        Some(super::super::dispatch::DispatchOutcome::StartGoal { .. })
    ));
    let notes = notes_of(&mut app);
    assert!(
        notes.iter().any(|n| n.contains(AUTO_RESUME_NOTE)),
        "{notes:?}"
    );
    let envelope = app.goal.as_ref().unwrap().envelope.clone().unwrap();
    assert!(!envelope.spent && envelope.start.is_none(), "re-baselined");
    app.goal_begin("sleep through it", &[], None, None);
    assert!(
        auto_resume(&session, &mut app, 9000).await.is_none(),
        "only once"
    );
}

#[tokio::test]
async fn a_goal_the_user_paused_never_resumes_by_itself() {
    let session = mock_session().await;
    let mut app = paused_at_cap("claude:20%w", Some(5000), 1000);
    assert!(auto_resume_due(&app, 6000));
    let outcome = goal_control_outcome(&session, &mut app, forge_tui::GoalControl::Pause)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        super::super::dispatch::DispatchOutcome::Handled
    ));
    assert!(!auto_resume_due(&app, 6000));
    assert!(auto_resume(&session, &mut app, 6000).await.is_none());
    // The user can still resume it by hand once the window has reset.
    let by_hand = goal_control_outcome(&session, &mut app, forge_tui::GoalControl::Resume)
        .await
        .unwrap();
    assert!(matches!(
        by_hand,
        super::super::dispatch::DispatchOutcome::StartGoal { .. }
    ));
}

#[tokio::test]
async fn a_usd_envelope_or_an_unknown_reset_never_resumes_by_itself() {
    let session = mock_session().await;
    let mut usd = paused_at_cap("$5", Some(5000), 1000);
    assert!(auto_resume(&session, &mut usd, 9000).await.is_none());
    let mut unknown = paused_at_cap("claude:20%w", None, 1000);
    assert!(auto_resume(&session, &mut unknown, 9000).await.is_none());
    let mut stopped = paused_at_cap("claude:20%w", Some(5000), 1000);
    stopped.goal_ended(false, "stopped");
    assert!(auto_resume(&session, &mut stopped, 9000).await.is_none());
    let mut unspent = paused_at_cap("claude:20%w", Some(5000), 1000);
    unspent
        .goal
        .as_mut()
        .unwrap()
        .envelope
        .as_mut()
        .unwrap()
        .spent = false;
    assert!(auto_resume(&session, &mut unspent, 9000).await.is_none());
}

#[tokio::test]
async fn a_restart_before_the_reset_keeps_the_scheduled_resume() {
    let session = mock_session().await;
    let store = forge_store::Store::open_in_memory().unwrap();
    let id = store.create_session("/tmp", "Default").unwrap();
    let before = paused_at_cap("claude:20%w", Some(5000), 1000);
    store
        .set_session_goal_state(&id, GoalRecord::of(&before).as_deref())
        .unwrap();
    let mut after = forge_tui::App::default();
    restore(&mut after, &store, &id);
    let notes = notes_of(&mut after);
    assert!(notes.iter().any(|n| n.contains("resumes at")), "{notes:?}");
    assert!(auto_resume(&session, &mut after, 4000).await.is_none());
    assert!(auto_resume(&session, &mut after, 5001).await.is_some());
}
