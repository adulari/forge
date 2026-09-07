//! Expandable tool cards for the full-screen chat transcript.
//!
//! A tool call used to print two separate scrollback lines — a `↳ name {raw json args…}` line
//! truncated mid-JSON, then an unrelated-looking `✓ name: exit 0 in 132ms` line further down once
//! the tool returned. Reading a transcript meant mentally pairing them up, and the part a human
//! actually wants (the command, the path, the output) was the part that got cut.
//!
//! A card is ONE row per call that owns both halves: the call on the left, its outcome on the
//! right. Clicking it (or Ctrl+T for the most recent) expands it in place into the full arguments
//! and the tool's own output, and clicking again collapses it. The card's lines live in
//! `App::main_log` like any other scrollback, so wrapping, scrolling, selection and copy keep
//! working unchanged — expanding simply splices a longer run of lines over the collapsed one.
//!
//! Cards exist only in full-screen mode: inline mode prints into the terminal's native scrollback,
//! which cannot be rewritten after the fact, so it keeps the original two-line rendering.

use ratatui::style::Style;
use ratatui::text::{Line as TextLine, Span};
use serde_json::Value;

use crate::surface::{DIM, ERRRED, OKGREEN, TEXT, TOOLCYAN, VERY_DIM, WARNYEL};

/// How a card's call ended. `Running` is the state between `ToolStart` and `ToolResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardStatus {
    Running,
    Ok,
    Failed,
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
    /// The result's first line (what the old `✓ name …` line showed).
    pub(crate) summary: String,
    /// Bounded raw output, when the emitter had any to hand over.
    pub(crate) detail: Option<String>,
    pub(crate) expanded: bool,
    /// Where this card's rendered lines currently sit in `App::main_log`.
    pub(crate) start: usize,
    pub(crate) len: usize,
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
            expanded: false,
            start: 0,
            len: 0,
        }
    }

    /// Whether opening this card would reveal anything the collapsed row does not already show.
    /// A call with no arguments and no output stays a plain row rather than offering an empty
    /// drawer — the affordance must never lie about having something behind it.
    pub(crate) fn has_detail(&self) -> bool {
        self.detail.is_some() || !arg_rows(&self.args).is_empty()
    }
}

/// Render one card: a single row when collapsed, the row plus its arguments and output when
/// expanded. `width` is the transcript's cell width; long values are wrapped here with a hanging
/// indent so a continuation line stays visually inside its field instead of starting at column 0.
pub(crate) fn card_lines(card: &ToolCard, width: u16) -> Vec<TextLine<'static>> {
    let width = width.max(20) as usize;
    let mut out = vec![header_line(card, width)];
    if !card.expanded {
        return out;
    }
    let rows = arg_rows(&card.args);
    let label_w = rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    for (key, value) in &rows {
        // "     ┆ " is 7 cells, then the label column and its two-space gutter.
        let indent = 7 + label_w + 2;
        let mut first = true;
        for chunk in wrap_plain(value, width.saturating_sub(indent + 1)) {
            let mut spans = vec![Span::styled("     ┆ ", Style::default().fg(VERY_DIM))];
            if first {
                spans.push(Span::styled(
                    format!("{key:<label_w$}  "),
                    Style::default().fg(TOOLCYAN),
                ));
                first = false;
            } else {
                spans.push(Span::raw(" ".repeat(label_w + 2)));
            }
            spans.push(Span::styled(chunk, Style::default().fg(TEXT)));
            out.push(TextLine::from(spans));
        }
    }
    if let Some(detail) = &card.detail {
        if !rows.is_empty() {
            out.push(TextLine::from(Span::styled(
                "     ┆ output",
                Style::default().fg(VERY_DIM),
            )));
        }
        let style = if card.status == CardStatus::Failed {
            Style::default().fg(ERRRED)
        } else {
            Style::default().fg(DIM)
        };
        for line in detail.lines() {
            for chunk in wrap_plain(line, width.saturating_sub(8)) {
                out.push(TextLine::from(vec![
                    Span::styled("     ┆ ", Style::default().fg(VERY_DIM)),
                    Span::styled(chunk, style),
                ]));
            }
        }
    }
    out.push(TextLine::from(Span::styled(
        "     ┆ click or Ctrl+T to collapse",
        Style::default().fg(VERY_DIM),
    )));
    out
}

