//! Drawing plan & dispatch: the coordinator card's state line, the worker chip, the header's zoom
//! chip, and the Dispatch tab — the approval checklist while a split is proposed, the progress
//! table once it runs.
//!
//! The Dispatch tab is a list with a cursor rather than a scrolling page: every row records a
//! click target, and the view scrolls only as far as it must to keep the cursor's row on screen.

use crate::theme::{color, styles};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::dispatch::{
    self, dispatch_status as ds, group_color, item_status as is, progress, Role,
};
use super::form::after_words;
use super::model::{fmt_age, fmt_cost, model_short, stop_reason_words, Card, Health};
use super::state::{BoardApp, Button, Focus, Hit};
use super::widgets::{self as w, Bind, Fit};
use super::wire::{DispatchInfo, DispatchItemInfo};

pub const HERO_LINE: &str =
    "D  plan & dispatch — describe the work, Forge splits it into parallel sessions";

/// `◆` in the dispatch's colour for a coordinator — unless it is blocked on the user, whose
/// pulsing dot must stay the loudest thing on the card.
pub(super) fn card_glyph(card: &Card) -> Option<(String, Color)> {
    let d = card.dispatch.as_ref()?;
    (d.role == Role::Coordinator && card.health != Health::Waiting)
        .then(|| ("◆".to_string(), group_color(&d.id)))
}

/// `◆ 2/5 ` before a worker's model: which dispatch, which part of it.
pub(super) fn worker_chip(card: &Card) -> Option<(String, Color)> {
    let d = card.dispatch.as_ref()?;
    match d.role {
        Role::Worker(n) => Some((format!("◆ {n}/{} ", d.total), group_color(&d.id))),
        Role::Coordinator => None,
    }
}

/// The line under a coordinator's title: the dispatch's state instead of the generic task line.
/// `None` when the coordinator itself is blocked or stalled — that line matters more.
pub(super) fn coordinator_line(app: &BoardApp, card: &Card, width: usize) -> Option<Line<'static>> {
    let cd = card.dispatch.as_ref()?;
    if cd.role != Role::Coordinator || card.past || card.waiting || card.health == Health::Stalled {
        return None;
    }
    let d = app.card_dispatch(card)?;
    let p = progress(d);
    let mut fit = Fit::new(width);
    match d.status.as_str() {
        ds::PLANNING => {
            fit.add(format!("{} ", w::spinner(app.tick)), styles::muted());
            fit.add("splitting the work…", styles::fg());
        }
        ds::PROPOSED => {
            fit.add("! ", styles::err().bold());
            fit.add(
                format!("split ready · {} sessions to review", d.items.len()),
                styles::err(),
            );
        }
        ds::RUNNING => {
            fit.extend(w::gauge(frac(p.finished, p.total), 4, color::ok()));
            fit.add(format!(" {}", running_words(&p)), styles::fg());
        }
        ds::DONE => {
            fit.add(
                format!("all {} finished · {} ✓", p.total, p.succeeded),
                styles::ok(),
            );
            if p.failed > 0 {
                fit.add(format!(" {} ✗", p.failed), styles::err());
            }
        }
        _ => fit.add("cancelled", styles::muted()),
    }
    Some(fit.line())
}

/// A right-aligned chip keeps one cell off the pane border.
fn pad(text: String) -> String {
    if text.is_empty() {
        text
    } else {
        format!("{text} ")
    }
}

fn frac(a: usize, b: usize) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 / b as f64
    }
}

fn running_words(p: &dispatch::Progress) -> String {
    let mut s = format!("{}/{} done", p.finished, p.total);
    if p.running > 0 {
        s.push_str(&format!(" · {} running", p.running));
    }
    if p.waiting > 0 {
        s.push_str(&format!(" · {} waiting", p.waiting));
    }
    s
}

pub(super) fn tab_count(app: &BoardApp, card: &Card) -> String {
    let Some(d) = app.card_dispatch(card) else {
        return String::new();
    };
    match d.status.as_str() {
        ds::PLANNING => " …".into(),
        ds::PROPOSED => format!(" {}", d.items.len()),
        _ => {
            let p = progress(d);
            format!(" {}/{}", p.finished, p.total)
        }
    }
}

