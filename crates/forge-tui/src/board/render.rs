//! The board itself: the two-row header, the four columns of session cards, the footer toasts,
//! the key hints in the header row, and the hand-off to the detail pane and the overlays.
//!
//! One rule governs everything here: a person glancing at this screen for one second must be able
//! to tell which agent is doing what, which one needs them, and whether things are going well.
//! Colour is therefore never decorative — it is the column's tone, the card's health, or an
//! alarm — and every clickable thing registers a [`Hit`] in the same pass that draws it.

use crate::theme::{color, styles};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::surface::{self};

use super::model::{fmt_cost, Column};
use super::state::{BoardApp, ConnState, Focus, Hit};
use super::widgets as w;
use super::{detail, dispatch_render as dr, form_render, overlays};

/// Narrower than this and a column stops being readable, so fewer are shown.
const MIN_COLUMN: u16 = 30;
/// Below this body height a card drops its last-line and gauge rows.
const COMPACT_BELOW: u16 = 22;

pub fn draw(app: &mut BoardApp, frame: &mut Frame) {
    app.hits.clear();
    let area = frame.area();
    surface::render_backdrop(frame, area);
    app.size = (area.width, area.height);
    if area.width < 4 || area.height < 4 {
        return;
    }

    let header_h = if area.height >= 6 { 2 } else { 1 };
    let footer_h = u16::from(area.height >= 5);
    let body_h = area.height.saturating_sub(header_h + footer_h);
    let header = Rect {
        height: header_h,
        ..area
    };
    let body = Rect {
        y: area.y + header_h,
        height: body_h,
        ..area
    };
    let footer = Rect {
        y: area.y + header_h + body_h,
        height: footer_h,
        ..area
    };

    let mut hits: Vec<(Rect, Hit)> = Vec::new();
    draw_header(app, frame, header, &mut hits);
    let (board_area, pane_area) = split_body(app, body);
    if let Some(b) = board_area {
        draw_board(app, frame, b, &mut hits);
    }
    app.hits = hits;

    if let Some(p) = pane_area {
        detail::draw_detail(app, frame, p);
    }
    if footer_h > 0 {
        draw_footer(app, frame, footer);
    }

    // Overlays own the screen while they are up, so they are painted over everything else.
    if app.confirm.is_some() {
        overlays::draw_confirm(app, frame, area);
    } else if app.composer.is_some() {
        overlays::draw_composer(app, frame, area);
    } else if app.dispatch.form.is_some() {
        form_render::draw_form(app, frame, area);
    }
    if app.focus == Focus::Help {
        overlays::draw_help(frame, area);
    }
}

/// How the body divides between the columns and the pane. A pane needs real estate to be worth
/// anything, so on a narrow terminal it simply takes the whole body.
fn split_body(app: &BoardApp, body: Rect) -> (Option<Rect>, Option<Rect>) {
    if !app.detail_open || app.selected_card().is_none() || body.height == 0 {
        return (Some(body), None);
    }
    if body.width < 110 {
        return (None, Some(body));
    }
    let pct = if body.width >= 140 { 50 } else { 42 };
    let board_w = (u32::from(body.width) * pct / 100) as u16;
    (
        Some(Rect {
            width: board_w,
            ..body
        }),
        Some(Rect {
            x: body.x + board_w,
            width: body.width - board_w,
            ..body
        }),
    )
}

// ───────────────────────────────────────────────────────────────────── header

