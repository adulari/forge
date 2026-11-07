//! A turn that stops short says so in the scrollback the moment it ends.

use super::*;

#[test]
fn the_stop_line_reaches_the_scrollback_live() {
    let mut app = App::default();
    app.apply(PresenterEvent::TurnStopped {
        reason: "stalled".into(),
        text: "Turn stopped (no progress): edits are kept; send continue to start a fresh attempt"
            .into(),
    });
    let text = drain_text_lines(&mut app);
    assert!(text.contains("Turn stopped (no progress)"), "{text}");
    assert_eq!(text.matches("Turn stopped").count(), 1, "{text}");
}
