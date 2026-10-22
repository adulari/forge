//! Tool groups: a run of tool calls the assistant made without saying anything in between,
//! folded into one dim summary row (`▸ Read 3 files, ran 2 shell commands       1 failed · 2.1s`).
//!
//! This module is pure: it turns the cards of a group into the summary text and the rows the
//! collapsed group shows. Membership, folding the transcript and the keys live in `group_flow`.

use super::lines::rail;
use super::tool_cards::{self, CardStatus, ToolCard};
use crate::copy_mark;
use crate::theme::glyphs::g;
use crate::theme::styles;
use forge_types::DiffKind;
use ratatui::text::{Line as TextLine, Span};
use serde_json::Value;

/// What a call did, for the summary. File changes split by what happened to the file, so a
/// deletion is never merged into "edited".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Category {
    Read,
    List,
    Search,
    Shell,
    Script,
    Created,
    Edited,
    Deleted,
    Fetch,
    WebSearch,
    Agents,
    Tasks,
    /// An MCP server's tool (`mcp__<server>__<tool>`), named by its server.
    Mcp(String),
    /// Any other tool, by name.
    Other(String),
}

/// The calls of one category, in the order the category first appeared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tally {
    pub(crate) category: Category,
    pub(crate) count: usize,
    pub(crate) adds: usize,
    pub(crate) dels: usize,
    /// The target of the first call, for a group of one call (`Ran cargo test`).
    pub(crate) target: String,
}

/// Everything the summary row says about a group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    /// Finished calls, by category.
    pub(crate) tallies: Vec<Tally>,
    pub(crate) finished: usize,
    pub(crate) failed: usize,
    pub(crate) running: usize,
    /// Total duration of the finished calls.
    pub(crate) ms: u128,
}

pub(crate) fn category(card: &ToolCard) -> Category {
    let name = card.name.as_str();
    if let Some(server) = name
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
        .map(|(server, _)| server)
    {
        return Category::Mcp(server.to_string());
    }
    match name {
        "read_file" => Category::Read,
        "list_dir" => Category::List,
        "search" | "grep" | "glob" => Category::Search,
        "shell" | "shell_job" => Category::Shell,
        "run_code" => Category::Script,
        "delete_file" => Category::Deleted,
        "write_file" | "edit_file" | "multi_edit" | "apply_patch" | "append_file"
        | "notebook_edit" => match card.diff.as_ref().map(|d| d.kind) {
            Some(DiffKind::Created) => Category::Created,
            Some(DiffKind::Deleted) => Category::Deleted,
            _ => Category::Edited,
        },
        "web_fetch" => Category::Fetch,
        "web_search" => Category::WebSearch,
        "spawn_agents" | "dispatch_sessions" | "task" | "agent" => Category::Agents,
        "update_tasks" => Category::Tasks,
        other => Category::Other(other.to_string()),
    }
}

/// How many units a call counts for: a read group card holds several reads, an agents call
/// spawns several agents.
fn units(card: &ToolCard, category: &Category) -> usize {
    match category {
        Category::Read => card.group.len() + 1,
        Category::Agents => serde_json::from_str::<Value>(&card.args)
            .ok()
            .and_then(|v| {
                ["agents", "tasks", "items"]
                    .iter()
                    .find_map(|k| v.get(*k).and_then(Value::as_array).map(Vec::len))
            })
            .filter(|n| *n > 0)
            .unwrap_or(1),
        _ => 1,
    }
}

