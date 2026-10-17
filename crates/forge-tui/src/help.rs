//! The Help takeover: one screen with Commands, Keys and Mouse sections and a search field.
//!
//! One row per command (its usage shows on the detail line for the selected row), the keys table
//! generated from the same registry the keybind configurator edits plus the fixed keys, one
//! canonical spelling everywhere. At 120 columns and wider, commands and keys sit side by side.
//! Enter on a command inserts it into the composer.

use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use forge_config::KeybindsConfig;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::Terminal;

use crate::app::list_view::{self, match_positions, Filter, Item, Row, View};
use crate::commands::{command_action, COMMANDS};
use crate::keybind_configurator::{action_desc, all_actions};
use crate::keybinds::combo_display;
use crate::surface::{self, SurfaceTone};
use crate::theme::glyphs::g;
use crate::theme::styles;

/// The section opened initially; `Tab` cycles through all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTab {
    Commands,
    Keybinds,
    Mouse,
}

impl HelpTab {
    fn next(self, back: bool) -> Self {
        match (self, back) {
            (Self::Commands, false) | (Self::Mouse, true) => Self::Keybinds,
            (Self::Keybinds, false) | (Self::Commands, true) => Self::Mouse,
            (Self::Mouse, false) | (Self::Keybinds, true) => Self::Commands,
        }
    }
}

const SECTIONS: [&str; 3] = ["Commands", "Keys", "Mouse"];

const GROUP_ORDER: [&str; 5] = [
    "Session",
    "History",
    "Models & routing",
    "Work",
    "Project & tools",
];

fn command_group(name: &str) -> &'static str {
    match name {
        "help" | "keys" | "config" | "mode" | "thinking" | "effort" | "statusline" | "status"
        | "heartbeat" => "Session",
        "sessions" | "replay" | "resume" | "new" | "undo" | "checkpoint" | "checkpoints"
        | "compact" | "uncompact" | "reground" | "refine" | "rewind" | "clear" | "export"
        | "btw" => "History",
        "model" | "models" | "usage" | "mesh" | "duel" | "subagents" => "Models & routing",
        "plan" | "execute" | "goal" | "pr" | "loop" | "workflow" | "assay" | "lattice"
        | "commit" | "dispatch" => "Work",
        _ => "Project & tools",
    }
}

/// Keys that are not rebindable, in canonical spelling.
const FIXED_KEYS: [(&str, &str); 12] = [
    ("Enter", "Send the message (queues it while a turn runs)"),
    ("Tab", "Accept the suggestion or the selected completion"),
    ("Esc", "Stop the turn, or close what is open"),
    ("Esc Esc", "Rewind to an earlier message (idle)"),
    ("Ctrl+T", "Expand or collapse the latest tool call"),
    ("Ctrl+O", "Show or hide task and agent details"),
    ("Ctrl+J", "Insert a newline (also Shift+Enter, Alt+Enter)"),
    ("Ctrl+A", "Start of line (Ctrl+E end of line)"),
    (
        "Ctrl+W",
        "Delete the word before the cursor (Ctrl+U the line)",
    ),
    ("Alt+B", "Move by word (Alt+F forward, also Alt+←, Alt+→)"),
    ("Shift+Tab", "Cycle the mode"),
    (
        "Ctrl+Home",
        "Jump to the previous message (Ctrl+End the bottom)",
    ),
];

const MOUSE_ROWS: [(&str, &str); 4] = [
    ("Wheel", "Scroll the transcript"),
    (
        "Click",
        "A tool call: expand or collapse its output (same as Ctrl+T)",
    ),
    ("Drag", "Select text; releasing the button copies it"),
    (
        "Click",
        "The jump bar: go to the latest or the previous message",
    ),
];

/// What a key did to the help screen.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HelpOutcome {
    Open,
    Close,
    /// Insert this text into the composer and close.
    Insert(String),
}

pub(crate) struct HelpState {
    keybinds: KeybindsConfig,
    pub tab: HelpTab,
    pub query: String,
    pub selected: usize,
}

fn matches_query(query: &str, parts: &[&str]) -> bool {
    let q = query.trim().trim_start_matches('/').to_lowercase();
    q.is_empty() || parts.iter().any(|p| p.to_lowercase().contains(&q))
}

