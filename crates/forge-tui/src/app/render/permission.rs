//! The decision block: a tool permission or a question that replaces the composer while it waits.
//!
//! A heavy rail (`┃`) runs down the left, the title says exactly what is being asked, a subject
//! row says what would change, and numbered choices list the legacy keys on the right. Permission
//! blocks use the warn rail so they are the only warn-coloured heavy rail on screen; question
//! blocks use the `user` hue so a question never reads as a permission request.

use unicode_width::UnicodeWidthStr;

use super::chrome::{hint, Hint};
use super::list_view::{fit, plain};
use super::*;
use crate::app::state::interaction_ui::{PendingPermission, PERMISSION_CHOICES};
use crate::app::tool_cards::{headline, CardStatus};
use crate::theme::glyphs::g;
use crate::theme::styles;

/// Rows of the shell command band at most; longer commands end in an ellipsis.
const SHELL_ROWS: usize = 5;
const PYTHON_ROWS: usize = 14;

/// A drawn decision block: the rows above its bottom border, and the hints inside that border.
pub(super) struct Decision {
    pub rail: Style,
    pub rows: Vec<TextLine<'static>>,
    pub hints: Vec<Hint>,
}

impl Decision {
    pub fn height(&self) -> u16 {
        self.rows.len() as u16 + 1
    }
}

/// Content width of a block drawn into `width` columns: the leading ` ┃ ` takes three.
pub(super) fn content_width(width: u16) -> usize {
    (width as usize).saturating_sub(3).max(8)
}

/// A title row: `left` and `right` pushed to opposite edges of `width`.
pub(super) fn split_row(
    mut left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
) -> TextLine<'static> {
    let used: usize = left.iter().map(|s| s.content.width()).sum();
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    if right_w > 0 && used + right_w + 2 <= width {
        left.push(Span::raw(" ".repeat(width - used - right_w)));
        left.extend(right);
    }
    TextLine::from(left)
}

/// Greedy word wrap on display width; an over-long word is clipped.
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let w = word.width();
            let cur = line.width();
            if cur == 0 {
                line = fit(word, width);
            } else if cur + 1 + w <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                line = fit(word, width);
            }
        }
        out.push(line);
    }
    while out.len() > 1 && out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// Draw `d` into `area`, anchored to the bottom: rail rows, then the bottom border with the hints.
pub(super) fn draw(frame: &mut Frame, area: Rect, d: &Decision) {
    if area.height == 0 || area.width < 8 {
        return;
    }
    let rows = d.rows.len().min(area.height as usize - 1);
    let mut lines: Vec<TextLine<'static>> = Vec::with_capacity(rows + 1);
    for row in d.rows.iter().take(rows) {
        let mut spans = vec![
            Span::raw(" "),
            Span::styled(g().rail_decision, d.rail),
            Span::raw(" "),
        ];
        spans.extend(row.spans.iter().cloned());
        let mut line = TextLine::from(spans);
        line.style = row.style;
        lines.push(line);
    }
    // Bottom border: ` ╰──────── hints ─`.
    let border = d.rail;
    let hints = super::chrome::fit_hints(d.hints.clone(), (area.width as usize).saturating_sub(10));
    let mut tail: Vec<Span<'static>> = Vec::new();
    if !hints.is_empty() {
        tail.push(Span::raw(" "));
        for (i, h) in hints.iter().enumerate() {
            if i > 0 {
                tail.push(Span::styled(format!(" {} ", g().sep), styles::faint()));
            }
            tail.extend(styles::hint_spans(&h.key, h.label));
        }
        tail.push(Span::raw(" "));
    }
    let tail_w: usize = tail.iter().map(|s| s.content.width()).sum();
    let fill = (area.width as usize).saturating_sub(2 + tail_w + 1);
    let mut bottom = vec![
        Span::raw(" "),
        Span::styled(g().border.bottom_left, border),
        Span::styled(g().border.horizontal_bottom.repeat(fill), border),
    ];
    bottom.extend(tail);
    bottom.push(Span::styled(g().border.horizontal_bottom, border));
    lines.push(TextLine::from(bottom));
    let height = lines.len() as u16;
    frame.render_widget(
        Paragraph::new(lines),
        Rect {
            y: area.bottom().saturating_sub(height),
            height,
            ..area
        },
    );
}

/// The tool and side effect behind the open permission prompt.
fn pending(app: &App) -> PendingPermission {
    if let Some(p) = &app.interaction_ui.permission {
        return p.clone();
    }
    // A prompt armed by another presenter: `allow <tool> (<SideEffect>…) …`.
    let text = app.prompt.as_deref().unwrap_or("");
    let rest = text.strip_prefix("allow ").unwrap_or(text);
    let (tool, tail) = rest.split_once(" (").unwrap_or((rest, ""));
    let side = tail
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("");
    PendingPermission {
        tool: tool.trim().to_string(),
        side_effect: side.to_string(),
        focus: 0,
        note: None,
        trust_hint: None,
    }
}