/// Fold the cards of a group into a summary. Running calls are counted but not tallied: the
/// summary speaks in the past tense, and the running call has a row of its own.
pub(crate) fn summarize<'a>(cards: impl IntoIterator<Item = &'a ToolCard>) -> Summary {
    let mut out = Summary::default();
    for card in cards {
        if card.status == CardStatus::Running {
            out.running += 1;
            continue;
        }
        out.finished += 1;
        if card.status == CardStatus::Failed {
            out.failed += 1;
        }
        out.ms += card.ms.unwrap_or(0) + card.group.iter().filter_map(|m| m.ms).sum::<u128>();
        let category = category(card);
        let n = units(card, &category);
        let (adds, dels) = card
            .diff
            .as_ref()
            .map(crate::render::diff_stat)
            .unwrap_or((0, 0));
        match out.tallies.iter_mut().find(|t| t.category == category) {
            Some(t) => {
                t.count += n;
                t.adds += adds;
                t.dels += dels;
            }
            None => out.tallies.push(Tally {
                target: if card.group.is_empty() {
                    tool_cards::headline(&card.name, &card.args)
                } else {
                    String::new()
                },
                category,
                count: n,
                adds,
                dels,
            }),
        }
    }
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// ` (+41 −7)`, or nothing for a change that carried no diff.
fn stat(t: &Tally) -> String {
    let minus = g().diff_minus;
    match (t.adds, t.dels) {
        (0, 0) => String::new(),
        (a, 0) => format!(" (+{a})"),
        (0, d) => format!(" ({minus}{d})"),
        (a, d) => format!(" (+{a} {minus}{d})"),
    }
}

/// One category in words, counted: `ran 4 shell commands`.
fn counted(t: &Tally) -> String {
    let n = t.count;
    match &t.category {
        Category::Read => format!("read {}", plural(n, "file", "files")),
        Category::List => format!("listed {}", plural(n, "directory", "directories")),
        Category::Search => format!("searched {}", plural(n, "pattern", "patterns")),
        Category::Shell => format!("ran {}", plural(n, "shell command", "shell commands")),
        Category::Script => format!("ran {}", plural(n, "script", "scripts")),
        Category::Created => format!("created {}{}", plural(n, "file", "files"), stat(t)),
        Category::Edited => format!("edited {}{}", plural(n, "file", "files"), stat(t)),
        Category::Deleted => format!("deleted {}{}", plural(n, "file", "files"), stat(t)),
        Category::Fetch => format!("fetched {}", plural(n, "page", "pages")),
        Category::WebSearch => format!("ran {}", plural(n, "web search", "web searches")),
        Category::Agents => format!("ran {}", plural(n, "agent", "agents")),
        Category::Tasks if n == 1 => "updated the task list".to_string(),
        Category::Tasks => format!("updated the task list {n} times"),
        Category::Mcp(name) | Category::Other(name) => {
            format!("called {name} {}", plural(n, "time", "times"))
        }
    }
}

/// The one call of a single-call group, by what it touched: `ran cargo test -p upload`. `None`
/// when the call has no target worth naming, so the counted form is used instead.
fn single(t: &Tally) -> Option<String> {
    let target = t.target.trim();
    if target.is_empty() || t.count != 1 {
        return None;
    }
    Some(match &t.category {
        Category::Read => format!("read {target}"),
        Category::List => format!("listed {target}"),
        Category::Search => format!("searched for {target}"),
        Category::Shell => format!("ran {target}"),
        Category::Created => format!("created {target}{}", stat(t)),
        Category::Edited => format!("edited {target}{}", stat(t)),
        Category::Deleted => format!("deleted {target}"),
        Category::Fetch => format!("fetched {target}"),
        Category::WebSearch => format!("searched the web for {target}"),
        _ => return None,
    })
}

/// The summary in natural English, categories in order of first appearance, the first word
/// capitalised: `Read 3 files, searched 2 patterns, ran 4 shell commands`.
pub(crate) fn summary_text(s: &Summary) -> String {
    let text = match s.tallies.as_slice() {
        [] => String::new(),
        [only] if s.finished == 1 => single(only).unwrap_or_else(|| counted(only)),
        many => many.iter().map(counted).collect::<Vec<_>>().join(", "),
    };
    capitalise(&text)
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// How a summary row is drawn.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RowState {
    pub(crate) expanded: bool,
    /// The animation tick while the group still has a call running and is collapsed.
    pub(crate) spinner: Option<usize>,
    /// Keyboard focus is on this group.
    pub(crate) focused: bool,
    /// A narrow terminal: the duration gives its cells to the summary.
    pub(crate) compact: bool,
}

/// The summary row: a disclosure triangle (or the spinner while it runs), the summary in the
/// muted tone, and the failures and duration flush right, where a narrow terminal never cuts
/// them.
pub(crate) fn summary_row(s: &Summary, state: RowState) -> TextLine<'static> {
    let glyphs = g();
    // Focus reads the way a selected list row does: the caret, bold text and a raised fill, so it
    // still shows in 16-colour and monochrome terminals where the fill is not painted.
    let (mark, mark_style) = match state.spinner {
        _ if state.focused => (glyphs.prompt.to_string(), styles::brand().bold()),
        Some(tick) => (glyphs.spin(tick).to_string(), styles::spinner(tick)),
        None if state.expanded => (glyphs.disclose_open.to_string(), styles::muted()),
        None => (glyphs.disclose_closed.to_string(), styles::muted()),
    };
    let mut text = summary_text(s);
    if text.is_empty() {
        text = "Working".to_string();
    }
    let text_style = if state.focused {
        styles::fg().bold()
    } else {
        styles::muted()
    };
    let mut spans = rail::rail_spans(rail::BAKED);
    spans.push(Span::styled(mark, mark_style));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(text, text_style));
    spans.push(copy_mark::right_marker());
    let sep = format!(" {} ", glyphs.sep);
    let mut right: Vec<Span<'static>> = Vec::new();
    if state.focused {
        let toggle = if state.expanded { "collapse" } else { "expand" };
        let hints: &[(&str, &str)] = if state.compact {
            &[("Enter", toggle), ("Esc", "done")]
        } else {
            &[("Enter", toggle), (glyphs.up_down, "move"), ("Esc", "done")]
        };
        right.extend(styles::hint_row(hints));
    } else {
        if s.failed > 0 {
            right.push(Span::styled(format!("{} failed", s.failed), styles::err()));
        }
        if s.ms > 0 && state.spinner.is_none() && !state.compact {
            if !right.is_empty() {
                right.push(Span::styled(sep.clone(), styles::faint()));
            }
            right.push(Span::styled(tool_cards::fmt_ms(s.ms), styles::faint()));
        }
    }
    spans.extend(right);
    let line = TextLine::from(spans);
    if state.focused {
        line.style(styles::selected_row())
    } else {
        line
    }
}

