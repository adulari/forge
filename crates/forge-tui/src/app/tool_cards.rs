//! Tool cards: one row per call that owns both halves — the call on the left, its outcome on
//! the right — in BOTH display modes.
//!
//! `✓ Write  src/lib.rs                                   +18 −3 · 0.2s ▸`
//!
//! The verb is humanised (`read_file` → Read), the target is decoded from the call's JSON (never
//! the raw envelope), the outcome and duration sit flush right. A card that changed a file carries
//! its diff in the card: up to eight rows with a line-number gutter, the rest behind Ctrl+T. A
//! failed call shows the last lines of its output without being asked. Consecutive reads fold into
//! one `◆ Read N files` card.
//!
//! In full-screen the card lives in `App::main_log` and is redrawn in place — clicking it (or
//! Ctrl+T for the newest) expands the full arguments and output, and clicking again collapses it.
//! Inline mode prints into the terminal's own scrollback, which cannot be rewritten, so the card
//! is printed ONCE, finished, when its result lands; the running call is shown only in the
//! composer border.

pub(crate) use super::card_text::{
    cells, display_path, group, oneline, shorten, shorten_in, size_label, truncate_cells,
    wrap_plain,
};
use super::lines::rail;
use crate::copy_mark;
use crate::theme::glyphs::g;
use crate::theme::styles::{self, RailKind};
use forge_types::FileDiff;
use ratatui::style::Style;
use ratatui::text::{Line as TextLine, Span};
use serde_json::Value;
use std::time::Instant;

/// Diff rows a collapsed card shows before folding the rest behind Ctrl+T.
const PREVIEW_DIFF_ROWS: usize = 8;
/// Output lines a failed, collapsed card shows without being asked.
const FAILURE_TAIL_LINES: usize = 3;
/// User-turn rows shown before the rest folds.
pub(crate) const USER_FOLD_ROWS: usize = 6;
/// Width of the verb column (the longest verb is six cells).
pub(crate) const VERB_W: usize = 6;

/// How a card's call ended. `Running` is the state between `ToolStart` and `ToolResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardStatus {
    Running,
    Ok,
    Failed,
}

/// An earlier call folded into a group card (`◆ Read 4 files`).
#[derive(Debug, Clone)]
pub(crate) struct Member {
    pub(crate) name: String,
    pub(crate) args: String,
    pub(crate) summary: String,
    pub(crate) ms: Option<u128>,
}

/// A long user turn folded to a few rows (full-screen only; Ctrl+T or a click expands it).
#[derive(Debug, Clone)]
pub(crate) struct UserFold {
    /// A canned command prompt shows this label instead of its text.
    pub(crate) label: Option<String>,
    /// The turn's text, wrapped, one entry per row.
    pub(crate) rows: Vec<String>,
}

/// One tool call in the transcript, expandable in place.
#[derive(Debug, Clone)]
pub(crate) struct ToolCard {
    /// Monotonic identity. Cards are looked up by id, never by index: the log ring drops the
    /// oldest cards as it trims, so an index captured earlier in a turn can point at a
    /// different card by the time the result arrives.
    pub(crate) id: u64,
    pub(crate) name: String,
    /// The raw args JSON exactly as the model emitted it.
    pub(crate) args: String,
    pub(crate) status: CardStatus,
    /// The result's first line.
    pub(crate) summary: String,
    /// Bounded raw output, when the emitter had any to hand over.
    pub(crate) detail: Option<String>,
    /// Where the call's complete output was kept, when the preview could not hold all of it.
    pub(crate) full: Option<forge_types::ToolOutputRef>,
    pub(crate) expanded: bool,
    /// Where this card's rendered lines currently sit in `App::main_log`.
    pub(crate) start: usize,
    pub(crate) len: usize,
    /// When the call began, and how long it took once it ended.
    pub(crate) started: Instant,
    pub(crate) ms: Option<u128>,
    /// The file change this call made (or is about to make), shown inside the card. Set it with
    /// [`ToolCard::set_diff`], which also counts it.
    pub(crate) diff: Option<FileDiff>,
    /// `(adds, dels)` of `diff`, counted once when it attached: the diff holds both whole files,
    /// so counting it again on every redraw or summary costs a full-file diff each time.
    pub(crate) diff_counts: (usize, usize),
    /// Earlier same-verb calls folded into this card; the fields above are the newest.
    pub(crate) group: Vec<Member>,
    /// Set on a card that stands for a folded user turn instead of a tool call.
    pub(crate) fold: Option<UserFold>,
    /// Whether the card can be expanded (full-screen). Inline cards are printed once.
    pub(crate) interactive: bool,
    /// The tool group this call belongs to (the id of the group's first card), full-screen only.
    pub(crate) batch: Option<u64>,
    /// The `run_code` call this call ran inside, full-screen only.
    pub(crate) parent: Option<u64>,
    /// Why a failed shell command failed, from the shell-error interceptor, shown under the card
    /// instead of as a notice repeating the command.
    pub(crate) note: Option<CardNote>,
}

