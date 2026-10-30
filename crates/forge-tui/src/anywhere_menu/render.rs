//! Renders the `/anywhere` menu: a centered card that scrolls to keep the focused row in view,
//! so it reads the same at 80x24 and on a wide terminal.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::{
    category_blurb, AnywhereMenu, AnywhereSnapshot, CategoryView, Connector, Devices, Load, Row,
    Tone,
};
use crate::app::App;
use crate::surface::{self, SurfaceTone};
use crate::theme::{glyphs::g, styles};

/// Card height for `content` rows: borders, plus the blank rows top and bottom that
/// `padded_panel` only draws once a card is tall enough.
pub(super) fn card_rows(content: u16) -> u16 {
    let padded = content + 2 + 2 * crate::overlays::PAD_Y;
    if padded >= crate::overlays::SHORT_REGION {
        padded
    } else {
        content + 2
    }
}

pub(super) fn width(text: &str) -> usize {
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
    let w = width(text);
    format!("{text}{}", " ".repeat(to.saturating_sub(w)))
}

fn pad_left(text: &str, to: usize) -> String {
    let w = width(text);
    format!("{}{text}", " ".repeat(to.saturating_sub(w)))
}

/// `4s ago`, `2m ago`, `3h ago`, `5d ago`.
pub(crate) fn ago(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms.saturating_sub(then_ms) / 1000).max(0);
    match secs {
        0..=4 => "just now".to_string(),
        5..=59 => format!("{secs}s ago"),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

struct Doc {
    lines: Vec<Line<'static>>,
    focus: Option<usize>,
    width: usize,
    /// Tall terminals get a subtitle and a little more air.
    roomy: bool,
}

impl Doc {
    fn push(&mut self, spans: Vec<Span<'static>>) {
        self.lines.push(Line::from(spans));
    }

    fn blank(&mut self) {
        self.lines.push(Line::default());
    }

    fn heading(&mut self, title: &str, right: Option<(String, Style)>) {
        let mut spans = vec![Span::styled(title.to_string(), styles::fg().bold())];
        if let Some((text, style)) = right {
            let used = width(title) + width(&text);
            spans.push(Span::raw(
                " ".repeat(self.width.saturating_sub(used).max(2)),
            ));
            spans.push(Span::styled(text, style));
        }
        self.push(spans);
    }

    fn row(&mut self, focused: bool, mut spans: Vec<Span<'static>>) {
        let caret = if focused { g().prompt } else { " " };
        spans.insert(
            0,
            Span::styled(
                format!("{caret} "),
                if focused {
                    styles::brand()
                } else {
                    styles::faint()
                },
            ),
        );
        if focused {
            self.focus = Some(self.lines.len());
        }
        let mut line = Line::from(spans);
        if focused {
            line.style = styles::selected_row();
        }
        self.lines.push(line);
    }
}

fn state_dot(on: bool) -> Span<'static> {
    if on {
        Span::styled(g().live_dot, styles::ok())
    } else {
        Span::styled(g().task_open, styles::faint())
    }
}

fn kv(label: &str, value: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(pad(label, 15), styles::muted())];
    spans.extend(value);
    spans
}

fn indented(mut spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    spans.insert(0, Span::raw("  "));
    spans
}

