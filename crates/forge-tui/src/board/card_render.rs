//! One session card on the board, in the Forgeline list/card grammar: no frame, a `❯` caret and
//! the raised fill on the selected card, a status glyph and an exceptions-only status pill on the
//! title row, then the detail lines indented under the title.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::theme::glyphs::g;
use crate::theme::{color, styles};

use super::dispatch_render as dr;
use super::model::{fmt_age, fmt_cost, model_short, Card, Health, SignalLevel};
use super::state::{BoardApp, Hit};
use super::widgets as w;

/// Columns the title text is indented: caret, space, status glyph, space.
const INDENT: u16 = 4;

/// A status pill: the health word on the raised fill in the health's tone. Idle and busy cards
/// need none (their column already says it), so only the exceptions get one.
pub(super) fn status_pill(health: Health) -> Option<(String, Style)> {
    let (text, tone) = match health {
        Health::Waiting => ("needs you", styles::err()),
        Health::Stalled => ("stalled", styles::warn()),
        Health::Failed => ("failed", styles::err()),
        Health::Busy | Health::Idle | Health::Finished => return None,
    };
    Some((format!(" {text} "), tone.patch(styles::selected_row())))
}

/// The always-present variant used in the detail header.
pub(super) fn health_pill(health: Health) -> (String, Style) {
    let tone = match health {
        Health::Waiting | Health::Failed => styles::err(),
        Health::Stalled => styles::warn(),
        Health::Idle => styles::ok(),
        Health::Busy => styles::muted(),
        Health::Finished => styles::faint(),
    };
    (
        format!(" {} ", health.label()),
        tone.patch(styles::selected_row()),
    )
}

