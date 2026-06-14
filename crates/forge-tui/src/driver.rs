//! The animated-TUI driver pieces. `ChannelPresenter` forwards a session's presenter
//! events over a channel (so a turn can run on a background task), and `Tui` owns the
//! terminal for the render loop. The actual loop lives in the binary (it owns the
//! `Session`, which this crate must not depend on).

use std::io::{self, Stdout};
use std::sync::mpsc::Sender;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use forge_types::SideEffect;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::{self, App, KeyKind};
use crate::{Presenter, PresenterEvent};

/// A message from a running turn to the render loop.
pub enum UiMsg {
    Event(PresenterEvent),
    Permission {
        tool: String,
        side_effect: SideEffect,
        reply: Sender<bool>,
    },
}

/// A presenter that forwards everything over a channel; safe to move onto a task.
pub struct ChannelPresenter {
    tx: Sender<UiMsg>,
}

impl ChannelPresenter {
    pub fn new(tx: Sender<UiMsg>) -> Self {
        Self { tx }
    }
}

impl Presenter for ChannelPresenter {
    fn emit(&mut self, event: PresenterEvent) {
        let _ = self.tx.send(UiMsg::Event(event));
    }

    fn confirm(&mut self, tool: &str, side_effect: SideEffect) -> bool {
        let (reply, answer) = std::sync::mpsc::channel();
        if self
            .tx
            .send(UiMsg::Permission {
                tool: tool.to_string(),
                side_effect,
                reply,
            })
            .is_err()
        {
            return false;
        }
        answer.recv().unwrap_or(false) // blocks this turn task until the loop answers
    }

    fn read_line(&mut self) -> Option<String> {
        None // input is handled by the render loop, not the presenter
    }
}

/// Owns the terminal for the render loop (raw mode + alternate screen).
pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    pub fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen)?;
        Ok(Self {
            terminal: Terminal::new(CrosstermBackend::new(out))?,
        })
    }

    pub fn draw(&mut self, app: &App) {
        let _ = self.terminal.draw(|f| app::render(f, app));
    }

    /// Non-blocking: returns a keystroke if one is pending, else `None`.
    pub fn poll_key(&self) -> io::Result<Option<KeyKind>> {
        if !event::poll(Duration::from_millis(0))? {
            return Ok(None);
        }
        if let Event::Key(k) = event::read()? {
            if k.kind == KeyEventKind::Press {
                let key = match k.code {
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        KeyKind::Esc
                    }
                    KeyCode::Char(c) => KeyKind::Char(c),
                    KeyCode::Backspace => KeyKind::Backspace,
                    KeyCode::Enter => KeyKind::Enter,
                    KeyCode::Esc => KeyKind::Esc,
                    _ => return Ok(None),
                };
                return Ok(Some(key));
            }
        }
        Ok(None)
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}