fn account_section(doc: &mut Doc, m: &AnywhereMenu, data: &AnywhereSnapshot) {
    let rows = m.rows();
    let focused = |row: Row| rows.get(m.cursor) == Some(&row);
    doc.heading("Account and connection", None);
    let who = match (&data.login, data.signed_in) {
        (Some(login), true) => vec![
            state_dot(true),
            Span::styled(format!(" {login}"), styles::fg()),
        ],
        (None, true) => vec![state_dot(true), Span::styled(" signed in", styles::fg())],
        _ => vec![
            state_dot(false),
            Span::styled(" not signed in", styles::warn()),
        ],
    };
    if !data.signed_in {
        doc.push(indented(kv("Account", who)));
        doc.push(vec![
            Span::raw("  "),
            Span::styled(
                trunc(
                    "Sign in to sync keys, skills, settings and MCP across machines.",
                    doc.width.saturating_sub(2),
                ),
                styles::muted(),
            ),
        ]);
        doc.row(
            focused(Row::SignInToAnywhere),
            vec![
                Span::styled("Sign in to Forge Anywhere".to_string(), styles::fg().bold()),
                Span::styled(
                    "   GitHub sign-in in your browser".to_string(),
                    styles::faint(),
                ),
            ],
        );
    } else {
        let (dot, style) = match data.connector {
            Connector::Live => (true, styles::fg()),
            Connector::Unhealthy => (false, styles::warn()),
            Connector::Off => (false, styles::muted()),
        };
        let hosts = match &m.devices {
            Devices::Loading => Span::styled(
                format!("{} loading", g().spin(m.tick as usize)),
                styles::faint(),
            ),
            Devices::Unavailable(_) => Span::styled("unavailable".to_string(), styles::faint()),
            Devices::Loaded {
                hosts_online,
                hosts_total,
                ..
            } => Span::styled(
                format!("{hosts_online} of {hosts_total} online"),
                styles::fg(),
            ),
        };
        let sync = if data.sync_enabled {
            Span::styled("on".to_string(), styles::fg())
        } else {
            Span::styled("off".to_string(), styles::warn())
        };
        let col = (doc.width.saturating_sub(2) / 2).clamp(30, 48);
        let two = |l: (&str, Vec<Span<'static>>), r: (&str, Vec<Span<'static>>)| {
            let used: usize = l.1.iter().map(|s| width(&s.content)).sum::<usize>() + 15;
            let mut spans = vec![Span::raw("  "), Span::styled(pad(l.0, 15), styles::muted())];
            spans.extend(l.1);
            spans.push(Span::raw(" ".repeat(col.saturating_sub(used).max(2))));
            spans.push(Span::styled(pad(r.0, 11), styles::muted()));
            spans.extend(r.1);
            spans
        };
        doc.push(two(
            ("Account", who),
            (
                "Connector",
                vec![
                    state_dot(dot),
                    Span::styled(
                        format!(
                            " {}",
                            trunc(&data.connector_text, doc.width.saturating_sub(col + 18))
                        ),
                        style,
                    ),
                ],
            ),
        ));
        doc.push(two(
            (
                "This device",
                vec![Span::styled(
                    trunc(&data.device_label, col.saturating_sub(18)),
                    styles::fg(),
                )],
            ),
            ("Hosts", vec![hosts]),
        ));
        doc.push(two(("Anywhere sync", vec![sync]), ("", vec![])));
        if !data.sync_enabled {
            doc.push(vec![
                Span::raw("  "),
                Span::styled(
                    trunc(
                        &format!("{} Account sync is paused while Anywhere sync is off (anywhere.sync in /config).", g().warn),
                        doc.width.saturating_sub(2),
                    ),
                    styles::warn(),
                ),
            ]);
        }
    }
    doc.row(
        focused(Row::OpenWeb),
        vec![
            Span::styled(
                "Open Recovery Center and approvals".to_string(),
                styles::fg(),
            ),
            Span::styled("   web, in your browser".to_string(), styles::faint()),
        ],
    );
}