/// A diagnosis attached to a failed call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CardNote {
    pub(crate) diagnosis: String,
    pub(crate) fix: Option<String>,
}

impl ToolCard {
    pub(crate) fn new(id: u64, name: String, args: String) -> Self {
        Self {
            id,
            name,
            args,
            status: CardStatus::Running,
            summary: String::new(),
            detail: None,
            full: None,
            expanded: false,
            start: 0,
            len: 0,
            started: Instant::now(),
            ms: None,
            diff: None,
            diff_counts: (0, 0),
            group: Vec::new(),
            fold: None,
            interactive: true,
            batch: None,
            parent: None,
            note: None,
        }
    }

    /// A card standing for a folded user turn.
    pub(crate) fn user_fold(id: u64, fold: UserFold) -> Self {
        let mut card = Self::new(id, "you".to_string(), String::new());
        card.status = CardStatus::Ok;
        card.fold = Some(fold);
        card
    }

    /// Whether opening this card would reveal anything the collapsed row does not already show.
    /// A call with no arguments and no output stays a plain row rather than offering an empty
    /// drawer — the affordance must never lie about having something behind it.
    pub(crate) fn has_detail(&self) -> bool {
        self.interactive
            && (self.fold.is_some()
                || self.diff.is_some()
                || !self.group.is_empty()
                || self.detail.is_some()
                || self.full.is_some()
                || !arg_rows(&self.args).is_empty())
    }

    /// Give the card the file change its call makes, counted here once.
    pub(crate) fn set_diff(&mut self, diff: FileDiff) {
        self.diff_counts = crate::render::diff_stat(&diff);
        self.diff = Some(diff);
    }

    /// Fold the finished call this card holds into its group and start over with a new call.
    pub(crate) fn fold_in(&mut self, name: String, args: String) {
        self.group.push(Member {
            name: std::mem::replace(&mut self.name, name),
            args: std::mem::replace(&mut self.args, args),
            summary: std::mem::take(&mut self.summary),
            ms: self.ms.take(),
        });
        self.status = CardStatus::Running;
        self.detail = None;
        self.full = None;
        self.diff = None;
        self.diff_counts = (0, 0);
        self.started = Instant::now();
    }
}

/// A bridged turn names Forge's own tools `mcp__forge__<tool>`. Treat those as the plain tool: the
/// card category, headline and permission prompt all key on the bare name, and with the namespace
/// left on, a bridged edit prompted "Allow Edit of ?" with no file and its card read as
/// `Called forge 2 times`. Any other server's tools keep their `mcp__<server>__` name.
pub(crate) fn bare_tool_name(name: &str) -> &str {
    name.strip_prefix("mcp__forge__").unwrap_or(name)
}

/// `read_file` → `Read`. An unknown tool shows its own name.
pub(crate) fn verb(name: &str) -> String {
    let name = crate::app::tool_display_name(name);
    match name {
        "read_file" => "Read",
        "write_file" => "Write",
        "edit_file" => "Edit",
        "apply_patch" => "Patch",
        "shell" => "Run",
        "grep" | "search" => "Search",
        "glob" => "Find",
        "list_dir" => "List",
        "web_fetch" => "Fetch",
        "web_search" => "Web",
        "spawn_agents" => "Agents",
        "submit_result" => "Result",
        "run_code" => "Script",
        other => return other.to_string(),
    }
    .to_string()
}

/// `4ms`, `0.2s`, `41s`, `2m05s`.
pub(crate) fn fmt_ms(ms: u128) -> String {
    match ms {
        0..=999 => format!("{ms}ms"),
        1_000..=9_999 => format!("{:.1}s", ms as f64 / 1000.0),
        10_000..=59_999 => format!("{}s", ms / 1000),
        _ => format!("{}m{:02}s", ms / 60_000, (ms % 60_000) / 1000),
    }
}

/// The result summary without the tool's own name in front and without a trailing
/// ` in 132ms` (the card measures the duration itself).
fn clean_summary(card_name: &str, summary: &str) -> String {
    let s = summary
        .strip_prefix(&format!("{card_name}: "))
        .unwrap_or(summary)
        .trim();
    if let Some(at) = s.rfind(" in ") {
        let tail = &s[at + 4..];
        let digits = tail.trim_end_matches(['m', 's']);
        if !digits.is_empty()
            && tail.ends_with('s')
            && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        {
            return s[..at].to_string();
        }
    }
    s.to_string()
}

/// The outcome alone: what the call did, in a few words.
pub(crate) fn outcome_text(card: &ToolCard) -> String {
    match card.status {
        CardStatus::Running => "running".to_string(),
        // A script's summary is the first line of whatever it returned (often an inner call's
        // `shell: exit 0`), which reads as this call's outcome when it is not.
        CardStatus::Ok if card.name == "run_code" => "done".to_string(),
        _ => {
            if let Some(diff) = &card.diff {
                return diff_outcome(diff, card.diff_counts);
            }
            let s = clean_summary(&card.name, &card.summary);
            if s.is_empty() {
                if card.status == CardStatus::Ok {
                    "done".to_string()
                } else {
                    "failed".to_string()
                }
            } else {
                truncate_cells(&s, 40)
            }
        }
    }
}

