//! The bottom panel's frame: its tabs and the key to close it. Checks, shells and the terminal
//! fill in later; the tab strip and focus handling are already the final shape.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{cells, put, rule_row};
use crate::ide::state::{BottomTab, Hit, IdeApp, Pane};
use crate::theme::styles;

pub(super) fn draw(app: &mut IdeApp, buf: &mut Buffer, rect: Rect, hits: &mut Vec<(Rect, Hit)>) {
    if rect.height < 3 {
        return;
    }
    rule_row(buf, rect, rect.y);
    let focused = app.focus == Pane::Bottom;
    let mut x = rect.x + 1;
    for (tab, label) in [
        (BottomTab::Checks, "Checks"),
        (BottomTab::Shells, "Shells"),
        (BottomTab::Terminal, "Terminal"),
    ] {
        let active = app.bottom_tab == tab;
        let style = if active && focused {
            styles::selected_row()
        } else if active {
            styles::fg().bold()
        } else {
            styles::muted()
        };
        let text = format!(" {label} ");
        let w = cells(&text) as u16;
        put(buf, x, rect.y + 1, &text, style, w);
        hits.push((
            Rect {
                x,
                y: rect.y + 1,
                width: w,
                height: 1,
            },
            Hit::BottomTab(tab),
        ));
        x += w + 1;
    }
    let close = "t close";
    let cw = cells(close) as u16;
    put(
        buf,
        rect.right() - cw - 1,
        rect.y + 1,
        close,
        styles::faint(),
        cw,
    );
    hits.push((
        Rect {
            x: rect.right() - cw - 1,
            y: rect.y + 1,
            width: cw,
            height: 1,
        },
        Hit::BottomToggle,
    ));
    let what = match app.bottom_tab {
        BottomTab::Checks => "Project checks, with file:line jumps, arrive in the next IDE update.",
        BottomTab::Shells => "Background shells and their live tail arrive in the next IDE update.",
        BottomTab::Terminal => "A terminal on the daemon arrives in the next IDE update.",
    };
    put(
        buf,
        rect.x + 2,
        rect.y + 3,
        what,
        styles::faint(),
        rect.width.saturating_sub(3),
    );
}