fn draw_header(app: &BoardApp, frame: &mut Frame, area: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let t = app.totals();
    let x0 = area.x + 1;
    let mut row = w::Row::at(x0, area.y);
    row.add(
        format!("{} board", crate::theme::glyphs::g().brand),
        styles::brand().bold(),
    );
    row.sep();
    let label = match app.project_filter.as_deref() {
        Some(p) => format!("▾ {}", w::clip_cells(p, 24)),
        None => "▾ all projects".to_string(),
    };
    let r = row.add(label, styles::user());
    w::push_hit(hits, r, area, Hit::ProjectFilter);
    row.sep();
    match dr::zoom_chip(app) {
        Some((text, color)) => {
            let r = row.add(
                text,
                Style::default()
                    .fg(color)
                    .patch(styles::selected_row())
                    .bold(),
            );
            w::push_hit(hits, r, area, Hit::ClearZoom);
        }
        None => {
            let r = row.add("◆ dispatch", styles::user());
            w::push_hit(hits, r, area, Hit::DispatchChip);
        }
    }

    let dot = if t.attention == 0 || w::pulse(app.tick) {
        "●"
    } else {
        "○"
    };
    let attn = if t.attention > 0 {
        styles::err().bold()
    } else {
        styles::faint()
    };
    let spin = if t.working > 0 {
        w::spinner(app.tick)
    } else {
        "○"
    };
    let mut chips: Vec<(String, Style)> = vec![
        (format!("{dot} {} needs you", t.attention), attn),
        (
            format!("{spin} {} working", t.working),
            Style::default().fg(if t.working > 0 {
                color::fg_muted()
            } else {
                color::fg_faint()
            }),
        ),
        (
            format!("{} ready", t.ready),
            Style::default().fg(if t.ready > 0 {
                color::ok()
            } else {
                color::fg_faint()
            }),
        ),
        (format!("{} done", t.done), styles::muted()),
    ];
    if t.subagents > 0 {
        chips.push((format!("⑂ {} subagents", t.subagents), styles::user()));
    }
    if t.cost_usd > 0.0 {
        chips.push((fmt_cost(t.cost_usd), styles::fg()));
    }

    let (conn_text, conn_style) = connection_chip(app);
    let items = vec![
        (conn_text, conn_style),
        ("  ".to_string(), Style::default()),
        ("? help".to_string(), styles::muted()),
    ];
    let total: u16 = items.iter().map(|(t, _)| w::cells(t) as u16).sum();
    // The chips never run under the connection indicator: whole chips are dropped from the
    // right until the row fits, so a narrow terminal loses "12 done" before it loses the brand.
    let avail = area.width.saturating_sub(total + 3);
    for (text, style) in chips {
        let need = row.width() + 3 + w::cells(&text) as u16;
        if need > avail {
            break;
        }
        row.sep();
        row.add(text, style);
    }
    let left_end = area.x + row.width() + 1;
    let left_area = Rect {
        width: avail,
        ..area
    };
    row.render(frame, left_area);
    let right_start = area.x + area.width.saturating_sub(1 + total);
    super::hints::draw_top_hints(frame, area, &w::keybar(app), left_end, right_start);
    let mut right = w::Row::at(right_start, area.y);
    let mut help_rect = Rect::default();
    for (text, style) in items {
        let is_help = text.starts_with('?');
        let r = right.add(text, style);
        if is_help {
            help_rect = r;
        }
    }
    w::push_hit(hits, help_rect, area, Hit::Help);
    right.render(frame, area);

    if area.height < 2 {
        return;
    }
    let second = Rect {
        y: area.y + 1,
        height: 1,
        ..area
    };
    if app.focus == Focus::Filter || !app.query.is_empty() {
        let mut row = w::Row::at(x0, second.y);
        row.add("/ filter: ", styles::user().bold());
        row.add(
            w::clip_cells(&app.query, second.width.saturating_sub(28) as usize),
            styles::fg(),
        );
        if app.focus == Focus::Filter {
            row.add(" ", styles::selected_row());
        }
        row.add("   Esc clears", styles::faint());
        row.render(frame, second);
    } else {
        w::rule(frame, second);
    }
}

fn connection_chip(app: &BoardApp) -> (String, Style) {
    match &app.connection {
        ConnState::Live => ("● live".into(), styles::ok()),
        ConnState::Connecting | ConnState::Reconnecting => {
            let word = if matches!(app.connection, ConnState::Connecting) {
                "connecting"
            } else {
                "reconnecting"
            };
            let style = if w::pulse(app.tick) {
                styles::warn()
            } else {
                styles::faint()
            };
            (format!("◌ {word}"), style)
        }
        ConnState::Offline(reason) => (
            format!("● offline: {}", w::clip_cells(reason, 28)),
            styles::err().bold(),
        ),
    }
}

// ───────────────────────────────────────────────────────────────────── columns