/// `◆ <title> ✕` while the board is zoomed to one dispatch.
pub(super) fn zoom_chip(app: &BoardApp) -> Option<(String, Color)> {
    let id = app.dispatch.zoom.as_ref()?;
    let title = app
        .zoomed()
        .map(dispatch::title)
        .unwrap_or_else(|| "dispatch".into());
    Some((
        format!("◆ {} ✕", w::clip_cells(&title, 28)),
        group_color(id),
    ))
}

pub(super) fn dispatch_keybar(app: &BoardApp) -> Vec<Bind> {
    let status = app
        .selected_card()
        .and_then(|c| app.card_dispatch(c))
        .map(|d| d.status.clone())
        .unwrap_or_default();
    match status.as_str() {
        ds::PROPOSED => vec![
            ("y", "start", true),
            ("Space", "tick", false),
            ("Enter", "unfold", false),
            ("e", "revise", false),
            ("n", "cancel", false),
            ("↑↓", "row", false),
            ("[ ]", "section", false),
            ("Esc", "back", false),
        ],
        ds::PLANNING => vec![
            ("n", "cancel", false),
            ("a", "attach", false),
            ("p", "prompt", false),
            ("[ ]", "section", false),
            ("Esc", "back", false),
        ],
        _ => {
            let mut v = vec![
                ("↑↓", "row", false),
                ("Enter", "open session", false),
                ("A", "merge finished", false),
            ];
            if status == ds::RUNNING {
                v.push(("n", "cancel the rest", false));
            }
            v.extend_from_slice(&[
                ("z", "zoom", false),
                ("[ ]", "section", false),
                ("Esc", "back", false),
            ]);
            v
        }
    }
}

// ───────────────────────────────────────────────────────────────── the tab

/// One drawn line of the tab: which item row it belongs to (for the cursor and clicks), and any
/// narrower click targets inside it as `(x offset, width, hit)`.
struct DLine {
    line: Line<'static>,
    row: Option<usize>,
    hits: Vec<(u16, u16, Hit)>,
}

fn plain(line: Line<'static>) -> DLine {
    DLine {
        line,
        row: None,
        hits: Vec::new(),
    }
}

fn row_line(line: Line<'static>, pos: usize) -> DLine {
    DLine {
        line,
        row: Some(pos),
        hits: Vec::new(),
    }
}

/// `(label, button, colour, enabled)`.
type Btn = (String, Button, Color, bool);

pub(super) fn draw_tab(
    app: &BoardApp,
    frame: &mut Frame,
    area: Rect,
    card: &Card,
    hits: &mut Vec<(Rect, Hit)>,
) -> Option<usize> {
    let d = app.card_dispatch(card)?;
    if area.width < 4 || area.height == 0 {
        return None;
    }
    let width = area.width as usize;
    let own = match card.dispatch.as_ref().map(|c| c.role) {
        Some(Role::Worker(n)) => Some(n),
        _ => None,
    };
    let (lines, buttons) = match d.status.as_str() {
        ds::PLANNING => (planning(app, d, width), Vec::new()),
        ds::PROPOSED => proposal(app, d, width),
        _ => progress_view(app, d, own, width),
    };
    let btn_h = button_rows(&buttons, area.width).min(area.height.saturating_sub(1));
    let content_h = area.height - btn_h;
    let gap = u16::from(btn_h > 0 && content_h >= 6);
    let content = Rect {
        height: content_h - gap,
        ..area
    };
    let h = content.height as usize;
    let cursor = Some(app.dispatch.cursor);
    let mut off = 0;
    if let (Some(first), Some(last)) = (
        lines.iter().position(|l| l.row == cursor),
        lines.iter().rposition(|l| l.row == cursor),
    ) {
        if last + 1 > h {
            off = last + 1 - h;
        }
        off = off.min(first);
    }
    for (i, dl) in lines.iter().skip(off).take(h).enumerate() {
        let r = Rect {
            y: content.y + i as u16,
            height: 1,
            ..content
        };
        frame.render_widget(Paragraph::new(dl.line.clone()), r);
        if let Some(pos) = dl.row {
            w::push_hit(hits, r, content, Hit::DispatchRow(pos));
        }
        for (x, wd, hit) in &dl.hits {
            let target = Rect {
                x: content.x + x,
                width: *wd,
                ..r
            };
            w::push_hit(hits, target, content, hit.clone());
        }
    }
    if btn_h > 0 {
        draw_buttons(
            frame,
            Rect {
                y: area.y + area.height - btn_h,
                height: btn_h,
                ..area
            },
            &buttons,
            hits,
        );
    }
    None
}

