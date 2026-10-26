//! The "a newer Forge is available" card: pure state, no terminal or install side effects.
//!
//! The shell feeds it offers and install progress and routes keys through [`UpdateCard::handle_key`];
//! the returned [`UpdateAction`] says what the shell must do (start the install, run a package
//! manager command, persist a skip, restart). Drawing lives in `app/render/update_band.rs`.

use crate::KeyKind;

/// How the offered version gets installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdatePlan {
    /// This binary can replace itself (standalone install).
    InPlace,
    /// A package manager owns the install; this is its exact command.
    Command(String),
}

/// A newer release, as the card shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOffer {
    pub version: String,
    pub current: String,
    /// The first lines of the release notes, already trimmed to display text.
    pub notes: Vec<String>,
    pub url: String,
    pub plan: UpdatePlan,
}

/// What the background daemon (`forge service`) is doing after an update.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ServiceNote {
    #[default]
    None,
    /// A service is running the old binary; `[v]` restarts it.
    Running,
    Restarting,
    Restarted,
    /// Not restarted, and why (a busy session, a failed restart).
    Held(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CardPhase {
    #[default]
    Hidden,
    Offer,
    Running(String),
    Done,
    Failed(String),
}

/// What the shell must do after a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateAction {
    None,
    StartInPlace,
    RunCommand(String),
    CopyCommand(String),
    /// Persist "never offer this version again".
    Skip(String),
    Restart,
    RestartService,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateCard {
    offer: Option<UpdateOffer>,
    phase: CardPhase,
    /// The version put off with "later" in this process: not offered again until the next launch.
    later: Option<String>,
    service: ServiceNote,
}

impl UpdateCard {
    pub fn phase(&self) -> &CardPhase {
        &self.phase
    }

    pub fn offer_ref(&self) -> Option<&UpdateOffer> {
        self.offer.as_ref()
    }

    pub fn service(&self) -> &ServiceNote {
        &self.service
    }

    /// A newer release is known. Ignored while an install is in flight or finished, and for a
    /// version already put off with "later" in this process.
    pub fn offer(&mut self, offer: UpdateOffer) {
        if self.later.as_deref() == Some(offer.version.as_str()) {
            return;
        }
        match self.phase {
            CardPhase::Hidden | CardPhase::Offer | CardPhase::Failed(_) => {
                self.offer = Some(offer);
                self.phase = CardPhase::Offer;
            }
            CardPhase::Running(_) | CardPhase::Done => {}
        }
    }

    /// Whether the card is on screen. An offer waits for the running turn to end rather than
    /// interrupting it; progress and results the user asked for are always shown.
    pub fn visible(&self, turn_busy: bool) -> bool {
        match self.phase {
            CardPhase::Hidden => false,
            CardPhase::Offer => !turn_busy,
            CardPhase::Running(_) | CardPhase::Done | CardPhase::Failed(_) => true,
        }
    }

    /// Whether letters belong to the card. With a draft in the composer they stay in the composer.
    pub fn captures_keys(&self, turn_busy: bool, composer_empty: bool) -> bool {
        composer_empty
            && self.visible(turn_busy)
            && matches!(
                self.phase,
                CardPhase::Offer | CardPhase::Done | CardPhase::Failed(_)
            )
    }

    pub fn progress(&mut self, text: &str) {
        if matches!(self.phase, CardPhase::Running(_)) {
            self.phase = CardPhase::Running(text.to_string());
        }
    }

    pub fn finished(&mut self, result: Result<String, String>) {
        self.phase = match result {
            Ok(_) => CardPhase::Done,
            Err(e) => CardPhase::Failed(e),
        };
    }

    pub fn set_service(&mut self, note: ServiceNote) {
        self.service = note;
    }

    pub fn handle_key(&mut self, key: KeyKind) -> UpdateAction {
        let Some(offer) = self.offer.clone() else {
            return UpdateAction::None;
        };
        let letter = match key {
            KeyKind::Char(c) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        let dismiss = matches!(key, KeyKind::Esc) || letter == Some('l');
        match self.phase.clone() {
            CardPhase::Offer | CardPhase::Failed(_) => {
                if dismiss {
                    self.later = Some(offer.version);
                    self.phase = CardPhase::Hidden;
                    return UpdateAction::None;
                }
                if letter == Some('s') && self.phase == CardPhase::Offer {
                    self.phase = CardPhase::Hidden;
                    return UpdateAction::Skip(offer.version);
                }
                if letter == Some('c') {
                    if let UpdatePlan::Command(cmd) = &offer.plan {
                        return UpdateAction::CopyCommand(cmd.clone());
                    }
                }
                if letter == Some('u') || matches!(key, KeyKind::Enter) {
                    return match &offer.plan {
                        UpdatePlan::InPlace => {
                            self.phase = CardPhase::Running("downloading…".to_string());
                            UpdateAction::StartInPlace
                        }
                        UpdatePlan::Command(cmd) => {
                            self.phase = CardPhase::Running(format!("running `{cmd}`…"));
                            UpdateAction::RunCommand(cmd.clone())
                        }
                    };
                }
                UpdateAction::None
            }
            CardPhase::Done => {
                if letter == Some('r') || matches!(key, KeyKind::Enter) {
                    return UpdateAction::Restart;
                }
                if letter == Some('v') && self.service == ServiceNote::Running {
                    self.service = ServiceNote::Restarting;
                    return UpdateAction::RestartService;
                }
                if dismiss {
                    self.later = Some(offer.version);
                    self.phase = CardPhase::Hidden;
                }
                UpdateAction::None
            }
            CardPhase::Hidden | CardPhase::Running(_) => UpdateAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(version: &str, plan: UpdatePlan) -> UpdateOffer {
        UpdateOffer {
            version: version.into(),
            current: "1.0.0".into(),
            notes: vec!["Faster startup".into()],
            url: "https://example.test/releases".into(),
            plan,
        }
    }

    fn key(c: char) -> KeyKind {
        KeyKind::Char(c)
    }

    #[test]
    fn an_offer_waits_for_the_turn_to_end() {
        let mut card = UpdateCard::default();
        card.offer(offer("2.0.0", UpdatePlan::InPlace));
        assert!(!card.visible(true), "never over a running turn");
        assert!(card.visible(false));
        assert!(!card.captures_keys(true, true));
    }

    #[test]
    fn letters_stay_in_a_composer_that_has_a_draft() {
        let mut card = UpdateCard::default();
        card.offer(offer("2.0.0", UpdatePlan::InPlace));
        assert!(card.captures_keys(false, true));
        assert!(!card.captures_keys(false, false));
    }

    #[test]
    fn update_now_runs_in_place_and_reports_done() {
        let mut card = UpdateCard::default();
        card.offer(offer("2.0.0", UpdatePlan::InPlace));
        assert_eq!(card.handle_key(key('u')), UpdateAction::StartInPlace);
        assert!(matches!(card.phase(), CardPhase::Running(_)));
        card.progress("verifying…");
        assert_eq!(card.phase(), &CardPhase::Running("verifying…".into()));
        assert!(!card.captures_keys(false, true), "running ignores keys");
        card.finished(Ok("2.0.0".into()));
        assert_eq!(card.phase(), &CardPhase::Done);
        assert_eq!(card.handle_key(key('r')), UpdateAction::Restart);
    }

    #[test]
    fn a_package_manager_install_runs_its_exact_command() {
        let mut card = UpdateCard::default();
        card.offer(offer(
            "2.0.0",
            UpdatePlan::Command("brew upgrade forge".into()),
        ));
        assert_eq!(
            card.handle_key(key('c')),
            UpdateAction::CopyCommand("brew upgrade forge".into())
        );
        assert_eq!(
            card.handle_key(key('u')),
            UpdateAction::RunCommand("brew upgrade forge".into())
        );
    }

    #[test]
    fn later_hides_the_version_for_this_process_only() {
        let mut card = UpdateCard::default();
        card.offer(offer("2.0.0", UpdatePlan::InPlace));
        assert_eq!(card.handle_key(key('l')), UpdateAction::None);
        assert_eq!(card.phase(), &CardPhase::Hidden);
        card.offer(offer("2.0.0", UpdatePlan::InPlace));
        assert_eq!(
            card.phase(),
            &CardPhase::Hidden,
            "same version stays put off"
        );
        card.offer(offer("2.1.0", UpdatePlan::InPlace));
        assert_eq!(card.phase(), &CardPhase::Offer, "a newer one is offered");
        let fresh_launch = UpdateCard::default();
        assert_eq!(fresh_launch.phase(), &CardPhase::Hidden);
    }

    #[test]
    fn skip_asks_the_shell_to_persist_the_version() {
        let mut card = UpdateCard::default();
        card.offer(offer("2.0.0", UpdatePlan::InPlace));
        assert_eq!(
            card.handle_key(key('s')),
            UpdateAction::Skip("2.0.0".into())
        );
        assert_eq!(card.phase(), &CardPhase::Hidden);
    }

    #[test]
    fn a_failed_install_can_be_retried_or_dismissed() {
        let mut card = UpdateCard::default();
        card.offer(offer("2.0.0", UpdatePlan::InPlace));
        card.handle_key(key('u'));
        card.finished(Err("permission denied".into()));
        assert!(card.visible(true), "a result is always shown");
        assert_eq!(card.handle_key(key('u')), UpdateAction::StartInPlace);
        card.finished(Err("again".into()));
        card.handle_key(KeyKind::Esc);
        assert_eq!(card.phase(), &CardPhase::Hidden);
    }

    #[test]
    fn service_restart_is_only_offered_once_a_service_is_known_to_run() {
        let mut card = UpdateCard::default();
        card.offer(offer("2.0.0", UpdatePlan::InPlace));
        card.handle_key(key('u'));
        card.finished(Ok("2.0.0".into()));
        assert_eq!(card.handle_key(key('v')), UpdateAction::None);
        card.set_service(ServiceNote::Running);
        assert_eq!(card.handle_key(key('v')), UpdateAction::RestartService);
        assert_eq!(card.service(), &ServiceNote::Restarting);
    }

    #[test]
    fn a_new_offer_never_replaces_an_install_in_flight() {
        let mut card = UpdateCard::default();
        card.offer(offer("2.0.0", UpdatePlan::InPlace));
        card.handle_key(key('u'));
        card.offer(offer("2.1.0", UpdatePlan::InPlace));
        assert!(matches!(card.phase(), CardPhase::Running(_)));
        assert_eq!(card.offer_ref().unwrap().version, "2.0.0");
    }
}
