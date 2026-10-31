//! Renders `/run`: a centered card with the two views, the list, and the output of the focused
//! action underneath. Sized from the region so it reads at 80x24, wide and 50 columns.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::{RunMenu, RunState, Tone, View};
use crate::app::App;
use crate::overlays::fmt_elapsed;
use crate::surface::{self, SurfaceTone};
use crate::theme::{glyphs::g, styles};

const SNIPPET: [&str; 4] = [
    "[[actions]]",
    "name = \"dev\"",
    "command = \"npm run dev\"",
    "cwd = \"web\"   # optional",
];

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

fn card_rows(content: u16) -> u16 {
    let padded = content + 2 + 2 * crate::overlays::PAD_Y;
    if padded >= crate::overlays::SHORT_REGION {
        padded
    } else {
        content + 2
    }
}

/// Rows of output shown under the list for a region this tall.
fn detail_rows(region_h: u16) -> u16 {
    match region_h {
        32.. => 8,
        24.. => 5,
        17.. => 3,
        _ => 0,
    }
}

fn host_label(host: &str) -> &'static str {
    if host == "loopback" {
        "local only"
    } else {
        "network"
    }
}

fn state_text(row: &super::ActionRow, now_ms: i64) -> (String, Style) {
    match row.state {
        RunState::Idle => ("idle".into(), styles::faint()),
        RunState::Running { since_ms } => {
            let secs = ((now_ms - since_ms).max(0) / 1000) as u64;
            (
                format!(
                    "running {}",
                    fmt_elapsed(std::time::Duration::from_secs(secs))
                ),
                styles::ok(),
            )
        }
        RunState::Exited { stopped: true, .. } => ("stopped".into(), styles::muted()),
        RunState::Exited { code: Some(0), .. } => ("exited 0".into(), styles::muted()),
        RunState::Exited { code: Some(c), .. } => (format!("exited {c}"), styles::err()),
        RunState::Exited { code: None, .. } => ("killed".into(), styles::err()),
    }
}

fn state_glyph(state: &RunState) -> Span<'static> {
    match state {
        RunState::Idle => Span::styled(g().task_open, styles::faint()),
        RunState::Running { .. } => Span::styled(g().live_dot, styles::ok()),
        RunState::Exited {
            code: Some(0) | None,
            ..
        } => Span::styled(g().ok, styles::muted()),
        RunState::Exited { .. } => Span::styled(g().fail, styles::err()),
    }
}

fn caret(focused: bool) -> Span<'static> {
    Span::styled(
        format!("{} ", if focused { g().prompt } else { " " }),
        if focused {
            styles::brand()
        } else {
            styles::faint()
        },
    )
}

fn row_line(spans: Vec<Span<'static>>, focused: bool) -> Line<'static> {
    let mut line = Line::from(spans);
    if focused {
        line.style = styles::selected_row();
    }
    line
}

fn action_lines(m: &RunMenu, w: usize) -> Vec<(Line<'static>, bool)> {
    let visible = m.visible_actions();
    let name_w = visible
        .iter()
        .map(|&i| width(&m.actions[i].action.name))
        .max()
        .unwrap_or(4)
        .clamp(4, 18);
    visible
        .iter()
        .enumerate()
        .map(|(n, &i)| {
            let row = &m.actions[i];
            let focused = n == m.cursor;
            let (status, status_style) = state_text(row, m.now_ms);
            let status_w = 16.min(w.saturating_sub(6));
            let fixed = 2 + 2 + name_w + 1;
            let cmd_w = w.saturating_sub(fixed + status_w + 1);
            let mut spans = vec![
                caret(focused),
                state_glyph(&row.state),
                Span::raw(" "),
                Span::styled(pad(&trunc(&row.action.name, name_w), name_w), styles::fg()),
                Span::raw(" "),
            ];
            let shown_cmd = if cmd_w >= 8 {
                let t = trunc(&row.action.command, cmd_w);
                let gap = cmd_w.saturating_sub(width(&t));
                spans.push(Span::styled(t, styles::faint()));
                spans.push(Span::raw(" ".repeat(gap + 1)));
                true
            } else {
                false
            };
            let status = trunc(&status, status_w.max(4));
            if !shown_cmd {
                let used = fixed + width(&status);
                spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
            }
            spans.push(Span::styled(status, status_style));
            (row_line(spans, focused), focused)
        })
        .collect()
}

fn port_lines(m: &RunMenu, w: usize) -> Vec<(Line<'static>, bool)> {
    m.visible_ports()
        .iter()
        .enumerate()
        .map(|(n, &i)| {
            let p = &m.ports[i];
            let focused = n == m.cursor;
            let source = match &p.action {
                Some(id) => format!("action {id}"),
                None => p.source.clone(),
            };
            let host = host_label(&p.host);
            let host_style = if p.host == "loopback" {
                styles::muted()
            } else {
                styles::warn()
            };
            let port_w = 7;
            let host_w = 10;
            let rest = w.saturating_sub(2 + port_w + host_w + 2);
            let proc_w = (rest / 2).clamp(6, 18).min(rest);
            let src_w = rest.saturating_sub(proc_w + 1);
            let mut spans = vec![
                caret(focused),
                Span::styled(pad(&format!(":{}", p.port), port_w), styles::brand().bold()),
                Span::styled(pad(&trunc(&p.process, proc_w), proc_w + 1), styles::fg()),
            ];
            if src_w >= 6 {
                spans.push(Span::styled(
                    pad(&trunc(&source, src_w), src_w + 1),
                    styles::faint(),
                ));
            }
            spans.push(Span::styled(host.to_string(), host_style));
            (row_line(spans, focused), focused)
        })
        .collect()
}

fn empty_lines(m: &RunMenu) -> Vec<Line<'static>> {
    let faint = |t: &str| Line::from(Span::styled(t.to_string(), styles::faint()));
    let filtered = !m.filter.trim().is_empty();
    if filtered {
        return vec![faint(&format!("Nothing matches \"{}\".", m.filter.trim()))];
    }
    match m.view {
        View::Actions if m.untrusted => vec![
            Line::from(Span::styled(
                "This project declares actions, but it is not trusted.",
                styles::warn(),
            )),
            Line::from(vec![
                Span::styled("Run ", styles::muted()),
                Span::styled("`forge trust`", styles::fg()),
                Span::styled(" to enable this project's actions.", styles::muted()),
            ]),
        ],
        View::Actions => {
            let mut out = vec![
                faint("No actions yet. Add some to .forge/config.toml:"),
                Line::default(),
            ];
            out.extend(
                SNIPPET
                    .iter()
                    .map(|l| Line::from(Span::styled(format!("  {l}"), styles::fg()))),
            );
            out
        }
        View::Ports if !m.ports_scanned => vec![faint("Looking for listening ports...")],
        View::Ports => vec![
            faint("Nothing is listening yet."),
            faint("Start an action, then come back; this view refreshes itself."),
        ],
    }
}