fn diff_outcome(diff: &FileDiff, (adds, dels): (usize, usize)) -> String {
    let minus = g().diff_minus;
    let stat = match (adds, dels) {
        (a, 0) => format!("+{a}"),
        (0, d) => format!("{minus}{d}"),
        (a, d) => format!("+{a} {minus}{d}"),
    };
    match diff.kind {
        forge_types::DiffKind::Created => format!("new file {} {stat}", g().sep),
        forge_types::DiffKind::Deleted => format!("deleted {} {stat}", g().sep),
        forge_types::DiffKind::Modified => stat,
    }
}

/// The right-hand text of the header row: outcome, kept-output size, duration.
pub(crate) fn right_text(card: &ToolCard) -> String {
    let sep = format!(" {} ", g().sep);
    let mut parts = vec![outcome_text(card)];
    if card.status != CardStatus::Running {
        if let Some(full) = &card.full {
            parts.push(format!("{} lines", group(full.lines)));
        }
        let ms = if card.group.is_empty() {
            card.ms
        } else {
            let total: u128 =
                card.group.iter().filter_map(|m| m.ms).sum::<u128>() + card.ms.unwrap_or(0);
            Some(total)
        };
        if let Some(ms) = ms {
            parts.push(fmt_ms(ms));
        }
    }
    parts.join(&sep)
}

/// Status glyph and its style. `tick` animates the running spinner when given.
pub(crate) fn status_mark(card: &ToolCard, tick: Option<usize>, awaiting: bool) -> (String, Style) {
    let glyphs = g();
    match card.status {
        CardStatus::Running if awaiting => (glyphs.awaiting.to_string(), styles::warn()),
        CardStatus::Running => (
            glyphs.spin(tick.unwrap_or(0)).to_string(),
            styles::spinner(tick.unwrap_or(0)),
        ),
        _ if !card.group.is_empty() => (glyphs.group.to_string(), styles::muted()),
        CardStatus::Ok => (glyphs.ok.to_string(), styles::ok()),
        CardStatus::Failed => (glyphs.fail.to_string(), styles::err().bold()),
    }
}

/// The always-present first row.
fn header_row(card: &ToolCard, tick: Option<usize>, awaiting: bool) -> TextLine<'static> {
    let (mark, mark_style) = status_mark(card, tick, awaiting);
    let (verb_text, target) = if card.group.is_empty() {
        (verb(&card.name), headline(&card.name, &card.args))
    } else {
        let n = card.group.len() + 1;
        let noun = if card.name == "read_file" {
            "files"
        } else {
            "calls"
        };
        (verb(&card.name), format!("{n} {noun}"))
    };
    let right = if awaiting && card.status == CardStatus::Running {
        "awaiting approval".to_string()
    } else {
        right_text(card)
    };
    let mut spans = rail::rail_spans(rail::BAKED);
    spans.push(Span::styled(mark, mark_style));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        format!("{verb_text:<VERB_W$} "),
        styles::muted().bold(),
    ));
    spans.push(Span::styled(target, styles::fg()));
    spans.push(copy_mark::right_marker());
    let right_style = if awaiting && card.status == CardStatus::Running {
        styles::warn()
    } else if card.status == CardStatus::Failed {
        styles::err()
    } else {
        styles::muted()
    };
    spans.push(Span::styled(right, right_style));
    if card.has_detail() && card.status != CardStatus::Running {
        let tri = if card.expanded {
            g().disclose_open
        } else {
            g().disclose_closed
        };
        spans.push(Span::styled(format!(" {tri}"), styles::faint()));
    }
    TextLine::from(spans)
}

/// The header of a still-running card as the draw loop shows it: an animated spinner, or the
/// `◇ … awaiting approval` state while a permission prompt is open.
pub(crate) fn live_header(card: &ToolCard, tick: usize, awaiting: bool) -> TextLine<'static> {
    header_row(card, Some(tick), awaiting)
}