fn verb(tool: &str) -> String {
    let name = tool
        .strip_prefix("mcp__")
        .and_then(|r| r.split_once("__"))
        .map_or(tool, |(_, t)| t);
    match name {
        "write_file" => "Write".into(),
        "edit_file" => "Edit".into(),
        "apply_patch" => "Patch".into(),
        "shell" => "Shell".into(),
        "web_fetch" => "Web fetch".into(),
        "web_search" => "Web search".into(),
        other => {
            let spaced = other.replace('_', " ");
            let mut chars = spaced.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// `path` relative to the working directory when it is under it, otherwise with `~` for home.
fn short_path(path: &str) -> String {
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(rest) = std::path::Path::new(path).strip_prefix(&cwd) {
            let rest = rest.to_string_lossy();
            if !rest.is_empty() {
                return rest.into_owned();
            }
        }
    }
    tilde(path)
}

fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// The pending call's raw arguments: the newest card of that tool (fullscreen), else the call
/// recorded when it started (inline has no cards).
pub(crate) fn pending_args(app: &App, tool: &str) -> String {
    app.tool_cards
        .iter()
        .rev()
        .find(|c| c.name == tool && c.status == CardStatus::Running)
        .map(|c| c.args.clone())
        .or_else(|| app.interaction_ui.call_args(tool).map(str::to_string))
        .or_else(|| {
            app.tool_cards
                .iter()
                .rev()
                .find(|c| c.name == tool)
                .map(|c| c.args.clone())
        })
        .unwrap_or_default()
}

fn python_cell(args: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(args).ok()?;
    value.get("code")?.as_str().map(str::to_string)
}

fn shell_command(args: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(args).ok()?;
    if let Some(cmd) = value.get("command").and_then(|v| v.as_str()) {
        return Some(cmd.to_string());
    }
    let list = value.get("commands")?.as_array()?;
    Some(
        list.iter()
            .filter_map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// The title: the exact action in words.
fn title(app: &App, p: &PendingPermission, args: &str) -> String {
    let mut target = headline(&p.tool, args);
    if target.is_empty() {
        // Inline has no tool card: the previewed diff still names the file.
        if let Some(d) = &app.pending_diff {
            target = short_path(&d.path);
        }
    }
    let diff_kind = app.pending_diff.as_ref().map(|d| d.kind.as_str());
    match p.tool.as_str() {
        "write_file" => match diff_kind {
            Some("created") => format!("Allow Write to create {target}?"),
            Some("modified") => format!("Allow Write to overwrite {target}?"),
            _ => format!("Allow Write to {target}?"),
        },
        "edit_file" | "apply_patch" => format!("Allow {} of {target}?", verb(&p.tool)),
        "shell" => "Allow Shell to run this command?".to_string(),
        "python" => "Allow Python to run this cell?".to_string(),
        _ if target.is_empty() => format!("Allow {}?", verb(&p.tool)),
        _ => format!("Allow {} {target}?", verb(&p.tool)),
    }
}

/// Subject rows: what changes.
fn subject(app: &App, p: &PendingPermission, args: &str, width: usize) -> Vec<TextLine<'static>> {
    let sep = format!(" {} ", g().sep);
    let command = match p.tool.as_str() {
        "shell" => shell_command(args).map(|c| (c, SHELL_ROWS)),
        "python" => python_cell(args).map(|c| (c, PYTHON_ROWS)),
        _ => None,
    };
    {
        if let Some((cmd, cap)) = command {
            let painted = crate::theme::t().paints_backgrounds();
            let mut rows = if p.tool == "python" {
                cmd.lines()
                    .map(|l| fit(&l.replace('\t', "    "), width.saturating_sub(4)))
                    .collect()
            } else {
                wrap(&plain(&cmd), width.saturating_sub(4))
            };
            if rows.len() > cap {
                rows.truncate(cap);
                if let Some(last) = rows.last_mut() {
                    *last = fit(&format!("{last} {}", g().ellipsis), width.saturating_sub(4));
                }
            }
            return rows
                .into_iter()
                .enumerate()
                .map(|(i, row)| {
                    let prefix = if painted {
                        " ".to_string()
                    } else if i == 0 {
                        if p.tool == "python" { ">>> " } else { "$ " }.to_string()
                    } else {
                        "  ".to_string()
                    };
                    let used = prefix.width() + row.width();
                    let mut spans = vec![
                        Span::styled(prefix, styles::faint()),
                        Span::styled(row, styles::fg()),
                    ];
                    if painted {
                        spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
                    }
                    let mut line = TextLine::from(spans);
                    if painted {
                        line = line.style(styles::surface());
                    }
                    line
                })
                .collect();
        }
    }
    let mut parts: Vec<Span<'static>> = Vec::new();
    let join = |parts: &mut Vec<Span<'static>>, text: String, style: Style| {
        if !parts.is_empty() {
            parts.push(Span::styled(sep.clone(), styles::faint()));
        }
        parts.push(Span::styled(text, style));
    };
    if let Some(d) = &app.pending_diff {
        match d.kind.as_str() {
            "created" => {
                join(&mut parts, "new file".into(), styles::muted());
                let unit = if d.adds == 1 { "line" } else { "lines" };
                join(&mut parts, format!("+{} {unit}", d.adds), styles::ok());
            }
            "deleted" => join(&mut parts, "delete".into(), styles::err()),
            _ => {
                join(&mut parts, "modified".into(), styles::muted());
                join(
                    &mut parts,
                    format!("+{} {}{}", d.adds, g().diff_minus, d.dels),
                    styles::muted(),
                );
            }
        }
        join(&mut parts, tilde(&d.path), styles::user());
    } else {
        let what = match p.side_effect.as_str() {
            "Write" => "changes files in your workspace",
            "Shell" => "runs a command on your machine",
            "Network" => "reaches the network",
            "External" => "calls an external MCP tool",
            _ => "needs your approval",
        };
        join(&mut parts, what.into(), styles::muted());
        let target = headline(&p.tool, args);
        if !target.is_empty() {
            join(
                &mut parts,
                fit(&target, width.saturating_sub(40)),
                styles::user(),
            );
        }
    }
    vec![TextLine::from(parts)]
}

/// One line saying why a prompt for a read-only command exists and how to stop it: the project is
/// not trusted. Shortened in steps as the panel narrows, down to the bare command; never wrapped,
/// so the choices stay put.
fn trust_row(what: &str, width: usize) -> TextLine<'static> {
    let sep = ": ";
    let command = "forge trust";
    let candidates = [
        format!("Trust this project to skip prompts for read-only {what}"),
        "Trust this project to skip these prompts".to_string(),
        "Skip these prompts".to_string(),
        "Skip prompts".to_string(),
    ];
    match candidates
        .iter()
        .find(|c| c.width() + sep.width() + command.width() <= width)
    {
        Some(lead) => TextLine::from(vec![
            Span::styled(format!("{lead}{sep}"), styles::muted()),
            Span::styled(command, styles::key_chip()),
        ]),
        None => TextLine::from(Span::styled(fit(command, width), styles::key_chip())),
    }
}

fn choice_row(
    focused: bool,
    number: usize,
    label: &str,
    keys: &str,
    width: usize,
) -> TextLine<'static> {
    let caret = if focused {
        format!("{} ", g().prompt)
    } else {
        "  ".to_string()
    };
    let label_style = if focused {
        styles::fg().bold()
    } else {
        styles::fg()
    };
    let left_w = 2 + 1 + 2;
    let keys_w = keys.width();
    let label = fit(label, width.saturating_sub(left_w + keys_w + 2));
    let used = left_w + label.width() + keys_w;
    let spans = vec![
        Span::styled(caret, styles::brand().bold()),
        Span::styled(number.to_string(), styles::muted()),
        Span::raw("  "),
        Span::styled(label, label_style),
        Span::raw(" ".repeat(width.saturating_sub(used))),
        Span::styled(keys.to_string(), styles::faint()),
    ];
    let line = TextLine::from(spans);
    if focused {
        line.style(styles::selected_row())
    } else {
        line
    }
}

