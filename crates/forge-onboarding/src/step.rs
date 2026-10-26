//! The ordered step catalog. Both front ends render the same steps; each surface asks which of
//! them apply to it and which are required there.

use serde::{Deserialize, Serialize};

use crate::machine::Machine;
use crate::state::{OnboardingState, StepStatus};

/// Bumped when a release adds a step. A state file written under an older flow shows only the
/// steps added since (once), never the whole flow again.
pub const FLOW_VERSION: u32 = 1;

/// Where the onboarding is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Tui,
    Desktop,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Tui => "tui",
            Surface::Desktop => "desktop",
        }
    }
}

/// Stable step identifiers, written to the state file and the daemon API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepId {
    /// Desktop only: the app has no `forge` binary yet, so it installs one.
    InstallCli,
    Welcome,
    Models,
    Behavior,
    WebSearch,
    Benchmarks,
    Import,
    Service,
    Anywhere,
    Surfaces,
    Preferences,
    /// Scrubbed automatic error reports: automatic, ask each time, or off.
    Reporting,
    Done,
}

impl StepId {
    pub const ALL: [StepId; 13] = [
        StepId::InstallCli,
        StepId::Welcome,
        StepId::Models,
        StepId::Behavior,
        StepId::WebSearch,
        StepId::Benchmarks,
        StepId::Import,
        StepId::Service,
        StepId::Anywhere,
        StepId::Surfaces,
        StepId::Preferences,
        StepId::Reporting,
        StepId::Done,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            StepId::InstallCli => "install_cli",
            StepId::Welcome => "welcome",
            StepId::Models => "models",
            StepId::Behavior => "behavior",
            StepId::WebSearch => "web_search",
            StepId::Benchmarks => "benchmarks",
            StepId::Import => "import",
            StepId::Service => "service",
            StepId::Anywhere => "anywhere",
            StepId::Surfaces => "surfaces",
            StepId::Preferences => "preferences",
            StepId::Reporting => "reporting",
            StepId::Done => "done",
        }
    }

    pub fn parse(s: &str) -> Option<StepId> {
        StepId::ALL.into_iter().find(|id| id.as_str() == s)
    }

    pub fn def(self) -> &'static StepDef {
        catalog()
            .iter()
            .find(|d| d.id == self)
            .expect("every StepId has a catalog entry")
    }
}

/// One catalog entry: static copy and rules. What was detected on the machine is not here; that
/// is [`Machine`].
#[derive(Debug, Clone, Copy)]
pub struct StepDef {
    pub id: StepId,
    /// Short name for the step rail.
    pub title: &'static str,
    /// One plain sentence: what this step is for.
    pub summary: &'static str,
    /// Surfaces that can show the step at all.
    pub surfaces: &'static [Surface],
    /// Surfaces where the step blocks completion until it passes.
    pub required_on: &'static [Surface],
    /// The [`FLOW_VERSION`] that introduced the step.
    pub since_flow: u32,
}

const BOTH: &[Surface] = &[Surface::Tui, Surface::Desktop];
const DESKTOP: &[Surface] = &[Surface::Desktop];
const NONE: &[Surface] = &[];

static CATALOG: [StepDef; 13] = [
    StepDef {
        id: StepId::InstallCli,
        title: "Install Forge",
        summary: "The desktop app needs the forge command to run sessions. Forge installs it for you.",
        surfaces: DESKTOP,
        required_on: DESKTOP,
        since_flow: 1,
    },
    StepDef {
        id: StepId::Welcome,
        title: "Welcome",
        summary: "What Forge found on this machine and what setup will do. About two minutes.",
        surfaces: BOTH,
        required_on: NONE,
        since_flow: 1,
    },
    StepDef {
        id: StepId::Models,
        title: "Models",
        summary: "Connect at least one model. Forge routes each task to the best one you have.",
        surfaces: BOTH,
        required_on: BOTH,
        since_flow: 1,
    },
    StepDef {
        id: StepId::Behavior,
        title: "Safety and cost",
        summary: "How careful Forge is with your files and how it spends your credits.",
        surfaces: BOTH,
        required_on: NONE,
        since_flow: 1,
    },
    StepDef {
        id: StepId::WebSearch,
        title: "Web search",
        summary: "Search works with no key. Add a key or a local search engine for better results.",
        surfaces: BOTH,
        required_on: NONE,
        since_flow: 1,
    },
    StepDef {
        id: StepId::Benchmarks,
        title: "Benchmarks",
        summary: "Forge ranks models with public benchmark data. No key needed.",
        surfaces: BOTH,
        required_on: NONE,
        since_flow: 1,
    },
    StepDef {
        id: StepId::Import,
        title: "Bring your setup",
        summary: "Carry over commands, skills, MCP servers and key bindings from other tools.",
        surfaces: BOTH,
        required_on: NONE,
        since_flow: 1,
    },
    StepDef {
        id: StepId::Service,
        title: "Background service",
        summary: "Keeps Forge running so the desktop app and your phone can reach it.",
        surfaces: BOTH,
        required_on: DESKTOP,
        since_flow: 1,
    },
    StepDef {
        id: StepId::Anywhere,
        title: "Forge Anywhere",
        summary: "Optional: reach your sessions from your phone, end to end encrypted.",
        surfaces: BOTH,
        required_on: NONE,
        since_flow: 1,
    },
    StepDef {
        id: StepId::Surfaces,
        title: "Other places",
        summary: "Optional: the desktop app from the terminal, or pairing your phone from the desktop.",
        surfaces: BOTH,
        required_on: NONE,
        since_flow: 1,
    },
    StepDef {
        id: StepId::Preferences,
        title: "Notifications and theme",
        summary: "Two quick switches: get told when a long turn ends, and pick light or dark.",
        surfaces: BOTH,
        required_on: NONE,
        since_flow: 1,
    },
    StepDef {
        id: StepId::Reporting,
        title: "Error reports",
        summary: "Help get bugs fixed with scrubbed automatic reports. Never your prompts, code, paths or secrets.",
        surfaces: BOTH,
        required_on: NONE,
        since_flow: 1,
    },
    StepDef {
        id: StepId::Done,
        title: "Ready",
        summary: "What works now, and your first session.",
        surfaces: BOTH,
        required_on: NONE,
        since_flow: 1,
    },
];

