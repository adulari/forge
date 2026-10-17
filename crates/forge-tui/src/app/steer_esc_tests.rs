//! Idle-Esc double-tap, the Ctrl-C/Esc split, and the mid-turn steer echo.

use super::*;

#[test]
fn ctrl_c_interrupt_quits_the_input_like_esc() {
    let mut buf = "draft".to_string();
    let mut cur = buf.len();
    assert_eq!(
        handle_key(&mut buf, &mut cur, KeyKind::Interrupt),
        InputOutcome::Quit
    );
}

#[test]
fn a_second_idle_esc_inside_the_window_opens_rewind() {
    let mut app = App::default();
    assert!(!app.esc_tap(), "first press only arms");
    assert!(app.esc_hint_active());
    assert!(app.esc_tap(), "second press completes the double-tap");
    assert!(!app.esc_hint_active(), "consumed");
    // Presses far apart never chain.
    assert!(!app.esc_tap());
    app.esc_armed = Some(std::time::Instant::now() - ESC_DOUBLE_TAP * 2);
    assert!(
        !app.esc_tap(),
        "a stale arm restarts the window instead of firing"
    );
}

#[test]
fn the_composer_border_reflects_esc_arming_and_startup_loading() {
    let mut app = App {
        loading: Some("starting session…"),
        ..Default::default()
    };
    assert!(crate::app::render::chrome::live_text(&app).contains("Starting session…"));
    app.loading = None;
    assert!(app.input.is_empty());
    assert!(crate::app::render::chrome::hint_text(&app).contains("Ctrl+K actions"));
    app.esc_tap();
    assert!(crate::app::render::chrome::live_text(&app).contains("Esc again to rewind"));
}

#[test]
fn a_steered_prompt_is_echoed_as_a_user_line() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.apply(PresenterEvent::Steered("also rename the helper".into()));
    let text = app
        .flush
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("steer"), "header names the steer: {text}");
    assert!(text.contains("also rename the helper"), "{text}");
}

#[test]
fn remote_transcript_rows_drop_card_padding_and_the_running_mark() {
    let call = LineOrigin::tool_call("read_file");
    assert_eq!(
        remote_transcript_text("⠋ Read   tools/frida/attach_cap.py          running", &call),
        "⠋ Read tools/frida/attach_cap.py"
    );
    let done = LineOrigin::tool_result("read_file", true);
    assert_eq!(
        remote_transcript_text("✓ Read   a.py            42 lines · 0.1s", &done),
        "✓ Read a.py 42 lines · 0.1s"
    );
    // A finished row whose own text ends in the word is left alone.
    assert_eq!(
        remote_transcript_text("✓ Run    make            still running", &done),
        "✓ Run make still running"
    );
    // Non-tool lines keep their spacing (code blocks, tables).
    assert_eq!(
        remote_transcript_text("    indented   code", &LineOrigin::default()),
        "    indented   code"
    );
}
