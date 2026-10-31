//! `/clis`: the subscription CLIs (Claude Code, Codex, Antigravity) on this machine, with
//! installed and latest versions, and a confirmed install or update that streams the vendor's own
//! output into the overlay.
//!
//! Data and key handling only. The binary feeds it row snapshots and performs the
//! [`MenuAction::Run`] it asks for; nothing here starts a process. Every run goes through a
//! confirmation that prints the exact command, because these are vendor installers.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, KeyKind};
use crate::surface::{self, SurfaceTone};
use crate::theme::styles;

#[cfg(test)]
mod tests;

const LOG_ROWS: usize = 5;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Job {
    #[default]
    Idle,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentRow {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub version: Option<String>,
    pub latest: Option<String>,
    pub outdated: bool,
    /// `WSL`, `Windows via WSL` or empty for a native install.
    pub runtime: String,
    /// The exact command a run would execute; `None` when there is nothing Forge can run.
    pub command: Option<String>,
    pub hint: Option<String>,
    pub job: Job,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Load {
    #[default]
    Loading,
    Ready,
    Failed(String),
}

/// What the binary should do after a keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    None,
    Close,
    Refresh,
    /// Run the vendor command for these agent ids, one after another.
    Run(Vec<String>),
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Confirm {
    ids: Vec<String>,
    /// `(name, exact command)` for each id, shown verbatim.
    commands: Vec<(String, String)>,
    on_confirm: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AgentsMenu {
    pub open: bool,
    pub load: Load,
    pub rows: Vec<AgentRow>,
    pub cursor: usize,
    pub log: Vec<String>,
    pub notice: Option<String>,
    confirm: Option<Confirm>,
    refresh: bool,
}

impl AgentsMenu {
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            refresh: true,
            ..Self::default()
        };
    }

    pub fn close(&mut self) {
        self.open = false;
        self.confirm = None;
    }

    pub fn take_refresh(&mut self) -> bool {
        std::mem::take(&mut self.refresh)
    }

    pub fn running(&self) -> bool {
        self.rows.iter().any(|r| r.job == Job::Running)
    }

    pub fn apply_rows(&mut self, result: Result<Vec<AgentRow>, String>) {
        match result {
            Ok(mut rows) => {
                // A refresh must not forget a job the driver is still running.
                for row in &mut rows {
                    if let Some(old) = self.rows.iter().find(|r| r.id == row.id) {
                        if matches!(old.job, Job::Running) {
                            row.job = Job::Running;
                        }
                    }
                }
                self.rows = rows;
                self.load = Load::Ready;
                self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
            }
            Err(error) if !self.rows.is_empty() => {
                self.notice = Some(format!("Could not refresh: {error}"));
            }
            Err(error) => self.load = Load::Failed(error),
        }
    }

    pub fn set_job(&mut self, id: &str, job: Job) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.id == id) {
            row.job = job;
        }
    }

    pub fn push_log(&mut self, line: String) {
        self.log.push(line);
        let excess = self.log.len().saturating_sub(40);
        self.log.drain(..excess);
    }

    fn confirm_for(&mut self, ids: Vec<String>) {
        let commands: Vec<(String, String)> = ids
            .iter()
            .filter_map(|id| self.rows.iter().find(|r| &r.id == id))
            .filter_map(|r| r.command.clone().map(|c| (r.name.clone(), c)))
            .collect();
        if commands.is_empty() {
            self.notice = Some("Nothing to run for this agent.".to_string());
            return;
        }
        self.notice = None;
        self.confirm = Some(Confirm {
            ids,
            commands,
            on_confirm: false,
        });
    }

    pub fn key(&mut self, key: KeyKind) -> MenuAction {
        if let Some(confirm) = self.confirm.as_mut() {
            return match key {
                KeyKind::Left | KeyKind::Right | KeyKind::Tab | KeyKind::CycleTemper => {
                    confirm.on_confirm = !confirm.on_confirm;
                    MenuAction::None
                }
                KeyKind::Char('y') => self.accept(),
                KeyKind::Enter if confirm.on_confirm => self.accept(),
                KeyKind::Enter | KeyKind::Esc | KeyKind::Char('n') | KeyKind::Interrupt => {
                    self.confirm = None;
                    MenuAction::None
                }
                _ => MenuAction::None,
            };
        }
        match key {
            KeyKind::Esc | KeyKind::Char('q') | KeyKind::Interrupt => {
                self.close();
                MenuAction::Close
            }
            KeyKind::Up | KeyKind::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                MenuAction::None
            }
            KeyKind::Down | KeyKind::Char('j') => {
                self.cursor = (self.cursor + 1).min(self.rows.len().saturating_sub(1));
                MenuAction::None
            }
            KeyKind::Char('r') => {
                self.refresh = true;
                MenuAction::Refresh
            }
            KeyKind::Enter => {
                if self.running() {
                    self.notice = Some("Wait for the running job to finish.".to_string());
                } else if let Some(row) = self.rows.get(self.cursor) {
                    match row.command {
                        Some(_) => self.confirm_for(vec![row.id.clone()]),
                        None => self.notice = row.hint.clone(),
                    }
                }
                MenuAction::None
            }
            KeyKind::Char('u') | KeyKind::Char('U') => {
                let ids: Vec<String> = self
                    .rows
                    .iter()
                    .filter(|r| r.outdated && r.command.is_some())
                    .map(|r| r.id.clone())
                    .collect();
                if self.running() {
                    self.notice = Some("Wait for the running job to finish.".to_string());
                } else if ids.is_empty() {
                    self.notice = Some("Every installed agent is up to date.".to_string());
                } else {
                    self.confirm_for(ids);
                }
                MenuAction::None
            }
            _ => MenuAction::None,
        }
    }

    fn accept(&mut self) -> MenuAction {
        let Some(confirm) = self.confirm.take() else {
            return MenuAction::None;
        };
        self.log.clear();
        for id in &confirm.ids {
            self.set_job(id, Job::Running);
        }
        MenuAction::Run(confirm.ids)
    }
}

