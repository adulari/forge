//! Drawing the `/config` editor: a framed takeover with the search row and scope control on top,
//! sections beside (or above) the settings of the current section, and the selected setting's
//! full help below. One value-widget grammar for every row kind.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line as TextLine, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::{ConfigEditor, RowKind, SettingRow};
use crate::surface;
use crate::theme::glyphs::g;
use crate::theme::styles;
use forge_types::truncate_ellipsis as truncate;

/// Total width from which sections get their own pane.
const TWO_PANE_MIN_WIDTH: u16 = 120;
const SECTION_PANE_WIDTH: u16 = 26;

fn upper(text: &str) -> String {
    text.to_uppercase()
}

/// The value widget for a row, in one grammar: `○ not set`, `✓ keyring`, `● on` / `○ off`,
/// `‹ auto-edit ›`, plain numbers and text.
fn value_spans(r: &SettingRow, selected: bool) -> Vec<Span<'static>> {
    let glyphs = g();
    let plain = if selected {
        styles::fg().bold()
    } else {
        styles::muted()
    };
    match &r.kind {
        RowKind::Bool => {
            let on = r.value == "true";
            if on {
                vec![Span::styled(
                    format!("{} on", glyphs.live_dot),
                    styles::ok(),
                )]
            } else {
                vec![Span::styled(
                    format!("{} off", glyphs.task_open),
                    styles::faint(),
                )]
            }
        }
        RowKind::Enum(_) => {
            let (l, rr) = if glyphs.unicode {
                ("‹", "›")
            } else {
                ("<", ">")
            };
            vec![Span::styled(format!("{l} {} {rr}", r.value), plain)]
        }
        RowKind::Secret => {
            let set = r.value.contains("set") && !r.value.contains("not");
            if set {
                vec![Span::styled(format!("{} keyring", glyphs.ok), styles::ok())]
            } else {
                vec![Span::styled(
                    format!("{} not set", glyphs.task_open),
                    styles::faint(),
                )]
            }
        }
        RowKind::ReadOnly => vec![Span::styled(
            "Enter edit in $EDITOR".to_string(),
            if selected {
                styles::warn()
            } else {
                styles::faint()
            },
        )],
        _ => {
            if r.value.is_empty() {
                vec![Span::styled("—".to_string(), styles::faint())]
            } else {
                vec![Span::styled(truncate(&r.value, 30), plain)]
            }
        }
    }
}

/// First sentence of a help text, for the muted description column.
fn short_help(help: &str) -> String {
    let first = help.split(". ").next().unwrap_or(help).trim();
    first.trim_end_matches('.').to_string()
}

fn right_aligned(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
) -> TextLine<'static> {
    let used: usize = left.iter().map(|s| s.content.width()).sum();
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let mut spans = left;
    spans.push(Span::raw(
        " ".repeat(width.saturating_sub(used + right_w).max(1)),
    ));
    spans.extend(right);
    TextLine::from(spans)
}

/// The scope control: the active scope is a raised, bold segment.
fn scope_spans(project: bool) -> Vec<Span<'static>> {
    let on = styles::selected_row();
    let off = styles::faint();
    let (user, proj) = if project { (off, on) } else { (on, off) };
    vec![
        Span::styled("scope: ", styles::faint()),
        Span::styled(" User ", user),
        Span::raw(" "),
        Span::styled(" Project ", proj),
    ]
}

/// One setting row: caret, label, muted description, value flush right.
fn setting_line(
    ed: &ConfigEditor,
    r: &SettingRow,
    selected: bool,
    label_w: usize,
    width: usize,
) -> TextLine<'static> {
    let glyphs = g();
    let caret = if selected { glyphs.prompt } else { " " };
    let value = if selected && ed.editing.is_some() {
        let buf = ed.editing.as_deref().unwrap_or("");
        let shown = if r.kind == RowKind::Secret {
            "•".repeat(buf.chars().count())
        } else {
            buf.to_string()
        };
        vec![Span::styled(
            format!("{shown}{}", glyphs.caret),
            styles::brand().bold(),
        )]
    } else {
        value_spans(r, selected)
    };
    let value_w: usize = value.iter().map(|s| s.content.width()).sum();
    // A label written as `Name — blurb` shows the name as the label and the blurb as the
    // description; otherwise the description is the first sentence of the help.
    let (name, blurb) = match r.label.split_once(" — ") {
        Some((name, blurb)) => (name, Some(blurb)),
        None => (r.label.as_str(), None),
    };
    let label = truncate(name, label_w);
    let label_pad = label_w.saturating_sub(label.width());
    let desc_room = width.saturating_sub(4 + label_w + 2 + value_w + 2);
    let desc = match (blurb, &r.help, desc_room >= 12) {
        (Some(blurb), _, true) => truncate(blurb, desc_room),
        (None, Some(help), true) => truncate(&short_help(help), desc_room),
        _ => String::new(),
    };
    let left = vec![
        Span::styled(
            format!(" {caret} "),
            if selected {
                styles::brand()
            } else {
                styles::faint()
            },
        ),
        Span::styled(
            format!("{label}{}", " ".repeat(label_pad)),
            if selected {
                styles::fg().bold()
            } else {
                styles::fg()
            },
        ),
        Span::styled(format!("  {desc}"), styles::muted()),
    ];
    let line = right_aligned(left, value, width);
    if !selected {
        return line;
    }
    let fill = styles::selected_row();
    TextLine::from(
        line.spans
            .into_iter()
            .map(|mut s| {
                s.style = s.style.patch(fill);
                s
            })
            .collect::<Vec<_>>(),
    )
}