/// The whole catalog, in order.
pub fn catalog() -> &'static [StepDef] {
    &CATALOG
}

impl StepDef {
    pub fn required(&self, surface: Surface) -> bool {
        self.required_on.contains(&surface)
    }

    /// Whether the step is shown on `surface` for this machine. A step that has nothing to
    /// offer (nothing to import, desktop app already installed) is dropped, not shown empty.
    pub fn applies(&self, machine: &Machine, surface: Surface) -> bool {
        if !self.surfaces.contains(&surface) {
            return false;
        }
        match self.id {
            StepId::InstallCli => !machine.forge.installed,
            StepId::Import => machine.imports.anything(),
            StepId::Surfaces => match surface {
                Surface::Tui => machine.desktop.supported && !machine.desktop.installed,
                Surface::Desktop => true,
            },
            _ => true,
        }
    }
}

/// How a run walks the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanMode {
    /// `forge setup`: every applicable step, from the welcome page.
    Full,
    /// Opened automatically because something required is missing: every applicable step, the
    /// cursor on the first one still open.
    Resume,
    /// A newer flow added steps: only those, once.
    WhatsNew,
}

/// One row of the step rail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: StepId,
    pub title: String,
    pub summary: String,
    pub required: bool,
    pub status: StepStatus,
    /// Added after the flow this state file was written under.
    pub new: bool,
    /// The machine already has what the step sets up (a working model, a running service), so a
    /// returning user sees it as done and the run lands on something that still needs them.
    pub satisfied: bool,
}

/// Whether detection says the step has nothing left to do.
pub fn satisfied(id: StepId, machine: &Machine, state: &OnboardingState, surface: Surface) -> bool {
    match id {
        StepId::InstallCli => machine.forge.installed,
        StepId::Welcome | StepId::Done => false,
        StepId::Models => {
            !machine.claude_needs_reconnect()
                && crate::state::gate(state, machine, surface)
                    .missing
                    .iter()
                    .all(|m| *m != StepId::Models)
        }
        StepId::Behavior => false,
        StepId::WebSearch => machine.search.brave_key || machine.search.searxng_running,
        StepId::Benchmarks => false,
        StepId::Import => machine.imports.keybinds_active,
        StepId::Service => machine.service.installed && machine.service.running,
        StepId::Anywhere => machine.anywhere.signed_in,
        StepId::Surfaces => machine.desktop.installed,
        StepId::Preferences => machine.settings.notifications,
        StepId::Reporting => machine.settings.reporting_chosen,
    }
}

/// The steps a run shows, in order.
pub fn plan(
    state: &OnboardingState,
    machine: &Machine,
    surface: Surface,
    mode: PlanMode,
) -> Vec<PlanStep> {
    catalog()
        .iter()
        .filter(|def| def.applies(machine, surface))
        .filter(|def| match mode {
            PlanMode::WhatsNew => def.since_flow > state.flow_version && !def.required(surface),
            _ => true,
        })
        .map(|def| PlanStep {
            id: def.id,
            title: def.title.to_string(),
            summary: def.summary.to_string(),
            required: def.required(surface),
            status: if mode == PlanMode::Full && def.id == StepId::Welcome {
                StepStatus::Pending
            } else {
                state.status(def.id)
            },
            new: def.since_flow > state.flow_version,
            satisfied: satisfied(def.id, machine, state, surface),
        })
        .collect()
}

