//! `TuiPresenter`: the interactive ratatui+crossterm renderer. It owns the terminal,
//! folds each [`PresenterEvent`] into [`app::App`], and repaints — so the UI updates live
//! as a turn progresses. `confirm` shows a permission prompt and blocks on a key. All the
//! rendering logic lives in `app` (pure, TestBackend-tested); this module is the I/O shell.

use std::io::{self, Stdout};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use forge_types::SideEffect;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::{self, App};
use crate::{Presenter, PresenterEvent};

pub struct TuiPresenter {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    app: App,
}

impl TuiPresenter {
    /// Enter raw mode + the alternate screen and take over the terminal.
    pub fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        // From here on, any failure must undo raw mode — Drop won't run because the
        // struct isn't constructed yet, which would otherwise leave the shell broken.
        Self::enter().inspect_err(|_| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
        })
    }

    fn enter() -> io::Result<Self> {
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(out))?;
        Ok(Self {
            terminal,
            app: App::default(),
        })
    }

    fn draw(&mut self) {
        let app = &self.app;
        let _ = self.terminal.draw(|f| app::render(f, app));
    }

    fn wait_for_quit(&self) {
        loop {
            match event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc | KeyCode::Enter => {
                        break
                    }
                    _ => {}
                },
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }

    fn restore(&mut self) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for TuiPresenter {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

impl Presenter for TuiPresenter {
    fn emit(&mut self, event: PresenterEvent) {
        let is_done = matches!(event, PresenterEvent::Done { .. });
        self.app.apply(event);
        self.draw();
        // Hold the final frame so the user can read it, then let the turn return.
        if is_done {
            self.wait_for_quit();
        }
    }

    fn confirm(&mut self, tool: &str, side_effect: SideEffect) -> bool {
        self.app.prompt = Some(format!("allow {tool} ({side_effect:?})?"));
        self.draw();

        let allowed = loop {
            match event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => break true,
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => break false,
                    _ => {}
                },
                Ok(_) => {}
                Err(_) => break false, // can't read input -> deny (safe)
            }
        };

        self.app.prompt = None;
        self.draw();
        allowed
    }
}