/// The key hints for the footer border, by what the selection can do.
fn hints(ed: &ConfigEditor) -> String {
    let sep = format!(" {} ", g().sep);
    let parts: Vec<&str> = if ed.editing.is_some() {
        vec!["type a value", "Enter save", "Esc cancel"]
    } else {
        match ed.selected_row().map(|r| &r.kind) {
            Some(RowKind::Bool) => vec![
                "Tab scope",
                "Enter toggle",
                "Del reset",
                "PgUp/PgDn section",
                "Esc close",
            ],
            Some(RowKind::Enum(_)) => vec![
                "Tab scope",
                "←→ change",
                "Del reset",
                "PgUp/PgDn section",
                "Esc close",
            ],
            Some(RowKind::ReadOnly) => vec!["Tab scope", "Enter edit in $EDITOR", "Esc close"],
            _ => vec![
                "Tab scope",
                "Enter edit",
                "Del reset",
                "PgUp/PgDn section",
                "Esc close",
            ],
        }
    };
    parts.join(&sep)
}

/// What the selected setting is, for its help block.
fn kind_label(r: &SettingRow) -> String {
    match &r.kind {
        RowKind::Bool => "on/off".to_string(),
        RowKind::Int | RowKind::Float => "number".to_string(),
        RowKind::Enum(o) => format!("one of: {}", o.join(" / ")),
        RowKind::Secret => "secret (keyring)".to_string(),
        RowKind::Text => "text".to_string(),
        RowKind::ReadOnly => "section".to_string(),
    }
}

/// The selected setting's help: full text wrapped, then a faint line with the action, source and
/// key path last (the path is for people editing files, not the headline).
fn help_lines(r: &SettingRow) -> Vec<TextLine<'static>> {
    let sep = format!(" {} ", g().sep);
    let mut lines = Vec::new();
    if let Some(help) = &r.help {
        lines.push(TextLine::from(Span::styled(help.clone(), styles::fg())));
    }
    let mut facts = vec![kind_label(r)];
    if r.kind != RowKind::Secret && r.kind != RowKind::ReadOnly {
        facts.push(format!(
            "default: {}",
            if r.default.is_empty() {
                "—"
            } else {
                &r.default
            }
        ));
    }
    facts.push(format!(
        "source: {}{}",
        r.source,
        if r.modified { " (modified)" } else { "" }
    ));
    lines.push(TextLine::from(vec![
        Span::styled(facts.join(&sep), styles::muted()),
        Span::styled(format!("{sep}{}", r.path), styles::faint()),
    ]));
    lines
}

