//! The composer's completion popups (slash palette, `@file` picker) and the modal lists (command
//! center, session/model/checkpoint/mode pickers). All of them are views over one list component
//! (`list_view`): the same frame, caret, match highlight, key hints and detail line.

use super::chrome::{hint, Hint};
use super::list_view::{self, match_positions, Filter, Item, Row, View};
use super::*;
use crate::commands::{command_action, COMMANDS, COMMAND_GROUPS};
use crate::keybinds::combo_display;
use crate::picker::PickerKind;
use crate::theme::glyphs::g;
use crate::theme::{color, styles};

/// List rows a completion popup shows before it scrolls (its detail line comes on top).
const POPUP_LIST_ROWS: usize = 8;

/// Marker the CLI appends to a skill's description so the palette can tag it.
const SKILL_SUFFIX: &str = "(skill)";

fn key_for(app: &App, name: &str) -> String {
    command_action(name)
        .and_then(|action| app.keybinds.binds.get(action))
        .map(combo_display)
        .unwrap_or_default()
}

/// A palette/command-center row: `/name`, one-line description, shortcut or `skill`/`command` tag.
fn command_row(app: &App, name: &str, desc: &str, query: &str) -> Row {
    let builtin = COMMANDS.iter().any(|c| c.name == name);
    let trimmed = desc.trim_end();
    let is_skill = trimmed.ends_with(SKILL_SUFFIX);
    let desc = if is_skill {
        trimmed
            .trim_end_matches(SKILL_SUFFIX)
            .trim_end()
            .to_string()
    } else {
        desc.to_string()
    };
    let meta = if builtin {
        key_for(app, name)
    } else if is_skill {
        "skill".to_string()
    } else {
        "command".to_string()
    };
    Row {
        label: format!("/{name}"),
        label_style: Some(styles::user()),
        desc,
        meta,
        marks: match_positions(name, query)
            .into_iter()
            .map(|i| i + 1)
            .collect(),
        ..Row::default()
    }
}

/// A description without the CLI's `(skill)` marker (the row shows it as a tag instead).
fn clean_desc(desc: &str) -> &str {
    let trimmed = desc.trim_end();
    trimmed
        .strip_suffix(SKILL_SUFFIX)
        .map_or(desc, |rest| rest.trim_end())
}

fn usage_detail(usage: &str, desc: &str) -> Option<Vec<Span<'static>>> {
    let desc = clean_desc(desc);
    if !usage.is_empty() {
        return Some(vec![
            Span::styled("usage  ", styles::muted()),
            Span::styled(usage.to_string(), styles::fg()),
        ]);
    }
    (!desc.is_empty()).then(|| vec![Span::styled(desc.to_string(), styles::muted())])
}

fn palette_view(app: &App) -> View {
    let entries = app.palette.matches();
    let recents = app.palette.recent_count();
    let n = entries.len();
    let detail = entries
        .get(app.palette.selected)
        .and_then(|e| usage_detail(&e.usage, &e.desc));
    let query = app.palette.query.as_str();
    View {
        title: if query.is_empty() {
            "Commands".to_string()
        } else {
            format!("/{query}")
        },
        context: match (query.is_empty(), recents) {
            (true, 0) => format!("{n}"),
            (true, _) => format!("{n} {} recent first", g().sep),
            (false, _) => format!("{n} match{}", if n == 1 { "" } else { "es" }),
        },
        items: entries
            .iter()
            .map(|e| Item::Row(command_row(app, &e.name, &e.desc, query)))
            .collect(),
        selected: app.palette.selected,
        detail,
        empty: "no commands match".to_string(),
        ..View::default()
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        "item"
    } else {
        "items"
    }
}

fn at_view(app: &App) -> View {
    let entries = app.at_picker.entries();
    let query = app.at_picker.query.as_str();
    let items = entries
        .iter()
        .map(|e| {
            let meta = if e.modified {
                if e.is_dir {
                    format!("modified {} {} {}", g().sep, e.items, plural(e.items))
                } else {
                    "modified".to_string()
                }
            } else if e.is_dir {
                format!("dir {} {} {}", g().sep, e.items, plural(e.items))
            } else {
                String::new()
            };
            let base = e.path.trim_end_matches('/');
            let from = base.len() - base.rsplit('/').next().map_or(0, str::len);
            let name = &e.path[from..];
            let marks = match_positions(name, query.rsplit('/').next().unwrap_or(query))
                .into_iter()
                .map(|i| i + base[..from].chars().count())
                .collect();
            Item::Row(Row {
                label: e.path.clone(),
                label_style: Some(if e.is_dir {
                    styles::fg()
                } else {
                    styles::user()
                }),
                meta,
                meta_style: e.modified.then(styles::warn),
                marks,
                ..Row::default()
            })
        })
        .collect::<Vec<_>>();
    let n = items.len();
    View {
        title: "@ files".to_string(),
        context: if query.contains('/') || query.is_empty() {
            format!("{n}")
        } else {
            format!("{n} {} fuzzy", g().sep)
        },
        items,
        selected: app.at_picker.selected,
        empty: "no files match".to_string(),
        more_hint: "type to narrow",
        ..View::default()
    }
}

