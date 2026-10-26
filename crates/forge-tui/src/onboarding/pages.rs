//! Copy and key handling for every page except the model step.

use crossterm::event::{KeyCode, KeyEvent};
use forge_onboarding::{Action, Exposure, Machine, StepId};

use super::screen::{Effect, Entry, NoticeKind, Purpose, Screen, SecretTarget};

pub(crate) struct Temper {
    pub key: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub example: &'static str,
}

pub(crate) const TEMPERS: [Temper; 4] = [
    Temper {
        key: "plan",
        label: "Read-only",
        desc: "Forge reads and plans. It never changes anything.",
        example: "It can explain how your login works, but it cannot edit a file.",
    },
    Temper {
        key: "default",
        label: "Ask",
        desc: "Forge asks before every file edit and every command.",
        example: "Before changing src/main.rs it shows you the change and waits.",
    },
    Temper {
        key: "accept-edits",
        label: "Auto-edit",
        desc: "Forge edits files by itself and still asks before running commands.",
        example: "It edits freely, but asks before it runs `npm install`.",
    },
    Temper {
        key: "auto",
        label: "Auto",
        desc: "Forge does safe things on its own and asks only about risky ones.",
        example: "It runs your tests without asking, but asks before `rm -rf`.",
    },
];

pub(crate) const RECOMMENDED_TEMPER: usize = 1;

pub(crate) struct Credit {
    pub key: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub example: &'static str,
}

pub(crate) const CREDITS: [Credit; 3] = [
    Credit {
        key: "normal",
        label: "Normal",
        desc: "Always the best model for each task.",
        example: "A hard refactor goes to the strongest model you have.",
    },
    Credit {
        key: "frugal",
        label: "Frugal",
        desc: "Prefers free and subscription models.",
        example: "A quick question goes to a free model, even if a paid one is a bit better.",
    },
    Credit {
        key: "strict",
        label: "Strict",
        desc: "Free and subscription models only. Never spends paid credit.",
        example: "An API key is never used, even for a hard task.",
    },
];

pub(crate) const THEMES: [&str; 3] = ["auto", "dark", "light"];

pub(crate) struct ReportMode {
    pub key: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
}

pub(crate) const REPORTING: [ReportMode; 3] = [
    ReportMode {
        key: "auto",
        label: "Automatic",
        desc: "Send a scrubbed report when Forge hits an error, so it gets fixed.",
    },
    ReportMode {
        key: "ask",
        label: "Ask me each time",
        desc: "Show what would be sent and let you decide.",
    },
    ReportMode {
        key: "off",
        label: "Off",
        desc: "Never send anything.",
    },
];

pub(crate) const REPORTING_DETAILS: &str = "What gets sent: the error message with file paths, user names and secrets removed, the Forge version and your operating system. Never your prompts, your code, your file paths or any key.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchOpt {
    Builtin,
    Searxng,
    Brave,
}