/// A body row under the header: rail, two blanks, the `┊` gutter, then the content.
fn body_row(spans: Vec<Span<'static>>) -> TextLine<'static> {
    prefixed(spans, Style::default())
}

fn prefixed(spans: Vec<Span<'static>>, line_style: Style) -> TextLine<'static> {
    let mut row = rail::rail_spans(rail::BAKED);
    row.push(Span::styled("  ", copy_mark::decor(Style::default())));
    row.push(Span::styled(
        g().card_gutter,
        copy_mark::decor(styles::faint()),
    ));
    row.push(Span::styled(
        " ",
        copy_mark::hang(copy_mark::decor(Style::default())),
    ));
    let line = TextLine::from(spans).style(line_style);
    rail::with_prefix(row, line)
}

pub(crate) fn text_row(text: String, style: Style) -> TextLine<'static> {
    body_row(vec![Span::styled(text, style)])
}

/// Render one card: a single row (plus a diff preview or a failure tail) when collapsed, the row
/// plus its arguments, diff and output when expanded. `width` is the transcript's cell width;
/// long values are wrapped here with a hanging indent so a continuation line stays visually
/// inside its field instead of starting at column 0.
pub(crate) fn card_lines(card: &ToolCard, width: u16) -> Vec<TextLine<'static>> {
    if let Some(fold) = &card.fold {
        return fold_lines(card, fold);
    }
    let width = width.max(20) as usize;
    let mut out = vec![header_row(card, None, false)];
    if !card.expanded {
        collapsed_body(card, &mut out);
        return out;
    }
    // "▏ ┊ " is 7 cells of prefix, then the label column and its two-space gutter.
    const PREFIX: usize = 7;
    let rows = arg_rows_for(card);
    let label_w = rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0)
        .max("tool".len());
    let kv = |out: &mut Vec<TextLine<'static>>, key: &str, value: &str| {
        let indent = PREFIX + label_w + 2;
        let mut first = true;
        for chunk in wrap_plain(value, width.saturating_sub(indent + 1)) {
            let mut spans = Vec::new();
            if first {
                spans.push(Span::styled(format!("{key:<label_w$}  "), styles::muted()));
                first = false;
            } else {
                spans.push(Span::raw(" ".repeat(label_w + 2)));
            }
            spans.push(Span::styled(chunk, styles::fg()));
            out.push(body_row(spans));
        }
    };
    for m in &card.group {
        let target = headline(&m.name, &m.args);
        let outcome = clean_summary(&m.name, &m.summary);
        let ms = m.ms.map(fmt_ms).unwrap_or_default();
        out.push(body_row(vec![
            Span::styled(format!("{target}  "), styles::fg()),
            Span::styled(
                [outcome, ms]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(&format!(" {} ", g().sep)),
                styles::muted(),
            ),
        ]));
    }
    kv(&mut out, "tool", crate::app::tool_display_name(&card.name));
    for (key, value) in &rows {
        kv(&mut out, key, value);
    }
    if let Some(diff) = &card.diff {
        for row in crate::render::diff_to_lines(diff) {
            out.push(prefixed(row.spans, row.style));
        }
    }
    if let Some(detail) = &card.detail {
        if !rows.is_empty() || card.diff.is_some() {
            out.push(text_row("output".to_string(), styles::faint()));
        }
        let style = if card.status == CardStatus::Failed {
            styles::err()
        } else {
            styles::muted()
        };
        for line in detail.lines() {
            for chunk in wrap_plain(line, width.saturating_sub(PREFIX + 1)) {
                out.push(text_row(chunk, style));
            }
        }
    }
    out.extend(note_rows(card));
    // The preview above is bounded; this row is the way to everything. It sits directly above the
    // collapse hint so `full_output_row` can find it without re-rendering the card.
    if let Some(full) = &card.full {
        out.push(body_row(vec![
            Span::styled("view full output", styles::muted().bold()),
            Span::styled(
                format!(
                    "  {} {} click here or /output",
                    size_label(full.lines, full.bytes),
                    g().sep
                ),
                styles::muted(),
            ),
        ]));
    }
    out.push(text_row(
        "click or Ctrl+T to collapse".to_string(),
        styles::faint(),
    ));
    out
}

/// What a collapsed card shows under its header: a diff preview, or a failure's last lines.
fn collapsed_body(card: &ToolCard, out: &mut Vec<TextLine<'static>>) {
    if let Some(diff) = &card.diff {
        if card.status == CardStatus::Failed {
            return;
        }
        let r = crate::render::diff_rows(diff, PREVIEW_DIFF_ROWS);
        for row in r.rows {
            out.push(prefixed(row.spans, row.style));
        }
        if r.hidden > 0 {
            let hint = if card.interactive {
                format!(
                    "{} {} more lines {} Ctrl+T expand",
                    g().ellipsis,
                    r.hidden,
                    g().sep
                )
            } else {
                format!("{} {} more lines", g().ellipsis, r.hidden)
            };
            out.push(text_row(hint, styles::faint()));
        }
        return;
    }
    if card.status == CardStatus::Failed {
        if let Some(detail) = &card.detail {
            let lines: Vec<&str> = detail.lines().filter(|l| !l.trim().is_empty()).collect();
            let from = lines.len().saturating_sub(FAILURE_TAIL_LINES);
            for l in &lines[from..] {
                out.push(text_row(truncate_cells(l, 200), styles::err()));
            }
        }
        out.extend(note_rows(card));
    }
}