fn center_view(app: &App) -> View {
    let entries = app.command_center.matches(&app.palette.extra);
    let query = app.command_center.query.as_str();
    let mut items = Vec::with_capacity(entries.len() + 8);
    let mut last: Option<&str> = None;
    for e in &entries {
        if query.is_empty() && last != Some(e.category) {
            items.push(Item::Header(e.category.to_string()));
            last = Some(e.category);
        }
        items.push(Item::Row(command_row(app, &e.name, &e.desc, query)));
    }
    let detail = entries
        .get(app.command_center.selected)
        .and_then(|e| usage_detail(&e.usage, &e.desc));
    let open_key = app
        .keybinds
        .binds
        .get("command_center")
        .map(combo_display)
        .unwrap_or_default();
    View {
        title: "Command center".to_string(),
        context: open_key,
        filter: Some(Filter {
            label: "search",
            query: query.to_string(),
            placeholder: "commands, skills, settings",
        }),
        tabs: Some((
            COMMAND_GROUPS.iter().map(|g| g.to_string()).collect(),
            app.command_center.group,
        )),
        items,
        selected: app.command_center.selected,
        detail,
        footer: vec![
            hint("↑↓", "move"),
            hint("Tab", "group"),
            hint("Enter", "open"),
            hint("Esc", "close"),
        ],
        empty: "No commands or skills match.".to_string(),
        padded: true,
        ..View::default()
    }
}

/// What Enter does on the selected row of a picker, shown on the detail line.
fn picker_enter_text(kind: Option<PickerKind>) -> &'static str {
    match kind {
        Some(PickerKind::Sessions) => "Enter resume this session",
        Some(PickerKind::Checkpoints) => "Enter choose what to restore from this point",
        Some(PickerKind::RewindAction) => "Enter apply, 1-5 pick, type to guide a summary",
        Some(PickerKind::Tempers) => "Enter switch to this mode",
        Some(PickerKind::Models) => "Enter open this provider or pin this model",
        Some(PickerKind::ModelPin) | Some(PickerKind::ModelRung) => "Enter pin for this session",
        Some(PickerKind::CopyBlocks) => "Enter copy to the clipboard, w write to a file",
        Some(PickerKind::Duel) => "Enter merge this candidate, Esc discard all",
        Some(PickerKind::ControlTools) => "Enter turn this tool on or off",
        _ => "Enter select",
    }
}

fn picker_view(app: &App) -> View {
    let p = &app.picker;
    let matches = p.matches();
    let tempers = p.kind == Some(PickerKind::Tempers);
    let models = p.kind == Some(PickerKind::Models);
    let model_pin = p.kind == Some(PickerKind::ModelPin);
    let items: Vec<Item> = matches
        .iter()
        .map(|row| {
            let style = if tempers {
                styles::temper(&row.title)
            } else if models {
                Style::default().fg(models_row_color(row))
            } else if model_pin {
                Style::default().fg(model_pin_row_color(row, app.tick))
            } else {
                styles::fg()
            };
            // A trailing `● current` marker is metadata: it belongs at the right edge.
            let (desc, meta) = match row.subtitle.rsplit_once("   ● ") {
                Some((desc, tag)) => (desc.to_string(), format!("● {tag}")),
                None => (row.subtitle.clone(), String::new()),
            };
            Item::Row(Row {
                label: row.title.clone(),
                label_style: Some(style),
                desc,
                meta,
                meta_style: Some(styles::ok()),
                marks: match_positions(&row.title, &p.query),
            })
        })
        .collect();
    let detail = matches.get(p.selected).map(|row| {
        let mut spans = vec![Span::styled(picker_enter_text(p.kind), styles::muted())];
        if !row.subtitle.is_empty() && row.subtitle.chars().count() > 60 {
            spans = vec![Span::styled(row.subtitle.clone(), styles::muted())];
        }
        spans
    });
    let mut footer = vec![hint("↑↓", "move"), hint("Enter", "select")];
    if p.kind == Some(PickerKind::Sessions) {
        footer.insert(0, hint("Tab", "archived"));
    }
    if p.kind == Some(PickerKind::Sessions) {
        footer.insert(1, hint("Del", "archive"));
    }
    if p.kind == Some(PickerKind::Sessions) {
        footer.insert(2, hint("^W", "delete"));
    }
    footer.push(hint(
        "Esc",
        if p.kind == Some(PickerKind::RewindAction) {
            "back"
        } else {
            "close"
        },
    ));
    // Headings carry their own key hints (`resume a session · Tab archived · …`); the frame's
    // footer shows those, so the title keeps only its first part.
    let title = p
        .heading
        .split(" · ")
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    View {
        title,
        context: if matches.is_empty() {
            "0".to_string()
        } else {
            format!("{} of {}", p.selected + 1, matches.len())
        },
        filter: Some(if p.kind == Some(PickerKind::RewindAction) {
            Filter {
                label: "focus",
                query: p.query.clone(),
                placeholder: "optional: what a summary should keep",
            }
        } else {
            Filter {
                label: "filter",
                query: p.query.clone(),
                placeholder: "type to filter",
            }
        }),
        items,
        selected: p.selected,
        detail,
        footer,
        empty: "no matches".to_string(),
        padded: true,
        ..View::default()
    }
}