fn draw_board(app: &mut BoardApp, frame: &mut Frame, area: Rect, hits: &mut Vec<(Rect, Hit)>) {
    if area.width < 8 || area.height < 2 {
        return;
    }
    if app.cards.is_empty() {
        draw_hero(app, frame, area);
        return;
    }
    let fit = ((area.width / MIN_COLUMN) as usize).clamp(1, 4);
    let mut top = area.y;
    let mut height = area.height;
    if fit < 4 {
        draw_tab_strip(app, frame, Rect { height: 1, ..area }, hits);
        top += 1;
        height = height.saturating_sub(1);
    }
    if height == 0 {
        return;
    }
    let page = clamp_page(app, fit);
    let compact = area.height < COMPACT_BELOW;
    let card_h: u16 = if compact { 4 } else { 6 };

    let base = area.width / fit as u16;
    let extra = area.width % fit as u16;
    let mut x = area.x;
    for slot in 0..fit {
        let width = base + u16::from((slot as u16) < extra);
        let col = Column::from_index(page + slot);
        let rect = Rect {
            x,
            y: top,
            width,
            height,
        };
        draw_column(app, frame, rect, col, card_h, hits);
        x += width;
    }
}

/// Keep the cursor's column on screen when not all four fit, and remember where the user was.
fn clamp_page(app: &mut BoardApp, fit: usize) -> usize {
    let max_page = 4 - fit;
    let cursor = app.cursor_col.index();
    let mut page = app.column_page.min(max_page);
    if cursor < page {
        page = cursor;
    } else if cursor >= page + fit {
        page = cursor + 1 - fit;
    }
    app.column_page = page;
    page
}

fn draw_tab_strip(app: &BoardApp, frame: &mut Frame, area: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let mut row = w::Row::at(area.x + 1, area.y);
    for (i, col) in Column::ALL.iter().enumerate() {
        if i > 0 {
            row.sep();
        }
        let n = app.columns[col.index()].len();
        let current = *col == app.cursor_col;
        let mut style = column_style(*col);
        if current {
            style = style.patch(styles::selected_row()).bold();
        }
        let r = row.add(format!(" {} {n} ", col.label().to_uppercase()), style);
        w::push_hit(hits, r, area, Hit::ColumnHeader(*col));
    }
    row.render(frame, area);
}

fn draw_column(
    app: &mut BoardApp,
    frame: &mut Frame,
    area: Rect,
    col: Column,
    card_h: u16,
    hits: &mut Vec<(Rect, Hit)>,
) {
    let inner_w = area.width.saturating_sub(1);
    if inner_w < 6 || area.height < 2 {
        return;
    }
    let tone = column_style(col);
    let count = app.columns[col.index()].len();
    let mut head = w::Row::at(area.x, area.y);
    head.add(col.label().to_uppercase(), tone.bold());
    head.add(format!(" {count}"), styles::muted());
    head.render(frame, area);
    w::rule(
        frame,
        Rect {
            y: area.y + 1,
            width: inner_w,
            height: 1,
            ..area
        },
    );
    w::push_hit(
        hits,
        Rect {
            height: 2,
            width: inner_w,
            ..area
        },
        area,
        Hit::ColumnHeader(col),
    );

    let list = Rect {
        y: area.y + 2,
        height: area.height.saturating_sub(2),
        width: inner_w,
        ..area
    };
    if list.height == 0 {
        return;
    }
    if count == 0 {
        let hint = match col {
            Column::Attention => "nothing needs you",
            Column::Working => "no session is working",
            Column::Ready => "nothing idle",
            Column::Done => "no past sessions",
        };
        let y = list.y + list.height / 3;
        frame.render_widget(
            Paragraph::new(
                Line::from(Span::styled(
                    w::clip_cells(hint, list.width as usize),
                    styles::faint(),
                ))
                .centered(),
            ),
            Rect {
                y,
                height: 1,
                ..list
            },
        );
        return;
    }

    let avail = list.height as usize;
    let mut cap = (avail / card_h as usize).max(1);
    let (s, e) = app.visible_window(col, cap);
    let reserve = usize::from(s > 0) + usize::from(e < count);
    if reserve > 0 {
        cap = (avail.saturating_sub(reserve) / card_h as usize).max(1);
    }
    let (start, end) = app.visible_window(col, cap);

    let mut y = list.y;
    if start > 0 && y < list.y + list.height {
        marker(frame, list, y, format!("↑ {start} more"));
        y += 1;
    }
    let indices: Vec<usize> = app.columns[col.index()][start..end].to_vec();
    let selected_id = app.selected.clone().unwrap_or_default();
    for idx in indices {
        if y + card_h > list.y + list.height {
            break;
        }
        let card = &app.cards[idx];
        let rect = Rect {
            x: list.x,
            y,
            width: list.width,
            height: card_h,
        };
        super::card_render::draw_card(app, frame, rect, card, card.id == selected_id, hits);
        y += card_h;
    }
    if end < count && y < list.y + list.height {
        marker(frame, list, y, format!("↓ {} more", count - end));
    }
}