fn category_row(doc: &mut Doc, m: &AnywhereMenu, i: usize, c: &CategoryView, focused: bool) {
    let chip = if c.enabled {
        Span::styled(format!("{} on  ", g().live_dot), styles::ok())
    } else {
        Span::styled(format!("{} off ", g().task_open), styles::faint())
    };
    let label_style = if c.enabled || focused {
        styles::fg().add_modifier(Modifier::BOLD)
    } else {
        styles::muted()
    };
    let items = if c.enabled || c.items > 0 {
        format!(
            "{} {}",
            c.items,
            if c.items == 1 { "item" } else { "items" }
        )
    } else {
        String::new()
    };
    let pending = if c.enabled && c.pending > 0 {
        format!("{} pending", c.pending)
    } else {
        String::new()
    };
    let (tail, tail_style) = if !c.conflicts.is_empty() {
        (
            format!(
                "{} {} {}",
                g().warn,
                c.conflicts.len(),
                if c.conflicts.len() == 1 {
                    "conflict"
                } else {
                    "conflicts"
                }
            ),
            styles::warn(),
        )
    } else if !c.skipped.is_empty() {
        (
            format!("{} {} skipped", g().info, c.skipped.len()),
            styles::muted(),
        )
    } else if c.remote_only > 0 && c.enabled {
        (
            format!("{} waiting to apply", c.remote_only),
            styles::muted(),
        )
    } else if c.remote_only > 0 {
        (
            format!("{} on other devices", c.remote_only),
            styles::muted(),
        )
    } else if let (true, Some(at)) = (c.enabled, c.last_sync_at) {
        (format!("synced {}", ago(m.now_ms, at)), styles::faint())
    } else {
        (String::new(), styles::faint())
    };
    let fixed = 2 + 6 + 17 + 9 + 12;
    let tail_room = doc.width.saturating_sub(fixed + 1);
    doc.row(
        focused,
        vec![
            chip,
            Span::styled(pad(&trunc(&c.label, 16), 17), label_style),
            Span::styled(pad_left(&items, 8), styles::muted()),
            Span::raw("  "),
            Span::styled(pad(&pending, 10), styles::muted()),
            Span::raw(" "),
            Span::styled(trunc(&tail, tail_room), tail_style),
        ],
    );
    let _ = i;
    let indent = "      ";
    let room = doc.width.saturating_sub(indent.len() + 4);
    if c.enabled {
        for (item, detail) in &c.conflicts {
            doc.push(vec![
                Span::raw(indent),
                Span::styled(format!("{} ", g().warn), styles::warn()),
                Span::styled(trunc(&format!("{item}: {detail}"), room), styles::muted()),
            ]);
        }
        for (item, reason) in &c.skipped {
            doc.push(vec![
                Span::raw(indent),
                Span::styled(format!("{} ", g().info), styles::faint()),
                Span::styled(
                    trunc(&format!("skipped {item}: {reason}"), room),
                    styles::muted(),
                ),
            ]);
        }
    }
    if focused {
        let blurb = category_blurb(&c.id);
        if !blurb.is_empty() {
            doc.push(vec![
                Span::raw(indent),
                Span::styled(trunc(blurb, room + 2), styles::faint()),
            ]);
        }
    }
}

fn sync_section(doc: &mut Doc, m: &AnywhereMenu, data: &AnywhereSnapshot) {
    let rows = m.rows();
    let cursor_row = rows.get(m.cursor).copied();
    doc.blank();
    let right = if m.syncing {
        (
            format!("{} syncing…", g().spin(m.tick as usize)),
            styles::muted(),
        )
    } else {
        match data.last_sync_at {
            Some(at) => (format!("last sync {}", ago(m.now_ms, at)), styles::faint()),
            None => ("never synced".to_string(), styles::faint()),
        }
    };
    doc.heading("Sync across machines", Some(right));
    if doc.roomy {
        doc.push(vec![
            Span::raw("  "),
            Span::styled(
                trunc(
                    "Per device, end-to-end encrypted. Off until you turn a category on.",
                    doc.width.saturating_sub(2),
                ),
                styles::faint(),
            ),
        ]);
    }
    let any_on = data.categories.iter().any(|c| c.enabled);
    if any_on && !data.worker_running {
        doc.push(vec![
            Span::raw("  "),
            Span::styled(
                trunc(
                    &format!(
                        "{} No sync worker answered here. Run `forge serve` to sync in the background.",
                        g().warn
                    ),
                    doc.width.saturating_sub(2),
                ),
                styles::warn(),
            ),
        ]);
    }
    if let Some(error) = &data.last_error {
        doc.push(vec![
            Span::raw("  "),
            Span::styled(
                trunc(
                    &format!("{} Last sync failed: {error}", g().fail),
                    doc.width.saturating_sub(2),
                ),
                styles::err(),
            ),
        ]);
    }
    for (i, c) in data.categories.iter().enumerate() {
        category_row(doc, m, i, c, cursor_row == Some(Row::Category(i)));
    }
    if !data.signed_in {
        return;
    }
    let busy = m.syncing;
    let label = if busy {
        format!("{} Syncing…", g().spin(m.tick as usize))
    } else {
        "Sync now".to_string()
    };
    doc.row(
        cursor_row == Some(Row::SyncNow),
        vec![
            Span::styled(label, styles::fg().bold()),
            Span::styled(
                "   ask the sync worker for a cycle".to_string(),
                styles::faint(),
            ),
        ],
    );
    if !data.sign_ins.is_empty() {
        doc.blank();
        doc.push(vec![
            Span::raw("  "),
            Span::styled("Sign in again".to_string(), styles::muted().bold()),
            Span::styled(
                "  used on your other devices; logins never sync".to_string(),
                styles::faint(),
            ),
        ]);
        for (i, s) in data.sign_ins.iter().enumerate() {
            doc.row(
                cursor_row == Some(Row::SignIn(i)),
                vec![
                    Span::styled(pad(&s.label, 22), styles::fg()),
                    Span::styled(
                        trunc(&s.command, doc.width.saturating_sub(26)),
                        styles::faint(),
                    ),
                ],
            );
        }
    }
    for tool in &data.missing_tools {
        doc.push(vec![
            Span::raw("  "),
            Span::styled(
                trunc(
                    &format!("{} {} missing here: {}", g().info, tool.label, tool.hint),
                    doc.width.saturating_sub(2),
                ),
                styles::muted(),
            ),
        ]);
    }
}

