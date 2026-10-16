//! Rewind, undo and checkpoints must agree with the DB's seq space, which runs ahead of the
//! session's in-memory counter whenever a side call (recap, suggestion, memory capture) records
//! its usage: that writes an inactive anchor row at MAX(seq)+1. Checkpoints used to be keyed by
//! the in-memory seq and `rewind_to` mapped seqs to transcript indexes by `seq - len`, so rewinding
//! to turn N soft-deleted the tail of turn N-1 while leaving its prompt active.

use super::*;

fn session_in(store: &Arc<Store>) -> Session {
    let mut config = Config::default();
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    fresh_session(Arc::clone(store), config)
}

/// A side call's usage anchor, the way recap/suggest/memory record themselves.
fn side_call(store: &Store, session: &Session) {
    store
        .record_side_call_usage(
            session.session_id(),
            "recap",
            &forge_types::Usage::default(),
        )
        .unwrap();
}

/// `(seq, role, active, content)` for every message row, anchors included.
fn rows(store: &Store, id: &str) -> Vec<(i64, String, bool, String)> {
    store.message_state_rows(id).unwrap()
}

fn user_seq(rows: &[(i64, String, bool, String)], prompt: &str) -> i64 {
    rows.iter()
        .find(|(_, role, _, content)| role == "user" && content == prompt)
        .unwrap_or_else(|| panic!("no user row {prompt:?}"))
        .0
}

#[tokio::test]
async fn checkpoints_are_the_db_seq_of_the_turns_user_message() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut session = session_in(&store);
    for prompt in ["one", "two", "three", "four"] {
        side_call(&store, &session);
        session.run_turn(prompt).await.unwrap();
        side_call(&store, &session);
        side_call(&store, &session);
    }
    let id = session.session_id().to_string();
    let all = rows(&store, &id);
    for cp in session.checkpoints().unwrap() {
        let label = cp.label.clone().unwrap();
        assert_eq!(
            cp.seq,
            user_seq(&all, &label),
            "checkpoint {label:?} must sit on its user row"
        );
    }
}

#[tokio::test]
async fn rewinding_to_a_turn_deactivates_only_that_turn_and_later() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut session = session_in(&store);
    for prompt in ["one", "two", "three"] {
        side_call(&store, &session);
        session.run_turn(prompt).await.unwrap();
        side_call(&store, &session);
    }
    let id = session.session_id().to_string();
    let three = session
        .checkpoints()
        .unwrap()
        .into_iter()
        .find(|c| c.label.as_deref() == Some("three"))
        .unwrap();

    let outcome = session.rewind_to(three.seq).unwrap();
    assert_eq!(outcome.rewound_prompt.as_deref(), Some("three"));

    let all = rows(&store, &id);
    let two_seq = user_seq(&all, "two");
    for (seq, role, active, content) in &all {
        if role == "system" && content == "recap" {
            continue; // usage anchors are always inactive
        }
        assert_eq!(
            *active,
            *seq < three.seq,
            "row seq {seq} ({role}) must be active iff it precedes turn three"
        );
    }
    // Turn two is whole: its prompt AND its answer are still there.
    assert!(all
        .iter()
        .any(|(seq, role, active, _)| *seq > two_seq && role == "assistant" && *active));

    // The model-facing transcript is exactly what a resume would load.
    let stored: Vec<String> = store
        .load_messages(&id)
        .unwrap()
        .into_iter()
        .map(|m| m.content)
        .collect();
    let live: Vec<String> = session
        .transcript
        .iter()
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(
        stored, live,
        "the live transcript must equal what a resume loads"
    );
    assert!(!stored.iter().any(|c| c == "three"));
    assert!(stored.iter().any(|c| c == "two"));

    // And the next turn lands after everything, never on a reused seq.
    session.run_turn("again").await.unwrap();
    let after = rows(&store, &id);
    assert!(user_seq(&after, "again") > all.iter().map(|r| r.0).max().unwrap());
}