fn marker(frame: &mut Frame, list: Rect, y: u16, text: String) {
    frame.render_widget(
        Paragraph::new(
            Line::from(Span::styled(
                w::clip_cells(&text, list.width as usize),
                styles::muted(),
            ))
            .centered(),
        ),
        Rect {
            y,
            height: 1,
            ..list
        },
    );
}

// ──────────────────────────────────────────────────────────────── hero/footer

/// Nothing to show is itself a state worth designing: say so, and say how to start.
fn draw_hero(app: &BoardApp, frame: &mut Frame, area: Rect) {
    let (conn, style) = connection_chip(app);
    let filtered = app.project_filter.is_some() || !app.query.trim().is_empty();
    let head = if filtered {
        "Nothing matches this filter"
    } else {
        "No sessions yet"
    };
    let sub = if filtered {
        "Esc clears it   ·   f cycles the project".to_string()
    } else {
        "N  start one here   ·   or run: forge serve --local".to_string()
    };
    let mut lines = vec![
        Line::from(Span::styled(head, styles::fg().bold())).centered(),
        Line::from(""),
        Line::from(Span::styled(sub, styles::muted())).centered(),
    ];
    if !filtered {
        lines.push(Line::from(Span::styled(dr::HERO_LINE, styles::muted())).centered());
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(conn, style)).centered());
    let h = lines.len() as u16;
    if area.height <= h {
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    frame.render_widget(
        Paragraph::new(lines),
        Rect {
            y: area.y + (area.height - h) / 2,
            height: h,
            ..area
        },
    );
}

fn draw_footer(app: &BoardApp, frame: &mut Frame, area: Rect) {
    frame.render_widget(Block::default().style(Style::default()), area);
    let toasts = w::toast_items(app);
    let toast_w: u16 = toasts.iter().map(|(t, _)| w::cells(t) as u16).sum();
    if toast_w > 0 && toast_w + 4 < area.width {
        w::right_row(toasts, area.x + area.width.saturating_sub(1), area.y).render(frame, area);
    }
}

