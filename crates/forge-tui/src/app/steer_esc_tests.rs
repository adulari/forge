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
fn statusline_hint_reflects_esc_arming_and_startup_loading() {
    let mut app = App::default();
    app.loading = Some("starting session…");
    assert_eq!(
        crate::app::render::status_line::statusline_hint(&app),
        "starting session…"
    );
    app.loading = None;
    assert!(app.input.is_empty());
    assert_eq!(
        crate::app::render::status_line::statusline_hint(&app),
        "Ctrl+K actions · / · ? keys"
    );
    app.esc_tap();
    assert_eq!(
        crate::app::render::status_line::statusline_hint(&app),
        "esc again to rewind · Ctrl-C quit"
    );
}

#[test]
fn a_steered_prompt_is_echoed_as_a_user_line() {
    let mut app = App::default();
    app.fullscreen = true;
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
        remote_transcript_text(
            "  ▸ read_file  tools/frida/attach_cap.py                 ◍ running",
            &call
        ),
        "▸ read_file tools/frida/attach_cap.py"
    );
    let done = LineOrigin::tool_result("read_file", true);
    assert_eq!(
        remote_transcript_text(
            "  ▸ read_file  a.py            ✓ #!/usr/bin/env python3",
            &done
        ),
        "▸ read_file a.py ✓ #!/usr/bin/env python3"
    );
    // Non-tool lines keep their spacing (code blocks, tables).
    assert_eq!(
        remote_transcript_text("    indented   code", &LineOrigin::default()),
        "    indented   code"
    );
}