pub(crate) fn search_options(machine: &Machine) -> Vec<SearchOpt> {
    let mut out = vec![SearchOpt::Builtin];
    if machine.search.docker || machine.search.searxng_running {
        out.push(SearchOpt::Searxng);
    }
    out.push(SearchOpt::Brave);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportKind {
    Config(&'static str),
    Mcp,
    Keybinds,
}

pub(crate) struct ImportItem {
    pub kind: ImportKind,
    pub label: String,
    pub detail: String,
}

pub(crate) fn import_items(machine: &Machine) -> Vec<ImportItem> {
    let i = &machine.imports;
    let mut out = Vec::new();
    if i.claude_dir {
        out.push(ImportItem {
            kind: ImportKind::Config("claude"),
            label: "Commands, skills and agents from Claude Code".into(),
            detail: format!("{} commands, {} skills found", i.commands, i.skills),
        });
    }
    if i.codex_dir {
        out.push(ImportItem {
            kind: ImportKind::Config("codex"),
            label: "Prompts from Codex".into(),
            detail: "copied as Forge commands".into(),
        });
    }
    if i.cursor_dir {
        out.push(ImportItem {
            kind: ImportKind::Config("cursor"),
            label: "Rules from Cursor".into(),
            detail: "copied as Forge guidance".into(),
        });
    }
    if i.mcp_servers > 0 {
        out.push(ImportItem {
            kind: ImportKind::Mcp,
            label: "MCP servers from your other tools".into(),
            detail: format!("{} found, you pick which to keep", i.mcp_servers),
        });
    }
    if i.claude_dir && !i.keybinds_active {
        out.push(ImportItem {
            kind: ImportKind::Keybinds,
            label: "Claude Code key bindings".into(),
            detail: "keeps Ctrl+K, Ctrl+N and friends meaning what they meant".into(),
        });
    }
    out
}

fn cycle(value: usize, len: usize, forward: bool) -> usize {
    if forward {
        (value + 1) % len
    } else {
        (value + len - 1) % len
    }
}

impl Screen {
    pub(crate) fn behavior_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Tab
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                self.ui.behavior_group = 1 - self.ui.behavior_group;
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_choice(false),
            KeyCode::Down | KeyCode::Char('j') => self.move_choice(true),
            KeyCode::Char('s') => return self.skip_step(),
            KeyCode::Enter => {
                let action = Action::SetBehavior {
                    permission: TEMPERS[self.ui.temper].key.to_string(),
                    credit_mode: CREDITS[self.ui.credit].key.to_string(),
                };
                let mut effects = self.advance();
                effects.push(Effect::Run {
                    action,
                    purpose: Purpose::Setting,
                });
                return effects;
            }
            _ => {}
        }
        Vec::new()
    }

    fn move_choice(&mut self, forward: bool) {
        if self.ui.behavior_group == 0 {
            self.ui.temper = cycle(self.ui.temper, TEMPERS.len(), forward);
        } else {
            self.ui.credit = cycle(self.ui.credit, CREDITS.len(), forward);
        }
    }

    pub(crate) fn search_enter_label(&self) -> String {
        let options = search_options(&self.session.machine);
        match options.get(self.cursor) {
            Some(SearchOpt::Searxng) if !self.session.machine.search.searxng_running => {
                "Start local search".into()
            }
            Some(SearchOpt::Brave) if !self.session.machine.search.brave_key => {
                "Paste a key".into()
            }
            _ => "Continue".into(),
        }
    }

    pub(crate) fn search_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let options = search_options(&self.session.machine);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(options.len() - 1)
            }
            KeyCode::Char('s') => return self.skip_step(),
            KeyCode::Char('c') => return self.advance(),
            KeyCode::Char(' ') | KeyCode::Enter => {
                let force = key.code == KeyCode::Char(' ');
                let machine = &self.session.machine;
                match options.get(self.cursor) {
                    Some(SearchOpt::Searxng) if !machine.search.searxng_running => {
                        return vec![Effect::Run {
                            action: Action::StartSearxng,
                            purpose: Purpose::Searxng,
                        }];
                    }
                    Some(SearchOpt::Brave) if !machine.search.brave_key || force => {
                        self.entry = Some(Entry {
                            target: SecretTarget::SearchKey,
                            label: "Paste your Brave Search key".into(),
                            hint: Some("Create one at brave.com/search/api".into()),
                            url: Some("https://brave.com/search/api/".into()),
                            buf: String::new(),
                        });
                        return Vec::new();
                    }
                    _ => {
                        if !force {
                            return self.advance();
                        }
                    }
                }
            }
            _ => {}
        }
        Vec::new()
    }

    pub(crate) fn benchmarks_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Char('k') => {
                self.entry = Some(Entry {
                    target: SecretTarget::BenchmarkKey,
                    label: "Paste your Artificial Analysis key".into(),
                    hint: Some("Create one at artificialanalysis.ai. Optional.".into()),
                    url: Some("https://artificialanalysis.ai".into()),
                    buf: String::new(),
                });
                Vec::new()
            }
            KeyCode::Char('s') => self.skip_step(),
            KeyCode::Enter => self.advance(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn import_enter_label(&self) -> String {
        if self.ui.import_checked.iter().any(|c| *c) {
            "Import the checked items".into()
        } else {
            "Continue".into()
        }
    }

    pub(crate) fn import_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let items = import_items(&self.session.machine);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(items.len().saturating_sub(1))
            }
            KeyCode::Char(' ') => {
                if let Some(c) = self.ui.import_checked.get_mut(self.cursor) {
                    *c = !*c;
                }
            }
            KeyCode::Char('s') => return self.skip_step(),
            KeyCode::Enter => {
                self.import_queue.clear();
                for (item, checked) in items.iter().zip(&self.ui.import_checked) {
                    if !*checked {
                        continue;
                    }
                    self.import_queue.push_back(match item.kind {
                        ImportKind::Config(tool) => Action::ImportConfig { tool: tool.into() },
                        ImportKind::Mcp => Action::ImportMcp,
                        ImportKind::Keybinds => Action::SetKeybindPreset {
                            preset: "claude-code".into(),
                        },
                    });
                }
                return self.import_next();
            }
            _ => {}
        }
        Vec::new()
    }

    /// Run the next queued import, or move on when the queue is empty.
    pub(crate) fn import_next(&mut self) -> Vec<Effect> {
        let Some(action) = self.import_queue.pop_front() else {
            if self.step() == StepId::Import {
                let mut effects = vec![Effect::Redetect];
                effects.extend(self.advance());
                return effects;
            }
            return Vec::new();
        };
        let terminal = matches!(action, Action::ImportMcp);
        vec![if terminal {
            Effect::Terminal {
                action,
                purpose: Purpose::Import,
            }
        } else {
            Effect::Run {
                action,
                purpose: Purpose::Import,
            }
        }]
    }

    pub(crate) fn service_enter_label(&self) -> String {
        let s = &self.session.machine.service;
        if s.installed && s.running {
            "Continue".into()
        } else {
            "Install and start".into()
        }
    }

    pub(crate) fn service_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Up | KeyCode::Down | KeyCode::Char('k') | KeyCode::Char('j') => {
                self.ui.exposure = 1 - self.ui.exposure;
            }
            KeyCode::Char('s') => return self.skip_step(),
            KeyCode::Enter => {
                let s = &self.session.machine.service;
                if s.installed && s.running {
                    return self.advance();
                }
                if self.running(&Purpose::Service) {
                    return Vec::new();
                }
                let exposure = if self.ui.exposure == 0 {
                    Exposure::Local
                } else {
                    Exposure::Lan
                };
                return vec![Effect::Run {
                    action: Action::InstallService { exposure },
                    purpose: Purpose::Service,
                }];
            }
            _ => {}
        }
        Vec::new()
    }

    pub(crate) fn anywhere_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Char('s') => self.skip_step(),
            KeyCode::Enter => {
                if self.session.machine.anywhere.signed_in {
                    return self.advance();
                }
                vec![Effect::Terminal {
                    action: Action::AnywhereSetup,
                    purpose: Purpose::Anywhere,
                }]
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn surfaces_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Char('s') => self.skip_step(),
            KeyCode::Enter => {
                if self.session.machine.desktop.installed {
                    return self.advance();
                }
                if self.running(&Purpose::Desktop) {
                    return Vec::new();
                }
                vec![Effect::Run {
                    action: Action::InstallDesktop,
                    purpose: Purpose::Desktop,
                }]
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn prefs_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Up | KeyCode::Down | KeyCode::Char('k') | KeyCode::Char('j') => {
                self.ui.prefs_row = 1 - self.ui.prefs_row;
            }
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char(' ')
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                let forward = !matches!(key.code, KeyCode::Left | KeyCode::Char('h'));
                if self.ui.prefs_row == 0 {
                    self.ui.notifications = !self.ui.notifications;
                } else {
                    self.ui.theme = cycle(self.ui.theme, THEMES.len(), forward);
                }
            }
            KeyCode::Char('s') => return self.skip_step(),
            KeyCode::Enter => {
                let mut effects = self.advance();
                effects.push(Effect::Run {
                    action: Action::SetNotifications {
                        enabled: self.ui.notifications,
                    },
                    purpose: Purpose::Setting,
                });
                effects.push(Effect::Run {
                    action: Action::SetTheme {
                        theme: THEMES[self.ui.theme].to_string(),
                    },
                    purpose: Purpose::Setting,
                });
                return effects;
            }
            _ => {}
        }
        Vec::new()
    }

    pub(crate) fn reporting_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.ui.reporting = cycle(self.ui.reporting, REPORTING.len(), false)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.ui.reporting = cycle(self.ui.reporting, REPORTING.len(), true)
            }
            KeyCode::Char('w') | KeyCode::Char('?') => {
                self.ui.reporting_details = !self.ui.reporting_details
            }
            KeyCode::Char('s') => return self.skip_step(),
            KeyCode::Enter => {
                let mut effects = self.advance();
                effects.push(Effect::Run {
                    action: Action::SetReporting {
                        mode: REPORTING[self.ui.reporting].key.to_string(),
                    },
                    purpose: Purpose::Setting,
                });
                return effects;
            }
            _ => {}
        }
        Vec::new()
    }

    pub(crate) fn done_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Char('t') if self.session.machine.project.git_repo => {
                self.ui.trust_folder = !self.ui.trust_folder;
                Vec::new()
            }
            KeyCode::Enter => {
                if !self.session.gate().complete {
                    self.notify(
                        NoticeKind::Warn,
                        "Forge cannot run yet: connect a model first (press b to go back).",
                    );
                    return Vec::new();
                }
                self.finish()
            }
            _ => Vec::new(),
        }
    }
}
