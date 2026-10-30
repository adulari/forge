//! The onboarding frame: step rail (or dots on a narrow terminal), the page, the live job
//! panel, a one-line notice and the key chips.

use forge_onboarding::StepStatus;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::screen::{NoticeKind, Screen};
use super::text::{chips, pad, truncate};
use super::view_pages;
use crate::surface::{self, SurfaceTone};
use crate::theme::glyphs::g;
use crate::theme::styles;

/// Width from which the step rail shows; below it the steps collapse to dots.
pub const RAIL_MIN_WIDTH: u16 = 100;
const RAIL_WIDTH: u16 = 28;

pub fn render(f: &mut Frame, s: &Screen) {
    let area = f.area();
    let inner = surface::render_panel_with_context(
        f,
        area,
        surface::title(
            if s.reconnect_card {
                "Forge"
            } else {
                "Forge setup"
            },
            SurfaceTone::Accent,
        ),
        (!s.reconnect_card).then(|| {
            Span::styled(
                format!(" step {} of {} ", s.page + 1, s.session.plan.len()),
                styles::faint(),
            )
        }),
        None,
        SurfaceTone::Accent,
    );
    if inner.width < 40 || inner.height < 8 {
        return;
    }
    let pad_x = if inner.width >= 70 { 2 } else { 1 };
    let top = u16::from(inner.height >= 26);
    let body = Rect {
        x: inner.x + pad_x,
        y: inner.y + top,
        width: inner.width - pad_x * 2,
        height: inner.height - top,
    };

    let job = if s.sign_in.as_ref().is_some_and(|x| x.qr) {
        Vec::new()
    } else {
        job_lines(s)
    };
    let notice = notice_line(s, body.width as usize);
    let wide = inner.width >= RAIL_MIN_WIDTH && !s.reconnect_card;
    let dots_h = u16::from(!wide && !s.reconnect_card);
    let parts = Layout::vertical([
        Constraint::Length(dots_h),
        Constraint::Min(3),
        Constraint::Length(job.len() as u16),
        Constraint::Length(u16::from(notice.is_some())),
        Constraint::Length(1),
    ])
    .split(body);

    if !wide && !s.reconnect_card {
        f.render_widget(Paragraph::new(dots_line(s, body.width as usize)), parts[0]);
    }
    let page_area = if wide {
        let cols = Layout::horizontal([
            Constraint::Length(RAIL_WIDTH),
            Constraint::Length(2),
            Constraint::Min(30),
        ])
        .split(parts[1]);
        f.render_widget(Paragraph::new(rail_lines(s, cols[0])), cols[0]);
        cols[2]
    } else {
        parts[1]
    };

    let (lines, focus) = view_pages::page(s, page_area.width as usize, page_area.height as usize);
    let scroll = focus
        .filter(|row| *row >= page_area.height as usize)
        .map(|row| (row + 1 - page_area.height as usize) as u16)
        .unwrap_or(0);
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), page_area);

    if !job.is_empty() {
        f.render_widget(Paragraph::new(job), parts[2]);
    }
    if let Some(line) = notice {
        f.render_widget(Paragraph::new(line), parts[3]);
    }
    f.render_widget(Paragraph::new(chip_line(s, body.width as usize)), parts[4]);
}

/// One mark per step for the rail and the dots.
fn mark(s: &Screen, index: usize) -> (&'static str, ratatui::style::Style) {
    let glyphs = g();
    let step = &s.session.plan[index];
    if index == s.page {
        return (glyphs.prompt, styles::brand().add_modifier(Modifier::BOLD));
    }
    if s.step_done(index) {
        return (glyphs.ok, styles::ok());
    }
    if s.session.state.status(step.id) == StepStatus::Skipped {
        return (glyphs.info, styles::faint());
    }
    (glyphs.task_open, styles::faint())
}

fn rail_lines(s: &Screen, area: Rect) -> Vec<Line<'static>> {
    let total = s.session.plan.len();
    let height = area.height as usize;
    let first = if total <= height || s.page < height / 2 {
        0
    } else {
        (s.page + height / 2 + 1).min(total).saturating_sub(height)
    };
    let width = area.width as usize;
    s.session
        .plan
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, step)| {
            let (glyph, style) = mark(s, i);
            let current = i == s.page;
            let required = step.required && !s.step_done(i);
            let tag = if required { " required" } else { "" };
            let title = truncate(&step.title, width.saturating_sub(3 + tag.len()));
            let mut spans = vec![
                Span::styled(format!("{glyph} "), style),
                Span::styled(
                    title,
                    if current {
                        styles::fg().bold()
                    } else if s.step_done(i) {
                        styles::muted()
                    } else {
                        styles::fg()
                    },
                ),
            ];
            if required {
                spans.push(Span::styled(tag.to_string(), styles::warn()));
            }
            Line::from(spans)
        })
        .collect()
}