/// Lane colours are the categorical tokens, so the board never borrows a status colour for a
/// column title; the label and count carry the meaning in mono.
fn column_style(col: Column) -> Style {
    styles::category(col.index())
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use crate::board::state::{ConnState, DetailTab, Toast, ToastLevel};
    use crate::board::wire::{FleetRow, LiveSnapshot, PastRow, QOption, Subagent, Task};
    use crate::board::BoardEvent;

    pub(super) const NOW: i64 = 1_700_000_000;

    pub(super) fn screen(app: &mut BoardApp, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal.draw(|f| app.draw(f)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn row(id: &str, title: &str) -> FleetRow {
        FleetRow {
            id: id.into(),
            title: title.into(),
            cwd: "/home/dev/forge".into(),
            model: "anthropic::claude-opus".into(),
            last_activity: NOW - 120,
            created_at: NOW - 3600,
            cost_usd: 1.25,
            context_tokens: 42_000,
            context_limit: Some(100_000),
            ..Default::default()
        }
    }

    pub(super) fn busy_app() -> BoardApp {
        let mut app = BoardApp::new(Some("/home/dev/forge".into()), NOW);
        let mut working = row("aaaa1111", "port the mesh router");
        working.busy = true;
        let mut idle = row("bbbb2222", "docs pass");
        idle.last_activity = NOW - 30;
        let mut needs = row("cccc3333", "risky migration");
        needs.waiting = true;
        app.apply(BoardEvent::Fleet(vec![working, idle, needs]));
        app.apply(BoardEvent::Snapshot(
            "aaaa1111".into(),
            LiveSnapshot {
                busy: true,
                model: "anthropic::claude-opus".into(),
                tier: Some("complex".into()),
                streaming: "rewriting the ranking pass".into(),
                tasks: vec![
                    Task {
                        title: "read the router".into(),
                        status: "done".into(),
                        ..Default::default()
                    },
                    Task {
                        title: "rank the catalog".into(),
                        status: "in_progress".into(),
                        ..Default::default()
                    },
                ],
                subagents: vec![Subagent {
                    agent: "builder".into(),
                    task: "port tests".into(),
                    ..Default::default()
                }],
                context_tokens: 42_000,
                context_limit: Some(100_000),
                ..Default::default()
            },
        ));
        app.apply(BoardEvent::Snapshot(
            "cccc3333".into(),
            LiveSnapshot {
                permission_prompt: Some("run: rm -rf target/debug".into()),
                prompt_seq: 7,
                model: "openai::gpt-5".into(),
                ..Default::default()
            },
        ));
        app.apply(BoardEvent::Past(vec![PastRow {
            id: "dddd4444".into(),
            title: "old release run".into(),
            cwd: "/home/dev/forge".into(),
            last_activity: NOW - 86_400,
            preview: Some("cut v2.14.1".into()),
            ..Default::default()
        }]));
        app.apply(BoardEvent::Connection(ConnState::Live));
        app
    }

    #[test]
    fn every_column_header_is_on_screen() {
        let mut app = busy_app();
        let s = screen(&mut app, 160, 40);
        assert!(s.contains("NEEDS YOU"), "{s}");
        assert!(s.contains("WORKING"), "{s}");
        assert!(s.contains("READY"), "{s}");
        assert!(s.contains("DONE"), "{s}");
    }

    #[test]
    fn a_card_shows_its_title_model_and_current_task() {
        let mut app = busy_app();
        let s = screen(&mut app, 160, 40);
        assert!(s.contains("port the mesh router"), "{s}");
        assert!(s.contains("claude-opus"), "{s}");
        assert!(s.contains("rank the catalog"), "{s}");
    }

    #[test]
    fn the_strongest_signal_replaces_the_task_line() {
        let mut app = busy_app();
        let s = screen(&mut app, 160, 40);
        assert!(s.contains("waiting on a permission"), "{s}");
    }

    #[test]
    fn cards_carry_an_age_and_a_cost() {
        let mut app = busy_app();
        let s = screen(&mut app, 160, 40);
        assert!(s.contains("2m"), "{s}");
        assert!(s.contains("$1.25"), "{s}");
    }

    #[test]
    fn the_header_counts_and_the_keybar_are_drawn() {
        let mut app = busy_app();
        let s = screen(&mut app, 160, 40);
        assert!(s.contains("✦ board"), "{s}");
        assert!(s.contains("1 needs you"), "{s}");
        assert!(s.contains("1 working"), "{s}");
        assert!(s.contains("all projects"), "{s}");
        assert!(s.contains("live"), "{s}");
        assert!(
            s.lines().next().unwrap_or_default().contains("↑↓ move"),
            "{s}"
        );
        let wide = screen(&mut app, 320, 40);
        let top = wide.lines().next().unwrap_or_default();
        assert!(top.contains("attach"), "{top}");
        assert!(top.contains("quit"), "{top}");
    }

    #[test]
    fn a_narrow_header_drops_hints_before_the_title() {
        let mut app = busy_app();
        let s = screen(&mut app, 60, 20);
        let top = s.lines().next().unwrap_or_default();
        assert!(top.contains("✦ board"), "{top}");
        assert!(top.contains("? help"), "{top}");
        assert!(!top.contains("quit"), "{top}");
    }

    #[test]
    fn a_waiting_selection_puts_allow_and_deny_first_in_the_keybar() {
        let mut app = busy_app();
        app.select("cccc3333");
        let s = screen(&mut app, 160, 40);
        let bar = s.lines().next().unwrap_or_default();
        assert!(bar.contains("allow"), "{bar}");
        assert!(bar.contains("deny"), "{bar}");
    }

    #[test]
    fn every_visible_card_and_column_registers_a_hit() {
        let mut app = busy_app();
        screen(&mut app, 160, 40);
        for id in ["aaaa1111", "bbbb2222", "cccc3333", "dddd4444"] {
            assert!(
                app.hits.iter().any(|(_, h)| *h == Hit::Card(id.into())),
                "no hit for {id}"
            );
        }
        for col in Column::ALL {
            assert!(app.hits.iter().any(|(_, h)| *h == Hit::ColumnHeader(col)));
        }
        assert!(app.hits.iter().any(|(_, h)| *h == Hit::ProjectFilter));
        assert!(app.hits.iter().any(|(_, h)| *h == Hit::Help));
    }

    #[test]
    fn a_narrow_board_falls_back_to_a_tab_strip() {
        let mut app = busy_app();
        let s = screen(&mut app, 60, 20);
        assert!(s.contains("NEEDS YOU"), "{s}");
        assert!(s.contains("WORKING"), "{s}");
        assert!(app
            .hits
            .iter()
            .any(|(_, h)| *h == Hit::ColumnHeader(Column::Done)));
    }

    #[test]
    fn a_tiny_terminal_does_not_panic() {
        let mut app = busy_app();
        screen(&mut app, 20, 5);
        screen(&mut app, 8, 4);
        screen(&mut app, 4, 4);
    }

    #[test]
    fn an_empty_board_explains_how_to_start_one() {
        let mut app = BoardApp::new(None, NOW);
        app.apply(BoardEvent::Connection(ConnState::Offline(
            "connection refused".into(),
        )));
        let s = screen(&mut app, 100, 24);
        assert!(s.contains("No sessions yet"), "{s}");
        assert!(s.contains("forge serve --local"), "{s}");
        assert!(s.contains("connection refused"), "{s}");
    }

    #[test]
    fn the_filter_row_replaces_the_header_rule() {
        let mut app = busy_app();
        app.query = "mesh".into();
        app.rebuild();
        let s = screen(&mut app, 120, 24);
        assert!(s.contains("filter: mesh"), "{s}");
        assert!(s.contains("Esc clears"), "{s}");
    }

    #[test]
    fn empty_columns_say_what_is_missing() {
        let mut app = BoardApp::new(None, NOW);
        let mut idle = row("bbbb2222", "docs pass");
        idle.last_activity = NOW - 30;
        app.apply(BoardEvent::Fleet(vec![idle]));
        let s = screen(&mut app, 160, 40);
        assert!(s.contains("nothing needs you"), "{s}");
        assert!(s.contains("no session is working"), "{s}");
        assert!(s.contains("no past sessions"), "{s}");
    }

    #[test]
    fn a_toast_lands_on_the_right_of_the_footer() {
        let mut app = busy_app();
        app.toasts.push_back(Toast {
            level: ToastLevel::Ok,
            text: "prompt sent".into(),
            born: 0,
        });
        let s = screen(&mut app, 160, 40);
        assert!(s.lines().last().unwrap_or_default().contains("prompt sent"));
    }

    #[test]
    fn a_question_card_offers_its_options_in_the_keybar() {
        let mut app = busy_app();
        app.apply(BoardEvent::Snapshot(
            "cccc3333".into(),
            LiveSnapshot {
                question: Some("which base branch?".into()),
                question_options: vec![QOption {
                    label: "main".into(),
                    description: "the default".into(),
                }],
                ..Default::default()
            },
        ));
        app.select("cccc3333");
        let s = screen(&mut app, 160, 40);
        assert!(s.contains("pick"), "{s}");
        assert!(s.contains("waiting on a question"), "{s}");
    }

    #[test]
    fn the_pane_takes_the_whole_body_on_a_narrow_terminal() {
        let mut app = busy_app();
        app.select("aaaa1111");
        app.detail_open = true;
        app.detail_tab = DetailTab::Overview;
        let s = screen(&mut app, 100, 30);
        assert!(!s.contains("NEEDS YOU · "), "board should be hidden: {s}");
        assert!(s.contains("Overview"), "{s}");
    }
}
