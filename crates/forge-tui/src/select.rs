//! A standalone, animated full-screen multi-select list — used by `forge mcp import` to pick
//! which discovered servers to import. Self-contained (its own alt-screen + raw-mode lifecycle)
//! so it works as a one-shot CLI prompt, independent of the chat TUI's inline viewport.
//!
//! Cross-platform: crossterm drives the terminal on Linux, macOS, and Windows alike.

use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::app::chrome_hint as hint;
use crate::app::list_view::{self, Item, Row, View};
use crate::theme::glyphs::g;
use crate::theme::styles;

/// One selectable row: a primary `label` and a dim `hint` (e.g. transport + source).
#[derive(Debug, Clone)]
pub struct SelectItem {
    pub label: String,
    pub hint: String,
    /// Whether the row starts checked.
    pub preselected: bool,
}

/// Run the animated multi-select. Returns the indices the user confirmed (Enter), or `None` if
/// they cancelled (Esc / `q` / Ctrl-C). Restores the terminal on every exit path.
pub fn select_multi(title: &str, items: &[SelectItem]) -> io::Result<Option<Vec<usize>>> {
    if items.is_empty() {
        return Ok(Some(vec![]));
    }
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
    )?;

    let mut state = State {
        cursor: 0,
        checked: items.iter().map(|i| i.preselected).collect(),
    };

    let result = run_loop(&mut terminal, title, items, &mut state);

    // Always restore, even on error.
    disable_raw_mode().ok();
    terminal.backend_mut().execute(LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    result
}

/// Run an animated single-select menu (same look as [`select_multi`], no checkboxes). Returns the
/// chosen index, or `None` on cancel (Esc / `q` / Ctrl-C). Restores the terminal on every path.
pub fn select_one(title: &str, items: &[SelectItem]) -> io::Result<Option<usize>> {
    if items.is_empty() {
        return Ok(None);
    }
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
    )?;
    let mut cursor = 0usize;
    let result = loop {
        terminal.draw(|f| draw_single(f, title, items, cursor))?;
        if event::poll(Duration::from_millis(80))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => break Ok(None),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        break Ok(None)
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        cursor = cursor.checked_sub(1).unwrap_or(items.len() - 1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => cursor = (cursor + 1) % items.len(),
                    KeyCode::Home => cursor = 0,
                    KeyCode::End => cursor = items.len() - 1,
                    KeyCode::Enter => break Ok(Some(cursor)),
                    _ => {}
                }
            }
        }
    };
    disable_raw_mode().ok();
    terminal.backend_mut().execute(LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    result
}

fn rows(items: &[SelectItem], checked: Option<&[bool]>) -> Vec<Item> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let label = match checked {
                Some(c) if c[i] => format!("{} {}", g().ok, item.label),
                Some(_) => format!("{} {}", g().task_open, item.label),
                None => item.label.clone(),
            };
            Item::Row(Row {
                label,
                label_style: Some(match checked {
                    Some(c) if c[i] => styles::fg(),
                    Some(_) => styles::muted(),
                    None => styles::fg(),
                }),
                desc: item.hint.clone(),
                ..Row::default()
            })
        })
        .collect()
}

fn draw_single(f: &mut ratatui::Frame, title: &str, items: &[SelectItem], cursor: usize) {
    let view = View {
        title: title.to_string(),
        context: format!("{} of {}", cursor + 1, items.len()),
        items: rows(items, None),
        selected: cursor,
        footer: vec![
            hint("↑↓", "move"),
            hint("Enter", "select"),
            hint("Esc", "cancel"),
        ],
        padded: true,
        ..View::default()
    };
    list_view::render(f, f.area(), &view);
}

struct State {
    cursor: usize,
    checked: Vec<bool>,
}

fn run_loop(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<Stdout>>,
    title: &str,
    items: &[SelectItem],
    state: &mut State,
) -> io::Result<Option<Vec<usize>>> {
    loop {
        terminal.draw(|f| draw(f, title, items, state))?;
        // ~12fps: poll keeps the accent animation ticking even with no input.
        if event::poll(Duration::from_millis(80))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(None)
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        state.cursor = state.cursor.checked_sub(1).unwrap_or(items.len() - 1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        state.cursor = (state.cursor + 1) % items.len();
                    }
                    KeyCode::Home => state.cursor = 0,
                    KeyCode::End => state.cursor = items.len() - 1,
                    KeyCode::Char(' ') => {
                        let c = &mut state.checked[state.cursor];
                        *c = !*c;
                    }
                    KeyCode::Char('a') => state.checked.iter_mut().for_each(|c| *c = true),
                    KeyCode::Char('n') => state.checked.iter_mut().for_each(|c| *c = false),
                    KeyCode::Enter => {
                        let chosen: Vec<usize> = state
                            .checked
                            .iter()
                            .enumerate()
                            .filter_map(|(i, &c)| c.then_some(i))
                            .collect();
                        return Ok(Some(chosen));
                    }
                    _ => {}
                }
            }
        }
    }
}

fn draw(f: &mut ratatui::Frame, title: &str, items: &[SelectItem], state: &State) {
    let n = state.checked.iter().filter(|&&c| c).count();
    let view = View {
        title: title.to_string(),
        context: format!("{n}/{} selected", items.len()),
        items: rows(items, Some(&state.checked)),
        selected: state.cursor,
        footer: vec![
            hint("↑↓", "move"),
            hint("Space", "toggle"),
            hint("a/n", "all/none"),
            hint("Enter", "import"),
            hint("Esc", "cancel"),
        ],
        padded: true,
        ..View::default()
    };
    list_view::render(f, f.area(), &view);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn items() -> Vec<SelectItem> {
        vec![
            SelectItem {
                label: "filesystem".into(),
                hint: "stdio · claude".into(),
                preselected: true,
            },
            SelectItem {
                label: "github".into(),
                hint: "http · cursor".into(),
                preselected: false,
            },
        ]
    }

    fn screen(draw_fn: impl FnOnce(&mut ratatui::Frame)) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(draw_fn).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..12u16)
            .map(|y| {
                (0..80u16)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_single_select_uses_the_shared_list() {
        let out = screen(|f| draw_single(f, "Pick one", &items(), 1));
        assert!(out.contains("Pick one") && out.contains("2 of 2"), "{out}");
        assert!(out.contains("❯ github"), "{out}");
        assert!(
            out.contains("Enter select") && out.contains("Esc cancel"),
            "{out}"
        );
    }

    #[test]
    fn the_multi_select_marks_checked_rows_and_counts_them() {
        let state = State {
            cursor: 0,
            checked: vec![true, false],
        };
        let out = screen(|f| draw(f, "Import", &items(), &state));
        assert!(out.contains("1/2 selected"), "{out}");
        assert!(
            out.contains("✓ filesystem") && out.contains("○ github"),
            "{out}"
        );
        assert!(
            out.contains("Space toggle") && out.contains("a/n all/none"),
            "{out}"
        );
    }
}