fn detail(m: &RunMenu, w: usize, rows: usize) -> Vec<Line<'static>> {
    let faint = |t: String| Line::from(Span::styled(t, styles::faint()));
    match m.view {
        View::Actions => {
            let Some(row) = m.selected_action() else {
                return Vec::new();
            };
            if row.output.is_empty() {
                let text = match row.state {
                    RunState::Idle => "No output yet. Enter runs it.",
                    _ => "No output yet.",
                };
                return vec![faint(text.to_string())];
            }
            let tail = row.output.len().saturating_sub(rows);
            row.output[tail..]
                .iter()
                .map(|line| {
                    let mut spans = crate::ansi_line::spans(line, styles::muted());
                    let mut used = 0;
                    spans.retain_mut(|s| {
                        let room = w.saturating_sub(used);
                        if room == 0 {
                            return false;
                        }
                        let text = trunc(&s.content, room);
                        used += width(&text);
                        s.content = text.into();
                        true
                    });
                    Line::from(spans)
                })
                .collect()
        }
        View::Ports => {
            let Some(p) = m.selected_port() else {
                return Vec::new();
            };
            vec![
                Line::from(vec![
                    Span::styled("url      ", styles::muted()),
                    Span::styled(format!("http://localhost:{}", p.port), styles::fg()),
                ]),
                Line::from(vec![
                    Span::styled("process  ", styles::muted()),
                    Span::styled(
                        trunc(
                            &format!("{} (pid {})", p.process, p.pid),
                            w.saturating_sub(9),
                        ),
                        styles::fg(),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("command  ", styles::muted()),
                    Span::styled(trunc(&p.command, w.saturating_sub(9)), styles::faint()),
                ]),
            ]
        }
    }
}

