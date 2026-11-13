//! Draws `/issues`: a centered card with the start options and a scrolling issue list.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::{IssueRow, IssuesMenu, Load, Tone};
use crate::app::App;
use crate::surface::{self, SurfaceTone};
use crate::theme::{glyphs::g, styles};

fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn trunc(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        if width(&out) + width(c.encode_utf8(&mut [0; 4])) + 1 > max {
            break;
        }
        out.push(c);
    }
    out.push_str(g().ellipsis);
    out
}

fn pad(text: &str, to: usize) -> String {
    format!("{text}{}", " ".repeat(to.saturating_sub(width(text))))
}

fn row_line(m: &IssuesMenu, r: &IssueRow, focused: bool, w: usize) -> Line<'static> {
    let mark = if m.marked.contains(&r.number) {
        "[x] "
    } else {
        "[ ] "
    };
    let number = pad(&format!("#{}", r.number), 7);
    let drain_state = m.drain.as_ref().and_then(|d| d.item_label(r.number));
    let labels = if let Some(state) = drain_state {
        state.to_string()
    } else if r.labels.is_empty() {
        String::new()
    } else {
        format!("[{}]", r.labels.join(", "))
    };
    let comments = if r.comments == 0 {
        String::new()
    } else {
        format!("{}c", r.comments)
    };
    let side_w = (w / 4).clamp(8, 28);
    let title_w = w.saturating_sub(2 + 4 + 7 + side_w + 5 + 2).max(8);
    let mut line = Line::from(vec![
        Span::styled(
            format!("{} ", if focused { g().prompt } else { " " }),
            if focused {
                styles::brand()
            } else {
                styles::faint()
            },
        ),
        Span::styled(
            mark.to_string(),
            if m.marked.contains(&r.number) {
                styles::brand()
            } else {
                styles::faint()
            },
        ),
        Span::styled(number, styles::muted()),
        Span::styled(pad(&trunc(&r.title, title_w), title_w + 1), styles::fg()),
        Span::styled(
            pad(&trunc(&labels, side_w), side_w + 1),
            if drain_state.is_some() {
                styles::muted()
            } else {
                styles::faint()
            },
        ),
        Span::styled(comments, styles::faint()),
    ]);
    if focused {
        line.style = styles::selected_row();
    }
    line
}

fn options_line(m: &IssuesMenu) -> Line<'static> {
    Line::from(vec![
        Span::styled("Worktree ".to_string(), styles::muted()),
        Span::styled(
            if m.worktree { " on " } else { " off " }.to_string(),
            styles::key_chip(),
        ),
        Span::raw("   "),
        Span::styled("PR Watch ".to_string(), styles::muted()),
        Span::styled(format!(" {} ", m.watch.label()), styles::key_chip()),
        Span::raw("   "),
        Span::styled("At once ".to_string(), styles::muted()),
        Span::styled(format!(" {} ", m.parallel), styles::key_chip()),
        Span::raw("   "),
        Span::styled("Budget ".to_string(), styles::muted()),
        Span::styled(format!(" {} ", m.budget.label()), styles::key_chip()),
    ])
}

/// The drain's progress line, with its envelope meter and why it is paused.
fn drain_line(m: &IssuesMenu, max: usize) -> Option<Line<'static>> {
    let d = m.drain.as_ref()?;
    let mut text = d.line.clone();
    if let Some(meter) = &d.meter {
        text.push_str(&format!("  {}  {meter}", g().sep));
    }
    if d.status == "paused" {
        if let Some(reason) = &d.paused_reason {
            text.push_str(&format!("  {}  {reason}", g().sep));
        }
    }
    let style = if d.is_live() {
        styles::brand()
    } else {
        styles::faint()
    };
    Some(Line::from(Span::styled(trunc(&text, max), style)))
}

fn footer(m: &IssuesMenu) -> String {
    if m.starting {
        return " Starting ".to_string();
    }
    let sep = g().sep;
    if !m.marked.is_empty() {
        return format!(
            " Space mark {sep} D drain {} {sep} +/- at once {sep} b budget {sep} u clear {sep} Esc close ",
            m.marked.len()
        );
    }
    let stop = if m.drain.as_ref().is_some_and(super::DrainView::is_live) {
        format!(" {sep} x stop drain")
    } else {
        String::new()
    };
    format!(
        " {} move {sep} Enter start {sep} Space mark {sep} w worktree {sep} Tab watch {sep} r refresh{stop} {sep} Esc close ",
        g().up_down,
    )
}

pub(crate) fn render_issues_menu(f: &mut Frame, app: &App, region: Rect) {
    let m = &app.issues_menu;
    if !m.open {
        return;
    }
    surface::dim_backdrop(f, region);
    let card_w = region.width.saturating_sub(2).clamp(1, 120);
    let inner_w = (card_w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let list_rows = m.rows.len().max(1) as u16;
    let want = list_rows + 5;
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
    let inner = crate::overlays::padded_panel(
        f,
        card,
        surface::title("GitHub issues", SurfaceTone::Brand),
        Some(Span::styled(
            format!(" {} open ", m.rows.len()),
            styles::faint(),
        )),
        Some(Span::styled(footer(m), styles::faint())),
        SurfaceTone::Brand,
    );
    if inner.width < 20 || inner.height < 4 {
        return;
    }
    f.render_widget(Paragraph::new(options_line(m)), Rect { height: 1, ..inner });
    if let Some(line) = drain_line(m, inner_w) {
        f.render_widget(
            Paragraph::new(line),
            Rect {
                y: inner.y + 1,
                height: 1,
                ..inner
            },
        );
    }
    let notice_rows = u16::from(m.notice.is_some());
    let body = Rect {
        y: inner.y + 2,
        height: inner.height.saturating_sub(2 + notice_rows),
        ..inner
    };
    let message = match &m.load {
        Load::Loading => Some((
            format!("{} Reading issues\u{2026}", g().spin(m.tick as usize)),
            styles::muted(),
        )),
        Load::Failed(e) => Some((format!("{} {}", g().fail, trunc(e, inner_w)), styles::err())),
        Load::Ready if m.rows.is_empty() => Some(("No open issues.".to_string(), styles::muted())),
        Load::Ready => None,
    };
    if let Some((text, style)) = message {
        f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), body);
    } else {
        let h = body.height as usize;
        let offset = m
            .cursor
            .saturating_sub(h.saturating_sub(1))
            .min(m.rows.len().saturating_sub(h));
        let lines: Vec<Line> = m
            .rows
            .iter()
            .enumerate()
            .skip(offset)
            .take(h)
            .map(|(i, r)| row_line(m, r, i == m.cursor, inner_w))
            .collect();
        f.render_widget(Paragraph::new(lines), body);
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
