//! Shared fixtures for the app tests, plus the per-surface test modules.

use super::*;
use crate::app::render::chrome::hint_text;
use crate::app::render::input::{input_spans, input_text_rows};
use crate::app::render::live::split_panel_budget;
use crate::app::render::permission::prompt_height;
use crate::app::render::{compact_band_height, cost_cell, fmt_dur, render_transcript_area};
use crate::app_remote::REMOTE_DIFF_MAX_FILES;
use crate::theme::color;
use crate::voice::VOICE_WAVEFORM_BARS;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

mod child_view;
mod chrome;
mod chrome_surfaces;
mod interaction;
mod notice_during_stream;
mod panels;
mod queue_band;
mod readability;
mod remote;
mod state;
mod transcript;
mod trust_hint;
mod update_band;
mod welcome_narrow;

/// Render the pinned live region at its natural (dynamic) live height — so the sticky task +
/// subagent panels get their own rows, exactly as the real I/O shell sizes the viewport.
fn screen(app: &App) -> String {
    screen_wh(app, 80, LIVE_H)
}

fn screen_wh(app: &App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| render_live(f, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}

/// One string per rendered row.
fn screen_rows(app: &App, w: u16, h: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| render_live(f, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect())
        .collect()
}

/// Concatenated text of everything queued for scrollback.
fn flush_text(app: &mut App) -> String {
    app.drain_flush()
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect::<Vec<_>>()
        .join("")
}

fn sample_diff(path: &str, old: &str, new: &str) -> forge_types::FileDiff {
    forge_types::FileDiff {
        path: path.into(),
        kind: forge_types::DiffKind::Modified,
        old: Some(old.into()),
        new: Some(new.into()),
        lang: Some("rust".into()),
        binary: false,
    }
}

fn write_result(app: &mut App, ok: bool) {
    app.apply(crate::PresenterEvent::ToolResult {
        name: "write_file".into(),
        ok,
        summary: if ok { "ok" } else { "denied" }.into(),
        detail: None,
    });
}

fn overlay_picker_rows() -> Vec<crate::commands::PickerRow> {
    vec![
        crate::commands::PickerRow {
            id: "row-a".into(),
            title: "Alpha".into(),
            subtitle: "first".into(),
        },
        crate::commands::PickerRow {
            id: "row-b".into(),
            title: "Beta".into(),
            subtitle: "second".into(),
        },
    ]
}

const NOTICE: &str =
    "[background job 3429249 `/tmp/batch.sh > /tmp/batch_result.log 2>&1` exited 0]";

fn drain_text_lines(app: &mut App) -> String {
    app.drain_flush()
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A full-screen app with a rendered transcript geometry, ready to receive tool events and
/// be clicked. Cards are full-screen-only, so `fullscreen` is what arms them.
fn card_app() -> App {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    // The assistant's label row opens the turn, so the first card sits on row 1.
    app.ensure_assistant_label();
    let _ = app.drain_flush();
    app.last_width.set(100);
    app.transcript_geom.set(Some(TranscriptGeom {
        col0: 0,
        row0: 0,
        width: 100,
        height: 40,
        scroll: 0,
    }));
    app
}

fn main_log_text(app: &App) -> String {
    app.main_log
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_a_tool(app: &mut App, name: &str, args: &str, detail: Option<&str>) {
    app.apply(PresenterEvent::ToolStart {
        name: name.into(),
        args: args.into(),
    });
    let _ = app.drain_flush();
    app.apply(PresenterEvent::ToolResult {
        name: name.into(),
        ok: true,
        summary: format!("{name}: exit 0 in 12ms"),
        detail: detail.map(str::to_string),
    });
    let _ = app.drain_flush();
}

fn todo(title: &str, status: forge_types::TodoStatus) -> forge_types::TodoItem {
    forge_types::TodoItem {
        title: title.into(),
        status,
        assignee: None,
    }
}
