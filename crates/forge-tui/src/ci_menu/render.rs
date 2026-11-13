//! Draws `/ci`: one row per planned step with its state, then the skipped ones with their reason.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::{CiMenu, CiRow, CiState, Tone};
use crate::app::App;
use crate::surface::{self, SurfaceTone};
use crate::theme::{glyphs::g, styles};

fn trunc(text: &str, max: usize) -> String {
    if UnicodeWidthStr::width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        if UnicodeWidthStr::width(out.as_str()) + 2 > max {
            break;
        }
        out.push(c);
    }
    out.push_str(g().ellipsis);
    out
}

fn row_line(r: &CiRow, focused: bool, inner_w: usize) -> Line<'static> {
    let (glyph, style, label) = match r.state {
        CiState::Pass => (g().ok, styles::ok(), "pass"),
        CiState::Cached => (g().ok, styles::ok(), "pass*"),
        CiState::Fail => (g().fail, styles::err(), "fail"),
        CiState::Pending => ("\u{00b7}", styles::muted(), "todo"),
        CiState::NotRun => ("\u{00b7}", styles::faint(), "not run"),
        CiState::Skip => ("-", styles::faint(), "skip"),
    };
    let name_w = (inner_w / 3).clamp(16, 44);
    let name = trunc(&r.name, name_w);
    let pad = " ".repeat(name_w.saturating_sub(UnicodeWidthStr::width(name.as_str())) + 1);
    let used = 2 + 2 + 8 + name_w + 1;
    let mut line = Line::from(vec![
        Span::styled(
            format!("{} ", if focused { g().prompt } else { " " }),
            if focused {
                styles::brand()
            } else {
                styles::faint()
            },
        ),
        Span::styled(format!("{glyph} "), style),
        Span::styled(format!("{label:<8}"), style),
        Span::styled(format!("{name}{pad}"), styles::fg()),
        Span::styled(
            trunc(&r.detail, inner_w.saturating_sub(used)),
            styles::faint(),
        ),
    ]);
    if focused {
        line.style = styles::selected_row();
    }
    line
}

fn footer(m: &CiMenu) -> String {
    let sep = g().sep;
    if m.running {
        return " Running the plan ".to_string();
    }
    format!(
        " {} move {sep} Enter {} {sep} r run {sep} R reload {sep} Esc close ",
        g().up_down,
        if m.expanded {
            "hide output"
        } else {
            "show output"
        }
    )
}

/// Rows the detail pane takes under the list when open.
const DETAIL_ROWS: usize = 9;

/// The focused row opened up: a skip's reason, or a step's command, environment and the tail of
/// its output. When it does not fit, the output keeps its last lines (where the error is).
fn detail_lines(r: &CiRow, rows: usize, width: usize) -> Vec<Line<'static>> {
    let faint = |t: String| Line::from(Span::styled(trunc(&t, width), styles::faint()));
    if r.state == CiState::Skip {
        return vec![faint(format!("Skipped: {}", r.detail))];
    }
    let mut head: Vec<Line<'static>> = r
        .command
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(3)
        .map(|l| Line::from(Span::styled(trunc(&format!("$ {l}"), width), styles::fg())))
        .collect();
    if !r.env.is_empty() {
        head.push(faint(format!("env  {}", r.env.join(" "))));
    }
    let mut tail: Vec<Line<'static>> = r
        .tail
        .lines()
        .map(|l| Line::from(Span::styled(trunc(l, width), styles::err())))
        .collect();
    if tail.is_empty() && r.state == CiState::Fail {
        tail.push(faint("(no output captured)".to_string()));
    }
    let room = rows.saturating_sub(head.len());
    if tail.len() > room {
        tail.drain(..tail.len() - room);
    }
    head.extend(tail);
    head.truncate(rows);
    head
}