impl HelpState {
    pub fn new(keybinds: &KeybindsConfig, tab: HelpTab) -> Self {
        Self {
            keybinds: keybinds.clone(),
            tab,
            query: String::new(),
            selected: 0,
        }
    }

    fn key_label(&self, action: &str) -> String {
        self.keybinds
            .binds
            .get(action)
            .map(combo_display)
            .unwrap_or_else(|| "(unset)".to_string())
    }

    /// Command rows, grouped when nothing is being searched. Enter inserts the second element.
    fn command_items(&self) -> (Vec<Item>, Vec<String>) {
        let mut ordered: Vec<_> = COMMANDS.iter().collect();
        let rank = |name: &str| {
            GROUP_ORDER
                .iter()
                .position(|g| *g == command_group(name))
                .unwrap_or(GROUP_ORDER.len())
        };
        let q = self.query.trim().trim_start_matches('/').to_lowercase();
        // Grouped when browsing; best match first (prefix before substring) when searching.
        ordered.sort_by_key(|c| {
            if q.is_empty() {
                (rank(c.name), 0)
            } else {
                (0, usize::from(!c.name.starts_with(&q)))
            }
        });
        let mut items = Vec::new();
        let mut inserts = Vec::new();
        let mut last = "";
        for c in ordered {
            if !matches_query(&self.query, &[c.name, c.desc]) {
                continue;
            }
            let group = command_group(c.name);
            if self.query.is_empty() && group != last {
                items.push(Item::Header(group.to_string()));
                last = group;
            }
            items.push(Item::Row(Row {
                label: format!("/{}", c.name),
                label_style: Some(styles::user()),
                desc: c.desc.to_string(),
                meta: command_action(c.name)
                    .and_then(|a| self.keybinds.binds.get(a))
                    .map(combo_display)
                    .unwrap_or_default(),
                marks: match_positions(c.name, &self.query)
                    .into_iter()
                    .map(|i| i + 1)
                    .collect(),
                ..Row::default()
            }));
            let args = c.usage.contains('<') || c.usage.contains('[');
            inserts.push(format!("/{}{}", c.name, if args { " " } else { "" }));
        }
        (items, inserts)
    }