/// Render the editor as a full-frame takeover (called last, on top of everything).
pub fn render_config_overlay(frame: &mut Frame, ed: &ConfigEditor) {
    if !ed.open {
        return;
    }
    let area = frame.area();
    if area.height < 8 || area.width < 24 {
        return;
    }
    let matches = ed.matches();
    let body = crate::overlays::padded_panel(
        frame,
        area,
        surface::title("Settings", surface::SurfaceTone::Accent),
        Some(Span::styled(
            format!(" {} settings {} /config ", matches.len(), g().sep),
            styles::faint(),
        )),
        Some(Span::styled(format!(" {} ", hints(ed)), styles::faint())),
        surface::SurfaceTone::Accent,
    );
    if body.width < 20 || body.height < 5 {
        return;
    }
    let width = body.width as usize;

    // Search row with the scope control flush right.
    let search = if ed.filter.is_empty() {
        Span::styled("type to filter", styles::faint())
    } else {
        Span::styled(ed.filter.clone(), styles::user())
    };
    let search_line = right_aligned(
        vec![
            Span::styled("search  ", styles::muted()),
            search,
            Span::styled(g().caret, styles::brand()),
        ],
        scope_spans(ed.project_scope),
        width,
    );

    // Help block: full help wrapped, plus the status of the last save.
    let selected = ed.selected_row();
    let mut help = selected.map(help_lines).unwrap_or_default();
    if let Some(status) = &ed.status {
        let style = if status.starts_with('✓') || status.starts_with(g().ok) {
            styles::ok()
        } else {
            styles::err()
        };
        help.push(TextLine::from(Span::styled(truncate(status, width), style)));
    }
    let help_h = (help.len() as u16 + 2).min(body.height.saturating_sub(4));

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(2),
        Constraint::Length(help_h),
    ])
    .split(body);
    frame.render_widget(Paragraph::new(search_line), rows[0]);

    let sections = ed.sections();
    let current = ed.section_index();
    let two_pane = area.width >= TWO_PANE_MIN_WIDTH;
    let (section_area, list_area) = if two_pane {
        let cols = Layout::horizontal([
            Constraint::Length(SECTION_PANE_WIDTH),
            Constraint::Length(1),
            Constraint::Min(20),
        ])
        .split(rows[2]);
        let rule: Vec<TextLine> = (0..cols[1].height)
            .map(|_| {
                TextLine::from(Span::styled(
                    if g().unicode { "│" } else { "|" },
                    styles::line(),
                ))
            })
            .collect();
        frame.render_widget(Paragraph::new(rule), cols[1]);
        (Some(cols[0]), cols[2])
    } else {
        (None, rows[2])
    };

    // Section list (two panes) or the section header line (one pane).
    let section_name = sections
        .get(current)
        .map(|(n, _)| n.clone())
        .unwrap_or_default();
    let list_area = match section_area {
        Some(sa) => {
            // Window the section list around the current section, with a `… N more` marker.
            let rows_avail = sa.height as usize;
            let window = rows_avail
                .saturating_sub(usize::from(sections.len() > rows_avail))
                .max(1);
            let first = current
                .saturating_sub(window.saturating_sub(1) / 2)
                .min(sections.len().saturating_sub(window));
            let hidden_below = sections.len().saturating_sub(first + window);
            let mut lines: Vec<TextLine> = sections
                .iter()
                .enumerate()
                .skip(first)
                .take(window)
                .map(|(i, (name, count))| {
                    let on = i == current;
                    let caret = if on { g().prompt } else { " " };
                    let left = vec![
                        Span::styled(
                            format!("{caret} "),
                            if on { styles::brand() } else { styles::faint() },
                        ),
                        Span::styled(
                            truncate(name, sa.width.saturating_sub(7) as usize),
                            if on {
                                styles::fg().bold()
                            } else {
                                styles::muted()
                            },
                        ),
                    ];
                    right_aligned(
                        left,
                        vec![Span::styled(count.to_string(), styles::faint())],
                        sa.width.saturating_sub(1) as usize,
                    )
                })
                .collect();
            if hidden_below > 0 || first > 0 {
                let note = match (first, hidden_below) {
                    (0, n) => format!("{n} more"),
                    (n, 0) => format!("{n} above"),
                    (a, b) => format!("{a} above, {b} more"),
                };
                lines.push(TextLine::from(Span::styled(
                    format!("  {} {note}", g().ellipsis),
                    styles::faint(),
                )));
            }
            frame.render_widget(Paragraph::new(lines), sa);
            list_area
        }
        None => {
            let split =
                Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(list_area);
            let header = right_aligned(
                vec![Span::styled(upper(&section_name), styles::muted().bold())],
                vec![Span::styled(
                    format!(
                        "PgUp/PgDn section {}/{}",
                        current + 1,
                        sections.len().max(1)
                    ),
                    styles::faint(),
                )],
                split[0].width as usize,
            );
            frame.render_widget(Paragraph::new(header), split[0]);
            split[1]
        }
    };

    // The current section's rows, windowed around the selection with a `… N more` marker.
    let section_rows: Vec<usize> = matches
        .iter()
        .copied()
        .filter(|&i| ed.rows[i].group == section_name)
        .collect();
    let selected_idx = matches.get(ed.selected).copied();
    let sel_pos = section_rows
        .iter()
        .position(|&i| Some(i) == selected_idx)
        .unwrap_or(0);
    let list_h = list_area.height as usize;
    let truncated = section_rows.len() > list_h && list_h > 1;
    let visible = if truncated { list_h - 1 } else { list_h.max(1) };
    let start = sel_pos
        .saturating_sub(visible.saturating_sub(1))
        .min(section_rows.len().saturating_sub(visible));
    let label_w = (list_area.width as usize / 4).clamp(14, 28);
    let mut lines: Vec<TextLine> = section_rows
        .iter()
        .skip(start)
        .take(visible)
        .map(|&i| {
            setting_line(
                ed,
                &ed.rows[i],
                Some(i) == selected_idx,
                label_w,
                list_area.width as usize,
            )
        })
        .collect();
    if truncated {
        let below = section_rows.len().saturating_sub(start + visible);
        let above = start;
        lines.push(TextLine::from(Span::styled(
            format!(
                "   {} {}",
                g().ellipsis,
                match (above, below) {
                    (0, n) => format!("{n} more"),
                    (n, 0) => format!("{n} above"),
                    (a, b) => format!("{a} above, {b} more"),
                }
            ),
            styles::faint(),
        )));
    }
    if section_rows.is_empty() {
        lines.push(TextLine::from(Span::styled(
            "No settings match.",
            styles::faint(),
        )));
    }
    frame.render_widget(Paragraph::new(lines), list_area);

    // Help block, under the list in the settings column.
    let help_area = Rect {
        x: if two_pane { list_area.x } else { rows[3].x },
        width: if two_pane {
            list_area.width
        } else {
            rows[3].width
        },
        ..rows[3]
    };
    frame.render_widget(Paragraph::new(help).wrap(Wrap { trim: false }), help_area);
}