/// The always-present first row: `▸ shell   <call headline>          ✓ exit 0 in 132ms`.
fn header_line(card: &ToolCard, width: usize) -> TextLine<'static> {
    let (glyph, glyph_style) = match (card.expanded, card.has_detail()) {
        (true, _) => ("▾ ", Style::default().fg(TOOLCYAN)),
        (false, true) => ("▸ ", Style::default().fg(TOOLCYAN)),
        // Nothing to open: no disclosure triangle, so the row does not advertise a drawer.
        (false, false) => ("· ", Style::default().fg(VERY_DIM)),
    };
    let (mark, mark_style) = match card.status {
        CardStatus::Running => ("◍", Style::default().fg(WARNYEL)),
        CardStatus::Ok => ("✓", Style::default().fg(OKGREEN)),
        CardStatus::Failed => ("✖", Style::default().fg(ERRRED).bold()),
    };
    let name = card.name.clone();
    let status = status_text(card);
    // Row layout, in cells: 2 indent + 2 glyph + name + 2 gap + headline + pad + 1 mark +
    // 1 space + status = exactly `width`. The headline is the elastic part and the pad is what
    // pushes the outcome to the right margin; the row must land ON the width, because a row one
    // cell over wraps into a blank second row and desynchronises click hit-testing.
    let base = 2 + 2 + cells(&name) + 2 + 1 + 1 + cells(&status);
    let headline = truncate_cells(
        &headline(&card.name, &card.args),
        width.saturating_sub(base + 1),
    );
    let pad = width.saturating_sub(base + cells(&headline)).max(1);
    TextLine::from(vec![
        Span::raw("  "),
        Span::styled(glyph, glyph_style),
        Span::styled(name, Style::default().fg(TOOLCYAN).bold()),
        Span::raw("  "),
        Span::styled(headline, Style::default().fg(DIM)),
        Span::raw(" ".repeat(pad)),
        Span::styled(mark, mark_style),
        Span::styled(format!(" {status}"), Style::default().fg(DIM)),
    ])
}

/// The right-hand outcome text: the result's own first line while it says something, else a
/// generic ok/failed so the row is never blank on the right.
fn status_text(card: &ToolCard) -> String {
    match card.status {
        CardStatus::Running => "running".to_string(),
        _ => {
            // The core prefixes many summaries with the tool name ("shell: exit 0 in 132ms"),
            // which the row already shows on the left — strip it rather than print it twice.
            let s = card
                .summary
                .strip_prefix(&format!("{}: ", card.name))
                .unwrap_or(&card.summary)
                .trim()
                .to_string();
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

/// The one-line "what did this call do" text: the argument a human identifies the call BY (the
/// shell command, the file path, the search pattern), never the raw JSON envelope.
fn headline(name: &str, args: &str) -> String {
    let Some(value) = serde_json::from_str::<Value>(args).ok() else {
        return oneline(args);
    };
    let get = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_string);
    let cwd = get("cwd");
    let show_path = |p: String| display_path(&p, cwd.as_deref());
    let first = match name {
        "shell" => get("command"),
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

/// How a path is shown: relative to the call's own `cwd` when it sits under it, else with the
/// home directory folded to `~`. A tool call's path is almost always inside the workspace, and
/// repeating that prefix on every row costs the width the interesting tail needs.
fn display_path(path: &str, cwd: Option<&str>) -> String {
    if let Some(cwd) = cwd.filter(|c| !c.is_empty()) {
        if let Some(rest) = path.strip_prefix(cwd) {
            let rest = rest.trim_start_matches('/');
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    shorten(path)
}

/// Replace the home directory with `~` so a path row reads as a path, not as a margin-eating
/// absolute prefix repeated on every call.
fn shorten(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// Collapse a multi-line value onto one line for the header row.
fn oneline(s: &str) -> String {
    let mut out = String::new();
    for (i, part) in s
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .enumerate()
    {
        if i > 0 {
            out.push_str(" ⏎ ");
        }
        out.push_str(part);
    }
    out
}

/// Hard-wrap plain text to `width` cells. Never returns an empty vector, so a blank line in tool
/// output still occupies a row (blank lines are structure in most command output).
fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cells = 0usize;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(1);
        if cells + w > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cells = 0;
        }
        cur.push(ch);
        cells += w;
    }
    out.push(cur);
    out
}

/// The display width of a string in terminal cells.
fn cells(s: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    s.chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(1))
        .sum()
}

/// Truncate to `cells` display columns with an ellipsis, counting terminal cells (a CJK glyph is
/// two) so the header's right-hand status column stays put on any content.
fn truncate_cells(s: &str, cells: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if cells == 0 {
        return String::new();
    }
    let total: usize = s
        .chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(1))
        .sum();
    if total <= cells {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(1);
        if used + w > cells.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(lines[0].contains("shell"), "{:?}", lines[0]);
        assert!(lines[0].contains("cargo test"), "{:?}", lines[0]);
        assert!(lines[0].contains("exit 0 in 132ms"), "{:?}", lines[0]);
        // The tool name is not repeated on the right: the row already says `shell` on the left.
        assert!(!lines[0].contains("shell: exit"), "{:?}", lines[0]);
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
        assert!(joined.starts_with("  ▾"), "expanded marker: {joined}");
    }

    #[test]
    fn a_card_with_nothing_behind_it_shows_no_disclosure_triangle() {
        let mut c = card("get_time", "{}");
        c.status = CardStatus::Ok;
        c.summary = "12:00".to_string();
        assert!(!c.has_detail());
        assert!(text(&card_lines(&c, 80))[0].starts_with("  ·"));
    }

    #[test]
    fn a_running_card_says_so_before_the_result_arrives() {
        let c = card("shell", r#"{"command":"sleep 5"}"#);
        let row = text(&card_lines(&c, 80)).remove(0);
        assert!(row.contains("running"), "{row}");
        assert!(row.contains("◍"), "{row}");
    }

    #[test]
    fn long_values_wrap_inside_their_field_instead_of_overflowing_the_row() {
        let mut c = card("shell", &format!(r#"{{"command":"{}"}}"#, "x".repeat(300)));
        c.expanded = true;
        let lines = card_lines(&c, 60);
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
}
