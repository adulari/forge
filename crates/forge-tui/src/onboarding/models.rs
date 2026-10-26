//! The model step: cards for every source, live badges, and the one primary action per card.

use crossterm::event::{KeyCode, KeyEvent};
use forge_onboarding::{
    Action, ModelSource, Secret, SecretSlot, SourceKind, SourceState, StepId, VerifyStatus,
};

use super::screen::{Effect, Entry, NoticeKind, Purpose, Screen, SecretTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardStatus {
    Verified,
    Checking,
    /// Saved, but the live check proved nothing (offline, no listing endpoint).
    Unchecked,
    Rejected,
    /// Found, not checked yet.
    Ready,
    NeedsLogin,
    NeedsSetup,
    NotInstalled,
}

impl CardStatus {
    /// Nothing is left to do for this card, so Enter continues instead of acting on it.
    pub fn settled(self) -> bool {
        matches!(
            self,
            CardStatus::Verified | CardStatus::Checking | CardStatus::Unchecked | CardStatus::Ready
        )
    }
}

impl Screen {
    pub fn cards(&self) -> Vec<ModelSource> {
        self.session.machine.model_sources()
    }

    pub fn card_status(&self, src: &ModelSource) -> CardStatus {
        let checking = self.running(&Purpose::Verify(src.id.clone()))
            || self.running(&Purpose::StoreKey(src.id.clone()));
        if checking {
            return CardStatus::Checking;
        }
        match self.session.state.verification(&src.id).map(|v| v.status) {
            Some(VerifyStatus::Verified) if src.state == SourceState::Ready => {
                return CardStatus::Verified
            }
            Some(VerifyStatus::Unchecked) if src.state == SourceState::Ready => {
                return CardStatus::Unchecked
            }
            Some(VerifyStatus::Rejected) => return CardStatus::Rejected,
            _ => {}
        }
        match src.state {
            SourceState::Ready => CardStatus::Ready,
            SourceState::NeedsLogin => CardStatus::NeedsLogin,
            SourceState::NeedsSetup => CardStatus::NeedsSetup,
            SourceState::NotInstalled => CardStatus::NotInstalled,
        }
    }

    fn focused_card(&self) -> Option<ModelSource> {
        let cards = self.cards();
        cards
            .get(self.cursor.min(cards.len().saturating_sub(1)))
            .cloned()
    }

    pub(crate) fn models_enter_label(&self) -> String {
        let Some(card) = self.focused_card() else {
            return "Continue".into();
        };
        match self.card_status(&card) {
            s if s.settled() => "Continue".into(),
            CardStatus::Rejected => match card.kind {
                SourceKind::ApiKey => "Replace the key".into(),
                _ => "Sign in again".into(),
            },
            CardStatus::NeedsLogin => "Sign in".into(),
            CardStatus::NotInstalled => "How to install".into(),
            _ => match card.kind {
                SourceKind::ApiKey => "Paste a key".into(),
                SourceKind::Local => "Set it up".into(),
                _ => "Sign in".into(),
            },
        }
    }

    /// Start a live check for every found source that has none yet.
    pub(crate) fn auto_verify(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        for card in self.cards() {
            if card.state != SourceState::Ready
                || self.session.state.verification(&card.id).is_some()
                || !self.requested.insert(card.id.clone())
            {
                continue;
            }
            effects.push(Effect::Run {
                action: Action::Verify {
                    source: card.id.clone(),
                },
                purpose: Purpose::Verify(card.id),
            });
        }
        effects
    }

    pub(crate) fn models_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let count = self.cards().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(count.saturating_sub(1));
            }
            KeyCode::Char('c') => return self.continue_models(),
            KeyCode::Char('s') => {
                self.session.postpone();
                self.exit = Some(super::screen::Exit::Left);
                return vec![Effect::Persist];
            }
            KeyCode::Char('r') => {
                self.requested.clear();
                return vec![Effect::Redetect];
            }
            KeyCode::Char('o') => {
                if let Some(url) = self.focused_card().and_then(|c| c.key_url) {
                    return vec![Effect::OpenUrl(url)];
                }
            }
            KeyCode::Char('t') => {
                if let Some(card) = self.focused_card() {
                    if card.kind == SourceKind::ClaudeCode
                        && card.state != SourceState::NotInstalled
                    {
                        return self.claude_token_flow();
                    }
                }
            }
            KeyCode::Char(' ') | KeyCode::Char('a') => {
                if let Some(card) = self.focused_card() {
                    return self.act_on(&card);
                }
            }
            KeyCode::Enter => {
                let Some(card) = self.focused_card() else {
                    return self.continue_models();
                };
                let status = self.card_status(&card);
                if status.settled() {
                    return self.continue_models();
                }
                return self.act_on(&card);
            }
            _ => {}
        }
        Vec::new()
    }

    fn continue_models(&mut self) -> Vec<Effect> {
        let missing = self.session.gate().missing.contains(&StepId::Models);
        if missing {
            self.notify(
                NoticeKind::Warn,
                "Connect at least one model to continue, or press s to finish this later.",
            );
            return Vec::new();
        }
        self.advance()
    }

    fn claude_token_flow(&mut self) -> Vec<Effect> {
        vec![Effect::Terminal {
            action: Action::ClaudeSetupToken,
            purpose: Purpose::ClaudeTokenFlow,
        }]
    }

    /// Do the card's setup action (or re-check it when it is already found).
    pub(crate) fn act_on(&mut self, card: &ModelSource) -> Vec<Effect> {
        let rejected = self.card_status(card) == CardStatus::Rejected;
        match card.kind {
            SourceKind::ApiKey => {
                if card.state == SourceState::Ready && !rejected {
                    return self.verify_now(card);
                }
                self.entry = Some(Entry {
                    target: SecretTarget::ApiKey {
                        provider: card.id.clone(),
                        replace: rejected || card.state == SourceState::Ready,
                    },
                    label: format!("Paste your {} API key", card.name),
                    hint: card.key_url.as_ref().map(|u| format!("Create one at {u}")),
                    url: card.key_url.clone(),
                    buf: String::new(),
                });
                Vec::new()
            }
            SourceKind::ClaudeCode => match card.state {
                SourceState::NotInstalled => {
                    self.notify(
                        NoticeKind::Info,
                        "Install Claude Code from claude.com/code, then press r to check again.",
                    );
                    Vec::new()
                }
                SourceState::Ready if !rejected => self.verify_now(card),
                _ => self.claude_token_flow(),
            },
            SourceKind::Codex | SourceKind::Antigravity => match card.state {
                SourceState::NotInstalled => {
                    self.notify(
                        NoticeKind::Info,
                        format!(
                            "{} is not installed. It is optional; press r after installing.",
                            card.name
                        ),
                    );
                    Vec::new()
                }
                SourceState::Ready if !rejected => self.verify_now(card),
                _ => {
                    let cmd = if card.kind == SourceKind::Codex {
                        "codex login"
                    } else {
                        "agy"
                    };
                    self.notify(
                        NoticeKind::Info,
                        format!("Run `{cmd}` in another terminal to sign in, then press r."),
                    );
                    Vec::new()
                }
            },
            SourceKind::CodexOauth | SourceKind::XaiOauth => {
                if card.state == SourceState::Ready && !rejected {
                    return self.verify_now(card);
                }
                let action = if card.kind == SourceKind::CodexOauth {
                    Action::CodexOauth
                } else {
                    Action::XaiOauth
                };
                vec![Effect::Terminal {
                    action,
                    purpose: Purpose::OauthSignIn(card.id.clone()),
                }]
            }
            SourceKind::Local => {
                if card.state == SourceState::Ready {
                    return self.verify_now(card);
                }
                let local = &self.session.machine.local;
                let Some(rec) = local.recommended.clone() else {
                    self.notify(
                        NoticeKind::Info,
                        "This machine is too small for a useful local model.",
                    );
                    return Vec::new();
                };
                if local.ollama_installed {
                    vec![Effect::Run {
                        action: Action::OllamaPull {
                            tag: rec.tag.clone(),
                        },
                        purpose: Purpose::LocalPull(rec.tag),
                    }]
                } else {
                    vec![Effect::Terminal {
                        action: Action::OllamaInstall,
                        purpose: Purpose::LocalInstall(rec.tag),
                    }]
                }
            }
        }
    }

    fn verify_now(&mut self, card: &ModelSource) -> Vec<Effect> {
        if self.running(&Purpose::Verify(card.id.clone())) {
            return Vec::new();
        }
        vec![Effect::Run {
            action: Action::Verify {
                source: card.id.clone(),
            },
            purpose: Purpose::Verify(card.id.clone()),
        }]
    }

    /// Submit the masked field.
    pub(crate) fn submit_entry(&mut self) -> Vec<Effect> {
        let Some(entry) = self.entry.take() else {
            return Vec::new();
        };
        if entry.buf.trim().is_empty() {
            self.notify(NoticeKind::Warn, "Nothing was entered.");
            return Vec::new();
        }
        let secret = Secret::new(entry.buf);
        match entry.target {
            SecretTarget::ApiKey { provider, replace } => vec![Effect::Run {
                action: Action::StoreKey {
                    provider: provider.clone(),
                    key: secret,
                    replace,
                },
                purpose: Purpose::StoreKey(provider),
            }],
            SecretTarget::ClaudeToken => vec![Effect::Run {
                action: Action::StoreSecret {
                    slot: SecretSlot::ClaudeOauthToken,
                    value: secret,
                },
                purpose: Purpose::StoreClaudeToken,
            }],
            SecretTarget::SearchKey => vec![Effect::Run {
                action: Action::StoreSecret {
                    slot: SecretSlot::Brave,
                    value: secret,
                },
                purpose: Purpose::StoreSearchKey,
            }],
            SecretTarget::BenchmarkKey => vec![Effect::Run {
                action: Action::StoreSecret {
                    slot: SecretSlot::ArtificialAnalysis,
                    value: secret,
                },
                purpose: Purpose::StoreBenchKey,
            }],
        }
    }
}