/// The permission block for `width` columns, unclamped.
pub(super) fn permission_block(app: &App, width: u16) -> Decision {
    let w = content_width(width);
    let p = pending(app);
    let args = pending_args(app, &p.tool);
    let mut rows = Vec::new();
    let temper = app.temper.clone();
    rows.push(split_row(
        vec![Span::styled(
            plain(&title(app, &p, &args)).into_owned(),
            styles::fg().bold(),
        )],
        if temper.is_empty() {
            Vec::new()
        } else {
            vec![Span::styled(
                format!("{temper} mode"),
                styles::temper(&app.temper),
            )]
        },
        w,
    ));
    rows.extend(subject(app, &p, &args, w));
    if let Some(what) = &p.trust_hint {
        rows.push(trust_row(what, w));
    }
    rows.push(TextLine::default());
    let scope = if p.tool.contains("__") {
        "this session"
    } else {
        ".forge/config.toml"
    };
    let long = format!(
        "Always allow {} here ({}{scope})",
        p.tool,
        if scope == ".forge/config.toml" {
            "saved to "
        } else {
            "for "
        }
    );
    let short = format!("Always allow {} here ({scope})", p.tool);
    let always = if long.width() + 12 <= w { long } else { short };
    let session_long = format!("Allow {} for this session (not saved)", p.tool);
    let session = if session_long.width() + 12 <= w {
        session_long
    } else {
        "Allow for this session".to_string()
    };
    let (always, session) = if p.tool == "python" {
        let once = "Allow once (python cells always ask)".to_string();
        (once.clone(), once)
    } else {
        (always, session)
    };
    let labels = [
        ("Allow once".to_string(), "y  Enter"),
        (always, "a"),
        ("Deny".to_string(), "n  Esc"),
        (session, "s"),
    ];
    for (i, (label, keys)) in labels.iter().enumerate() {
        rows.push(choice_row(i == p.focus, i + 1, label, keys, w));
    }
    debug_assert_eq!(labels.len(), PERMISSION_CHOICES.len());
    if let Some(note) = &p.note {
        rows.push(note_row(note, w));
    }
    Decision {
        rail: super::chrome::border_style(app),
        rows,
        hints: choice_hints(app),
    }
}