    fn key_items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        let mut actions: Vec<Row> = Vec::new();
        for action in all_actions() {
            let desc = action_desc(action);
            let label = self.key_label(action);
            if !matches_query(&self.query, &[&label, desc]) {
                continue;
            }
            actions.push(Row {
                label,
                label_style: Some(styles::key_chip()),
                desc: desc.to_string(),
                ..Row::default()
            });
        }
        let fixed: Vec<Row> = FIXED_KEYS
            .iter()
            .filter(|(k, d)| matches_query(&self.query, &[k, d]))
            .map(|(k, d)| Row {
                label: k.to_string(),
                label_style: Some(styles::key_chip()),
                desc: d.to_string(),
                ..Row::default()
            })
            .collect();
        if !actions.is_empty() {
            items.push(Item::Header("Shortcuts you can change".to_string()));
            items.extend(actions.into_iter().map(Item::Row));
        }
        if !fixed.is_empty() {
            items.push(Item::Header("Fixed keys".to_string()));
            items.extend(fixed.into_iter().map(Item::Row));
        }
        items
    }

    fn mouse_items(&self) -> Vec<Item> {
        MOUSE_ROWS
            .iter()
            .filter(|(k, d)| matches_query(&self.query, &[k, d]))
            .map(|(k, d)| {
                Item::Row(Row {
                    label: k.to_string(),
                    label_style: Some(styles::key_chip()),
                    desc: d.to_string(),
                    ..Row::default()
                })
            })
            .collect()
    }

    fn selectable_rows(&self) -> usize {
        match self.tab {
            HelpTab::Commands => self.command_items().1.len(),
            HelpTab::Keybinds => self
                .key_items()
                .iter()
                .filter(|i| matches!(i, Item::Row(_)))
                .count(),
            HelpTab::Mouse => self.mouse_items().len(),
        }
    }

    pub fn key(&mut self, code: KeyCode, mods: KeyModifiers) -> HelpOutcome {
        let last = self.selectable_rows().saturating_sub(1);
        match code {
            KeyCode::Esc => return HelpOutcome::Close,
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => {
                return HelpOutcome::Close
            }
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => self.selected = (self.selected + 1).min(last),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(8),
            KeyCode::PageDown => self.selected = (self.selected + 8).min(last),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = last,
            KeyCode::Tab | KeyCode::BackTab => {
                self.tab = self.tab.next(code == KeyCode::BackTab);
                self.selected = 0;
            }
            KeyCode::Enter if self.tab == HelpTab::Commands => {
                if let Some(text) = self.command_items().1.get(self.selected) {
                    return HelpOutcome::Insert(text.clone());
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.selected = 0;
            }
            KeyCode::Char(c) if !c.is_control() && !mods.contains(KeyModifiers::CONTROL) => {
                self.query.push(c);
                self.selected = 0;
            }
            _ => {}
        }
        HelpOutcome::Open
    }

    fn selected_detail(&self) -> Option<Vec<Span<'static>>> {
        if self.tab != HelpTab::Commands {
            return None;
        }
        let (_, inserts) = self.command_items();
        let name = inserts
            .get(self.selected)?
            .trim()
            .trim_start_matches('/')
            .to_string();
        let c = COMMANDS.iter().find(|c| c.name == name)?;
        Some(vec![
            Span::styled("usage  ", styles::muted()),
            Span::styled(c.usage.to_string(), styles::fg()),
        ])
    }

    /// The frames to draw for `width` columns: one list, or commands beside keys when wide.
    pub fn views(&self, width: u16) -> Vec<View> {
        let help_key = self.key_label("help");
        let footer = vec![
            crate::app::chrome_hint("Tab", "section"),
            crate::app::chrome_hint("↑↓", "move"),
            crate::app::chrome_hint("type", "to search"),
            crate::app::chrome_hint("Enter", "insert"),
            crate::app::chrome_hint("Esc", "close"),
        ];
        let tabs = (
            SECTIONS.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            match self.tab {
                HelpTab::Commands => 0,
                HelpTab::Keybinds => 1,
                HelpTab::Mouse => 2,
            },
        );
        let filter = Filter {
            label: "search",
            query: self.query.clone(),
            placeholder: "type to filter",
        };
        let wide = width >= 120 && self.tab != HelpTab::Mouse;
        let base = |title: &str, items: Vec<Item>, selected: usize| View {
            title: title.to_string(),
            items,
            selected,
            padded: true,
            tone: SurfaceTone::Tool,
            empty: "nothing matches".to_string(),
            more_hint: "",
            ..View::default()
        };
        if wide {
            let commands_focus = self.tab == HelpTab::Commands;
            let mut left = base(
                "Help",
                self.command_items().0,
                if commands_focus {
                    self.selected
                } else {
                    usize::MAX
                },
            );
            left.context = format!("{help_key} {} ?", g().sep);
            left.filter = Some(filter);
            left.tabs = Some(tabs);
            left.detail = self.selected_detail();
            left.footer = footer;
            let right = base(
                "Keys",
                self.key_items(),
                if commands_focus {
                    usize::MAX
                } else {
                    self.selected
                },
            );
            return vec![left, right];
        }
        let (title, items) = match self.tab {
            HelpTab::Commands => ("Help", self.command_items().0),
            HelpTab::Keybinds => ("Help", self.key_items()),
            HelpTab::Mouse => ("Help", self.mouse_items()),
        };
        let mut view = base(title, items, self.selected);
        view.context = format!("{help_key} {} ?", g().sep);
        view.filter = Some(filter);
        view.tabs = Some(tabs);
        view.detail = self.selected_detail();
        view.footer = footer;
        vec![view]
    }
}

fn draw(frame: &mut ratatui::Frame, state: &HelpState) {
    let area = frame.area();
    surface::render_backdrop(frame, area);
    let views = state.views(area.width);
    match views.as_slice() {
        [one] => {
            list_view::render(frame, area, one);
        }
        [left, right] => {
            let half = area.width / 2;
            list_view::render(
                frame,
                Rect {
                    width: half,
                    ..area
                },
                left,
            );
            list_view::render(
                frame,
                Rect {
                    x: area.x + half,
                    width: area.width - half,
                    ..area
                },
                right,
            );
        }
        _ => {}
    }
}

