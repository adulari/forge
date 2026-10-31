//! The details rail: on wide terminals the same tasks and agents content as the band, plus
//! changed files and the context gauge, beside the transcript so it keeps its height.

use super::band::{context_spans, task_counts};
use super::common::{aligned, fmt_elapsed};
use super::*;
use crate::theme::glyphs::g;
use crate::theme::styles;

/// Width of the rail, border column included.
pub(crate) const RAIL_WIDTH: u16 = 38;
/// Terminal width from which details open as a rail instead of a bottom band.
pub(crate) const RAIL_MIN_TERMINAL: u16 = 150;

const MAX_TASK_ROWS: usize = 8;
const MAX_AGENT_ROWS: usize = 6;
const MAX_CHANGED_ROWS: usize = 5;

/// Whether details render as a rail for a region `area` wide.
pub(crate) fn wanted(app: &App, area: Rect) -> bool {
    app.fullscreen
        && !app.sidebar.visible(app.fullscreen, area.width)
        && app.activity_focused
        && area.width >= RAIL_MIN_TERMINAL
        && (!app.tasks.is_empty() || app.has_activity())
}

fn label(text: String) -> TextLine<'static> {
    TextLine::from(Span::styled(format!("  {text}"), styles::muted().bold()))
}

fn task_rows(app: &App, w: usize, out: &mut Vec<TextLine<'static>>) {
    use forge_types::TodoStatus::*;
    if app.tasks.is_empty() {
        return;
    }
    let glyphs = g();
    let (done, total) = task_counts(&app.tasks);
    out.push(label(format!("TASKS {done}/{total}")));
    // Open work first when the list is long.
    let mut order: Vec<usize> = (0..app.tasks.len()).collect();
    if order.len() > MAX_TASK_ROWS {
        order.sort_by_key(|&i| match app.tasks[i].status {
            InProgress => 0,
            Pending => 1,
            Done => 2,
        });
    }
    let hidden = order.len().saturating_sub(MAX_TASK_ROWS);
    for &i in order.iter().take(MAX_TASK_ROWS) {
        let task = &app.tasks[i];
        let (glyph, style) = match task.status {
            Done => (glyphs.task_done, styles::ok()),
            InProgress => (glyphs.task_active, styles::brand()),
            Pending => (glyphs.task_open, styles::faint()),
        };
        let elapsed = app.panel_elapsed(
            &format!("task:{}", task.title),
            task.status == InProgress,
            task.status == Done,
        );
        let right = elapsed
            .map(|d| vec![Span::styled(fmt_elapsed(d), styles::faint())])
            .unwrap_or_default();
        let text_style = if task.status == Done {
            styles::muted()
        } else {
            styles::fg()
        };
        out.push(aligned(
            vec![
                Span::styled(format!("  {glyph} "), style),
                Span::styled(task.title.clone(), text_style),
            ],
            right,
            w,
            1,
        ));
    }
    if hidden > 0 {
        out.push(TextLine::from(Span::styled(
            format!("    {} +{hidden} more", glyphs.ellipsis),
            styles::faint(),
        )));
    }
    out.push(TextLine::default());
}