fn button(label: &str, active: bool, style: Style) -> Span<'static> {
    let s = if active {
        style.add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        styles::muted()
    };
    Span::styled(format!(" {label} "), s)
}

fn version_text(row: &AgentRow) -> String {
    if !row.installed {
        return "not installed".to_string();
    }
    let version = row.version.as_deref().unwrap_or("version unknown");
    match (&row.latest, row.outdated) {
        (Some(latest), true) => format!("{version} -> {latest} available"),
        (Some(_), false) => format!("{version} (latest)"),
        _ => version.to_string(),
    }
}

fn confirm_lines(c: &Confirm) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "Forge will run these commands on this machine:",
            styles::muted(),
        )),
        Line::default(),
    ];
    for (name, command) in &c.commands {
        lines.push(Line::from(Span::styled(name.clone(), styles::fg())));
        lines.push(Line::from(Span::styled(
            format!("  $ {command}"),
            styles::warn(),
        )));
        if command.contains("| bash") || command.contains("| sh") {
            lines.push(Line::from(Span::styled(
                "  Downloads a script from the internet and runs it.",
                styles::err(),
            )));
        }
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![
        button("Run", c.on_confirm, styles::warn()),
        Span::raw("   "),
        button("Cancel", !c.on_confirm, styles::fg()),
    ]));
    lines
}

