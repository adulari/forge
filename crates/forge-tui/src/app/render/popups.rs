use super::*;
use crate::theme::{color, styles};

/// Centered, searchable entry point for the complete command and skill catalogue. It is rendered
/// over the chat rather than replacing it, preserving users' place in the current conversation.
pub(crate) fn render_command_center(frame: &mut Frame, area: Rect, app: &App) {
    if !app.command_center.open || area.width < 28 || area.height < 8 {
        return;
    }

    let box_area = surface::modal_area(area, 92, 18);
    let inner = surface::render_panel(
        frame,
        box_area,
        surface::title("Command center", surface::SurfaceTone::Accent),
        None,
        surface::SurfaceTone::Accent,
    );
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let query = if app.command_center.query.is_empty() {
        "Search commands, skills, settings...".to_string()
    } else {
        app.command_center.query.clone()
    };
    let query_style = if app.command_center.query.is_empty() {
        styles::muted()
    } else {
        styles::fg()
    };
    frame.render_widget(
        Paragraph::new(TextLine::from(vec![
            Span::styled("  > ", styles::brand().bold()),
            Span::styled(query, query_style),
        ])),
        rows[0],
    );

    let entries = app.command_center.matches(&app.palette.extra);
    let visible = rows[1].height as usize;
    let start = app
        .command_center
        .selected
        .saturating_sub(visible.saturating_sub(1));
    let mut previous_category: Option<&str> = None;
    let mut lines = Vec::with_capacity(visible);
    for (index, entry) in entries.iter().enumerate().skip(start).take(visible) {
        if previous_category != Some(entry.category) {
            if lines.len() >= visible {
                break;
            }
            lines.push(TextLine::from(Span::styled(
                format!("  {}", entry.category),
                styles::muted().add_modifier(Modifier::BOLD),
            )));
            previous_category = Some(entry.category);
        }
        if lines.len() >= visible {
            break;
        }
        let selected = index == app.command_center.selected;
        let marker = if selected { "› " } else { "  " };
        let command_style = if selected {
            styles::selected_row().bold()
        } else {
            styles::muted()
        };
        let desc_style = if selected {
            styles::selected_row()
        } else {
            styles::muted()
        };
        lines.push(TextLine::from(vec![
            Span::styled(format!("{marker}/{:<18}", entry.name), command_style),
            Span::styled(entry.desc.clone(), desc_style),
        ]));
    }
    if lines.is_empty() {
        lines.push(TextLine::from(Span::styled(
            "  No commands or skills match.",
            styles::muted(),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), rows[1]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            "↑/↓ select  Enter open  Esc close",
            styles::muted(),
        )),
        rows[2],
    );
}