fn devices_section(doc: &mut Doc, m: &AnywhereMenu) {
    let rows = m.rows();
    let cursor_row = rows.get(m.cursor).copied();
    doc.blank();
    match &m.devices {
        Devices::Loading => {
            doc.heading("Devices", None);
            doc.push(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{} Loading devices…", g().spin(m.tick as usize)),
                    styles::faint(),
                ),
            ]);
        }
        Devices::Unavailable(why) => {
            doc.heading("Devices", None);
            doc.push(vec![
                Span::raw("  "),
                Span::styled(
                    trunc(
                        &format!("{} Could not load devices: {why}", g().warn),
                        doc.width.saturating_sub(2),
                    ),
                    styles::warn(),
                ),
            ]);
        }
        Devices::Loaded { devices, .. } => {
            doc.heading(
                "Devices",
                Some((format!("{} enrolled", devices.len()), styles::faint())),
            );
            for (i, d) in devices.iter().enumerate() {
                let seen = d
                    .last_seen_at
                    .map_or_else(|| "never".into(), |at| ago(m.now_ms, at));
                let mut spans = vec![
                    state_dot(d.this_device),
                    Span::raw(" "),
                    Span::styled(pad(&trunc(&d.name, 20), 21), styles::fg()),
                    Span::styled(
                        pad(if d.this_device { "this device" } else { "" }, 13),
                        styles::ok(),
                    ),
                    Span::styled(pad(&format!("seen {seen}"), 18), styles::muted()),
                ];
                let enrolled = format!("enrolled {}", d.enrolled);
                if doc.width >= 2 + 2 + 21 + 13 + 18 + width(&enrolled) {
                    spans.push(Span::styled(enrolled, styles::faint()));
                }
                doc.row(cursor_row == Some(Row::Device(i)), spans);
            }
            if devices.len() > 1 {
                doc.push(vec![
                    Span::raw("    "),
                    Span::styled(
                        trunc(
                            "Revoking needs your recovery phrase: forge anywhere devices --revoke <id>",
                            doc.width.saturating_sub(4),
                        ),
                        styles::faint(),
                    ),
                ]);
            }
        }
    }
}

fn build(m: &AnywhereMenu, inner_w: usize, roomy: bool) -> Doc {
    let mut doc = Doc {
        lines: Vec::new(),
        focus: None,
        width: inner_w,
        roomy,
    };
    match (&m.load, &m.data) {
        (Load::Failed(error), None) => {
            doc.push(vec![Span::styled(
                format!("{} Could not read Anywhere status", g().fail),
                styles::err().bold(),
            )]);
            doc.push(vec![Span::styled(trunc(error, inner_w), styles::muted())]);
            doc.blank();
            doc.push(vec![
                Span::styled("r".to_string(), styles::key_chip()),
                Span::styled(" try again".to_string(), styles::faint()),
            ]);
        }
        (_, None) => {
            doc.push(vec![Span::styled(
                format!("{} Reading Anywhere status…", g().spin(m.tick as usize)),
                styles::muted(),
            )]);
        }
        (_, Some(data)) => {
            account_section(&mut doc, m, data);
            sync_section(&mut doc, m, data);
            if data.signed_in {
                devices_section(&mut doc, m);
            }
        }
    }
    doc
}

