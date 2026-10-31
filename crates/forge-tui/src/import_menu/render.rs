//! Draws `/import`: a centered card with a search line, the agent filter, and a scrolling list
//! that reads the same at 80x24 and on a wide terminal.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::{AgentFilter, ImportMenu, ImportRow, Load, Tone};
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

/// `3d ago`, `5h ago`, `2m ago`.
fn age(now: i64, then: i64) -> String {
    let s = (now - then).max(0);
    match s {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

/// `~/code/forge` for a path under home, else the path; the tail survives truncation.
fn short_dir(cwd: &str, max: usize) -> String {
    let home = forge_config::home_dir().map(|h| h.display().to_string());
    let shown = match home.as_deref().and_then(|h| cwd.strip_prefix(h)) {
        Some(rest) => format!("~{rest}"),
        None => cwd.to_string(),
    };
    if width(&shown) <= max {
        return shown;
    }
    let tail: Vec<char> = shown.chars().rev().take(max.saturating_sub(1)).collect();
    format!(
        "{}{}",
        g().ellipsis,
        tail.into_iter().rev().collect::<String>()
    )
}

fn row_line(m: &ImportMenu, r: &ImportRow, focused: bool, w: usize, now: i64) -> Line<'static> {
    let mark = if r.imported {
        "-"
    } else if m.is_selected(r) {
        g().task_done
    } else {
        g().task_open
    };
    let agent = pad(&r.agent, 7);
    let when = pad(&trunc(&age(now, r.modified), 8), 9);
    let state = pad(if r.imported { "imported" } else { "" }, 8);
    let dir_w = (w / 3).clamp(10, 34);
    let fixed = 2 + 2 + 7 + 1 + dir_w + 1 + 9 + 8;
    let title_w = w.saturating_sub(fixed).max(8);
    let dir = match (&r.cwd, r.cwd_exists) {
        (Some(c), true) => short_dir(c, dir_w),
        (Some(c), false) => format!("{} (missing)", short_dir(c, dir_w.saturating_sub(10))),
        (None, _) => "(no folder)".to_string(),
    };
    let dim = r.imported;
    let title_style = if dim { styles::faint() } else { styles::fg() };
    let mut spans = vec![
        Span::styled(
            format!("{} ", if focused { g().prompt } else { " " }),
            if focused {
                styles::brand()
            } else {
                styles::faint()
            },
        ),
        Span::styled(
            format!("{mark} "),
            if m.is_selected(r) {
                styles::ok()
            } else {
                styles::faint()
            },
        ),
        Span::styled(agent, styles::muted()),
        Span::styled(pad(&trunc(&r.title, title_w), title_w + 1), title_style),
        Span::styled(pad(&dir, dir_w + 1), styles::faint()),
        Span::styled(when, styles::faint()),
        Span::styled(state, styles::faint()),
    ];
    if !focused && dim {
        spans.iter_mut().for_each(|s| s.style = styles::faint());
    }
    let mut line = Line::from(spans);
    if focused {
        line.style = styles::selected_row();
    }
    line
}

fn filter_line(m: &ImportMenu) -> Line<'static> {
    let mut spans = vec![Span::styled("Search ".to_string(), styles::muted())];
    spans.push(Span::styled(m.query.clone(), styles::fg()));
    spans.push(Span::styled("\u{258f}".to_string(), styles::brand()));
    spans.push(Span::raw("   "));
    for f in [AgentFilter::All, AgentFilter::Claude, AgentFilter::Codex] {
        let on = f == m.agent;
        spans.push(Span::styled(
            format!(" {} ", f.label()),
            if on {
                styles::key_chip()
            } else {
                styles::faint()
            },
        ));
    }
    Line::from(spans)
}

fn footer(m: &ImportMenu, max: usize) -> String {
    let mut pairs: Vec<(String, &str)> = vec![
        (g().up_down.to_string(), "move"),
        ("Enter".into(), "import"),
        ("Space".into(), "select"),
        (g().left_right.to_string(), "all/none"),
        ("Tab".into(), "agent"),
        ("Esc".into(), "close"),
    ];
    if m.importing {
        pairs = vec![("Importing".into(), "")];
    }
    loop {
        let text = format!(
            " {} ",
            pairs
                .iter()
                .map(|(k, l)| if l.is_empty() {
                    k.clone()
                } else {
                    format!("{k} {l}")
                })
                .collect::<Vec<_>>()
                .join(&format!(" {} ", g().sep))
        );
        if width(&text) <= max || pairs.len() <= 2 {
            return text;
        }
        let drop = pairs.len() - 2;
        pairs.remove(drop);
    }
}