fn cursor_marker(app: &BoardApp, pos: usize) -> (&'static str, Style) {
    if app.dispatch.cursor != pos {
        return ("  ", Style::default());
    }
    let color = if app.focus == Focus::Detail {
        color::user()
    } else {
        color::fg_muted()
    };
    ("▸ ", Style::default().fg(color).bold())
}

fn text_line(prefix: &str, text: &str, style: Style, width: usize) -> Line<'static> {
    let mut fit = Fit::new(width);
    fit.add(prefix.to_string(), Style::default());
    fit.add(text.to_string(), style);
    fit.line()
}

fn planning(app: &BoardApp, d: &DispatchInfo, width: usize) -> Vec<DLine> {
    let mut out = Vec::new();
    let mut fit = Fit::new(width);
    fit.add(format!("  {} ", w::spinner(app.tick)), styles::muted());
    fit.add(
        "Reading the project and splitting the work…",
        styles::fg().bold(),
    );
    out.push(plain(fit.line()));
    out.push(plain(Line::from("")));
    out.push(plain(w::heading("request", width)));
    for l in w::wrap(&d.prompt, width.saturating_sub(4))
        .into_iter()
        .take(8)
    {
        out.push(plain(text_line("    ", &l, styles::fg(), width)));
    }
    out.push(plain(Line::from("")));
    out.push(plain(w::heading("coordinator", width)));
    let last = app
        .card(&d.coordinator_session_id)
        .and_then(|c| c.last_line.clone());
    let (text, color) = match last {
        Some(l) => (l, color::fg_muted()),
        None => ("no output yet".to_string(), color::fg_faint()),
    };
    out.push(plain(w::bullet("·", &text, color, width)));
    out
}

fn proposal(app: &BoardApp, d: &DispatchInfo, width: usize) -> (Vec<DLine>, Vec<Btn>) {
    let selection = app.dispatch_selection(&d.id);
    let on = |index: usize| selection.is_none_or(|s| s.contains(&index));
    let mut out = Vec::new();
    let total = d.items.len();
    out.push(plain(w::heading(
        &format!(
            "proposed split · {total} session{}",
            if total == 1 { "" } else { "s" }
        ),
        width,
    )));
    for l in w::wrap(&d.summary, width.saturating_sub(4))
        .into_iter()
        .take(6)
    {
        if !l.is_empty() {
            out.push(plain(text_line("    ", &l, styles::fg(), width)));
        }
    }
    out.push(plain(Line::from("")));
    for (pos, item) in d.items.iter().enumerate() {
        let ticked = on(item.index);
        let (marker, mstyle) = cursor_marker(app, pos);
        let chip = if item.depends_on.is_empty() {
            String::new()
        } else {
            after_words(&item.depends_on)
        };
        let mut fit = Fit::new(width);
        fit.add(marker, mstyle);
        fit.add(
            if ticked { "[✓]" } else { "[ ]" },
            Style::default()
                .fg(if ticked {
                    color::ok()
                } else {
                    color::fg_muted()
                })
                .bold(),
        );
        fit.add(format!(" {:>2}  ", item.index), styles::muted());
        let mut tstyle = if ticked {
            styles::fg().bold()
        } else {
            styles::muted()
        };
        if app.dispatch.cursor == pos {
            tstyle = tstyle.patch(styles::selected_row());
        }
        let room = fit.left().saturating_sub(w::cells(&chip) + 3);
        fit.add(w::clip_cells(&item.title, room), tstyle);
        let mut dl = row_line(fit.right(pad(chip), styles::user()), pos);
        dl.hits.push((2, 3, Hit::DispatchToggle(pos)));
        out.push(dl);
        if app.dispatch.expanded == Some(item.index) {
            for l in w::wrap(&item.prompt, width.saturating_sub(12)) {
                out.push(row_line(
                    text_line("          ", &l, styles::fg(), width),
                    pos,
                ));
            }
        } else {
            let preview = dispatch::first_line(&item.prompt);
            out.push(row_line(
                text_line(
                    "          ",
                    &preview,
                    w::italic(if ticked {
                        color::fg_muted()
                    } else {
                        color::fg_faint()
                    }),
                    width,
                ),
                pos,
            ));
        }
    }
    out.push(plain(Line::from("")));
    let n = d.items.iter().filter(|i| on(i.index)).count();
    let mut fit = Fit::new(width);
    fit.add(
        format!("  {n} of {total} selected"),
        Style::default().fg(if n == 0 { color::warn() } else { color::fg() }),
    );
    fit.add(
        format!(
            " · {} at once · worktrees {}",
            d.max_running,
            if d.worktree { "on" } else { "off" }
        ),
        styles::muted(),
    );
    out.push(plain(fit.line()));
    let buttons = vec![
        (
            format!("[ y  start {n} session{} ]", if n == 1 { "" } else { "s" }),
            Button::Approve,
            color::ok(),
            n > 0,
        ),
        (
            "[ e  revise ]".to_string(),
            Button::Revise,
            color::user(),
            true,
        ),
        (
            "[ n  cancel ]".to_string(),
            Button::CancelDispatch,
            color::err(),
            true,
        ),
    ];
    (out, buttons)
}