/// A failed call's diagnosis, dim under its header: the likely cause, then the fix.
pub(crate) fn note_rows(card: &ToolCard) -> Vec<TextLine<'static>> {
    let Some(note) = &card.note else {
        return Vec::new();
    };
    let mut out: Vec<TextLine<'static>> = note
        .diagnosis
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| text_row(l.trim().to_string(), styles::muted()))
        .collect();
    if let Some(fix) = note.fix.as_deref().filter(|f| !f.trim().is_empty()) {
        out.push(body_row(vec![
            Span::styled("fix ", styles::muted().bold()),
            Span::styled(fix.trim().to_string(), styles::muted()),
        ]));
    }
    out
}

/// The rows of a folded user turn.
fn fold_lines(card: &ToolCard, fold: &UserFold) -> Vec<TextLine<'static>> {
    let user = |text: &str| rail::text_row(RailKind::User, text, styles::fg());
    let hint = |text: String| rail::text_row(RailKind::User, &text, styles::faint());
    let (sep, ellipsis) = (g().sep, g().ellipsis);
    if card.expanded {
        let mut out: Vec<TextLine<'static>> = fold.rows.iter().map(|r| user(r)).collect();
        out.push(hint("Ctrl+T hide".to_string()));
        return out;
    }
    match &fold.label {
        Some(label) => {
            let mut row = rail::label_row(
                RailKind::User,
                vec![Span::styled(label.clone(), styles::fg())],
                Vec::new(),
            );
            row.spans.push(Span::styled(
                format!("  {sep} full prompt hidden {sep} Ctrl+T show"),
                styles::faint(),
            ));
            vec![row]
        }
        None => {
            let mut out: Vec<TextLine<'static>> = fold
                .rows
                .iter()
                .take(USER_FOLD_ROWS)
                .map(|r| user(r))
                .collect();
            out.push(hint(format!(
                "{ellipsis} {} more lines {sep} Ctrl+T show",
                fold.rows.len().saturating_sub(USER_FOLD_ROWS)
            )));
            out
        }
    }
}

/// Which of a card's rendered lines is its "view full output" row, when it shows one.
pub(crate) fn full_output_row(card: &ToolCard) -> Option<usize> {
    (card.expanded && card.full.is_some() && card.len >= 2).then(|| card.len - 2)
}

/// The argument rows of an expanded card. When the card carries its diff, the body arguments
/// that only restate it (`content`, `old_string`, …) are left out.
fn arg_rows_for(card: &ToolCard) -> Vec<(String, String)> {
    let mut rows = arg_rows(&card.args);
    if card.diff.is_some() {
        rows.retain(|(k, _)| {
            !matches!(
                k.as_str(),
                "content" | "old_string" | "new_string" | "patch" | "diff" | "old" | "new"
            )
        });
    }
    rows
}

/// The one-line "what did this call do" text: the argument a human identifies the call BY (the
/// shell command, the file path, the search pattern), never the raw JSON envelope.
pub(crate) fn headline(name: &str, args: &str) -> String {
    if name == "run_code" {
        return super::script_row::headline(args);
    }
    let Some(value) = serde_json::from_str::<Value>(args).ok() else {
        return oneline(args);
    };
    let get = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_string);
    let cwd = get("cwd");
    let show_path = |p: String| display_path(&p, cwd.as_deref());
    let first = match name {
        "shell" => get("command").map(|c| shorten_in(&c)).or_else(|| {
            let list = value.get("commands")?.as_array()?;
            let first = list.first()?.as_str()?;
            Some(match list.len() {
                1 => shorten_in(first),
                n => format!("{} (+{} more)", shorten_in(first), n - 1),
            })
        }),
        "read_file" | "write_file" | "edit_file" | "list_dir" => get("path").map(show_path),
        "apply_patch" => get("path").map(show_path).or_else(|| get("cwd")),
        "search" | "grep" => get("pattern").or_else(|| get("query")),
        "glob" => get("pattern"),
        "web_fetch" => get("url"),
        "web_search" => get("query"),
        _ => None,
    };
    first
        .or_else(|| {
            // Unknown tool: show its first string argument rather than `{"a":1,…}`.
            value
                .as_object()
                .and_then(|o| o.values().find_map(Value::as_str).map(str::to_string))
        })
        .map(|s| oneline(&s))
        .unwrap_or_default()
}

