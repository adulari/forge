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
    assert!(msg.contains(&reset_label(4600)), "{msg}");
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
    assert!(record.envelope.is_usd());
    app.goal_clear();
    assert!(GoalRecord::of(&app).is_none());
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