fn dots_line(s: &Screen, width: usize) -> Line<'static> {
    let glyphs = g();
    let (filled, empty) = if glyphs.unicode {
        ("●", "○")
    } else {
        ("*", "o")
    };
    let mut spans = Vec::new();
    for i in 0..s.session.plan.len() {
        let (text, style) = if i == s.page {
            (filled, styles::brand())
        } else if s.step_done(i) {
            (filled, styles::ok())
        } else {
            (empty, styles::faint())
        };
        spans.push(Span::styled(text.to_string(), style));
    }
    let used = s.session.plan.len();
    let title = truncate(
        &s.session.plan[s.page].title,
        width.saturating_sub(used + 3),
    );
    spans.push(Span::styled(format!("  {title}"), styles::fg().bold()));
    Line::from(spans)
}

fn job_lines(s: &Screen) -> Vec<Line<'static>> {
    let Some(job) = s
        .jobs
        .iter()
        .rev()
        .find(|j| j.state == forge_onboarding::JobState::Running)
    else {
        return Vec::new();
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{} ", g().spin(s.tick)), styles::spinner(s.tick)),
        Span::styled(job.label.clone(), styles::fg()),
    ])];
    let tail: Vec<&String> = s.log.iter().rev().take(3).collect();
    for line in tail.into_iter().rev() {
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", g().card_gutter), styles::faint()),
            Span::styled(truncate(line, 90), styles::faint()),
        ]));
    }
    lines
}

fn notice_line(s: &Screen, width: usize) -> Option<Line<'static>> {
    let notice = s.notice.as_ref()?;
    let (glyph, style) = match notice.kind {
        NoticeKind::Ok => (g().ok, styles::ok()),
        NoticeKind::Info => (g().info, styles::muted()),
        NoticeKind::Warn => (g().warn, styles::warn()),
        NoticeKind::Err => (g().fail, styles::err()),
    };
    Some(Line::from(vec![
        Span::styled(format!("{glyph} "), style),
        Span::styled(truncate(&notice.text, width.saturating_sub(3)), style),
    ]))
}

/// The key chips for the current page, in priority order. When they do not fit, the
/// page-specific ones go first so Enter, Back and Leave always stay.
fn chip_line(s: &Screen, width: usize) -> Line<'static> {
    let (enter, mut specific) = view_pages::chips_for(s);
    let mut tail: Vec<(String, String)> = Vec::new();
    if s.entry.is_none() {
        if s.page > 0 && s.sign_in.is_none() && !s.reconnect_card {
            tail.push(("b".into(), "Back".into()));
        }
        tail.push(("q".into(), "Not now".into()));
    }
    loop {
        let all: Vec<(String, String)> = std::iter::once(enter.clone())
            .chain(specific.iter().cloned())
            .chain(tail.iter().cloned())
            .collect();
        let text_width: usize = all.iter().map(|(k, l)| k.width() + 1 + l.width() + 3).sum();
        if text_width <= width || (specific.is_empty() && tail.is_empty()) {
            let pairs: Vec<(&str, &str)> =
                all.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();
            return chips(&pairs);
        }
        if !specific.is_empty() {
            specific.pop();
        } else {
            tail.remove(0);
        }
    }
}

/// A selectable row: caret and raised fill when focused, padded to `width`.
pub(crate) fn selectable(spans: Vec<Span<'static>>, selected: bool, width: usize) -> Line<'static> {
    let caret = Span::styled(
        format!("{} ", if selected { g().prompt } else { " " }),
        if selected {
            styles::brand()
        } else {
            styles::faint()
        },
    );
    let mut out = vec![caret];
    out.extend(spans);
    if selected {
        let fill = styles::selected_row();
        let used: usize = out.iter().map(|sp| sp.content.width()).sum();
        for span in &mut out {
            span.style = span.style.patch(fill);
        }
        out.push(Span::styled(" ".repeat(width.saturating_sub(used)), fill));
    }
    Line::from(out)
}

pub(crate) fn padded(text: &str, width: usize) -> String {
    pad(&truncate(text, width), width)
}