#[tokio::test]
async fn undo_walks_back_one_turn_at_a_time_across_anchor_rows() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut session = session_in(&store);
    for prompt in ["one", "two"] {
        side_call(&store, &session);
        session.run_turn(prompt).await.unwrap();
        side_call(&store, &session);
    }
    let id = session.session_id().to_string();

    let first = session.undo().unwrap().unwrap();
    assert_eq!(first.rewound_prompt.as_deref(), Some("two"));
    let stored: Vec<String> = store
        .load_messages(&id)
        .unwrap()
        .into_iter()
        .map(|m| m.content)
        .collect();
    assert!(stored.iter().any(|c| c == "one") && !stored.iter().any(|c| c == "two"));

    let second = session.undo().unwrap().unwrap();
    assert_eq!(second.rewound_prompt.as_deref(), Some("one"));
    assert!(store.load_messages(&id).unwrap().is_empty());
    assert!(session.undo().unwrap().is_none());
}

#[tokio::test]
async fn rewinding_across_a_compaction_boundary_restores_whole_turns() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut session = session_in(&store);
    for prompt in ["t1", "t2", "t3", "t4", "t5", "t6"] {
        side_call(&store, &session);
        session.run_turn(prompt).await.unwrap();
        side_call(&store, &session);
    }
    let id = session.session_id().to_string();
    let t3 = session
        .checkpoints()
        .unwrap()
        .into_iter()
        .find(|c| c.label.as_deref() == Some("t3"))
        .unwrap();
    // Fold everything but the last few rows into a summary, like /compact does.
    store
        .compact_session_store(&id, "summary of t1..t5", 4)
        .unwrap();
    assert!(session.was_compacted());

    let outcome = session.rewind_to(t3.seq).unwrap();
    assert_eq!(outcome.rewound_prompt.as_deref(), Some("t3"));

    let all = rows(&store, &id);
    for (seq, role, active, content) in &all {
        if role == "system" && content == "recap" {
            continue;
        }
        assert_eq!(
            *active,
            *seq < t3.seq,
            "after rewinding across the compaction, row {seq} ({role} {content:?}) \
             must be active iff it precedes turn t3"
        );
    }
    let stored: Vec<String> = store
        .load_messages(&id)
        .unwrap()
        .into_iter()
        .map(|m| m.content)
        .collect();
    assert!(stored.iter().any(|c| c == "t1") && stored.iter().any(|c| c == "t2"));
    assert!(!stored.iter().any(|c| c == "t3"));
    assert!(
        stored.iter().all(|c| !c.contains("summary of")),
        "the summary is gone once its source rows are back"
    );
}

#[tokio::test]
async fn swapping_sessions_resets_the_gauge_and_totals_the_display_holds() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut config = Config::default();
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    let mut session = Session::start(
        Arc::clone(&store),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(CapturePresenter {
            events: events.clone(),
            attended: true,
            ..Default::default()
        }),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    let old = session.session_id().to_string();
    let answer = store
        .add_message(&old, 0, Role::Assistant, "an answer", Some("m::x"))
        .unwrap();
    store
        .record_usage(
            &old,
            &answer,
            &forge_types::Usage {
                input_tokens: 175,
                output_tokens: 30,
                ..Default::default()
            },
            Some("m::x"),
        )
        .unwrap();

    let last_cost = |events: &Arc<Mutex<Vec<PresenterEvent>>>| {
        events.lock().unwrap().iter().rev().find_map(|e| match e {
            PresenterEvent::Cost {
                session_in,
                session_out,
                context_tokens,
                ..
            } => Some((*session_in, *session_out, *context_tokens)),
            _ => None,
        })
    };

    session.reset_fresh(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(
        last_cost(&events),
        Some((0, 0, 0)),
        "/new must zero the totals and the context gauge"
    );

    session.reset_resumed(&old).unwrap();
    let (session_in, session_out, _) = last_cost(&events).unwrap();
    assert_eq!(
        (session_in, session_out),
        (175, 30),
        "/resume shows the resumed session's totals"
    );
}