fn footer(m: &RunMenu, max: usize) -> String {
    if m.filtering {
        return " type to filter · Enter keep · Esc clear ".to_string();
    }
    let mut pairs: Vec<(&str, &str)> = vec![(g().up_down, "move")];
    match m.view {
        View::Actions => {
            let running = m
                .selected_action()
                .is_some_and(|r| matches!(r.state, RunState::Running { .. }));
            pairs.push(("Enter", if running { "stop" } else { "run" }));
            pairs.push(("r", "restart"));
        }
        View::Ports => {
            pairs.push(("c", "copy"));
            pairs.push(("o", "open"));
        }
    }
    pairs.extend([("Tab", "view"), ("/", "filter"), ("Esc", "close")]);
    loop {
        let text = format!(
            " {} ",
            pairs
                .iter()
                .map(|(k, l)| format!("{k} {l}"))
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

fn tab(label: String, active: bool) -> Span<'static> {
    if active {
        Span::styled(format!(" {label} "), styles::selected_row())
    } else {
        Span::styled(format!(" {label} "), styles::muted())
    }
}

/// `region` is the rows between the header and the composer: the card never covers either.
pub(crate) fn render_run_menu(f: &mut Frame, app: &App, region: Rect) {
    let m = &app.run_menu;
    if !m.open {
        return;
    }
    surface::dim_backdrop(f, region);
    let card_w = region.width.saturating_sub(2).clamp(1, 110);
    let inner_w = (card_w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let d_rows = detail_rows(region.height);
    let list: Vec<(Line<'static>, bool)> = match m.view {
        View::Actions => action_lines(m, inner_w),
        View::Ports => port_lines(m, inner_w),
    };
    let empty = if list.is_empty() {
        empty_lines(m)
    } else {
        Vec::new()
    };
    let list_rows = list.len().max(empty.len()).clamp(3, 10) as u16;
    let extra = u16::from(m.filtering || !m.filter.is_empty()) + u16::from(m.notice.is_some());
    let detail_block = if d_rows > 0 && !list.is_empty() {
        d_rows + 1
    } else {
        0
    };
    // header, tabs, blank, list, [detail rule + rows], [filter], [notice]
    let content = 3 + list_rows + detail_block + extra;
    let card_h = card_rows(content).max(6).min(region.height);
    let card = Rect {
        x: region.x + region.width.saturating_sub(card_w) / 2,
        y: region.y + region.height.saturating_sub(card_h) / 2,
        width: card_w,
        height: card_h,
    };
    let inner = crate::overlays::padded_panel(
        f,
        card,
        surface::title("Run", SurfaceTone::Brand),
        Some(Span::styled(" /run ", styles::faint())),
        Some(Span::styled(
            footer(m, (card.width as usize).saturating_sub(6)),
            styles::faint(),
        )),
        SurfaceTone::Brand,
    );
    if inner.width < 20 || inner.height < 5 {
        return;
    }
    let running = m.running_count();
    let status = if running > 0 {
        (format!("{running} running"), styles::ok())
    } else {
        ("none running".to_string(), styles::faint())
    };
    let project = trunc(
        &m.project,
        (inner.width as usize).saturating_sub(width(&status.0) + 2),
    );
    let gap = (inner.width as usize).saturating_sub(width(&project) + width(&status.0));
    let header = Line::from(vec![
        Span::styled(project, styles::fg().bold()),
        Span::raw(" ".repeat(gap)),
        Span::styled(status.0, status.1),
    ]);
    let tabs = Line::from(vec![
        tab(
            format!("Actions {}", m.actions.len()),
            m.view == View::Actions,
        ),
        Span::raw(" "),
        tab(format!("Ports {}", m.ports.len()), m.view == View::Ports),
    ]);
    f.render_widget(
        Paragraph::new(vec![header, tabs]),
        Rect { height: 2, ..inner },
    );

    let mut bottom = 0u16;
    if let Some(notice) = &m.notice {
        let (glyph, style) = match notice.tone {
            Tone::Ok => (g().ok, styles::ok()),
            Tone::Warn => (g().warn, styles::warn()),
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                trunc(&format!("{glyph} {}", notice.text), inner.width as usize),
                style,
            ))),
            Rect {
                y: inner.y + inner.height - 1,
                height: 1,
                ..inner
            },
        );
        bottom += 1;
    }
    if m.filtering || !m.filter.is_empty() {
        let cursor = if m.filtering { "_" } else { "" };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("/ ", styles::brand()),
                Span::styled(format!("{}{cursor}", m.filter), styles::fg()),
            ])),
            Rect {
                y: inner.y + inner.height - 1 - bottom,
                height: 1,
                ..inner
            },
        );
        bottom += 1;
    }

    let top = 3u16;
    let avail = inner.height.saturating_sub(top + bottom);
    let detail_h = if detail_block > 0 && avail >= detail_block + 2 {
        detail_block
    } else {
        0
    };
    let list_h = avail.saturating_sub(detail_h) as usize;
    let list_area = Rect {
        y: inner.y + top,
        height: list_h as u16,
        ..inner
    };
    if list.is_empty() {
        f.render_widget(Paragraph::new(empty), list_area);
        return;
    }
    let offset = (m.cursor + 1).saturating_sub(list_h.max(1));
    let shown: Vec<Line> = list
        .into_iter()
        .skip(offset)
        .take(list_h)
        .map(|(l, _)| l)
        .collect();
    f.render_widget(Paragraph::new(shown), list_area);
    if detail_h > 0 {
        let y = list_area.y + list_area.height;
        let title = match (m.view, m.selected_action()) {
            (View::Actions, Some(row)) => format!("{} output", row.action.name),
            (View::Actions, None) => "output".to_string(),
            (View::Ports, _) => "port".to_string(),
        };
        let h = g().border.horizontal_top;
        let head = format!(
            "{h} {} ",
            trunc(&title, (inner.width as usize).saturating_sub(6))
        );
        let rule = format!(
            "{head}{}",
            h.repeat((inner.width as usize).saturating_sub(width(&head)))
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(rule, styles::faint()))),
            Rect {
                y,
                height: 1,
                ..inner
            },
        );
        f.render_widget(
            Paragraph::new(detail(m, inner.width as usize, (detail_h - 1) as usize)),
            Rect {
                y: y + 1,
                height: detail_h - 1,
                ..inner
            },
        );
    }
}