fn list_lines(m: &AgentsMenu, width: usize) -> Vec<Line<'static>> {
    match &m.load {
        Load::Loading if m.rows.is_empty() => {
            return vec![Line::from(Span::styled(
                "Reading installed versions...",
                styles::muted(),
            ))]
        }
        Load::Failed(e) => {
            return vec![Line::from(Span::styled(
                format!("Could not read agents: {e}"),
                styles::err(),
            ))]
        }
        _ => {}
    }
    let outdated = m.rows.iter().filter(|r| r.outdated).count();
    let mut lines = vec![Line::from(Span::styled(
        match outdated {
            0 => "Subscription CLIs on this machine.".to_string(),
            1 => "1 agent is outdated. Press U to update it.".to_string(),
            n => format!("{n} agents are outdated. Press U to update them all."),
        },
        if outdated > 0 {
            styles::warn()
        } else {
            styles::muted()
        },
    ))];
    lines.push(Line::default());
    for (i, row) in m.rows.iter().enumerate() {
        let focused = i == m.cursor;
        let marker = if focused { "> " } else { "  " };
        let (tag, tag_style) = match row.job {
            Job::Running => ("running", styles::brand()),
            Job::Done => ("done", styles::ok()),
            Job::Failed => ("failed", styles::err()),
            Job::Idle if row.outdated => ("outdated", styles::warn()),
            Job::Idle => ("", styles::muted()),
        };
        let mut spans = vec![
            Span::styled(
                marker,
                if focused {
                    styles::brand()
                } else {
                    styles::faint()
                },
            ),
            Span::styled(
                format!("{:<20}", row.name),
                if focused {
                    styles::fg().add_modifier(Modifier::BOLD)
                } else {
                    styles::fg()
                },
            ),
            Span::styled(version_text(row), styles::muted()),
        ];
        if !row.runtime.is_empty() && row.installed {
            spans.push(Span::styled(
                format!("  [{}]", row.runtime),
                styles::faint(),
            ));
        }
        if !tag.is_empty() {
            spans.push(Span::styled(format!("  {tag}"), tag_style));
        }
        lines.push(Line::from(spans));
        if focused {
            let detail = match (&row.command, &row.hint) {
                (Some(c), _) => format!("    runs: {c}"),
                (None, Some(h)) => format!("    {h}"),
                _ => String::new(),
            };
            let max = width.saturating_sub(1);
            let detail: String = detail.chars().take(max).collect();
            lines.push(Line::from(Span::styled(detail, styles::faint())));
        }
    }
    if !m.log.is_empty() {
        lines.push(Line::default());
        let skip = m.log.len().saturating_sub(LOG_ROWS);
        for l in m.log.iter().skip(skip) {
            lines.push(Line::from(Span::styled(
                l.chars().take(width.saturating_sub(2)).collect::<String>(),
                styles::muted(),
            )));
        }
    }
    if let Some(n) = &m.notice {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(n.clone(), styles::warn())));
    }
    lines
}

pub(crate) fn render_agents_menu(f: &mut Frame, app: &App, region: Rect) {
    let m = &app.agents_menu;
    if !m.open {
        return;
    }
    surface::dim_backdrop(f, region);
    let card_w = region.width.saturating_sub(2).clamp(1, 96);
    let inner_w = (card_w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let (lines, title, footer, tone) = match &m.confirm {
        Some(c) => (
            confirm_lines(c),
            "Run vendor installer?",
            " y run · Tab switch · Esc cancel ",
            SurfaceTone::Warning,
        ),
        None => (
            list_lines(m, inner_w),
            "Agent CLIs",
            " Enter install/update · U update all · r refresh · Esc close ",
            SurfaceTone::Brand,
        ),
    };
    let content = lines.len() as u16;
    let padded = content + 2 + 2 * crate::overlays::PAD_Y;
    let want = if padded >= crate::overlays::SHORT_REGION {
        padded
    } else {
        content + 2
    };
    let card_h = want.max(6).min(region.height);
    let card = Rect {
        x: region.x + region.width.saturating_sub(card_w) / 2,
        y: region.y + region.height.saturating_sub(card_h) / 2,
        width: card_w,
        height: card_h,
    };
    let inner = crate::overlays::padded_panel(
        f,
        card,
        surface::title(title, tone),
        Some(Span::styled(" /clis ", styles::faint())),
        Some(Span::styled(footer.to_string(), styles::faint())),
        tone,
    );
    if inner.width < 20 || inner.height < 2 {
        return;
    }
    // Keep the tail visible when a long confirmation or log overflows a short terminal.
    let skip = lines.len().saturating_sub(inner.height as usize);
    let visible: Vec<Line> = lines.into_iter().skip(skip).collect();
    f.render_widget(Paragraph::new(visible), inner);
}
