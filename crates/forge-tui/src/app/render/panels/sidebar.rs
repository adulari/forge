//! The right sidebar (Ctrl+B): a persistent glance at the session on wide terminals. Context
//! gauge, spend, MCP health, tasks and where the chat is working, in one column beside the
//! transcript, so none of it needs a command to see.

use super::band::context_spans;
use super::common::aligned;
use super::*;
use crate::app::render::status_widgets::fmt_cost;
use crate::app::render::{human, model_short};
use crate::sidebar::WIDTH;
use crate::theme::glyphs::g;
use crate::theme::styles;

const MAX_TASK_ROWS: usize = 8;
const MAX_MCP_ROWS: usize = 8;

/// Columns the sidebar takes from `area`, zero when it is off or the frame is too narrow.
pub(crate) fn width(app: &App, area: Rect) -> u16 {
    if app.sidebar.visible(app.fullscreen, area.width) {
        WIDTH
    } else {
        0
    }
}

fn label(text: &str) -> TextLine<'static> {
    TextLine::from(Span::styled(format!(" {text}"), styles::muted().bold()))
}

fn clip(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max || max < 2 {
        return text.to_string();
    }
    let head: String = chars[..max - 1].iter().collect();
    format!("{head}{}", g().ellipsis)
}

fn dot_style(status: &str) -> ratatui::style::Style {
    match status {
        "connected" => styles::ok(),
        "reconnecting" | "slow" => styles::warn(),
        "failed" | "unauthorized" => styles::err(),
        _ => styles::faint(),
    }
}

fn session_rows(app: &App, w: usize, out: &mut Vec<TextLine<'static>>) {
    out.push(label("SESSION"));
    let model = app
        .routing
        .as_ref()
        .map(|r| model_short(Some(&r.model)))
        .unwrap_or_else(|| "no model yet".to_string());
    out.push(TextLine::from(Span::styled(
        format!(" {}", clip(&model, w.saturating_sub(2))),
        styles::fg(),
    )));
    let place = match (&app.git_location, app.workspace_path()) {
        (Some(loc), Some(dir)) => {
            let leaf = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("{leaf}:{}", loc.branch_label())
        }
        (Some(loc), None) => format!("{}:{}", loc.repo, loc.branch_label()),
        (None, Some(dir)) => dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        (None, None) => String::new(),
    };
    if !place.is_empty() {
        out.push(TextLine::from(Span::styled(
            format!(" {}", clip(&place, w.saturating_sub(2))),
            styles::muted(),
        )));
    }
    out.push(TextLine::default());
}

fn context_rows(app: &App, out: &mut Vec<TextLine<'static>>) {
    out.push(label("CONTEXT"));
    let mut ctx = vec![Span::raw(" ")];
    ctx.extend(context_spans(app));
    out.push(TextLine::from(ctx));
    out.push(TextLine::default());
}

fn spend_rows(app: &App, w: usize, out: &mut Vec<TextLine<'static>>) {
    out.push(label("SPEND"));
    out.push(aligned(
        vec![Span::styled(
            format!(" {}", fmt_cost(app.cost_usd)),
            styles::fg(),
        )],
        vec![Span::styled(
            format!("{} tok ", human(app.context_tokens)),
            styles::faint(),
        )],
        w,
        0,
    ));
    out.push(TextLine::default());
}

fn mcp_rows(app: &App, w: usize, out: &mut Vec<TextLine<'static>>) {
    if app.sidebar.mcp.is_empty() {
        return;
    }
    out.push(label("MCP"));
    for server in app.sidebar.mcp.iter().take(MAX_MCP_ROWS) {
        let quiet = server.status == "connected";
        out.push(aligned(
            vec![
                Span::styled(format!(" {} ", g().live_dot), dot_style(&server.status)),
                Span::styled(
                    clip(&server.name, w.saturating_sub(16)),
                    if quiet { styles::muted() } else { styles::fg() },
                ),
            ],
            if quiet {
                Vec::new()
            } else {
                vec![Span::styled(format!("{} ", server.status), styles::faint())]
            },
            w,
            0,
        ));
    }
    let hidden = app.sidebar.mcp.len().saturating_sub(MAX_MCP_ROWS);
    if hidden > 0 {
        out.push(TextLine::from(Span::styled(
            format!("   {} +{hidden} more", g().ellipsis),
            styles::faint(),
        )));
    }
    out.push(TextLine::default());
}

fn task_rows(app: &App, w: usize, out: &mut Vec<TextLine<'static>>) {
    use forge_types::TodoStatus::*;
    if app.tasks.is_empty() {
        return;
    }
    let (done, total) = band::task_counts(&app.tasks);
    out.push(label(&format!("TASKS {done}/{total}")));
    let mut order: Vec<usize> = (0..app.tasks.len()).collect();
    if order.len() > MAX_TASK_ROWS {
        order.sort_by_key(|&i| match app.tasks[i].status {
            InProgress => 0,
            Pending => 1,
            Done => 2,
        });
    }
    let glyphs = g();
    for &i in order.iter().take(MAX_TASK_ROWS) {
        let task = &app.tasks[i];
        let (glyph, style) = match task.status {
            Done => (glyphs.task_done, styles::ok()),
            InProgress => (glyphs.task_active, styles::brand()),
            Pending => (glyphs.task_open, styles::faint()),
        };
        let text_style = if task.status == Done {
            styles::muted()
        } else {
            styles::fg()
        };
        out.push(TextLine::from(vec![
            Span::styled(format!(" {glyph} "), style),
            Span::styled(clip(&task.title, w.saturating_sub(4)), text_style),
        ]));
    }
    let hidden = app.tasks.len().saturating_sub(MAX_TASK_ROWS);
    if hidden > 0 {
        out.push(TextLine::from(Span::styled(
            format!("   {} +{hidden} more", glyphs.ellipsis),
            styles::faint(),
        )));
    }
    out.push(TextLine::default());
}

/// The sidebar's lines for a content width of `w` cells.
pub(crate) fn lines(app: &App, w: usize) -> Vec<TextLine<'static>> {
    let mut out = Vec::new();
    session_rows(app, w, &mut out);
    context_rows(app, &mut out);
    spend_rows(app, w, &mut out);
    mcp_rows(app, w, &mut out);
    task_rows(app, w, &mut out);
    out
}

/// Draw the sidebar into `area`: a rule down its left edge, the sections, and the key to hide it
/// on the last row.
pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
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
    let mut rows = lines(app, body.width as usize);
    let keep = usize::from(body.height).saturating_sub(1);
    rows.truncate(keep);
    frame.render_widget(Paragraph::new(rows), body);
    let hint = TextLine::from(Span::styled(" Ctrl+B hide", styles::faint()));
    frame.render_widget(
        Paragraph::new(hint),
        Rect {
            y: body.y + body.height - 1,
            height: 1,
            ..body
        },
    );
}