/// The open note field: what the user typed, sent to the model with the answer Enter gives.
fn note_row(note: &str, width: usize) -> TextLine<'static> {
    let label = "  note  ";
    let room = width.saturating_sub(label.width() + 1);
    let text = if note.is_empty() {
        Span::styled(
            fit("why, or what to do instead (sent to the model)", room),
            styles::faint(),
        )
    } else {
        Span::styled(tail_within(note, room), styles::fg())
    };
    TextLine::from(vec![
        Span::styled(label, styles::muted()),
        text,
        Span::styled(g().caret, styles::brand()),
    ])
}

/// The end of `text` that fits `width` columns, so the cursor end stays visible while typing.
fn tail_within(text: &str, width: usize) -> String {
    let mut used = 0;
    let mut tail: Vec<char> = Vec::new();
    for c in text.chars().rev() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        tail.push(c);
    }
    tail.into_iter().rev().collect()
}

fn choice_hints(app: &App) -> Vec<Hint> {
    if app.permission_note().is_some() {
        return vec![
            hint("Enter", "answer with note"),
            hint("↑↓", "choose"),
            hint("Esc", "drop note"),
        ];
    }
    let mut hints = vec![
        hint("↑↓", "choose"),
        hint("Enter", "confirm"),
        hint("1-4", "pick"),
        hint("Tab", "add note"),
    ];
    // Ctrl+T expands a tool card; inline has none (the diff is already in the scrollback).
    if app.fullscreen {
        hints.push(hint("Ctrl+T", "show diff"));
    }
    hints.push(hint("?", "keys"));
    hints
}

/// How many rows the block may take: the screen minus the header, the status line and a few
/// transcript rows.
fn row_budget(app: &App) -> usize {
    let (_, h) = app.interaction_ui.screen_size();
    (h as usize).saturating_sub(2 + 3).max(5)
}

/// Shrink `d` to `budget` rows total (bottom border included): drop the blank row first, then
/// trim subject rows, never the title or the choices.
fn clamp(mut d: Decision, budget: usize, keep_tail: usize) -> Decision {
    while d.height() as usize > budget && d.rows.len() > keep_tail + 1 {
        let at = d.rows.len() - keep_tail - 1;
        if at < 2 && !d.rows[at].spans.is_empty() {
            break;
        }
        d.rows.remove(at);
    }
    d
}

fn block(app: &App, width: u16) -> Option<Decision> {
    if app.form.is_some() {
        return Some(super::question::question_block(app, width, row_budget(app)));
    }
    app.prompt.as_ref()?;
    let note_rows = usize::from(app.permission_note().is_some());
    Some(clamp(
        permission_block(app, width),
        row_budget(app),
        PERMISSION_CHOICES.len() + note_rows,
    ))
}

/// Height reserved for the active decision block (zero when nothing waits).
pub(crate) fn prompt_height(app: &App) -> u16 {
    let (w, _) = app.interaction_ui.screen_size();
    block(app, w).map_or(0, |d| d.height())
}

pub(crate) fn render_permission(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(d) = block(app, area.width) {
        draw(frame, area, &d);
    }
}

/// Rows the decision block needs; zero when nothing is waiting for the user.
pub(crate) fn block_height(app: &App) -> u16 {
    prompt_height(app)
}

/// Whether the decision block takes the composer's place while it is open.
pub(crate) fn replaces_composer(app: &App) -> bool {
    app.prompt.is_some() || app.form.is_some()
}

/// Composer hints while a permission prompt or a question waits. `None` when neither does.
pub(crate) fn hints(app: &App) -> Option<Vec<Hint>> {
    if app.form.is_some() {
        return Some(super::question::question_hints(app));
    }
    if app.prompt.is_some() {
        return Some(choice_hints(app));
    }
    None
}
