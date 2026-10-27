//! "Set up everything recommended": the actions that need no decision, in the order a front end
//! should run them, plus where a browser sign-in or a real choice stops the run.

use serde::{Deserialize, Serialize};

use crate::action::{Action, Exposure};
use crate::machine::Machine;
use crate::state::{OnboardingState, StepStatus};
use crate::step::{StepDef, StepId, Surface};

/// One line of the recommended checklist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recommended {
    /// The step this belongs to; it is marked done when the item succeeds.
    pub step: StepId,
    /// What the checklist says, plain: "Reconnect Claude Code".
    pub title: String,
    /// What to run. `None` means the person has to choose something (the model step).
    pub action: Option<Action>,
    /// The action opens a browser sign-in, so the run pauses on it until the person finishes.
    pub needs_you: bool,
}

/// The checklist for this machine. Items whose step does not apply, or that are already done on
/// the machine, are left out, so an up-to-date install gets an empty list.
pub fn recommended(
    machine: &Machine,
    state: &OnboardingState,
    surface: Surface,
) -> Vec<Recommended> {
    let applies = |step: StepId| step.def().applies(machine, surface);
    let pending = |step: StepId| state.status(step) == StepStatus::Pending;
    let mut out = Vec::new();

    if machine.claude_needs_reconnect() {
        out.push(Recommended {
            step: StepId::Models,
            title: "Reconnect Claude Code".into(),
            action: Some(Action::ConnectClaude),
            needs_you: true,
        });
    } else if machine.ready_sources().is_empty() {
        out.push(Recommended {
            step: StepId::Models,
            title: "Connect a model".into(),
            action: None,
            needs_you: true,
        });
    }

    if applies(StepId::Behavior) && pending(StepId::Behavior) {
        let settings = &machine.settings;
        let permission = if settings.permission.is_empty() {
            "default"
        } else {
            &settings.permission
        };
        let credit = if settings.credit_mode.is_empty() {
            "normal"
        } else {
            &settings.credit_mode
        };
        out.push(Recommended {
            step: StepId::Behavior,
            title: "Keep the safe defaults: ask before edits, best model per task".into(),
            action: Some(Action::SetBehavior {
                permission: permission.to_string(),
                credit_mode: credit.to_string(),
            }),
            needs_you: false,
        });
    }

    if applies(StepId::Import) && pending(StepId::Import) {
        let i = &machine.imports;
        for (found, tool) in [
            (i.claude_dir, "claude"),
            (i.codex_dir, "codex"),
            (i.cursor_dir, "cursor"),
        ] {
            if found {
                out.push(Recommended {
                    step: StepId::Import,
                    title: format!("Import commands and skills from {tool}"),
                    action: Some(Action::ImportConfig { tool: tool.into() }),
                    needs_you: false,
                });
            }
        }
        if i.claude_dir && !i.keybinds_active {
            out.push(Recommended {
                step: StepId::Import,
                title: "Use Claude Code key bindings".into(),
                action: Some(Action::SetKeybindPreset {
                    preset: "claude-code".into(),
                }),
                needs_you: false,
            });
        }
    }

    let service_def: &StepDef = StepId::Service.def();
    let service_done = machine.service.installed && machine.service.running;
    if service_def.applies(machine, surface)
        && machine.service.supported
        && !service_done
        && (pending(StepId::Service) || service_def.required(surface))
    {
        out.push(Recommended {
            step: StepId::Service,
            title: "Install the background service (this computer only)".into(),
            action: Some(Action::InstallService {
                exposure: Exposure::Local,
            }),
            needs_you: false,
        });
    }

    if applies(StepId::Reporting) && !machine.settings.reporting_chosen {
        out.push(Recommended {
            step: StepId::Reporting,
            title: "Send scrubbed error reports automatically".into(),
            action: Some(Action::SetReporting {
                mode: "auto".into(),
            }),
            needs_you: false,
        });
    }
    out
}