/// Rows the completion popup above the composer takes out of `main`: its content, capped, and
/// never more than the region leaves after a few transcript rows.
pub(crate) fn attached_height(app: &App, main: Rect) -> u16 {
    let view = if app.palette.open {
        palette_view(app)
    } else {
        at_view(app)
    };
    let detail_rows = if view.detail.is_some() { 2 } else { 0 };
    let want = view.desired_height(POPUP_LIST_ROWS + detail_rows);
    let keep = if app.fullscreen { 3 } else { 2 };
    want.min(main.height.saturating_sub(keep).max(main.height.min(4)))
        .min(main.height)
}

/// Width of a centered modal: the whole row on a narrow terminal (so no chat shows through at
/// its left edge), otherwise at most 100 cells with a margin.
fn modal_width(total: u16) -> u16 {
    if total <= 90 {
        total
    } else {
        (total - 2).min(100)
    }
}

fn modal_rect(area: Rect, preferred_height: u16) -> Rect {
    let width = modal_width(area.width);
    let height = preferred_height.min(area.height.saturating_sub(1)).max(1);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

/// The command center: a centered modal over the dimmed chat, searchable, with group tabs.
pub(crate) fn render_command_center(frame: &mut Frame, area: Rect, app: &App) {
    if !app.command_center.open || area.width < 28 || area.height < 8 {
        return;
    }
    surface::dim_backdrop(frame, area);
    let box_area = modal_rect(area, 24);
    list_view::render(frame, box_area, &center_view(app));
}

/// The slash-command palette: a framed list attached to the composer.
pub(crate) fn render_palette(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    list_view::render(frame, area, &palette_view(app));
}

/// The `@path` picker: a framed list attached to the composer.
pub(crate) fn render_at_path_picker(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    list_view::render(frame, area, &at_view(app));
}

/// The session/checkpoint/model/mode picker: a centered modal over the dimmed chat. Constrained
/// to the (fixed-height) inline live region, so it scrolls rather than growing.
pub(crate) fn render_picker(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return; // degenerate viewport: nothing to draw, never clamp(1, 0).
    }
    let view = picker_view(app);
    let width = modal_width(area.width);
    let height = view.desired_height(usize::MAX / 2).max(12).min(area.height);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 3,
        width,
        height,
    };
    list_view::render(frame, rect, &view);
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
pub(crate) fn hints(app: &App) -> Option<Vec<Hint>> {
    if app.command_center.open {
        return Some(vec![
            hint("↑↓", "move"),
            hint("Enter", "open"),
            hint("Esc", "close"),
            hint("?", "keys"),
        ]);
    }
    if app.picker.open {
        return Some(vec![
            hint("↑↓", "move"),
            hint("Enter", "select"),
            hint("type", "to filter"),
            hint("Esc", "close"),
            hint("?", "keys"),
        ]);
    }
    if app.palette.open {
        return Some(vec![
            hint("↑↓", "move"),
            hint("Enter", "run"),
            hint("Tab", "complete"),
            hint("Esc", "close"),
            hint("?", "keys"),
        ]);
    }
    if app.at_picker.open {
        return Some(vec![
            hint("↑↓", "move"),
            hint("Enter", "insert"),
            hint("Tab", "into dir"),
            hint("Esc", "close"),
            hint("?", "keys"),
        ]);
    }
    None
}
