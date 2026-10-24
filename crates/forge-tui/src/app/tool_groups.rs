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

impl Category {
    /// Whether the summary counts what the calls touched rather than the calls: three reads of
    /// one file are "read 1 file", three edits to it "edited 1 file".
    fn by_subject(&self) -> bool {
        matches!(
            self,
            Self::Read
                | Self::List
                | Self::Search
                | Self::Created
                | Self::Edited
                | Self::Deleted
                | Self::Fetch
        )
    }
}

/// The calls of one category, in the order the category first appeared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tally {
    pub(crate) category: Category,
    /// Calls, or for an agents call the agents it spawned.
    pub(crate) calls: usize,
    /// The distinct files, directories, patterns or pages touched, for a category counted by
    /// what it touched.
    pub(crate) subjects: Vec<String>,
    /// Calls of such a category with nothing to tell them apart; each counts on its own.
    pub(crate) unnamed: usize,
    pub(crate) adds: usize,
    pub(crate) dels: usize,
    /// The target of the first call, for a tally of one (`Ran cargo test`).
    pub(crate) target: String,
}

impl Tally {
    fn new(category: Category, target: String) -> Self {
        Self {
            category,
            calls: 0,
            subjects: Vec::new(),
            unnamed: 0,
            adds: 0,
            dels: 0,
            target,
        }
    }

    /// The number the summary says.
    pub(crate) fn count(&self) -> usize {
        if self.category.by_subject() {
            self.subjects.len() + self.unnamed
        } else {
            self.calls
        }
    }

    fn touch(&mut self, subject: Option<String>) {
        match subject {
            Some(s) if self.subjects.contains(&s) => {}
            Some(s) => self.subjects.push(s),
            None => self.unnamed += 1,
        }
    }
}

/// Everything the summary row says about a group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    /// Calls that succeeded, by category.
    pub(crate) tallies: Vec<Tally>,
    /// Calls that failed, by category. They never count as done: the summary only falls back to
    /// them ("Tried to edit sample.rs") when nothing in the group succeeded.
    pub(crate) failed_tallies: Vec<Tally>,
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

/// What one call touched, so repeats of it count once: the path (`./src/a.rs` and `src/a.rs`
/// are the same file), the pattern searched for, the page fetched.
fn subject(category: &Category, args: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(args).ok()?;
    let get = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| value.get(*k).and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let found = match category {
        Category::Search => get(&["pattern", "query"])?.to_string(),
        Category::Fetch => get(&["url"])?.to_string(),
        _ => {
            let path = get(&["path", "file_path", "notebook_path"])?;
            let mut path = path;
            while let Some(rest) = path.strip_prefix("./") {
                path = rest;
            }
            path.to_string()
        }
    };
    Some(found)
}

/// How many agents an agents call spawned.
fn agents_spawned(args: &str) -> usize {
    serde_json::from_str::<Value>(args)
        .ok()
        .and_then(|v| {
            ["agents", "tasks", "items"]
                .iter()
                .find_map(|k| v.get(*k).and_then(Value::as_array).map(Vec::len))
        })
        .filter(|n| *n > 0)
        .unwrap_or(1)
}

