//! Drawing IDE mode: regions that follow the terminal width, and the hit rectangles mouse input
//! resolves. Everything writes straight into the frame buffer and reads only what is visible, so
//! a frame costs the same for a ten-line diff and a five-thousand-line one.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::state::{Hit, IdeApp, Pane, RailFocus, RailTab};
use crate::theme::glyphs::g;
use crate::theme::styles;

mod agent;
mod bottom;
mod edit;
mod editor;
mod files;
mod help;
mod hints;
mod rail;
mod why;

/// Rail, editor and agent panel side by side from this width.
pub const WIDE: u16 = 160;
/// Rail and editor from this width, the agent panel as a drawer; narrower is one pane at a time.
pub const MID: u16 = 110;
pub const RAIL_W: u16 = 30;
pub const AGENT_W: u16 = 44;
const BOTTOM_H: u16 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Wide,
    Mid,
    Narrow,
}

pub fn mode_for(width: u16) -> Mode {
    if width >= WIDE {
        Mode::Wide
    } else if width >= MID {
        Mode::Mid
    } else {
        Mode::Narrow
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Regions {
    pub header: Rect,
    pub tabbar: Option<Rect>,
    pub rail: Option<Rect>,
    pub editor: Option<Rect>,
    pub agent: Option<Rect>,
    pub bottom: Option<Rect>,
    pub hints: Rect,
}

pub(super) fn regions(app: &IdeApp, area: Rect) -> Regions {
    let mode = mode_for(area.width);
    let header = Rect { height: 1, ..area };
    let hints = Rect {
        y: area.y + area.height.saturating_sub(1),
        height: 1,
        ..area
    };
    let tab_h = u16::from(mode == Mode::Narrow);
    let tabbar = (tab_h == 1).then_some(Rect {
        y: area.y + 1,
        height: 1,
        ..area
    });
    let body = Rect {
        y: area.y + 1 + tab_h,
        height: area.height.saturating_sub(2 + tab_h),
        ..area
    };
    let mut out = Regions {
        header,
        tabbar,
        hints,
        ..Regions::default()
    };
    match mode {
        Mode::Narrow => {
            let pane = match app.focus {
                Pane::Bottom if !app.bottom_open => Pane::Editor,
                other => other,
            };
            match pane {
                Pane::Rail => out.rail = Some(body),
                Pane::Editor => out.editor = Some(body),
                Pane::Agent => out.agent = Some(body),
                Pane::Bottom => out.bottom = Some(body),
            }
        }
        Mode::Mid | Mode::Wide => {
            let rail = Rect {
                width: RAIL_W.min(body.width),
                ..body
            };
            let mut rest = Rect {
                x: body.x + rail.width,
                width: body.width.saturating_sub(rail.width),
                ..body
            };
            out.rail = Some(rail);
            let agent_open = mode == Mode::Wide || app.focus == Pane::Agent;
            if agent_open {
                let w = AGENT_W.min(rest.width.saturating_sub(20));
                out.agent = Some(Rect {
                    x: rest.x + rest.width - w,
                    width: w,
                    ..rest
                });
                rest.width -= w;
            }
            if app.bottom_open && rest.height > BOTTOM_H + 6 {
                out.bottom = Some(Rect {
                    y: rest.y + rest.height - BOTTOM_H,
                    height: BOTTOM_H,
                    ..rest
                });
                rest.height -= BOTTOM_H;
            }
            out.editor = Some(rest);
        }
    }
    out
}

pub fn draw(app: &mut IdeApp, frame: &mut Frame) {
    app.hits.clear();
    let area = frame.area();
    app.size = (area.width, area.height);
    crate::surface::render_backdrop(frame, area);
    if area.width < 30 || area.height < 8 {
        let buf = frame.buffer_mut();
        put(
            buf,
            area.x,
            area.y,
            "terminal too small for IDE mode",
            styles::muted(),
            area.width,
        );
        return;
    }
    let regions = regions(app, area);
    let mut hits: Vec<(Rect, Hit)> = Vec::new();
    {
        let buf = frame.buffer_mut();
        draw_header(app, buf, regions.header);
        if let Some(bar) = regions.tabbar {
            draw_tabbar(app, buf, bar, &mut hits);
        }
        let narrow = regions.tabbar.is_some();
        if let Some(rect) = regions.rail {
            rail::draw(app, buf, rect, narrow, &mut hits);
        }
        if let Some(rect) = regions.editor {
            editor::draw(app, buf, rect, &mut hits);
        }
        if let Some(rect) = regions.agent {
            agent::draw(app, buf, rect, &mut hits);
        }
        if let Some(rect) = regions.bottom {
            bottom::draw(app, buf, rect, &mut hits);
        }
        // Hairlines between the panes.
        let rule = if g().unicode { "│" } else { "|" };
        if let (Some(rail), Some(_)) = (regions.rail, regions.editor) {
            if !narrow {
                vline(
                    buf,
                    rail.x + rail.width.saturating_sub(1),
                    rail.y,
                    rail.height,
                    rule,
                );
            }
        }
        if let (Some(agent), Some(_)) = (regions.agent, regions.editor) {
            if !narrow {
                vline(buf, agent.x, agent.y, agent.height, rule);
            }
        }
        hints::draw(app, buf, regions.hints, &mut hits);
    }
    if app.tools.why_popup.is_some() {
        why::draw(app, frame, area);
        hits.push((area, Hit::Overlay));
    }
    if app.help_open {
        help::draw(frame, area);
        hits.push((area, Hit::Overlay));
    }
    app.hits = hits;
}

fn vline(buf: &mut Buffer, x: u16, y: u16, h: u16, glyph: &str) {
    for row in y..y + h {
        put(buf, x, row, glyph, styles::line(), 1);
    }
}

fn draw_header(app: &IdeApp, buf: &mut Buffer, rect: Rect) {
    let mut x = rect.x;
    x = put(buf, x, rect.y, " IDE ", styles::brand().bold(), rect.width);
    x = put(
        buf,
        x,
        rect.y,
        &app.project,
        styles::fg().bold(),
        rect.right().saturating_sub(x),
    );
    let branch = app
        .panel_session()
        .and_then(|id| app.git.get(&id).map(|s| s.branch.clone()))
        .filter(|b| !b.is_empty())
        .or_else(|| {
            app.panel_agent()
                .map(|a| a.branch.clone())
                .filter(|b| !b.is_empty())
        });
    if let Some(branch) = branch {
        x = put(
            buf,
            x,
            rect.y,
            &format!("  {} {}", g().branch, branch),
            styles::muted(),
            rect.right().saturating_sub(x),
        );
    }
    let to_review = app.to_review();
    let waiting = app.waiting_count();
    let mut tail = format!("  {to_review} to review");
    if waiting > 0 {
        tail.push_str(&format!(" {} {waiting} waiting", g().sep));
    }
    x = put(
        buf,
        x,
        rect.y,
        &tail,
        styles::faint(),
        rect.right().saturating_sub(x),
    );
    if waiting > 0 {
        let _ = x;
    }
}

fn draw_tabbar(app: &IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    let tabs: [(&str, Hit, bool); 7] = [
        (
            "Agents",
            Hit::NarrowRail(RailFocus::Agents, app.rail_tab),
            app.focus == Pane::Rail && app.rail_focus == RailFocus::Agents,
        ),
        (
            "Changes",
            Hit::NarrowRail(RailFocus::List, RailTab::Changes),
            app.focus == Pane::Rail
                && app.rail_focus == RailFocus::List
                && app.rail_tab == RailTab::Changes,
        ),
        (
            "Files",
            Hit::NarrowRail(RailFocus::List, RailTab::Files),
            app.focus == Pane::Rail
                && app.rail_focus == RailFocus::List
                && app.rail_tab == RailTab::Files,
        ),
        (
            "Git",
            Hit::NarrowRail(RailFocus::List, RailTab::Git),
            app.focus == Pane::Rail
                && app.rail_focus == RailFocus::List
                && app.rail_tab == RailTab::Git,
        ),
        ("Editor", Hit::Pane(Pane::Editor), app.focus == Pane::Editor),
        ("Agent", Hit::Pane(Pane::Agent), app.focus == Pane::Agent),
        ("Bottom", Hit::Pane(Pane::Bottom), app.focus == Pane::Bottom),
    ];
    let mut x = rect.x + 1;
    for (label, hit, active) in tabs {
        let w = label.width() as u16 + 2;
        if x + w > rect.right() {
            break;
        }
        let style = if active {
            styles::selected_row()
        } else {
            styles::muted()
        };
        put(buf, x, rect.y, &format!(" {label} "), style, w);
        hits.push((
            Rect {
                x,
                width: w,
                ..rect
            },
            hit,
        ));
        x += w + 1;
    }
}

// ----------------------------------------------------------------------------- helpers

/// Write `text` at `(x, y)`, at most `max` cells wide; returns the column after it.
pub(super) fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style, max: u16) -> u16 {
    if max == 0 {
        return x;
    }
    let area = buf.area;
    if y < area.y || y >= area.bottom() || x >= area.right() {
        return x;
    }
    let max = max.min(area.right() - x);
    let (next, _) = buf.set_stringn(x, y, text, usize::from(max), style);
    next
}

pub(super) fn fill(buf: &mut Buffer, rect: Rect, style: Style) {
    buf.set_style(rect, style);
}

pub(super) fn put_line(buf: &mut Buffer, x: u16, y: u16, line: &Line<'_>, max: u16) {
    if max == 0 || y >= buf.area.bottom() || y < buf.area.y {
        return;
    }
    buf.set_line(x, y, line, max.min(buf.area.right().saturating_sub(x)));
}

pub(super) fn cells(text: &str) -> usize {
    text.width()
}

/// Cut `text` to `max` cells, ending in an ellipsis when it had to.
pub(super) fn clip(text: &str, max: usize) -> String {
    let text = text.replace(['\n', '\r', '\t'], " ");
    if max == 0 {
        return String::new();
    }
    if text.width() <= max {
        return text;
    }
    if max == 1 {
        return g().ellipsis.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push_str(g().ellipsis);
    out
}

/// Draw `left` at the start and `right` flush to the end of `rect`'s row `y`; the left side gives
/// way first.
pub(super) fn split_row(
    buf: &mut Buffer,
    rect: Rect,
    y: u16,
    left: &str,
    left_style: Style,
    right: &str,
    right_style: Style,
) {
    let right_w = cells(right) as u16;
    let room = rect.width.saturating_sub(right_w + 1);
    put(
        buf,
        rect.x,
        y,
        &clip(left, usize::from(room)),
        left_style,
        room,
    );
    if right_w < rect.width {
        put(buf, rect.right() - right_w, y, right, right_style, right_w);
    }
}

pub(super) fn section_label(
    buf: &mut Buffer,
    rect: Rect,
    y: u16,
    label: &str,
    right: &str,
    right_style: Style,
) {
    split_row(
        buf,
        rect,
        y,
        &format!(" {label}"),
        styles::muted().bold(),
        right,
        right_style,
    );
}

pub(super) fn rule_row(buf: &mut Buffer, rect: Rect, y: u16) {
    let glyph = if g().unicode { "─" } else { "-" };
    for x in rect.x..rect.right() {
        put(buf, x, y, glyph, styles::line(), 1);
    }
}

pub(super) fn money(usd: f64) -> String {
    crate::board::fmt_cost(usd)
}
