//! A notice that arrives while an answer streams must not cut the answer in two.

use super::*;

const QUEUED: &str = "⏳ queued (1 pending) — sent at this turn's next step";

#[test]
fn a_notice_mid_stream_waits_for_the_end_of_the_answer_segment() {
    let mut app = App::default();
    app.apply(PresenterEvent::AssistantDelta("The renderer ke".into()));
    let _label = drain_text_lines(&mut app);

    app.note(QUEUED);
    assert!(app.streaming_active, "the notice must not end the stream");
    assert_eq!(app.streaming, "The renderer ke");
    assert!(
        !drain_text_lines(&mut app).contains("queued"),
        "nothing is printed into the middle of the line"
    );

    app.apply(PresenterEvent::AssistantDelta(
        "eps each frame cheap.".into(),
    ));
    app.apply(PresenterEvent::AssistantDone);
    let text = drain_text_lines(&mut app);
    let sentence = text
        .find("The renderer keeps each frame cheap.")
        .unwrap_or_else(|| panic!("the answer was split: {text}"));
    let notice = text.find("queued (1 pending)").expect("notice printed");
    assert!(sentence < notice, "notice follows the answer: {text}");
}

#[test]
fn notices_deferred_by_a_stream_print_in_arrival_order_after_it() {
    let mut app = App::default();
    app.apply(PresenterEvent::AssistantDelta("partial".into()));
    app.note("first notice");
    app.note("second notice");
    app.apply(PresenterEvent::AssistantDone);
    let text = drain_text_lines(&mut app);
    let (answer, first, second) = (
        text.find("partial").unwrap(),
        text.find("first notice").unwrap(),
        text.find("second notice").unwrap(),
    );
    assert!(answer < first && first < second, "{text}");
    assert!(app.transcript_ui.deferred_notices.is_empty());
}

#[test]
fn a_tool_call_closes_the_segment_and_releases_the_notice_before_its_card() {
    let mut app = App::default();
    app.apply(PresenterEvent::AssistantDelta("Looking at it".into()));
    app.note(QUEUED);
    app.apply(PresenterEvent::ToolStart {
        name: "read_file".into(),
        args: r#"{"path":"a.rs"}"#.into(),
    });
    let text = drain_text_lines(&mut app);
    let (answer, notice) = (
        text.find("Looking at it").unwrap(),
        text.find("queued (1 pending)").unwrap(),
    );
    assert!(answer < notice, "{text}");
}

#[test]
fn a_notice_with_no_stream_in_flight_prints_at_once() {
    let mut app = App::default();
    app.note("compacted 12 messages");
    assert!(drain_text_lines(&mut app).contains("compacted 12 messages"));
    assert!(app.transcript_ui.deferred_notices.is_empty());
}

#[test]
fn a_session_warning_still_ends_the_segment_it_arrived_in() {
    // The completeness review and failover notices are emitted by the turn between two model
    // steps, so they are real boundaries of the answer (a re-answer must not glue onto it).
    let mut app = App::default();
    app.apply(PresenterEvent::AssistantDelta("First answer.".into()));
    app.apply(PresenterEvent::Warning("heads up".into()));
    assert!(!app.streaming_active && app.streaming.is_empty());
}

#[test]
fn a_new_user_turn_releases_notices_held_behind_a_stream_nobody_finished() {
    let mut app = App::default();
    app.apply(PresenterEvent::AssistantDelta("cut off".into()));
    app.note(QUEUED);
    app.submit_user("next prompt");
    let text = drain_text_lines(&mut app);
    let (answer, notice, prompt) = (
        text.find("cut off").unwrap(),
        text.find("queued (1 pending)").unwrap(),
        text.find("next prompt").unwrap(),
    );
    assert!(answer < notice && notice < prompt, "{text}");
}
