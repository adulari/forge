//! `/anywhere`: the Forge Anywhere menu. Account and connection, per-device account sync
//! (keys, skills, settings, MCP servers, instructions) and the enrolled devices, in one
//! navigable overlay.
//!
//! This module is data and key handling only. The binary feeds it snapshots (labels and counts,
//! never a secret value) and performs the side effects a [`MenuAction`] asks for: it owns the
//! config writes, the daemon call and the sign-in subprocesses. Rendering is in [`render`].

use crate::app::KeyKind;

mod confirm;
mod render;
pub(crate) use render::render_anywhere_menu;

#[cfg(test)]
mod tests;

/// What a category syncs, shown under the focused row so a switch is never a guess.
pub fn category_blurb(id: &str) -> &'static str {
    match id {
        "keys" => {
            "API keys and static MCP secrets. Switched on at this machine only, after a prompt."
        }
        "skills" => "Skills, commands, agents, skill packs and marketplaces.",
        "settings" => "Theme, statusline, notifications, permissions, voice and telemetry prefs.",
        "mcp" => "MCP server definitions. Stdio servers apply only where the command is installed.",
        "instructions" => "Your user-level AGENTS.md.",
        _ => "",
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CategoryView {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub items: usize,
    pub pending: usize,
    /// Epoch milliseconds of the last applied change.
    pub last_sync_at: Option<i64>,
    /// Items other devices hold that this one has not applied (or has switched off).
    pub remote_only: usize,
    /// `(item label, detail)`.
    pub conflicts: Vec<(String, String)>,
    /// `(item label, reason)`.
    pub skipped: Vec<(String, String)>,
    /// True for the credential category: it can only be switched on at this machine.
    pub requires_direct: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SignInView {
    pub provider: String,
    pub label: String,
    /// The `forge ...` command that signs in again.
    pub command: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MissingToolView {
    pub label: String,
    pub hint: String,
}

/// How the connector looks from this machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Connector {
    /// Relaying and recently heard from.
    Live,
    /// Configured but not healthy.
    Unhealthy,
    /// Anywhere is not enabled on this machine.
    #[default]
    Off,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnywhereSnapshot {
    pub signed_in: bool,
    pub login: Option<String>,
    pub connector: Connector,
    pub connector_text: String,
    /// Short device id and the host name, e.g. `a1b2c3d4 · workstation`.
    pub device_label: String,
    /// `[anywhere] sync`: record sync, which the account-sync worker needs.
    pub sync_enabled: bool,
    /// Whether the sync worker (`forge serve`) answered on this machine.
    pub worker_running: bool,
    pub last_sync_at: Option<i64>,
    pub last_error: Option<String>,
    pub categories: Vec<CategoryView>,
    pub sign_ins: Vec<SignInView>,
    pub missing_tools: Vec<MissingToolView>,
    /// Wall clock when this was read, so ages stay right between refreshes.
    pub taken_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub enrolled: String,
    /// Epoch milliseconds.
    pub last_seen_at: Option<i64>,
    pub this_device: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Devices {
    #[default]
    Loading,
    /// Offline, signed out or the service refused; the text says why.
    Unavailable(String),
    Loaded {
        devices: Vec<DeviceView>,
        hosts_online: usize,
        hosts_total: usize,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Load {
    #[default]
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Err,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub tone: Tone,
    pub text: String,
    set_at: u32,
}

/// The rows ↑/↓ visit, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    SignInToAnywhere,
    OpenWeb,
    Category(usize),
    SyncNow,
    SignIn(usize),
    Device(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Account,
    Sync,
    Devices,
}

impl Row {
    pub fn section(self) -> Section {
        match self {
            Row::SignInToAnywhere | Row::OpenWeb => Section::Account,
            Row::Category(_) | Row::SyncNow | Row::SignIn(_) => Section::Sync,
            Row::Device(_) => Section::Devices,
        }
    }
}

/// The confirmation that stands between a keystroke and syncing credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeysConfirm {
    /// True when the primary button has focus. Opens on Cancel.
    pub on_confirm: bool,
}

/// What the binary should do after a keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    None,
    Close,
    Refresh,
    Toggle { category: String, enabled: bool },
    SyncNow,
    SignInProvider { label: String, command: String },
    SetupAnywhere,
    OpenWeb,
}

const NOTICE_TICKS: u32 = 420;

#[derive(Debug, Clone, Default)]
pub struct AnywhereMenu {
    pub open: bool,
    pub load: Load,
    pub data: Option<AnywhereSnapshot>,
    pub devices: Devices,
    pub cursor: usize,
    pub confirm: Option<KeysConfirm>,
    pub notice: Option<Notice>,
    pub tick: u32,
    pub now_ms: i64,
    /// True from "Sync now" until the next snapshot lands.
    pub syncing: bool,
    pub(crate) scroll: usize,
    refresh: bool,
}

impl AnywhereMenu {
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            now_ms: self.now_ms,
            refresh: true,
            ..Self::default()
        };
    }

    pub fn close(&mut self) {
        self.open = false;
        self.confirm = None;
    }

    /// Whether the binary should start a refresh now; clears the request.
    pub fn take_refresh(&mut self) -> bool {
        std::mem::take(&mut self.refresh)
    }

    pub fn signed_in(&self) -> bool {
        self.data.as_ref().is_some_and(|d| d.signed_in)
    }

    pub fn apply_snapshot(&mut self, result: Result<AnywhereSnapshot, String>) {
        self.syncing = false;
        match result {
            Ok(snapshot) => {
                self.now_ms = self.now_ms.max(snapshot.taken_at_ms);
                let first = self.data.is_none();
                self.data = Some(snapshot);
                self.load = Load::Ready;
                if first {
                    self.cursor = self.default_cursor();
                }
                self.clamp();
            }
            Err(error) if self.data.is_some() => {
                self.notify(Tone::Warn, format!("Could not refresh: {error}"));
            }
            Err(error) => self.load = Load::Failed(error),
        }
    }

    pub fn apply_devices(&mut self, devices: Devices) {
        self.devices = devices;
        self.clamp();
    }

    pub fn notify(&mut self, tone: Tone, text: impl Into<String>) {
        self.notice = Some(Notice {
            tone,
            text: text.into(),
            set_at: self.tick,
        });
    }

    /// Reflect a switch immediately; the next snapshot confirms or corrects it.
    pub fn set_enabled(&mut self, category: &str, enabled: bool) {
        if let Some(c) = self
            .data
            .as_mut()
            .and_then(|d| d.categories.iter_mut().find(|c| c.id == category))
        {
            c.enabled = enabled;
        }
    }

    /// Advance the animation clock. True when something visible changed.
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
        expired
            || self.syncing
            || matches!(self.load, Load::Loading)
            || matches!(self.devices, Devices::Loading)
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if !self.signed_in() && self.data.is_some() {
            rows.push(Row::SignInToAnywhere);
        }
        if self.data.is_some() {
            rows.push(Row::OpenWeb);
        }
        if let Some(data) = &self.data {
            rows.extend((0..data.categories.len()).map(Row::Category));
            if data.signed_in {
                rows.push(Row::SyncNow);
            }
            rows.extend((0..data.sign_ins.len()).map(Row::SignIn));
        }
        if let (true, Devices::Loaded { devices, .. }) = (self.signed_in(), &self.devices) {
            rows.extend((0..devices.len()).map(Row::Device));
        }
        rows
    }

    pub fn focused(&self) -> Option<Row> {
        self.rows().get(self.cursor).copied()
    }

    fn default_cursor(&self) -> usize {
        let rows = self.rows();
        let target = if self.signed_in() {
            rows.iter().position(|r| matches!(r, Row::Category(_)))
        } else {
            rows.iter().position(|r| *r == Row::SignInToAnywhere)
        };
        target.unwrap_or(0)
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
    }

    fn jump_section(&mut self, forward: bool) {
        let rows = self.rows();
        let Some(current) = rows.get(self.cursor).map(|r| r.section()) else {
            return;
        };
        let order = [Section::Account, Section::Sync, Section::Devices];
        let present: Vec<Section> = order
            .into_iter()
            .filter(|s| rows.iter().any(|r| r.section() == *s))
            .collect();
        let Some(at) = present.iter().position(|s| *s == current) else {
            return;
        };
        let next = if forward {
            (at + 1) % present.len()
        } else {
            (at + present.len() - 1) % present.len()
        };
        if let Some(index) = rows.iter().position(|r| r.section() == present[next]) {
            self.cursor = index;
        }
    }

    pub fn key(&mut self, key: KeyKind) -> MenuAction {
        if let Some(confirm) = self.confirm {
            return self.confirm_key(confirm, key);
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
                self.cursor = (self.cursor + 1).min(self.rows().len().saturating_sub(1));
                MenuAction::None
            }
            KeyKind::Home => {
                self.cursor = 0;
                MenuAction::None
            }
            KeyKind::End => {
                self.cursor = self.rows().len().saturating_sub(1);
                MenuAction::None
            }
            KeyKind::PageUp => {
                self.cursor = self.cursor.saturating_sub(5);
                MenuAction::None
            }
            KeyKind::PageDown => {
                self.cursor = (self.cursor + 5).min(self.rows().len().saturating_sub(1));
                MenuAction::None
            }
            KeyKind::Tab => {
                self.jump_section(true);
                MenuAction::None
            }
            KeyKind::CycleTemper => {
                self.jump_section(false);
                MenuAction::None
            }
            KeyKind::Char('r') => {
                self.refresh = true;
                self.devices = match self.devices {
                    Devices::Loaded { .. } => std::mem::take(&mut self.devices),
                    _ => Devices::Loading,
                };
                MenuAction::Refresh
            }
            KeyKind::Char('s') => self.sync_now(),
            KeyKind::Char('o') => MenuAction::OpenWeb,
            KeyKind::Char(' ') | KeyKind::Enter => self.activate(),
            KeyKind::Left | KeyKind::Char('h') => self.switch(Some(false)),
            KeyKind::Right | KeyKind::Char('l') => self.switch(Some(true)),
            _ => MenuAction::None,
        }
    }

    fn confirm_key(&mut self, confirm: KeysConfirm, key: KeyKind) -> MenuAction {
        match key {
            KeyKind::Esc | KeyKind::Char('n') | KeyKind::Char('q') | KeyKind::Interrupt => {
                self.confirm = None;
                MenuAction::None
            }
            KeyKind::Char('y') => self.confirm_keys(),
            KeyKind::Left | KeyKind::Right | KeyKind::Tab | KeyKind::CycleTemper => {
                self.confirm = Some(KeysConfirm {
                    on_confirm: !confirm.on_confirm,
                });
                MenuAction::None
            }
            KeyKind::Enter | KeyKind::Char(' ') if confirm.on_confirm => self.confirm_keys(),
            KeyKind::Enter | KeyKind::Char(' ') => {
                self.confirm = None;
                MenuAction::None
            }
            _ => MenuAction::None,
        }
    }

    fn confirm_keys(&mut self) -> MenuAction {
        self.confirm = None;
        self.set_enabled("keys", true);
        MenuAction::Toggle {
            category: "keys".into(),
            enabled: true,
        }
    }

    fn sync_now(&mut self) -> MenuAction {
        if !self.signed_in() {
            self.notify(Tone::Warn, "Sign in to Forge Anywhere before syncing.");
            return MenuAction::None;
        }
        self.syncing = true;
        MenuAction::SyncNow
    }

    /// `want`: `Some(state)` for ←/→, `None` is a plain toggle (Space and Enter).
    fn switch(&mut self, want: Option<bool>) -> MenuAction {
        let Some(Row::Category(i)) = self.focused() else {
            return MenuAction::None;
        };
        let Some(category) = self
            .data
            .as_ref()
            .and_then(|d| d.categories.get(i))
            .cloned()
        else {
            return MenuAction::None;
        };
        let enabled = want.unwrap_or(!category.enabled);
        if enabled == category.enabled {
            return MenuAction::None;
        }
        if enabled && !self.signed_in() {
            self.notify(
                Tone::Warn,
                "Sign in to Forge Anywhere first, then turn sync on.",
            );
            return MenuAction::None;
        }
        if enabled && category.requires_direct {
            self.confirm = Some(KeysConfirm::default());
            return MenuAction::None;
        }
        self.set_enabled(&category.id, enabled);
        MenuAction::Toggle {
            category: category.id,
            enabled,
        }
    }

    fn activate(&mut self) -> MenuAction {
        match self.focused() {
            Some(Row::SignInToAnywhere) => MenuAction::SetupAnywhere,
            Some(Row::OpenWeb) => MenuAction::OpenWeb,
            Some(Row::Category(_)) => self.switch(None),
            Some(Row::SyncNow) => self.sync_now(),
            Some(Row::SignIn(i)) => {
                let Some(s) = self.data.as_ref().and_then(|d| d.sign_ins.get(i)) else {
                    return MenuAction::None;
                };
                MenuAction::SignInProvider {
                    label: s.label.clone(),
                    command: s.command.clone(),
                }
            }
            Some(Row::Device(i)) => {
                if let Devices::Loaded { devices, .. } = &self.devices {
                    if let Some(d) = devices.get(i).filter(|d| !d.this_device) {
                        let text = format!(
                            "Revoking needs your recovery phrase: forge anywhere devices --revoke {}",
                            d.id
                        );
                        self.notify(Tone::Ok, text);
                    }
                }
                MenuAction::None
            }
            None => MenuAction::None,
        }
    }
}
