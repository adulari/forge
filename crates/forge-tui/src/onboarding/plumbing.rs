//! Screen plumbing shared by every page: page setup, the masked entry field, and what happens
//! when a job or a terminal hand-off ends.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use forge_onboarding::{Action, ActionResult, StepId, VerifyStatus};

use super::pages::{import_items, CREDITS, REPORTING, TEMPERS, THEMES};
use super::screen::{Effect, Entry, NoticeKind, Purpose, Screen, SecretTarget};

impl Screen {
    pub(crate) fn init_ui(&mut self) {
        let settings = self.session.machine.settings.clone();
        self.ui.temper = TEMPERS
            .iter()
            .position(|t| t.key == settings.permission)
            .unwrap_or(1);
        self.ui.credit = CREDITS
            .iter()
            .position(|c| c.key == settings.credit_mode)
            .unwrap_or(0);
        self.ui.theme = THEMES
            .iter()
            .position(|t| *t == settings.theme)
            .unwrap_or(0);
        self.ui.notifications = settings.notifications;
        self.ui.reporting = REPORTING
            .iter()
            .position(|r| r.key == settings.reporting)
            .unwrap_or(0);
        self.ui.local_autostart = true;
    }

    pub(crate) fn enter_page(&mut self) -> Vec<Effect> {
        self.cursor = 0;
        self.entry = None;
        self.ui.reporting_details = false;
        if self.step() == StepId::Import {
            self.ui.import_checked = vec![true; import_items(&self.session.machine).len()];
        }
        self.auto_verify()
    }

    pub(crate) fn entry_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(entry) = self.entry.as_mut() else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Esc => {
                self.entry = None;
            }
            KeyCode::Enter => return self.submit_entry(),
            KeyCode::Backspace => {
                entry.buf.pop();
            }
            KeyCode::Char('u') if ctrl => entry.buf.clear(),
            KeyCode::Char('o') if ctrl => {
                if let Some(url) = entry.url.clone() {
                    return vec![Effect::OpenUrl(url)];
                }
            }
            KeyCode::Char(c) if !ctrl && !c.is_control() => entry.buf.push(c),
            _ => {}
        }
        Vec::new()
    }

    pub(crate) fn job_succeeded(
        &mut self,
        purpose: &Purpose,
        result: Option<ActionResult>,
    ) -> Vec<Effect> {
        let mut effects = vec![Effect::Persist];
        if let Some(result) = &result {
            self.session.apply(result);
            if let ActionResult::Verified { status, detail, .. } = result {
                if *status == VerifyStatus::Rejected {
                    let text = detail
                        .clone()
                        .unwrap_or_else(|| "That did not work.".into());
                    self.notify(NoticeKind::Err, text);
                }
            }
        }
        match purpose {
            Purpose::Verify(_) | Purpose::Setting | Purpose::LocalInstall(_) => {}
            Purpose::StoreKey(_) | Purpose::StoreBenchKey => {
                effects.push(Effect::Redetect);
            }
            Purpose::ClaudeTokenFlow | Purpose::OauthSignIn(_) | Purpose::Anywhere => {}
            Purpose::StoreClaudeToken => {
                self.notify(NoticeKind::Ok, "Token saved. Checking Claude Code now.");
                self.requested.remove("claude-cli");
                self.session.state.verified.remove("claude-cli");
                effects.push(Effect::Redetect);
            }
            Purpose::LocalPull(tag) => {
                effects.push(Effect::Run {
                    action: Action::SetLocalModel {
                        tag: tag.clone(),
                        autostart: self.ui.local_autostart,
                    },
                    purpose: Purpose::LocalSave,
                });
            }
            Purpose::LocalSave => {
                self.requested.remove("ollama");
                self.session.state.verified.remove("ollama");
                effects.push(Effect::Redetect);
            }
            Purpose::Searxng => {
                effects.push(Effect::Redetect);
                self.notify(NoticeKind::Ok, "Local search is running.");
                if self.step() == StepId::WebSearch {
                    effects.extend(self.advance());
                }
            }
            Purpose::StoreSearchKey => {
                effects.push(Effect::Redetect);
                self.notify(NoticeKind::Ok, "Brave key saved.");
                if self.step() == StepId::WebSearch {
                    effects.extend(self.advance());
                }
            }
            Purpose::Import => effects.extend(self.import_next()),
            Purpose::Service => {
                effects.push(Effect::Redetect);
                self.notify(NoticeKind::Ok, "The background service is running.");
                if self.step() == StepId::Service {
                    effects.extend(self.advance());
                }
            }
            Purpose::Desktop => {
                effects.push(Effect::Redetect);
                self.notify(NoticeKind::Ok, "The desktop app is installed.");
                if self.step() == StepId::Surfaces {
                    effects.extend(self.advance());
                }
            }
        }
        effects
    }

    pub(crate) fn job_failed(&mut self, purpose: &Purpose, error: &str) -> Vec<Effect> {
        let first = error
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("It did not finish.");
        self.notify(NoticeKind::Err, first.trim().to_string());
        match purpose {
            Purpose::Import => self.import_next(),
            Purpose::LocalPull(_) => {
                self.notify(
                    NoticeKind::Err,
                    format!("The download did not finish: {first}"),
                );
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// A terminal hand-off ended. `ok` is the child's exit status.
    pub fn terminal_finished(&mut self, purpose: &Purpose, ok: bool) -> Vec<Effect> {
        let mut effects = vec![Effect::Redetect];
        match purpose {
            Purpose::ClaudeTokenFlow => {
                self.entry = Some(Entry {
                    target: SecretTarget::ClaudeToken,
                    label: "Paste the token Claude printed".into(),
                    hint: Some(
                        "It starts with sk-ant-. Forge keeps it in your keyring, never in a file."
                            .into(),
                    ),
                    url: None,
                    buf: String::new(),
                });
            }
            Purpose::OauthSignIn(id) => {
                self.requested.remove(id);
                self.session.state.verified.remove(id);
                if ok {
                    effects.push(Effect::Run {
                        action: Action::Verify { source: id.clone() },
                        purpose: Purpose::Verify(id.clone()),
                    });
                } else {
                    self.notify(
                        NoticeKind::Warn,
                        "The sign-in did not finish. You can try again.",
                    );
                }
            }
            Purpose::LocalInstall(tag) => {
                if ok {
                    effects.push(Effect::Run {
                        action: Action::OllamaPull { tag: tag.clone() },
                        purpose: Purpose::LocalPull(tag.clone()),
                    });
                } else {
                    self.notify(
                        NoticeKind::Warn,
                        "Ollama did not install. You can try again or skip this.",
                    );
                }
            }
            Purpose::Anywhere => {
                if ok {
                    self.notify(NoticeKind::Ok, "Forge Anywhere is set up.");
                    if self.step() == StepId::Anywhere {
                        effects.extend(self.advance());
                    }
                } else {
                    self.notify(
                        NoticeKind::Warn,
                        "Setup stopped. Press Enter to try again or s to skip.",
                    );
                }
            }
            Purpose::Import => effects.extend(self.import_next()),
            _ => {}
        }
        effects
    }
}