/// A card row shown under a collapsed group (the running call, or a failure), indented two
/// cells so it reads as part of the group above it.
pub(crate) fn indented(mut row: TextLine<'static>) -> TextLine<'static> {
    let at = row.spans.len().min(3);
    row.spans
        .insert(at, Span::styled("  ", copy_mark::decor(Default::default())));
    row
}

/// [`indented`] for a row already fitted to the width: the right-alignment padding gives up the
/// two cells, so the right-hand status stays on screen.
pub(crate) fn indented_fitted(row: TextLine<'static>) -> TextLine<'static> {
    let mut row = indented(row);
    let pad = (4..row.spans.len()).rev().find(|&i| {
        let s = &row.spans[i];
        s.content.chars().count() >= 3
            && s.content.chars().all(|c| c == ' ')
            && copy_mark::is_decor(s.style.add_modifier)
    });
    if let Some(i) = pad {
        let n = row.spans[i].content.chars().count() - 2;
        row.spans[i].content = " ".repeat(n).into();
    }
    row
}

/// Whether a row was indented by [`indented`] (a card shown inside an open group).
pub(crate) fn is_indented(row: &TextLine<'_>) -> bool {
    row.spans
        .get(3)
        .is_some_and(|s| s.content.as_ref() == "  " && copy_mark::is_decor(s.style.add_modifier))
        && row.spans.first().is_some_and(copy_mark::is_rail)
}

/// A card header without its trailing disclosure triangle.
pub(crate) fn without_disclosure(mut row: TextLine<'static>) -> TextLine<'static> {
    let glyphs = g();
    let last = row.spans.last().map(|s| s.content.trim().to_string());
    if last.is_some_and(|t| t == glyphs.disclose_closed || t == glyphs.disclose_open) {
        row.spans.pop();
    }
    row
}

/// The failed calls a collapsed group keeps on screen, newest last, at most this many.
pub(crate) const FAILED_ROWS: usize = 3;

#[cfg(test)]
#[path = "tool_groups_tests.rs"]
mod tests;