/// Index into `plan` of the step a run opens on: the first required step that is not done, else
/// the first step still pending, else the last (the summary). A full run always starts at 0.
pub fn start_index(plan: &[PlanStep], mode: PlanMode) -> usize {
    if mode == PlanMode::Full {
        return 0;
    }
    // A first run opens on the welcome: what was found, and the one-key setup. A run picked back
    // up after leaving (the welcome was passed) goes to the first step still open.
    if plan
        .first()
        .is_some_and(|s| s.id == StepId::Welcome && s.status == StepStatus::Pending)
    {
        return 0;
    }
    plan.iter()
        .position(|s| {
            s.id != StepId::Welcome && s.required && s.status != StepStatus::Done && !s.satisfied
        })
        .or_else(|| {
            plan.iter().position(|s| {
                s.id != StepId::Welcome
                    && s.id != StepId::Done
                    && s.status == StepStatus::Pending
                    && !s.satisfied
            })
        })
        .unwrap_or_else(|| plan.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_step_id_has_exactly_one_catalog_entry_in_order() {
        let ids: Vec<StepId> = catalog().iter().map(|d| d.id).collect();
        assert_eq!(ids, StepId::ALL.to_vec());
        for id in StepId::ALL {
            assert_eq!(StepId::parse(id.as_str()), Some(id));
            assert_eq!(id.def().id, id);
        }
    }

    #[test]
    fn step_ids_serialize_as_their_snake_case_names() {
        for id in StepId::ALL {
            assert_eq!(
                serde_json::to_string(&id).unwrap(),
                format!("\"{}\"", id.as_str())
            );
        }
    }

    #[test]
    fn models_is_required_everywhere_and_the_service_only_on_desktop() {
        assert!(StepId::Models.def().required(Surface::Tui));
        assert!(StepId::Models.def().required(Surface::Desktop));
        assert!(!StepId::Service.def().required(Surface::Tui));
        assert!(StepId::Service.def().required(Surface::Desktop));
    }

    #[test]
    fn install_cli_exists_for_the_desktop_only() {
        let mut machine = Machine::default();
        machine.forge.installed = false;
        assert!(!StepId::InstallCli.def().applies(&machine, Surface::Tui));
        assert!(StepId::InstallCli.def().applies(&machine, Surface::Desktop));
        machine.forge.installed = true;
        assert!(!StepId::InstallCli.def().applies(&machine, Surface::Desktop));
    }

    #[test]
    fn steps_with_nothing_to_offer_are_dropped() {
        let mut machine = Machine::default();
        machine.forge.installed = true;
        assert!(!StepId::Import.def().applies(&machine, Surface::Tui));
        machine.imports.claude_dir = true;
        assert!(StepId::Import.def().applies(&machine, Surface::Tui));

        machine.desktop.supported = true;
        assert!(StepId::Surfaces.def().applies(&machine, Surface::Tui));
        machine.desktop.installed = true;
        assert!(!StepId::Surfaces.def().applies(&machine, Surface::Tui));
        assert!(StepId::Surfaces.def().applies(&machine, Surface::Desktop));
    }

    #[test]
    fn a_first_run_opens_on_the_welcome_and_a_resumed_one_on_the_first_open_required_step() {
        let mut machine = Machine::default();
        machine.forge.installed = true;
        let mut state = OnboardingState::new();
        let p = plan(&state, &machine, Surface::Tui, PlanMode::Resume);
        assert_eq!(p[start_index(&p, PlanMode::Resume)].id, StepId::Welcome);
        state.mark_done(StepId::Welcome);
        let p = plan(&state, &machine, Surface::Tui, PlanMode::Resume);
        assert_eq!(p[start_index(&p, PlanMode::Resume)].id, StepId::Models);
        let full = plan(&state, &machine, Surface::Tui, PlanMode::Full);
        assert_eq!(start_index(&full, PlanMode::Full), 0);
        assert_eq!(full[0].id, StepId::Welcome);
    }

    #[test]
    fn a_whats_new_run_lists_only_steps_added_since_the_recorded_flow() {
        let mut machine = Machine::default();
        machine.forge.installed = true;
        let mut state = OnboardingState::new();
        state.flow_version = FLOW_VERSION;
        assert!(plan(&state, &machine, Surface::Tui, PlanMode::WhatsNew).is_empty());
        state.flow_version = 0;
        let p = plan(&state, &machine, Surface::Tui, PlanMode::WhatsNew);
        assert!(!p.is_empty());
        assert!(p.iter().all(|s| !s.required && s.new));
    }
}