fn agent_rows(app: &App, w: usize, out: &mut Vec<TextLine<'static>>) {
    let views = app.activity_summaries();
    if views.len() < 2 {
        return;
    }
    let glyphs = g();
    out.push(label("AGENTS".to_string()));
    // Keep the selected row on screen when the list is longer than the rail shows.
    let start = if app.activity_focused {
        (app.activity_idx + 1).saturating_sub(MAX_AGENT_ROWS)
    } else {
        0
    };
    for (i, v) in views.iter().enumerate().skip(start).take(MAX_AGENT_ROWS) {
        let selected = app.activity_focused && i == app.activity_idx;
        let running = v.status == ActivityStatus::Running;
        let (mark, mark_style) = match (v.kind, v.status) {
            (ActivityKind::MainChat, _) => (glyphs.live_dot.to_string(), styles::faint()),
            (_, ActivityStatus::Running) => {
                (glyphs.spin(app.tick).to_string(), styles::spinner(app.tick))
            }
            (_, ActivityStatus::Done) => (glyphs.ok.to_string(), styles::ok()),
            (_, ActivityStatus::Failed) => (glyphs.fail.to_string(), styles::err()),
            (_, ActivityStatus::Skipped) => ("-".to_string(), styles::faint()),
        };
        let elapsed = (v.kind != ActivityKind::MainChat)
            .then(|| app.panel_elapsed(&format!("agent:{i}:{}", v.title), running, !running))
            .flatten();
        let right = elapsed
            .map(|d| vec![Span::styled(fmt_elapsed(d), styles::faint())])
            .unwrap_or_default();
        let title_style = if selected {
            styles::fg().bold()
        } else if running {
            styles::fg()
        } else {
            styles::muted()
        };
        let caret = if selected { glyphs.prompt } else { " " };
        let row = aligned(
            vec![
                Span::styled(
                    format!(" {caret} "),
                    if selected {
                        styles::brand()
                    } else {
                        styles::faint()
                    },
                ),
                Span::styled(format!("{mark} "), mark_style),
                Span::styled(v.title.clone(), title_style),
            ],
            right,
            w,
            1,
        );
        let mut spans = row.spans;
        if selected {
            let fill = styles::selected_row();
            for span in &mut spans {
                span.style = span.style.patch(fill);
            }
        }
        out.push(TextLine::from(spans));
    }
    out.push(TextLine::default());
}

fn changed_rows(app: &App, w: usize, out: &mut Vec<TextLine<'static>>) {
    if app.turn_diffs.is_empty() {
        return;
    }
    let glyphs = g();
    out.push(label("CHANGED".to_string()));
    for diff in app.turn_diffs.iter().take(MAX_CHANGED_ROWS) {
        let path = std::path::Path::new(&diff.path)
            .strip_prefix(
                app.git_workspace
                    .as_deref()
                    .unwrap_or(std::path::Path::new("")),
            )
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| diff.path.clone());
        let right = vec![
            Span::styled(format!("+{}", diff.adds), styles::ok()),
            Span::raw(" "),
            Span::styled(format!("{}{}", glyphs.diff_minus, diff.dels), styles::err()),
        ];
        out.push(aligned(
            vec![Span::styled(
                format!("  {}", clip_front(&path, w.saturating_sub(14))),
                styles::fg(),
            )],
            right,
            w,
            1,
        ));
    }
    let hidden = app.turn_diffs.len().saturating_sub(MAX_CHANGED_ROWS) + app.turn_diffs_skipped;
    if hidden > 0 {
        out.push(TextLine::from(Span::styled(
            format!("    {} +{hidden} more", glyphs.ellipsis),
            styles::faint(),
        )));
    }
    out.push(TextLine::default());
}

/// Keep the tail of a path (the file name matters most), with a leading ellipsis when clipped.
fn clip_front(path: &str, max: usize) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= max || max < 2 {
        return path.to_string();
    }
    let tail: String = chars[chars.len() - (max - 1)..].iter().collect();
    format!("{}{tail}", g().ellipsis)
}

/// The rail's lines for a content width of `w` cells.
pub(crate) fn rail_lines(app: &App, w: usize) -> Vec<TextLine<'static>> {
    let mut out = Vec::new();
    task_rows(app, w, &mut out);
    agent_rows(app, w, &mut out);
    changed_rows(app, w, &mut out);
    out.push(label("CONTEXT".to_string()));
    let mut ctx = vec![Span::raw("  ")];
    ctx.extend(context_spans(app));
    out.push(TextLine::from(ctx));
    out
}

/// Draw the rail into `area`: a `line` rule down its left edge, then the sections.
pub(crate) fn render_rail(frame: &mut Frame, area: Rect, app: &App) {
    if area.width < 4 || area.height == 0 {
        return;
    }
    let rule = if g().unicode { "│" } else { "|" };
    let border: Vec<TextLine> = (0..area.height)
        .map(|_| TextLine::from(Span::styled(rule, styles::line())))
        .collect();
    frame.render_widget(Paragraph::new(border), Rect { width: 1, ..area });
    let body = Rect {
        x: area.x + 1,
        width: area.width - 1,
        ..area
    };
    let mut lines = rail_lines(app, body.width as usize);
    lines.truncate(body.height as usize);
    frame.render_widget(Paragraph::new(lines), body);
}