pub(super) fn draw_card(
    app: &BoardApp,
    frame: &mut Frame,
    rect: Rect,
    card: &Card,
    selected: bool,
    hits: &mut Vec<(Rect, Hit)>,
) {
    if rect.width < 8 || rect.height < 3 {
        return;
    }
    w::push_hit(hits, rect, rect, Hit::Card(card.id.clone()));
    if selected {
        frame.render_widget(Block::default().style(styles::selected_row()), rect);
    }

    let (glyph, gcolor) = dr::card_glyph(card).unwrap_or_else(|| w::health_glyph(card, app.tick));
    let age = fmt_age(app.now.saturating_sub(card.last_activity));
    let finished = app.just_finished(&card.id);
    let is_new = app.is_new(&card.id);
    let pill = status_pill(card.health);
    let chip_w = if finished { 11 } else { 0 };
    let pill_w = pill.as_ref().map_or(0, |(t, _)| w::cells(t) + 1);
    let title_room = (rect.width as usize)
        .saturating_sub(INDENT as usize + 2 + w::cells(&age) + chip_w + pill_w)
        .max(4);
    let caret = if selected { g().prompt } else { " " };
    let mut tstyle = styles::fg().bold();
    if is_new {
        tstyle = styles::brand().bold();
    }
    let mut title = vec![
        Span::styled(format!(" {caret} "), styles::brand()),
        Span::styled(format!("{glyph} "), Style::default().fg(gcolor)),
        Span::styled(w::clip_cells(&card.display_title(), title_room), tstyle),
    ];
    if finished {
        title.push(Span::styled(" ✓ finished", styles::ok().bold()));
    }
    let used: usize = title.iter().map(|s| w::cells(&s.content)).sum();
    let mut right: Vec<Span> = Vec::new();
    if let Some((text, style)) = pill {
        right.push(Span::styled(text, style));
        right.push(Span::raw(" "));
    }
    right.push(Span::styled(age.clone(), styles::muted()));
    let right_w: usize = right.iter().map(|s| w::cells(&s.content)).sum();
    let gap = (rect.width as usize).saturating_sub(used + right_w + 1);
    title.push(Span::raw(" ".repeat(gap)));
    title.extend(right);
    frame.render_widget(
        Paragraph::new(Line::from(title)),
        Rect { height: 1, ..rect },
    );

    let inner = Rect {
        x: rect.x + INDENT,
        y: rect.y + 1,
        width: rect.width.saturating_sub(INDENT + 1),
        height: rect.height - 2,
    };
    let width = inner.width as usize;
    let compact = rect.height <= 4;
    let signal = card
        .signals
        .iter()
        .find(|s| s.level >= SignalLevel::Warn)
        .cloned();
    let mut lines: Vec<Line> = Vec::new();

    let mut l1 = w::Fit::new(width);
    if let Some((chip, color)) = dr::worker_chip(card) {
        l1.add(chip, Style::default().fg(color).bold());
    }
    if card.past {
        l1.add(format!("{} messages", card.message_count), styles::muted());
    } else {
        l1.add(model_short(&card.model), styles::fg());
    }
    if let Some(tier) = &card.tier {
        l1.add(format!(" {tier} "), styles::faint());
    }
    if card.worktree.is_some() {
        l1.add(format!(" {}", g().branch), styles::user());
    }
    if compact {
        if let Some(task) = &card.current_task {
            l1.add(format!("  {} ", g().task_active), styles::brand());
            l1.add(task.clone(), styles::fg());
        }
    } else if app.project_filter.is_none() {
        l1.add(format!("  {}", card.project()), styles::muted());
    }
    lines.push(l1.line());

    let dispatch_line = dr::coordinator_line(app, card, width);
    if !compact {
        lines.push(
            dispatch_line
                .clone()
                .unwrap_or_else(|| line_two(card, signal.as_ref(), width)),
        );
        let mut l3 = w::Fit::new(width);
        if card.streaming {
            l3.add(format!("{} ", w::spinner(app.tick)), styles::muted());
        }
        match &card.last_line {
            Some(t) => l3.add(t.clone(), w::italic(color::fg_muted())),
            None => l3.add("no output yet", w::italic(color::fg_faint())),
        }
        lines.push(l3.line());
    }
    match dispatch_line.filter(|_| compact) {
        Some(l) => lines.push(l),
        None => lines.push(gauges(card, if compact { signal } else { None }, width)),
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// The one line that says what this agent is actually doing — or what is wrong with it.
fn line_two(card: &Card, signal: Option<&super::model::Signal>, width: usize) -> Line<'static> {
    let mut fit = w::Fit::new(width);
    match signal {
        Some(s) if s.level == SignalLevel::Danger => {
            fit.add(
                format!("{} ", crate::theme::glyphs::g().fail),
                styles::err().bold(),
            );
            fit.add(s.text.clone(), styles::err());
        }
        Some(s) => {
            fit.add(
                format!("{} ", crate::theme::glyphs::g().warn),
                styles::warn().bold(),
            );
            fit.add(s.text.clone(), styles::warn());
        }
        None => match &card.current_task {
            Some(task) => {
                fit.add(
                    format!("{} ", crate::theme::glyphs::g().task_active),
                    styles::brand().bold(),
                );
                fit.add(task.clone(), styles::fg());
            }
            None => fit.add(card.health.label(), styles::muted()),
        },
    }
    fit.line()
}

/// Progress, context pressure, fan-out, queue depth and spend — the numbers that decide whether
/// a session is worth interrupting.
fn gauges(card: &Card, spill: Option<super::model::Signal>, width: usize) -> Line<'static> {
    let cost = fmt_cost(card.cost_usd);
    let mut fit = w::Fit::new(width);
    if let Some(s) = spill {
        let color = if s.level == SignalLevel::Danger {
            color::err()
        } else {
            color::warn()
        };
        fit.add(
            if s.level == SignalLevel::Danger {
                "! "
            } else {
                "▲ "
            },
            Style::default().fg(color).bold(),
        );
        fit.add(s.text, Style::default().fg(color));
        return fit.right(cost, styles::muted());
    }
    if card.tasks_total > 0 {
        fit.extend(w::gauge(
            card.tasks_done as f64 / card.tasks_total as f64,
            5,
            color::ok(),
        ));
        fit.add(
            format!(" {}/{} tasks", card.tasks_done, card.tasks_total),
            styles::muted(),
        );
    }
    if let Some(pct) = card.context_pct {
        if fit.used() > 0 {
            fit.add("  ", Style::default());
        }
        fit.add("ctx ", styles::muted());
        fit.extend(w::gauge(f64::from(pct) / 100.0, 4, w::context_color(pct)));
        fit.add(
            format!(" {pct}%"),
            Style::default().fg(w::context_color(pct)),
        );
    }
    let running = card.subagents.iter().filter(|s| !s.done).count();
    if running > 0 {
        fit.add(format!("  ⑂ {running}"), styles::user());
    }
    if card.queued > 0 {
        fit.add(format!("  ⇥ {}", card.queued), styles::warn());
    }
    fit.right(cost, styles::muted())
}