fn progress_view(
    app: &BoardApp,
    d: &DispatchInfo,
    own: Option<usize>,
    width: usize,
) -> (Vec<DLine>, Vec<Btn>) {
    let p = progress(d);
    let mut out = Vec::new();
    let (words, color) = match d.status.as_str() {
        ds::DONE => {
            let mut s = format!("all {} finished · {} ✓", p.total, p.succeeded);
            if p.failed > 0 {
                s.push_str(&format!(" {} ✗", p.failed));
            }
            (
                s,
                if p.failed > 0 {
                    color::warn()
                } else {
                    color::ok()
                },
            )
        }
        ds::CANCELLED => {
            let mut s = format!("cancelled · {}/{} finished", p.finished, p.total);
            if p.running > 0 {
                s.push_str(&format!(" · {} still running", p.running));
            }
            (s, color::fg_muted())
        }
        _ => (running_words(&p), color::ok()),
    };
    let mut fit = Fit::new(width);
    fit.add("  ", Style::default());
    fit.extend(w::gauge(
        frac(p.finished, p.total),
        (width / 4).clamp(6, 20),
        color,
    ));
    fit.add(format!("  {words}"), styles::fg());
    out.push(plain(fit.line()));
    if !d.worktree {
        out.push(plain(text_line(
            "  ",
            "shared directory — sessions edit the project in place",
            styles::faint(),
            width,
        )));
    }
    for l in w::wrap(&d.summary, width.saturating_sub(4))
        .into_iter()
        .take(3)
    {
        if !l.is_empty() {
            out.push(plain(text_line("  ", &l, styles::muted(), width)));
        }
    }
    out.push(plain(Line::from("")));
    for (pos, item) in d.items.iter().enumerate() {
        let (glyph, gcolor) = item_glyph(&item.status, app.tick);
        let (marker, mstyle) = cursor_marker(app, pos);
        let mine = own == Some(item.index);
        let label = format!(
            "{}{}",
            status_label(&item.status),
            if mine { " · this session" } else { "" }
        );
        let mut fit = Fit::new(width);
        fit.add(marker, mstyle);
        fit.add(glyph, Style::default().fg(gcolor).bold());
        fit.add(format!(" {:>2}  ", item.index), styles::muted());
        let mut tstyle = Style::default()
            .fg(if mine {
                group_color(&d.id)
            } else {
                color::fg()
            })
            .bold();
        if app.dispatch.cursor == pos {
            tstyle = tstyle.patch(styles::selected_row());
        }
        let room = fit.left().saturating_sub(w::cells(&label) + 3);
        fit.add(w::clip_cells(&item.title, room), tstyle);
        out.push(row_line(
            fit.right(pad(label), Style::default().fg(gcolor)),
            pos,
        ));
        let (detail, dcolor) = item_detail(app, d, item);
        out.push(row_line(
            text_line("        ", &detail, Style::default().fg(dcolor), width),
            pos,
        ));
    }
    let mergeable = d
        .items
        .iter()
        .any(|i| i.status == is::SUCCEEDED && i.session_id.is_some());
    let mut buttons = vec![(
        "[ A  merge all finished ]".to_string(),
        Button::MergeAll,
        color::user(),
        mergeable && d.worktree,
    )];
    if d.status == ds::RUNNING {
        buttons.push((
            "[ n  cancel remaining ]".to_string(),
            Button::CancelDispatch,
            color::err(),
            true,
        ));
    }
    (out, buttons)
}