pub(crate) fn render_ci_menu(f: &mut Frame, app: &App, region: Rect) {
    let m = &app.ci_menu;
    if !m.open {
        return;
    }
    surface::dim_backdrop(f, region);
    let card_w = region.width.saturating_sub(2).clamp(1, 120);
    let inner_w = (card_w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let want = m.rows.len().max(1) as u16
        + 4
        + if m.expanded {
            DETAIL_ROWS as u16 + 1
        } else {
            0
        };
    let padded = want + 2 + 2 * crate::overlays::PAD_Y;
    let card_h = if padded >= crate::overlays::SHORT_REGION {
        padded
    } else {
        want + 2
    }
    .max(9)
    .min(region.height);
    let card = Rect {
        x: region.x + region.width.saturating_sub(card_w) / 2,
        y: region.y + region.height.saturating_sub(card_h) / 2,
        width: card_w,
        height: card_h,
    };
    let (pass, fail, skip) = m.counts();
    let inner = crate::overlays::padded_panel(
        f,
        card,
        surface::title("Local CI", SurfaceTone::Brand),
        Some(Span::styled(
            format!(
                " {pass} pass {} {fail} fail {} {skip} skipped ",
                g().sep,
                g().sep
            ),
            styles::faint(),
        )),
        Some(Span::styled(footer(m), styles::faint())),
        SurfaceTone::Brand,
    );
    if inner.width < 20 || inner.height < 3 {
        return;
    }
    let head = if m.loading {
        format!(
            "{} Reading the workflows\u{2026}",
            g().spin(m.tick as usize)
        )
    } else if m.running {
        format!(
            "{} Running {}\u{2026} fail-fast, results appear when it ends",
            g().spin(m.tick as usize),
            m.progress.as_deref().unwrap_or("")
        )
    } else {
        m.summary
            .clone()
            .unwrap_or_else(|| "No local run recorded yet. Press r to run.".to_string())
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            trunc(&head, inner_w),
            styles::muted(),
        ))),
        Rect { height: 1, ..inner },
    );
    let notice_rows = u16::from(m.notice.is_some());
    let body = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(1 + notice_rows),
        ..inner
    };
    let detail_h = if m.expanded && !m.rows.is_empty() {
        DETAIL_ROWS.min((body.height as usize).saturating_sub(4))
    } else {
        0
    };
    let h = (body.height as usize).saturating_sub(if detail_h > 0 { detail_h + 1 } else { 0 });
    let offset = m
        .cursor
        .saturating_sub(h.saturating_sub(1))
        .min(m.rows.len().saturating_sub(h));
    let lines: Vec<Line> = if m.rows.is_empty() && !m.loading {
        vec![Line::from(Span::styled(
            "No workflow steps can run locally in this project.",
            styles::muted(),
        ))]
    } else {
        m.rows
            .iter()
            .enumerate()
            .skip(offset)
            .take(h)
            .map(|(i, r)| row_line(r, i == m.cursor, inner_w))
            .collect()
    };
    f.render_widget(
        Paragraph::new(lines),
        Rect {
            height: h as u16,
            ..body
        },
    );
    if detail_h > 0 {
        if let Some(row) = m.rows.get(m.cursor) {
            let y = body.y + h as u16;
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "\u{2500}".repeat(inner_w),
                    styles::faint(),
                ))),
                Rect {
                    y,
                    height: 1,
                    ..body
                },
            );
            f.render_widget(
                Paragraph::new(detail_lines(row, detail_h, inner_w)),
                Rect {
                    y: y + 1,
                    height: detail_h as u16,
                    ..body
                },
            );
        }
    }
    if let Some(n) = &m.notice {
        let (style, glyph) = match n.tone {
            Tone::Ok => (styles::ok(), g().ok),
            Tone::Warn => (styles::warn(), g().warn),
            Tone::Err => (styles::err(), g().fail),
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                trunc(&format!("{glyph} {}", n.text), inner.width as usize),
                style,
            ))),
            Rect {
                y: inner.y + inner.height - 1,
                height: 1,
                ..inner
            },
        );
    }
}