fn footer(m: &AnywhereMenu, max: usize) -> String {
    if m.data.is_none() {
        return match m.load {
            Load::Failed(_) => " r try again · Esc close ".to_string(),
            _ => " Esc close ".to_string(),
        };
    }
    let focus = m.focused();
    let act = match focus {
        Some(Row::Category(_)) => Some(("Space", "toggle")),
        Some(Row::SyncNow) => Some(("Enter", "sync")),
        Some(Row::SignIn(_)) | Some(Row::SignInToAnywhere) => Some(("Enter", "sign in")),
        Some(Row::OpenWeb) => Some(("Enter", "open")),
        _ => None,
    };
    let mut pairs: Vec<(&str, &str)> = vec![(g().up_down, "move")];
    pairs.extend(act);
    pairs.extend([
        ("Tab", "section"),
        ("s", "sync"),
        ("r", "refresh"),
        ("Esc", "close"),
    ]);
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

/// `region` is the rows between the header and the composer: the card never covers either.
pub(crate) fn render_anywhere_menu(f: &mut Frame, app: &App, region: Rect) {
    let m = &app.anywhere_menu;
    if !m.open {
        return;
    }
    let area = region;
    surface::dim_backdrop(f, area);
    let card_w = area.width.saturating_sub(2).clamp(1, 112);
    let inner_w = (card_w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let doc = build(m, inner_w, area.height >= 32);
    let notice_rows = u16::from(m.notice.is_some());
    let want = card_rows(doc.lines.len() as u16 + notice_rows);
    let card_h = want.max(6).min(area.height);
    let card = Rect {
        x: area.x + area.width.saturating_sub(card_w) / 2,
        y: area.y + area.height.saturating_sub(card_h) / 2,
        width: card_w,
        height: card_h,
    };
    let inner = crate::overlays::padded_panel(
        f,
        card,
        surface::title("Forge Anywhere", SurfaceTone::Brand),
        Some(Span::styled(" /anywhere ", styles::faint())),
        Some(Span::styled(
            footer(m, (card.width as usize).saturating_sub(6)),
            styles::faint(),
        )),
        SurfaceTone::Brand,
    );
    if inner.width < 20 || inner.height < 3 {
        return;
    }
    let body_h = inner.height.saturating_sub(notice_rows) as usize;
    let total = doc.lines.len();
    let focus = doc.focus.unwrap_or(0);
    let mut offset = m.scroll.min(total.saturating_sub(body_h));
    if focus < offset + 2 {
        offset = focus.saturating_sub(2);
    } else if focus + 3 > offset + body_h {
        offset = (focus + 3).saturating_sub(body_h);
    }
    if doc.focus.is_none() {
        offset = 0;
    }
    offset = offset.min(total.saturating_sub(body_h));
    let body = Rect {
        height: body_h as u16,
        ..inner
    };
    let visible: Vec<Line> = doc
        .lines
        .iter()
        .skip(offset)
        .take(body_h)
        .cloned()
        .collect();
    f.render_widget(Paragraph::new(visible), body);
    let edge = body.x + body.width;
    for (shown, y, glyph) in [
        (offset > 0, body.y, g().up),
        (
            offset + body_h < total,
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
    if let Some(notice) = &m.notice {
        let style = match notice.tone {
            Tone::Ok => styles::ok(),
            Tone::Warn => styles::warn(),
            Tone::Err => styles::err(),
        };
        let glyph = match notice.tone {
            Tone::Ok => g().ok,
            Tone::Warn => g().warn,
            Tone::Err => g().fail,
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
    }
    if let Some(confirm) = m.confirm {
        super::confirm::render_keys_confirm(f, card, confirm.on_confirm);
    }
}