#[cfg(test)]
thread_local! {
    /// How many times this thread built a summary, so a test can prove a rewrap reuses them.
    pub(crate) static SUMMARIZED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Fold the cards of a group into a summary. Running calls are counted but not tallied: the
/// summary speaks in the past tense, and the running call has a row of its own. File stats come
/// from each card's cached counts; nothing here diffs a file. A script that made calls is
/// counted by those calls, not as a script as well ("ran 3 scripts, ran 8 shell commands" for 8
/// commands); one that failed still counts, so its failure shows.
pub(crate) fn summarize<'a>(cards: impl IntoIterator<Item = &'a ToolCard>) -> Summary {
    #[cfg(test)]
    SUMMARIZED.with(|n| n.set(n.get() + 1));
    let cards: Vec<&ToolCard> = cards.into_iter().collect();
    let mut out = Summary::default();
    for &card in &cards {
        if card.status == CardStatus::Running {
            out.running += 1;
            continue;
        }
        let made_calls = cards.iter().any(|c| c.parent == Some(card.id));
        if card.status == CardStatus::Ok && made_calls {
            continue;
        }
        out.finished += 1;
        out.ms += card.ms.unwrap_or(0) + card.group.iter().filter_map(|m| m.ms).sum::<u128>();
        let failed = card.status == CardStatus::Failed;
        let tallies = if failed {
            out.failed += 1;
            &mut out.failed_tallies
        } else {
            &mut out.tallies
        };
        let category = category(card);
        let at = match tallies.iter().position(|t| t.category == category) {
            Some(at) => at,
            None => {
                let target = tool_cards::headline(&card.name, &card.args);
                tallies.push(Tally::new(category.clone(), target));
                tallies.len() - 1
            }
        };
        let tally = &mut tallies[at];
        match &category {
            Category::Agents => tally.calls += agents_spawned(&card.args),
            // A read card holds the earlier reads folded into it.
            Category::Read => {
                tally.calls += card.group.len() + 1;
                for args in card.group.iter().map(|m| m.args.as_str()) {
                    tally.touch(subject(&category, args));
                }
                tally.touch(subject(&category, &card.args));
            }
            c if c.by_subject() => {
                tally.calls += 1;
                let from_diff = card.diff.as_ref().map(|d| d.path.trim().to_string());
                tally.touch(subject(c, &card.args).or(from_diff));
            }
            _ => tally.calls += 1,
        }
        if !failed {
            tally.adds += card.diff_counts.0;
            tally.dels += card.diff_counts.1;
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

/// Past tense for what was done, the infinitive after "Tried to" for what failed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tense {
    Past,
    Tried,
}

/// The files a change tally touched: one or two are named, because a collapsed group is all the
/// transcript keeps of what the user just approved; more are counted.
fn changed_files(t: &Tally) -> String {
    if t.unnamed == 0 && (1..=2).contains(&t.subjects.len()) {
        t.subjects
            .iter()
            .map(|s| tool_cards::display_path(s, None))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        plural(t.count(), "file", "files")
    }
}

/// One category in words, counted: `ran 4 shell commands`, or `run 4 shell commands` after
/// "Tried to".
fn counted(t: &Tally, tense: Tense) -> String {
    let n = t.count();
    let verb = |past: &str, inf: &str| -> String {
        if tense == Tense::Past {
            past.to_string()
        } else {
            inf.to_string()
        }
    };
    match &t.category {
        Category::Read => format!("{} {}", verb("read", "read"), plural(n, "file", "files")),
        Category::List => format!(
            "{} {}",
            verb("listed", "list"),
            plural(n, "directory", "directories")
        ),
        Category::Search => format!(
            "{} {}",
            verb("searched for", "search for"),
            plural(n, "pattern", "patterns")
        ),
        Category::Shell => format!(
            "{} {}",
            verb("ran", "run"),
            plural(n, "shell command", "shell commands")
        ),
        Category::Script => format!("{} {}", verb("ran", "run"), plural(n, "script", "scripts")),
        Category::Created => format!(
            "{} {}{}",
            verb("created", "create"),
            changed_files(t),
            stat(t)
        ),
        Category::Edited => format!("{} {}{}", verb("edited", "edit"), changed_files(t), stat(t)),
        Category::Deleted => format!(
            "{} {}{}",
            verb("deleted", "delete"),
            changed_files(t),
            stat(t)
        ),
        Category::Fetch => format!(
            "{} {}",
            verb("fetched", "fetch"),
            plural(n, "page", "pages")
        ),
        Category::WebSearch => format!(
            "{} {}",
            verb("ran", "run"),
            plural(n, "web search", "web searches")
        ),
        Category::Agents => format!("{} {}", verb("ran", "run"), plural(n, "agent", "agents")),
        Category::Tasks if tense == Tense::Tried => "update the task list".to_string(),
        Category::Tasks if n == 1 => "updated the task list".to_string(),
        Category::Tasks => format!("updated the task list {n} times"),
        Category::Mcp(name) | Category::Other(name) => {
            format!(
                "{} {name} {}",
                verb("called", "call"),
                plural(n, "time", "times")
            )
        }
    }
}

/// A tally of one thing, by what it touched: `ran cargo test -p upload`, `read src/big.rs` for
/// three slices of that file. `None` when there is no target worth naming, so the counted form
/// is used instead.
fn single(t: &Tally, tense: Tense) -> Option<String> {
    let target = t.target.trim();
    if target.is_empty() || t.count() != 1 {
        return None;
    }
    let past = tense == Tense::Past;
    let (done, tried) = match &t.category {
        Category::Read => ("read", "read"),
        Category::List => ("listed", "list"),
        Category::Search => ("searched for", "search for"),
        Category::Shell => ("ran", "run"),
        Category::Created => ("created", "create"),
        Category::Edited => ("edited", "edit"),
        Category::Deleted => ("deleted", "delete"),
        Category::Fetch => ("fetched", "fetch"),
        Category::WebSearch => ("searched the web for", "search the web for"),
        _ => return None,
    };
    let verb = if past { done } else { tried };
    let stat = match t.category {
        Category::Created | Category::Edited => stat(t),
        _ => String::new(),
    };
    Some(format!("{verb} {target}{stat}"))
}

/// The tallies the summary speaks of, and how: what succeeded, or what was tried when nothing
/// did.
fn spoken(s: &Summary) -> (&[Tally], Tense) {
    if s.tallies.is_empty() {
        (&s.failed_tallies, Tense::Tried)
    } else {
        (&s.tallies, Tense::Past)
    }
}

/// The summary in natural English, categories in order of first appearance, the first word
/// capitalised: `Read 3 files, searched for 2 patterns, ran 4 shell commands`. Failed calls are
/// left out (the row counts them in red); a group where nothing succeeded says what it tried.
pub(crate) fn summary_text(s: &Summary) -> String {
    sentence(s, true)
}

/// The sentence; `name` lets a tally of one name what it touched. An open group's header
/// counts instead, because the call itself is listed right under it.
fn sentence(s: &Summary, name: bool) -> String {
    let (tallies, tense) = spoken(s);
    let text = match tallies {
        [] => String::new(),
        [only] if name => single(only, tense).unwrap_or_else(|| counted(only, tense)),
        [only] => counted(only, tense),
        many => many
            .iter()
            .map(|t| counted(t, tense))
            .collect::<Vec<_>>()
            .join(", "),
    };
    let text = if tense == Tense::Tried && !text.is_empty() {
        format!("tried to {text}")
    } else {
        text
    };
    capitalise(&text)
}

/// One category as a short count, for a terminal too narrow for the sentence: `4 commands`.
fn short(t: &Tally) -> String {
    let n = t.count();
    let stat = match t.category {
        Category::Created | Category::Edited | Category::Deleted => stat(t),
        _ => String::new(),
    };
    match &t.category {
        Category::Read => plural(n, "file read", "files read"),
        Category::List => plural(n, "dir", "dirs"),
        Category::Search => plural(n, "search", "searches"),
        Category::Shell => plural(n, "command", "commands"),
        Category::Script => plural(n, "script", "scripts"),
        Category::Created => format!("{}{stat}", plural(n, "new file", "new files")),
        Category::Edited => format!("{}{stat}", plural(n, "file edited", "files edited")),
        Category::Deleted => format!("{}{stat}", plural(n, "file deleted", "files deleted")),
        Category::Fetch => plural(n, "page", "pages"),
        Category::WebSearch => plural(n, "web search", "web searches"),
        Category::Agents => plural(n, "agent", "agents"),
        Category::Tasks => "task list".to_string(),
        Category::Mcp(name) | Category::Other(name) => format!("{name} ×{n}"),
    }
}

/// The summary as counts only (`1 dir, 2 searches, 4 commands`), for a row too narrow
/// for [`summary_text`]. A single named call keeps its sentence, which is short already.
pub(crate) fn summary_text_short(s: &Summary) -> String {
    let (tallies, tense) = spoken(s);
    match tallies {
        [only] if single(only, tense).is_some() => summary_text(s),
        many => many.iter().map(short).collect::<Vec<_>>().join(", "),
    }
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// How a summary row is drawn.
#[derive(Debug, Clone, Default)]
pub(crate) struct RowState {
    pub(crate) expanded: bool,
    /// The animation tick while the group still has a call running and is collapsed.
    pub(crate) spinner: Option<usize>,
    /// Keyboard focus is on this group.
    pub(crate) focused: bool,
    /// A narrow terminal: the duration gives its cells to the summary.
    pub(crate) compact: bool,
    /// The cells the row may use; 0 for no limit. A summary that does not fit switches to its
    /// short form before anything is cut.
    pub(crate) width: usize,
    /// How to open the group (`click or Alt+O to expand`), shown faint after the summary when it
    /// fits whole.
    pub(crate) hint: Option<String>,
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
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
    let text_style = if state.focused {
        styles::fg().bold()
    } else {
        styles::muted()
    };
    let sep = format!(" {} ", glyphs.sep);
    // The failure count stays on the right in every state, focus included.
    let mut right: Vec<Span<'static>> = Vec::new();
    if s.failed > 0 {
        right.push(Span::styled(format!("{} failed", s.failed), styles::err()));
    }
    if state.focused {
        let toggle = if state.expanded { "collapse" } else { "expand" };
        let hints: &[(&str, &str)] = if state.compact {
            &[("Enter", toggle), ("Esc", "done")]
        } else {
            &[("Enter", toggle), (glyphs.up_down, "move"), ("Esc", "done")]
        };
        if !right.is_empty() {
            right.push(Span::styled(sep.clone(), styles::faint()));
        }
        right.extend(styles::hint_row(hints));
    } else if s.ms > 0 && state.spinner.is_none() && !state.compact {
        if !right.is_empty() {
            right.push(Span::styled(sep.clone(), styles::faint()));
        }
        right.push(Span::styled(tool_cards::fmt_ms(s.ms), styles::faint()));
    }
    let mut spans = rail::rail_spans(rail::BAKED);
    spans.push(Span::styled(mark, mark_style));
    spans.push(Span::raw(" "));
    // What the summary may use: the row less its head, the gap before the right-hand side, and
    // that side.
    let room = match state.width {
        0 => usize::MAX,
        w => w.saturating_sub(spans_width(&spans) + 2 + spans_width(&right)),
    };
    let mut text = sentence(s, !state.expanded);
    if text.is_empty() {
        text = "Working".to_string();
    }
    if Span::raw(text.as_str()).width() > room {
        let short = summary_text_short(s);
        if !short.is_empty() {
            text = short;
        }
    }
    let used = Span::raw(text.as_str()).width();
    spans.push(Span::styled(text, text_style));
    if let Some(hint) = state
        .hint
        .filter(|_| !state.focused && state.spinner.is_none())
    {
        let hint = format!("  {hint}");
        if used + Span::raw(hint.as_str()).width() <= room {
            spans.push(Span::styled(hint, styles::faint()));
        }
    }
    spans.push(copy_mark::right_marker());
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

/// The failed calls a collapsed group keeps on screen, newest last, at most this many.
pub(crate) const FAILED_ROWS: usize = 3;

#[cfg(test)]
#[path = "tool_groups_tests.rs"]
mod tests;
