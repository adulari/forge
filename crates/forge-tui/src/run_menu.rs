//! `/run`: project actions (`[[actions]]` in `.forge/config.toml`) and the ports they open.
//!
//! This module is data and key handling only. The binary feeds it snapshots (the configured
//! actions with their state and output tail, the listening ports) and performs the side effects
//! a [`MenuAction`] asks for: it owns the child processes. Rendering is in [`render`].

use crate::app::KeyKind;

mod render;
pub(crate) use render::render_run_menu;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionView {
    pub id: String,
    pub name: String,
    pub command: String,
    /// Relative to the project, as configured.
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RunState {
    #[default]
    Idle,
    Running {
        /// Epoch milliseconds.
        since_ms: i64,
    },
    Exited {
        code: Option<i32>,
        /// True when the person stopped it, so a signal exit is not shown as a failure.
        stopped: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionRow {
    pub action: ActionView,
    pub state: RunState,
    /// The tail of stdout and stderr, oldest first, ANSI escapes included.
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortView {
    pub port: u16,
    /// `loopback`, `all` or `other`.
    pub host: String,
    pub pid: u32,
    pub process: String,
    pub command: String,
    pub source: String,
    pub action: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Actions,
    Ports,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub tone: Tone,
    pub text: String,
    set_at: u32,
}

/// What the binary should do after a keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    None,
    Close,
    Start(String),
    Stop(String),
    Restart(String),
    CopyUrl(u16),
    OpenUrl(u16),
}

const NOTICE_TICKS: u32 = 420;

#[derive(Debug, Clone, Default)]
pub struct RunMenu {
    pub open: bool,
    pub project: String,
    pub view: View,
    pub actions: Vec<ActionRow>,
    pub ports: Vec<PortView>,
    /// False until the first port scan lands.
    pub ports_scanned: bool,
    /// The project declares actions but trust keeps them out.
    pub untrusted: bool,
    pub cursor: usize,
    pub filter: String,
    pub filtering: bool,
    pub notice: Option<Notice>,
    pub now_ms: i64,
    pub tick: u32,
    refresh: bool,
    last_second: i64,
}

impl RunMenu {
    pub fn open(&mut self) {
        self.open = true;
        self.view = View::Actions;
        self.cursor = 0;
        self.filter.clear();
        self.filtering = false;
        self.notice = None;
        self.ports_scanned = false;
        self.refresh = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.filtering = false;
    }

    /// Whether the binary should reload the configured actions now; clears the request.
    pub fn take_refresh(&mut self) -> bool {
        std::mem::take(&mut self.refresh)
    }

    pub fn running_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|r| matches!(r.state, RunState::Running { .. }))
            .count()
    }

    /// The running action's name when exactly one runs.
    pub fn sole_running_name(&self) -> Option<&str> {
        let mut running = self
            .actions
            .iter()
            .filter(|r| matches!(r.state, RunState::Running { .. }));
        match (running.next(), running.next()) {
            (Some(row), None) => Some(row.action.name.as_str()),
            _ => None,
        }
    }

    pub fn apply_actions(&mut self, rows: Vec<ActionRow>, untrusted: bool) {
        self.actions = rows;
        self.untrusted = untrusted;
        self.clamp();
    }

    pub fn apply_ports(&mut self, ports: Vec<PortView>) {
        self.ports = ports;
        self.ports_scanned = true;
        self.clamp();
    }

    pub fn notify(&mut self, tone: Tone, text: impl Into<String>) {
        self.notice = Some(Notice {
            tone,
            text: text.into(),
            set_at: self.tick,
        });
    }

    /// Advance the clock. True when something visible changed: a notice expired, or a running
    /// action's elapsed time ticked over a second while the overlay is open.
    pub fn advance(&mut self, now_ms: i64) -> bool {
        self.now_ms = now_ms;
        self.tick = self.tick.wrapping_add(1);
        let expired = self
            .notice
            .as_ref()
            .is_some_and(|n| self.tick.wrapping_sub(n.set_at) > NOTICE_TICKS);
        if expired {
            self.notice = None;
        }
        let second = now_ms / 1000;
        let ticked = self.open && second != self.last_second && self.running_count() > 0;
        self.last_second = second;
        expired || ticked
    }

    fn matches(&self, haystack: &[&str]) -> bool {
        let needle = self.filter.trim().to_lowercase();
        needle.is_empty() || haystack.iter().any(|h| h.to_lowercase().contains(&needle))
    }

    /// Indices into [`Self::actions`] that pass the filter.
    pub fn visible_actions(&self) -> Vec<usize> {
        (0..self.actions.len())
            .filter(|&i| {
                let a = &self.actions[i].action;
                self.matches(&[&a.name, &a.command])
            })
            .collect()
    }

    /// Indices into [`Self::ports`] that pass the filter.
    pub fn visible_ports(&self) -> Vec<usize> {
        (0..self.ports.len())
            .filter(|&i| {
                let p = &self.ports[i];
                let port = p.port.to_string();
                self.matches(&[
                    &port,
                    &p.process,
                    &p.source,
                    p.action.as_deref().unwrap_or(""),
                ])
            })
            .collect()
    }

    fn len(&self) -> usize {
        match self.view {
            View::Actions => self.visible_actions().len(),
            View::Ports => self.visible_ports().len(),
        }
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.len().saturating_sub(1));
    }

    pub fn selected_action(&self) -> Option<&ActionRow> {
        let visible = self.visible_actions();
        visible.get(self.cursor).map(|&i| &self.actions[i])
    }

    pub fn selected_port(&self) -> Option<&PortView> {
        let visible = self.visible_ports();
        visible.get(self.cursor).map(|&i| &self.ports[i])
    }

    fn switch_view(&mut self) {
        self.view = match self.view {
            View::Actions => View::Ports,
            View::Ports => View::Actions,
        };
        self.cursor = 0;
        self.clamp();
    }

    pub fn key(&mut self, key: KeyKind) -> MenuAction {
        if self.filtering {
            match key {
                KeyKind::Char(c) => {
                    self.filter.push(c);
                    self.cursor = 0;
                }
                KeyKind::Backspace => {
                    self.filter.pop();
                    self.cursor = 0;
                }
                KeyKind::Enter => self.filtering = false,
                KeyKind::Esc | KeyKind::Interrupt => {
                    self.filter.clear();
                    self.filtering = false;
                }
                KeyKind::Up => self.cursor = self.cursor.saturating_sub(1),
                KeyKind::Down => self.cursor = (self.cursor + 1).min(self.len().saturating_sub(1)),
                _ => {}
            }
            return MenuAction::None;
        }
        match key {
            KeyKind::Esc | KeyKind::Char('q') | KeyKind::Interrupt => {
                if self.filter.is_empty() {
                    self.close();
                    MenuAction::Close
                } else {
                    self.filter.clear();
                    self.cursor = 0;
                    MenuAction::None
                }
            }
            KeyKind::Char('/') => {
                self.filtering = true;
                MenuAction::None
            }
            KeyKind::Tab | KeyKind::CycleTemper | KeyKind::Left | KeyKind::Right => {
                self.switch_view();
                MenuAction::None
            }
            KeyKind::Up | KeyKind::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                MenuAction::None
            }
            KeyKind::Down | KeyKind::Char('j') => {
                self.cursor = (self.cursor + 1).min(self.len().saturating_sub(1));
                MenuAction::None
            }
            KeyKind::Home => {
                self.cursor = 0;
                MenuAction::None
            }
            KeyKind::End => {
                self.cursor = self.len().saturating_sub(1);
                MenuAction::None
            }
            KeyKind::PageUp => {
                self.cursor = self.cursor.saturating_sub(5);
                MenuAction::None
            }
            KeyKind::PageDown => {
                self.cursor = (self.cursor + 5).min(self.len().saturating_sub(1));
                MenuAction::None
            }
            KeyKind::Enter | KeyKind::Char(' ') => self.activate(),
            KeyKind::Char('r') => self.with_action(MenuAction::Restart),
            KeyKind::Char('s') => match self.selected_action() {
                Some(row) if matches!(row.state, RunState::Running { .. }) => {
                    MenuAction::Stop(row.action.id.clone())
                }
                _ => MenuAction::None,
            },
            KeyKind::Char('c') => self.with_port(MenuAction::CopyUrl),
            KeyKind::Char('o') => self.with_port(MenuAction::OpenUrl),
            _ => MenuAction::None,
        }
    }

    fn with_action(&self, make: fn(String) -> MenuAction) -> MenuAction {
        match (self.view, self.selected_action()) {
            (View::Actions, Some(row)) => make(row.action.id.clone()),
            _ => MenuAction::None,
        }
    }

    fn with_port(&self, make: fn(u16) -> MenuAction) -> MenuAction {
        match (self.view, self.selected_port()) {
            (View::Ports, Some(port)) => make(port.port),
            _ => MenuAction::None,
        }
    }

    fn activate(&self) -> MenuAction {
        match self.view {
            View::Actions => match self.selected_action() {
                Some(row) if matches!(row.state, RunState::Running { .. }) => {
                    MenuAction::Stop(row.action.id.clone())
                }
                Some(row) => MenuAction::Start(row.action.id.clone()),
                None => MenuAction::None,
            },
            View::Ports => self.with_port(MenuAction::CopyUrl),
        }
    }
}
