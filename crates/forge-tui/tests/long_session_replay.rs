use std::time::{Duration, Instant};

use forge_tui::App;
use forge_types::ReplayItem;

#[test]
fn native_history_sized_replay_stays_bounded_and_responsive() {
    let mut items = Vec::with_capacity(15_000);
    // More turns and tool events than the largest real Codex/Claude history observed by the
    // aggregate-only history profiler (654 user turns / 6,995 tool calls).
    for turn in 0..768 {
        items.push(ReplayItem::User(format!(
            "continue the long-running task at checkpoint {turn}"
        )));
        items.push(ReplayItem::Assistant(format!(
            "Completed checkpoint **{turn}**.\n\n- preserved context\n- verified state"
        )));
        for tool in 0..10 {
            items.push(ReplayItem::Tool {
                name: "shell".to_string(),
                args: format!(r#"{{"command":"verify checkpoint {turn}-{tool}"}}"#),
            });
            items.push(ReplayItem::ToolResult {
                name: "shell".to_string(),
                ok: true,
                summary: format!("checkpoint {turn}-{tool} verified"),
            });
        }
    }

    let started = Instant::now();
    let mut app = App::default();
    app.replay_history(&items);
    let lines = app.drain_flush();

    assert!(
        started.elapsed() < Duration::from_secs(5),
        "replaying a 768-turn / 7,680-tool history became pathologically slow: {:?}",
        started.elapsed()
    );
    assert!(
        lines.len() >= items.len(),
        "every replay item should remain represented in terminal scrollback"
    );
    let (retained_rows, _) = app.transcript_metrics(160, 1);
    assert!(
        retained_rows <= 5_000,
        "the in-process full-screen transcript must remain bounded, got {retained_rows} rows"
    );
}