/// Steps the recommended run settles without an action: they work with their defaults (keyless
/// web search, shared benchmark data, notification and theme defaults).
pub const DEFAULT_ONLY_STEPS: [StepId; 3] =
    [StepId::WebSearch, StepId::Benchmarks, StepId::Preferences];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{BridgeFacts, KeyFacts, Login, ServiceFacts};

    fn machine() -> Machine {
        let mut m = Machine::default();
        m.forge.installed = true;
        m.service = ServiceFacts {
            supported: true,
            ..ServiceFacts::default()
        };
        m
    }

    fn ready_key(m: &mut Machine) {
        m.keys.push(KeyFacts {
            id: "groq".into(),
            name: "Groq".into(),
            configured: true,
            featured: true,
            ..KeyFacts::default()
        });
    }

    #[test]
    fn a_machine_with_no_model_stops_at_the_model_choice() {
        let items = recommended(&machine(), &OnboardingState::new(), Surface::Tui);
        assert_eq!(items[0].step, StepId::Models);
        assert!(items[0].action.is_none() && items[0].needs_you);
    }

    #[test]
    fn a_machine_that_works_gets_every_decision_free_item_with_safe_defaults() {
        let mut m = machine();
        ready_key(&mut m);
        m.imports.claude_dir = true;
        m.imports.commands = 4;
        let items = recommended(&m, &OnboardingState::new(), Surface::Tui);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert!(
            titles
                .iter()
                .any(|t| t.starts_with("Keep the safe defaults")),
            "{titles:?}"
        );
        assert!(
            titles.iter().any(|t| t.contains("from claude")),
            "{titles:?}"
        );
        assert!(titles.iter().any(|t| t.contains("key bindings")));
        assert!(titles.iter().any(|t| t.contains("background service")));
        assert!(titles.iter().any(|t| t.contains("error reports")));
        assert!(items.iter().all(|i| !i.needs_you && i.action.is_some()));
        assert!(
            items
                .iter()
                .all(|i| !matches!(i.action, Some(Action::ImportMcp))),
            "the MCP picker is a real choice and is left to the step"
        );
        let Some(Action::InstallService { exposure }) = items.iter().find_map(|i| {
            i.action
                .clone()
                .filter(|a| matches!(a, Action::InstallService { .. }))
        }) else {
            panic!("service missing");
        };
        assert_eq!(
            exposure,
            Exposure::Local,
            "never opens the network by default"
        );
    }

    #[test]
    fn claude_with_its_own_login_but_no_token_leads_with_a_reconnect_that_needs_the_person() {
        let mut m = machine();
        m.bridges.push(BridgeFacts {
            id: "claude-cli".into(),
            name: "Claude Code".into(),
            installed: true,
            login: Login::SignedOut,
            main_login: true,
            ..BridgeFacts::default()
        });
        let items = recommended(&m, &OnboardingState::new(), Surface::Tui);
        assert_eq!(items[0].action, Some(Action::ConnectClaude));
        assert!(items[0].needs_you);
        assert!(items[0].title.contains("Reconnect Claude Code"));
    }

    #[test]
    fn what_is_already_done_is_left_out() {
        let mut m = machine();
        ready_key(&mut m);
        m.service.installed = true;
        m.service.running = true;
        m.settings.reporting_chosen = true;
        let mut state = OnboardingState::new();
        state.mark_done(StepId::Behavior);
        assert!(recommended(&m, &state, Surface::Tui).is_empty());
    }

    #[test]
    fn the_desktop_always_wants_the_service() {
        let mut m = machine();
        ready_key(&mut m);
        let mut state = OnboardingState::new();
        state.mark_skipped(StepId::Service);
        assert!(recommended(&m, &state, Surface::Tui)
            .iter()
            .all(|i| !matches!(i.action, Some(Action::InstallService { .. }))));
        assert!(recommended(&m, &state, Surface::Desktop)
            .iter()
            .any(|i| matches!(i.action, Some(Action::InstallService { .. }))));
    }

    #[test]
    fn recommendations_serialize_for_the_daemon() {
        let json = serde_json::to_value(Recommended {
            step: StepId::Models,
            title: "Reconnect Claude Code".into(),
            action: Some(Action::ConnectClaude),
            needs_you: true,
        })
        .unwrap();
        assert_eq!(json["action"]["action"], "connect_claude");
        assert_eq!(json["step"], "models");
    }
}