fn item_glyph(status: &str, tick: u64) -> (&'static str, Color) {
    match status {
        is::RUNNING => (w::spinner(tick), color::fg_muted()),
        is::SUCCEEDED => ("✓", color::ok()),
        is::FAILED => ("✗", color::err()),
        is::STOPPED => ("■", color::warn()),
        is::CANCELLED => ("–", color::fg_muted()),
        is::SKIPPED => ("·", color::fg_muted()),
        is::MERGED => ("⇡", color::user()),
        is::DISCARDED => ("⌫", color::fg_muted()),
        _ => ("○", color::fg_muted()),
    }
}

fn status_label(status: &str) -> &'static str {
    match status {
        is::QUEUED | is::PROPOSED => "queued",
        is::RUNNING => "running",
        is::SUCCEEDED => "finished",
        is::FAILED => "failed",
        is::STOPPED => "stopped",
        is::CANCELLED => "cancelled",
        is::SKIPPED => "not selected",
        is::MERGED => "merged",
        is::DISCARDED => "discarded",
        _ => "",
    }
}

/// The second line of a row: what its live worker is doing, or why it has none.
fn item_detail(app: &BoardApp, d: &DispatchInfo, item: &DispatchItemInfo) -> (String, Color) {
    let card = item
        .session_id
        .as_deref()
        .and_then(|s| app.card(s))
        .filter(|c| !c.past);
    if let Some(c) = card {
        let doing = c
            .current_task
            .as_ref()
            .map(|t| format!("▸ {t}"))
            .or_else(|| c.last_line.clone())
            .unwrap_or_else(|| c.health.label().to_string());
        let age = fmt_age(app.now.saturating_sub(c.last_activity));
        let text = format!(
            "{} · {doing} · {} · {age}",
            model_short(&c.model),
            fmt_cost(c.cost_usd)
        );
        return (text, color::fg_muted());
    }
    let text = match item.status.as_str() {
        is::QUEUED | is::PROPOSED => {
            let waits = dispatch::waits_for(d, item);
            if waits.is_empty() {
                "waiting for a slot".to_string()
            } else {
                format!("waits for {}", dispatch::list_words(&waits))
            }
        }
        is::SKIPPED => "not selected".into(),
        is::CANCELLED => "not started".into(),
        is::MERGED => "merged back".into(),
        is::DISCARDED => "discarded".into(),
        is::RUNNING => "starting…".into(),
        _ => item
            .outcome
            .as_deref()
            .map(|o| stop_reason_words(Some(o)))
            .unwrap_or_else(|| "no session".into()),
    };
    (text, color::fg_faint())
}

fn button_rows(buttons: &[Btn], width: u16) -> u16 {
    if buttons.is_empty() {
        return 0;
    }
    let mut rows = 1;
    let mut used: u16 = 0;
    for (label, ..) in buttons {
        let bw = w::cells(label) as u16 + 2;
        if used > 0 && used + bw + 2 > width {
            rows += 1;
            used = 0;
        }
        used += bw;
    }
    rows
}

fn draw_buttons(frame: &mut Frame, area: Rect, buttons: &[Btn], hits: &mut Vec<(Rect, Hit)>) {
    let mut y = area.y;
    let x = area.x + 2;
    let mut row = w::Row::at(x, y);
    for (label, button, color, enabled) in buttons {
        let bw = w::cells(label) as u16 + 2;
        if row.width() > 0 && row.width() + bw + 2 > area.width {
            row.render(frame, area);
            y += 1;
            if y >= area.y + area.height {
                return;
            }
            row = w::Row::at(x, y);
        }
        let style = if *enabled {
            Style::default()
                .fg(*color)
                .patch(styles::selected_row())
                .bold()
        } else {
            styles::faint()
        };
        let r = row.add(label.clone(), style);
        row.add("  ", Style::default());
        w::push_hit(hits, r, area, Hit::Button(*button));
    }
    row.render(frame, area);
}