pub(crate) fn render_import_menu(f: &mut Frame, app: &App, region: Rect) {
    let m = &app.import_menu;
    if !m.open {
        return;
    }
    surface::dim_backdrop(f, region);
    let card_w = region.width.saturating_sub(2).clamp(1, 120);
    let inner_w = (card_w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let visible = m.visible();
    let list_rows = visible.len().max(1) as u16;
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
    let right = if m.selected_count() > 0 {
        format!(" {} selected ", m.selected_count())
    } else {
        format!(" {} found ", m.rows.iter().filter(|r| !r.imported).count())
    };
    let inner = crate::overlays::padded_panel(
        f,
        card,
        surface::title("Import conversations", SurfaceTone::Brand),
        Some(Span::styled(right, styles::faint())),
        Some(Span::styled(
            footer(m, (card.width as usize).saturating_sub(6)),
            styles::faint(),
        )),
        SurfaceTone::Brand,
    );
    if inner.width < 20 || inner.height < 4 {
        return;
    }
    f.render_widget(Paragraph::new(filter_line(m)), Rect { height: 1, ..inner });
    let hint = visible
        .get(m.cursor)
        .filter(|r| !r.cwd_exists && !r.imported)
        .map(|_| {
            format!(
                "Its folder is not on this machine: it opens in {}.",
                m.fallback
            )
        });
    let notice_rows = u16::from(m.notice.is_some() || hint.is_some());
    let body = Rect {
        y: inner.y + 2,
        height: inner.height.saturating_sub(2 + notice_rows),
        ..inner
    };
    let message = match &m.load {
        Load::Scanning => Some((
            format!(
                "{} Looking for Claude Code and Codex conversations\u{2026}",
                g().spin(m.tick as usize)
            ),
            styles::muted(),
        )),
        Load::Failed(e) => Some((format!("{} {}", g().fail, trunc(e, inner_w)), styles::err())),
        Load::Ready if m.rows.is_empty() => Some((
            "No importable sessions found on this computer.".to_string(),
            styles::muted(),
        )),
        Load::Ready if visible.is_empty() => {
            Some(("Nothing matches that search.".to_string(), styles::muted()))
        }
        Load::Ready => None,
    };
    if let Some((text, style)) = message {
        f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), body);
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let h = body.height as usize;
        let offset = m
            .cursor
            .saturating_sub(h.saturating_sub(1))
            .min(visible.len().saturating_sub(h));
        let offset = if m.cursor < offset { m.cursor } else { offset };
        let lines: Vec<Line> = visible
            .iter()
            .enumerate()
            .skip(offset)
            .take(h)
            .map(|(i, r)| row_line(m, r, i == m.cursor, inner_w, now))
            .collect();
        f.render_widget(Paragraph::new(lines), body);
        let edge = body.x + body.width;
        for (shown, y, glyph) in [
            (offset > 0, body.y, g().up),
            (
                offset + h < visible.len(),
                body.y + body.height.saturating_sub(1),
                g().down,
            ),
        ] {
            if shown {
                f.buffer_mut()[(edge, y)]
                    .set_symbol(glyph)
                    .set_style(styles::muted());
            }
        }
    }
    let (text, style, glyph) = match (&m.notice, hint) {
        (None, Some(h)) => (h, styles::muted(), g().info),
        (Some(n), _) => (
            n.text.clone(),
            match n.tone {
                Tone::Ok => styles::ok(),
                Tone::Warn => styles::warn(),
                Tone::Err => styles::err(),
            },
            match n.tone {
                Tone::Ok => g().ok,
                Tone::Warn => g().warn,
                Tone::Err => g().fail,
            },
        ),
        (None, None) => return,
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            trunc(&format!("{glyph} {text}"), inner.width as usize),
            style,
        ))),
        Rect {
            y: inner.y + inner.height - 1,
            height: 1,
            ..inner
        },
    );
}