/// The expanded card's argument rows, in a stable, human-first order: the identifying argument
/// first, then the rest alphabetically. Values are decoded from JSON so escaped quotes and
/// newlines read as themselves instead of as `\"` and `\n`.
fn arg_rows(args: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<Value>(args) else {
        let text = args.trim();
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![("args".to_string(), text.to_string())]
        };
    };
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let cwd = object
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut keys: Vec<&String> = object.keys().collect();
    keys.sort();
    // The argument the call is identified by leads, whatever the tool.
    const LEAD: [&str; 6] = ["command", "path", "pattern", "query", "url", "paths"];
    keys.sort_by_key(|k| LEAD.iter().position(|l| l == k).unwrap_or(LEAD.len()));
    keys.into_iter()
        .filter_map(|k| {
            let rendered = match &object[k] {
                Value::String(s) => s.clone(),
                Value::Null => return None,
                other => other.to_string(),
            };
            if rendered.trim().is_empty() {
                return None;
            }
            let rendered = match k.as_str() {
                "path" => display_path(&rendered, cwd.as_deref()),
                "cwd" => shorten(&rendered),
                _ => rendered,
            };
            Some((k.clone(), rendered))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_batch_headline_names_the_first_command_and_the_rest() {
        let args = r#"{"commands":["cargo build","cargo test"],"cwd":"/w"}"#;
        assert_eq!(headline("shell", args), "cargo build (+1 more)");
    }

    #[test]
    fn a_script_row_names_what_it_calls_never_its_source() {
        let code = "await tools.edit_file({path: 'calc.py', old: 'a - b', new: 'a + b'});\n\
                    const r = await tools.shell({command: 'pytest'});\n\
                    await tools.edit_file({path: 'b.py', old: 'x', new: 'y'}); // tools.read_file";
        assert_eq!(headline("run_code", code), "edit, run");
        let json = serde_json::json!({"code": code, "timeout_secs": 30}).to_string();
        assert_eq!(headline("run_code", &json), "edit, run");
        assert_eq!(headline("run_code", "return 1 + 1"), "");
        let mut c = card("run_code", code);
        c.status = CardStatus::Ok;
        c.summary = "script finished".to_string();
        let row = &text(&card_lines(&c, 100))[0];
        assert!(row.contains("Script"), "{row:?}");
        assert!(!row.contains("old:") && !row.contains("await"), "{row:?}");
    }

    fn text(lines: &[TextLine<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    fn card(name: &str, args: &str) -> ToolCard {
        ToolCard::new(1, name.to_string(), args.to_string())
    }

    #[test]
    fn a_collapsed_card_is_one_row_carrying_both_the_call_and_its_outcome() {
        // The whole point of the card: the call and the result that used to be two unrelated
        // lines are one row, so a transcript can be read without pairing them up by eye.
        let mut c = card("shell", r#"{"command":"cargo test","cwd":"/tmp/x"}"#);
        c.status = CardStatus::Ok;
        c.summary = "shell: exit 0 in 132ms".to_string();
        let lines = text(&card_lines(&c, 100));
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("✓ Run "),
            "humanised verb: {:?}",
            lines[0]
        );
        assert!(lines[0].contains("cargo test"), "{:?}", lines[0]);
        assert!(lines[0].contains("exit 0"), "{:?}", lines[0]);
        // The duration is measured by the card, and the tool name is not repeated on the right.
        assert!(
            !lines[0].contains("132ms") && !lines[0].contains("shell"),
            "{:?}",
            lines[0]
        );
    }

    #[test]
    fn expanding_reveals_the_full_arguments_and_the_output() {
        // Collapsed rows truncate; the expanded card is where the untruncated command has to be,
        // otherwise "click to expand" buys the reader nothing.
        let long = "adb -s emulator-5554 shell dumpsys media_session | grep -i -A20 spotify";
        let mut c = card(
            "shell",
            &format!(r#"{{"command":"{long}","cwd":"/tmp/x"}}"#),
        );
        c.status = CardStatus::Ok;
        c.summary = "shell: exit 0 in 132ms".to_string();
        c.detail = Some("line one\nline two".to_string());
        c.expanded = true;
        let joined = text(&card_lines(&c, 120)).join("\n");
        assert!(joined.contains(long), "full command shown: {joined}");
        assert!(joined.contains("line one") && joined.contains("line two"));
        assert!(
            joined.lines().next().unwrap().ends_with('▾'),
            "expanded marker: {joined}"
        );
        assert!(
            joined.contains("tool     shell"),
            "the raw tool name lives in the body: {joined}"
        );
    }

    #[test]
    fn a_card_with_nothing_behind_it_shows_no_disclosure_triangle() {
        let mut c = card("get_time", "{}");
        c.status = CardStatus::Ok;
        c.summary = "12:00".to_string();
        assert!(!c.has_detail());
        let row = text(&card_lines(&c, 80)).remove(0);
        assert!(!row.contains('▸') && !row.contains('▾'), "{row}");
    }

    #[test]
    fn a_running_card_says_so_before_the_result_arrives() {
        let c = card("shell", r#"{"command":"sleep 5"}"#);
        let row = text(&card_lines(&c, 80)).remove(0);
        assert!(row.contains("running"), "{row}");
        assert!(row.contains("⠋"), "spinner glyph: {row}");
    }

    #[test]
    fn long_values_wrap_inside_their_field_instead_of_overflowing_the_row() {
        let mut c = card("shell", &format!(r#"{{"command":"{}"}}"#, "x".repeat(300)));
        c.expanded = true;
        let lines = crate::transcript::wrap_lines(&card_lines(&c, 60), 59);
        assert!(lines.len() > 3, "wrapped into several rows");
        for l in &lines {
            let cells: usize = l
                .spans
                .iter()
                .flat_map(|s| s.content.chars())
                .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(1))
                .sum();
            assert!(cells <= 60, "row fits the width: {cells}");
        }
    }

    #[test]
    fn paths_are_shown_relative_to_home() {
        let home = std::env::var("HOME").unwrap_or_default();
        if home.is_empty() {
            return;
        }
        let mut c = card("read_file", &format!(r#"{{"path":"{home}/notes.md"}}"#));
        c.expanded = true;
        let joined = text(&card_lines(&c, 100)).join("\n");
        assert!(joined.contains("~/notes.md"), "{joined}");
        assert!(!joined.contains(&home), "{joined}");
    }

    #[test]
    fn a_path_under_the_calls_own_cwd_is_shown_relative_to_it() {
        // Every call in a session repeats the same workspace prefix; spending the row's width on
        // it pushes out the part that differs between calls.
        let mut c = card(
            "read_file",
            r#"{"path":"/work/repo/crates/core/src/lib.rs","cwd":"/work/repo"}"#,
        );
        c.expanded = true;
        let joined = text(&card_lines(&c, 100)).join("\n");
        assert!(joined.contains("crates/core/src/lib.rs"), "{joined}");
        assert!(
            !joined.contains("/work/repo/crates"),
            "the prefix is dropped from the path row: {joined}"
        );
        // The cwd itself is still shown, so the relative path is never ambiguous.
        assert!(joined.contains("/work/repo"), "{joined}");
    }

    #[test]
    fn malformed_args_still_render_rather_than_being_dropped() {
        // Tool args come from a model; a card must never depend on them parsing as JSON.
        let mut c = card("shell", "not json at all");
        c.expanded = true;
        let joined = text(&card_lines(&c, 80)).join("\n");
        assert!(joined.contains("not json at all"), "{joined}");
    }

    fn kept(lines: usize) -> forge_types::ToolOutputRef {
        forge_types::ToolOutputRef {
            path: "/tmp/out.log".into(),
            lines,
            bytes: 4096,
        }
    }

    #[test]
    fn a_card_whose_output_was_kept_offers_the_full_view_from_its_open_drawer() {
        let mut c = card("shell", r#"{"command":"cargo test"}"#);
        c.status = CardStatus::Failed;
        c.summary = "shell: exit 101 in 9s".into();
        c.detail = Some("head\n… 4800 lines hidden · full output: /output\ntail".into());
        c.full = Some(kept(5_012));
        let collapsed = text(&card_lines(&c, 120));
        assert!(
            collapsed[0].contains("5,012 lines"),
            "the closed row already says there is more: {collapsed:?}"
        );
        c.expanded = true;
        let lines = card_lines(&c, 120);
        c.len = lines.len();
        let rows = text(&lines);
        let at = full_output_row(&c).expect("an open card with kept output has the row");
        assert!(rows[at].contains("view full output"), "{rows:?}");
        assert!(rows[at].contains("5,012 lines · 4 KB"), "{rows:?}");
    }

    #[test]
    fn a_closed_card_has_no_full_output_row_to_click() {
        let mut c = card("shell", r#"{"command":"ls"}"#);
        c.full = Some(kept(10));
        c.len = 1;
        assert_eq!(full_output_row(&c), None);
        assert!(c.has_detail(), "kept output alone is worth a drawer");
    }

    #[test]
    fn sizes_read_the_way_a_person_would_write_them() {
        assert_eq!(size_label(1, 12), "1 line · 12 B");
        assert_eq!(size_label(1_234_567, 3 << 20), "1,234,567 lines · 3.0 MB");
    }

    fn diff_card(kind: forge_types::DiffKind, old: Option<&str>, new: &str) -> ToolCard {
        let mut c = card("write_file", r#"{"path":"src/lib.rs","content":"X"}"#);
        c.status = CardStatus::Ok;
        c.summary = "wrote".into();
        c.ms = Some(4);
        c.set_diff(FileDiff {
            path: "src/lib.rs".into(),
            kind,
            old: old.map(str::to_string),
            new: Some(new.to_string()),
            lang: Some("rust".into()),
            binary: false,
        });
        c
    }

    #[test]
    fn a_write_card_carries_its_diff_and_no_separate_header() {
        let c = diff_card(forge_types::DiffKind::Created, None, "one\ntwo\n");
        let rows = text(&card_lines(&c, 100));
        assert!(
            rows[0].contains("✓ Write") && rows[0].contains("new file · +2"),
            "{rows:?}"
        );
        assert!(rows[0].contains("4ms"), "{rows:?}");
        assert!(
            rows[1].contains("┊") && rows[1].contains("1 + one"),
            "{rows:?}"
        );
        assert!(
            !rows.iter().any(|r| r.contains("✎")),
            "no separate header: {rows:?}"
        );
    }

    #[test]
    fn a_long_diff_folds_behind_ctrl_t_and_the_hint_counts_the_rest() {
        let old: String = (1..=20).map(|i| format!("line {i}\n")).collect();
        let new: String = (1..=20).map(|i| format!("line {i} edited\n")).collect();
        let mut c = diff_card(forge_types::DiffKind::Modified, Some(&old), &new);
        let collapsed = text(&card_lines(&c, 100));
        assert!(
            collapsed.len() <= 1 + 1 + 8 + 1,
            "header, separator, 8 rows, hint: {}",
            collapsed.len()
        );
        let last = collapsed.last().unwrap();
        assert!(
            last.contains("more lines") && last.contains("Ctrl+T expand"),
            "{last}"
        );
        c.expanded = true;
        assert!(text(&card_lines(&c, 100)).len() > collapsed.len());
        // Inline cannot expand, so it must not promise to.
        c.expanded = false;
        c.interactive = false;
        let inline = text(&card_lines(&c, 100));
        assert!(!inline.last().unwrap().contains("Ctrl+T"), "{inline:?}");
    }

    #[test]
    fn a_failed_call_shows_the_last_three_lines_of_its_output() {
        let mut c = card("shell", r#"{"command":"cargo test"}"#);
        c.status = CardStatus::Failed;
        c.summary = "exit 101".into();
        c.detail = Some("a\nb\nc\nd\ne".into());
        let rows = text(&card_lines(&c, 100));
        assert!(rows[0].contains("✗ Run"), "{rows:?}");
        assert_eq!(rows.len(), 4, "header + 3 lines: {rows:?}");
        assert!(rows[1].contains('c') && rows[3].contains('e'), "{rows:?}");
        assert!(
            !rows.iter().any(|r| r.ends_with(" a")),
            "older lines stay behind the drawer"
        );
    }

    #[test]
    fn consecutive_reads_fold_into_one_group_card() {
        let mut c = card("read_file", r#"{"path":"a.rs"}"#);
        c.status = CardStatus::Ok;
        c.ms = Some(40);
        c.fold_in("read_file".into(), r#"{"path":"b.rs"}"#.into());
        c.status = CardStatus::Ok;
        c.ms = Some(60);
        let row = text(&card_lines(&c, 100)).remove(0);
        assert!(row.contains("◆ Read") && row.contains("2 files"), "{row}");
        assert!(row.contains("100ms"), "durations add up: {row}");
        c.expanded = true;
        let joined = text(&card_lines(&c, 100)).join("\n");
        assert!(
            joined.contains("a.rs") && joined.contains("b.rs"),
            "members listed: {joined}"
        );
    }

    #[test]
    fn verbs_are_humanised_and_unknown_tools_keep_their_name() {
        for (raw, shown) in [
            ("read_file", "Read"),
            ("write_file", "Write"),
            ("edit_file", "Edit"),
            ("shell", "Run"),
            ("grep", "Search"),
            ("glob", "Find"),
            ("web_fetch", "Fetch"),
            ("web_search", "Web"),
            ("spawn_agents", "Agents"),
            ("submit_result", "Result"),
            ("mcp__srv__lookup", "lookup"),
            ("custom_tool", "custom_tool"),
        ] {
            assert_eq!(verb(raw), shown, "{raw}");
        }
    }

    #[test]
    fn durations_read_like_a_person_would_say_them() {
        assert_eq!(fmt_ms(4), "4ms");
        assert_eq!(fmt_ms(1_500), "1.5s");
        assert_eq!(fmt_ms(41_000), "41s");
        assert_eq!(fmt_ms(125_000), "2m05s");
    }

    #[test]
    fn a_long_user_paste_folds_to_six_rows_and_a_hint() {
        let rows: Vec<String> = (1..=12).map(|i| format!("row {i}")).collect();
        let mut c = ToolCard::user_fold(1, UserFold { label: None, rows });
        let collapsed = text(&card_lines(&c, 80));
        assert_eq!(collapsed.len(), 7, "{collapsed:?}");
        assert!(collapsed[6].contains("6 more lines") && collapsed[6].contains("Ctrl+T show"));
        c.expanded = true;
        let open = text(&card_lines(&c, 80));
        assert_eq!(open.len(), 13, "all rows plus the hide hint: {open:?}");
    }

    #[test]
    fn a_canned_prompt_folds_to_one_labelled_row() {
        let c = ToolCard::user_fold(
            1,
            UserFold {
                label: Some("Set up Forge for this project".into()),
                rows: vec!["a very long internal prompt".into()],
            },
        );
        let rows = text(&card_lines(&c, 80));
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].contains("Set up Forge for this project"),
            "{rows:?}"
        );
        assert!(
            rows[0].contains("full prompt hidden") && rows[0].contains("Ctrl+T show"),
            "{rows:?}"
        );
        assert!(!rows[0].contains("internal prompt"));
    }
}