/// Run the Help takeover. `Ok(Some(text))` is a command the user chose to insert.
pub fn run_help_pick(
    keybinds: &KeybindsConfig,
    initial_tab: HelpTab,
) -> io::Result<Option<String>> {
    enable_raw_mode()?;
    crossterm::execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    let mut state = HelpState::new(keybinds, initial_tab);
    let picked = loop {
        terminal.draw(|frame| draw(frame, &state))?;
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match state.key(key.code, key.modifiers) {
                HelpOutcome::Open => {}
                HelpOutcome::Close => break None,
                HelpOutcome::Insert(text) => break Some(text),
            }
        }
    };
    disable_raw_mode()?;
    crossterm::execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(picked)
}

/// Run the Help takeover, discarding any insertion (kept for callers without a composer).
pub fn run_help(keybinds: &KeybindsConfig, initial_tab: HelpTab) -> io::Result<()> {
    run_help_pick(keybinds, initial_tab).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn state(tab: HelpTab) -> HelpState {
        HelpState::new(&KeybindsConfig::default(), tab)
    }

    fn rows_of(view: &View) -> Vec<String> {
        view.rows().map(|r| r.label.clone()).collect()
    }

    fn screen(state: &HelpState, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw(f, state)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_registered_command_has_a_help_row() {
        let s = state(HelpTab::Commands);
        let rows = rows_of(&s.views(80)[0]);
        for c in COMMANDS {
            assert!(
                rows.contains(&format!("/{}", c.name)),
                "/{} missing",
                c.name
            );
        }
    }

    #[test]
    fn every_bound_action_is_listed_once_with_its_canonical_label() {
        let s = state(HelpTab::Keybinds);
        let rows = rows_of(&s.views(80)[0]);
        let defaults = KeybindsConfig::default();
        for action in all_actions() {
            let combo = defaults.binds.get(action).expect("default bind");
            let label = combo_display(combo);
            let count = rows.iter().filter(|r| **r == label).count();
            assert!(count >= 1, "{action} ({label}) missing");
        }
        for (key, _) in FIXED_KEYS {
            assert!(rows.contains(&key.to_string()), "{key} missing");
        }
        assert!(
            rows.iter()
                .all(|r| !r.contains('-') || r.contains('/') || r.len() < 2),
            "no Ctrl-c spellings: {rows:?}"
        );
        assert!(rows.contains(&"Ctrl+K".to_string()) && rows.contains(&"Alt+M".to_string()));
    }

    #[test]
    fn search_filters_and_enter_inserts_the_command() {
        let mut s = state(HelpTab::Commands);
        for c in "compa".chars() {
            s.key(KeyCode::Char(c), KeyModifiers::NONE);
        }
        let rows = rows_of(&s.views(80)[0]);
        assert_eq!(rows[0], "/compact", "prefix match first: {rows:?}");
        match s.key(KeyCode::Enter, KeyModifiers::NONE) {
            HelpOutcome::Insert(text) => assert_eq!(text.trim(), "/compact"),
            other => panic!("{other:?}"),
        }
        assert_eq!(s.key(KeyCode::Esc, KeyModifiers::NONE), HelpOutcome::Close);
    }

    #[test]
    fn a_command_that_takes_arguments_is_inserted_with_a_trailing_space() {
        let mut s = state(HelpTab::Commands);
        for c in "loop".chars() {
            s.key(KeyCode::Char(c), KeyModifiers::NONE);
        }
        match s.key(KeyCode::Enter, KeyModifiers::NONE) {
            HelpOutcome::Insert(text) => assert!(text.starts_with("/loop"), "{text}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tab_cycles_the_three_sections_and_wide_screens_show_two_columns() {
        let mut s = state(HelpTab::Commands);
        s.key(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(s.tab, HelpTab::Keybinds);
        s.key(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(s.tab, HelpTab::Mouse);
        assert_eq!(s.views(160).len(), 1, "mouse is a single column");
        s.key(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(s.tab, HelpTab::Commands);
        assert_eq!(s.views(80).len(), 1);
        assert_eq!(s.views(160).len(), 2);
        let wide = screen(&s, 160, 36);
        assert!(wide.contains("/help") && wide.contains("Ctrl+K"), "{wide}");
    }

    #[test]
    fn the_80x24_screen_has_sections_search_and_a_usage_line() {
        let s = state(HelpTab::Commands);
        let out = screen(&s, 80, 24);
        assert!(out.contains("Commands") && out.contains("Keys") && out.contains("Mouse"));
        assert!(out.contains("search"), "{out}");
        assert!(out.contains("usage  /"), "{out}");
        assert!(out.contains("Esc close"), "{out}");
    }
}
