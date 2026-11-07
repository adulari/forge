//! Draws `/queue`: a centered card with one block per repository and a row per PR.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::{Load, QueueMenu, QueueRow, Tone};
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

fn state_style(state: &str) -> Style {
    match state {
        "head" | "checks" => styles::brand(),
        "held" | "skipped" => styles::warn(),
        "merged" => styles::ok(),
        "blocked" => styles::err(),
        _ => styles::fg(),
    }
}

/// One line per repository block and one per PR, in list order; `focus` is the PR line index.
fn lines(m: &QueueMenu, w: usize) -> (Vec<Line<'static>>, usize) {
    let mut out = Vec::new();
    let mut focus = 0;
    let mut repo: Option<&str> = None;
    for (i, r) in m.rows.iter().enumerate() {
        if repo != Some(r.repo.as_str()) {
            repo = Some(r.repo.as_str());
            out.push(Line::from(Span::styled(trunc(&r.repo, w), styles::muted())));
        }
        if i == m.cursor {
            focus = out.len();
        }
        out.push(row_line(r, i == m.cursor, w));
    }
    (out, focus)
}

fn row_line(r: &QueueRow, focused: bool, w: usize) -> Line<'static> {
    let place = r
        .position
        .map_or_else(|| "-".to_string(), |p| format!("#{p}"));
    let state = pad(r.state_label(), 14);
    let reason_w = w.saturating_sub(2 + 4 + 9 + 15).max(8);
    let mut reason = r.reason.clone();
    if r.pushes > 0 {
        reason.push_str(&format!(
            " ({} push{})",
            r.pushes,
            if r.pushes == 1 { "" } else { "es" }
        ));
    }
    let mut line = Line::from(vec![
        Span::styled(
            format!("{} ", if focused { g().prompt } else { " " }),
            if focused {
                styles::brand()
            } else {
                styles::faint()
            },
        ),
        Span::styled(pad(&place, 4), styles::muted()),
        Span::styled(pad(&r.label(), 9), styles::fg()),
        Span::styled(state, state_style(&r.state)),
        Span::styled(trunc(reason.trim(), reason_w), styles::faint()),
    ]);
    if focused {
        line.style = styles::selected_row();
    }
    line
}

fn footer(m: &QueueMenu) -> String {
    if m.busy {
        return " Working ".to_string();
    }
    let sep = g().sep;
    format!(
        " {} move {sep} h hold {sep} r release {sep} f front {sep} t retry {sep} Enter fit {sep} R refresh {sep} Esc close ",
        g().up_down
    )
}

pub(crate) fn render_queue_menu(f: &mut Frame, app: &App, region: Rect) {
    let m = &app.queue_menu;
    if !m.open {
        return;
    }
    surface::dim_backdrop(f, region);
    let card_w = region.width.saturating_sub(2).clamp(1, 120);
    let inner_w = (card_w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let repos = {
        let mut n = 0;
        let mut last: Option<&str> = None;
        for r in &m.rows {
            if last != Some(r.repo.as_str()) {
                n += 1;
                last = Some(r.repo.as_str());
            }
        }
        n
    };
    let list_rows = (m.rows.len() + repos).max(1) as u16;
    let want = list_rows + 3;
    let padded = want + 2 + 2 * crate::overlays::PAD_Y;
    let card_h = if padded >= crate::overlays::SHORT_REGION {
        padded
    } else {
        want + 2
    }
    .max(8)
    .min(region.height);
    let card = Rect {
        x: region.x + region.width.saturating_sub(card_w) / 2,
        y: region.y + region.height.saturating_sub(card_h) / 2,
        width: card_w,
        height: card_h,
    };
    let waiting = m.rows.iter().filter(|r| r.position.is_some()).count();
    let inner = crate::overlays::padded_panel(
        f,
        card,
        surface::title("Merge queue", SurfaceTone::Brand),
        Some(Span::styled(
            format!(" {waiting} waiting "),
            styles::faint(),
        )),
        Some(Span::styled(footer(m), styles::faint())),
        SurfaceTone::Brand,
    );
    if inner.width < 20 || inner.height < 3 {
        return;
    }
    let notice_rows = u16::from(m.notice.is_some());
    let body = Rect {
        height: inner.height.saturating_sub(notice_rows),
        ..inner
    };
    let message = match &m.load {
        Load::Loading => Some((
            format!("{} Reading the queue\u{2026}", g().spin(m.tick as usize)),
            styles::muted(),
        )),
        Load::Failed(e) => Some((format!("{} {}", g().fail, trunc(e, inner_w)), styles::err())),
        Load::Ready if m.rows.is_empty() => Some((
            "No PRs are queued. Arm PR Watch in merge mode (/ship, /issues) to queue them."
                .to_string(),
            styles::muted(),
        )),
        Load::Ready => None,
    };
    if let Some((text, style)) = message {
        f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), body);
    } else {
        let (all, focus) = lines(m, inner_w);
        let h = body.height as usize;
        let offset = focus
            .saturating_sub(h.saturating_sub(1))
            .min(all.len().saturating_sub(h));
        let shown: Vec<Line> = all.into_iter().skip(offset).take(h).collect();
        f.render_widget(Paragraph::new(shown), body);
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