/// The inline slash-command palette: a scrolling window of filtered commands, selected row
/// highlighted, revealed by an ease-in animation (RFC session-management-and-commands).
pub(crate) fn render_palette(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return; // degenerate viewport (e.g. 0-height terminal) — nothing to draw, never clamp(1,0).
    }
    let matches = app.palette.matches();
    if matches.is_empty() {
        frame.render_widget(
            Paragraph::new(TextLine::from(Span::styled(
                "  no commands match",
                styles::muted(),
            ))),
            area,
        );
        return;
    }
    let h = area.height as usize;
    // Ease-in reveal: rows appear over the first few frames after opening.
    let revealed = ((app.palette.anim * h as f32).ceil() as usize).clamp(1, h);
    // Scroll so the selected row stays visible within the window.
    let start = app.palette.selected.saturating_sub(h.saturating_sub(1));
    let lines: Vec<TextLine> = matches
        .iter()
        .enumerate()
        .skip(start)
        .take(revealed)
        .map(|(i, c)| {
            let selected = i == app.palette.selected;
            let marker = if selected { "▸ " } else { "  " };
            let name_style = if selected {
                styles::user().bold()
            } else {
                styles::user()
            };
            let mut spans = vec![
                Span::styled(format!("  {marker}/{}", c.name), name_style),
                Span::styled(format!("  {}", c.desc), styles::muted()),
            ];
            // Inline usage/arg hint on the highlighted row so non-obvious args (`/assay`,
            // `/replay`, `/model`, and fixed-enum args like `/effort [low|…]`) are discoverable.
            if selected && !c.usage.is_empty() {
                spans.push(Span::styled(format!("   {}", c.usage), styles::muted()));
                // Best-effort enum-value completion candidates for fixed-arg commands.
                let values = crate::commands::arg_values(&c.name);
                if !values.is_empty() {
                    spans.push(Span::styled(
                        format!("   ⇥ {}", values.join(" · ")),
                        styles::muted(),
                    ));
                }
            }
            TextLine::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// The `@path` file-path picker: a scrolling, filter-narrowed list of files, revealed by the
/// same ease-in animation as the command palette.
pub(crate) fn render_at_path_picker(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let matches = app.at_picker.matches();
    if matches.is_empty() {
        frame.render_widget(
            Paragraph::new(TextLine::from(Span::styled(
                "  no files match",
                styles::muted(),
            ))),
            area,
        );
        return;
    }
    let h = area.height as usize;
    let revealed = ((app.at_picker.anim * h as f32).ceil() as usize).clamp(1, h);
    let start = app.at_picker.selected.saturating_sub(h.saturating_sub(1));
    let lines: Vec<TextLine> = matches
        .iter()
        .enumerate()
        .skip(start)
        .take(revealed)
        .map(|(i, path)| {
            let selected = i == app.at_picker.selected;
            let marker = if selected { "▸ " } else { "  " };
            let style = if selected {
                styles::muted().bold()
            } else {
                styles::user()
            };
            TextLine::from(Span::styled(format!("  {marker}@{path}"), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// The interactive session/checkpoint picker: a heading + a scrolling, filter-narrowed window of
/// rows, the selected one highlighted, revealed by the same ease-in as the palette. Constrained
/// to the (fixed-height) inline live region, so it scrolls rather than growing.
pub(crate) fn render_picker(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return; // degenerate viewport — never clamp(1, 0).
    }
    let p = &app.picker;
    let matches = p.matches();
    let h = area.height as usize;
    let mut lines: Vec<TextLine> = Vec::with_capacity(h);

    // Heading: title · live filter (or hint) · position.
    let mut head = vec![Span::styled(
        format!("  {} ", p.heading),
        styles::user().bold(),
    )];
    if p.query.is_empty() {
        head.push(Span::styled("(type to filter)", styles::muted()));
    } else {
        head.push(Span::styled(format!("/{}", p.query), styles::user()));
    }
    if !matches.is_empty() {
        head.push(Span::styled(
            format!("  {}/{}", p.selected + 1, matches.len()),
            styles::muted(),
        ));
    }
    lines.push(TextLine::from(head));

    if matches.is_empty() {
        lines.push(TextLine::from(Span::styled(
            "  no matches",
            styles::muted(),
        )));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let list_h = h.saturating_sub(1); // rows below the heading
    let revealed = ((p.anim * list_h as f32).ceil() as usize).clamp(1, list_h.max(1));
    let start = p.selected.saturating_sub(list_h.saturating_sub(1));
    let tempers = p.kind == Some(crate::commands::PickerKind::Tempers);
    let models = p.kind == Some(crate::commands::PickerKind::Models);
    let model_pin = p.kind == Some(crate::commands::PickerKind::ModelPin);
    let tick = app.tick;
    for (i, row) in matches.iter().enumerate().skip(start).take(revealed) {
        let selected = i == p.selected;
        let marker = if selected { "▸ " } else { "  " };
        // Color rows by kind: tempers by posture, models browser by category, model-pin picker
        // by tier with the "mesh" row animated (cycles accent colors).
        let base = if tempers {
            styles::temper_color(&row.title)
        } else if models {
            models_row_color(row)
        } else if model_pin {
            model_pin_row_color(row, tick)
        } else {
            color::user()
        };
        let title_style = if selected {
            Style::default().fg(base).bold()
        } else {
            Style::default().fg(base)
        };
        // ModelPin: add a tier badge between title and subtitle for at-a-glance scanning.
        let subtitle_str = if model_pin {
            truncate(&row.subtitle, 52)
        } else {
            truncate(&row.subtitle, 44)
        };
        lines.push(TextLine::from(vec![
            Span::styled(format!("  {marker}{}", row.title), title_style),
            Span::styled(format!("  {subtitle_str}"), styles::muted()),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), area);
}
/// Colour a `/models` browser row: provider rows blue; model rows by category (subscription=green,
/// frontier=brand, free=blue, paid=yellow). Provider vs model is told apart by the `::` in `id`.
pub(crate) fn models_row_color(row: &crate::commands::PickerRow) -> Color {
    if !row.id.contains("::") {
        return color::user(); // a provider header row
    }
    let s = row.subtitle.to_lowercase();
    if s.contains("subscription") {
        color::ok()
    } else if s.contains("frontier") {
        color::brand()
    } else if s.contains("free") {
        color::user()
    } else {
        color::warn() // paid
    }
}

/// Colour a `/model` pin-picker row: "mesh" cycles through the categorical colours;
/// subscription=green, frontier=brand, free=blue, paid=yellow, benched=faint.
pub(crate) fn model_pin_row_color(row: &crate::commands::PickerRow, tick: usize) -> Color {
    if row.id == "mesh" {
        let cat = crate::theme::t().palette.cat;
        let step = if crate::theme::t().animates() {
            tick / 8
        } else {
            0
        };
        return cat[step % cat.len()].fg;
    }
    let s = row.subtitle.to_lowercase();
    if s.contains("subscription") {
        color::ok()
    } else if s.contains("frontier") {
        color::brand()
    } else if s.contains("free") {
        color::user()
    } else if s.contains("benched") {
        color::fg_faint()
    } else {
        color::warn() // paid
    }
}

/// Composer hints while a popup owns the keyboard. `None` when no popup is open.
pub(crate) fn hints(app: &App) -> Option<Vec<super::chrome::Hint>> {
    use super::chrome::hint;
    if app.command_center.open || app.picker.open {
        return Some(vec![
            hint("↑↓", "move"),
            hint("Enter", "select"),
            hint("type", "to filter"),
            hint("Esc", "close"),
            hint("?", "keys"),
        ]);
    }
    if app.palette.open || app.at_picker.open {
        let verb = if app.palette.open { "run" } else { "insert" };
        return Some(vec![
            hint("↑↓", "move"),
            hint("Enter", verb),
            hint("Tab", "complete"),
            hint("Esc", "close"),
            hint("?", "keys"),
        ]);
    }
    None
}
