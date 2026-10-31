//! Saved crews in a turn: `@crew:<name>` in the prompt, or a crew selected for the session,
//! adds the crew's delegation directive to that turn's guidance.

use super::*;

impl Session {
    /// Allow or withhold delegation for this thread (`/delegation on|off`).
    pub fn set_delegation_allowed(&mut self, allowed: bool) {
        self.delegation_off = !allowed;
        crate::delegation_flag::set(&self.id, self.delegation_off);
    }

    /// The refusal for a delegation tool called while this thread has delegation off. The specs
    /// are hidden, but a transcript can still show the model earlier calls to copy.
    pub(crate) fn delegation_refusal(&self, tool: &str) -> Option<String> {
        const GATED: &[&str] = &[
            "send_to_agent",
            "steer_subagent",
            "run_workflow",
            "wait_for_subagents",
        ];
        (self.delegation_off && GATED.contains(&tool)).then(|| {
            "delegation is switched off for this thread by the user; do the work yourself with \
             your own tools"
                .to_string()
        })
    }

    /// Whether this thread may delegate to subagents.
    pub fn delegation_allowed(&self) -> bool {
        !self.delegation_off
    }

    /// Select (`Some`) or clear (`None`) the crew every turn of this session delegates to.
    /// `Err` names the problem (no such crew) and leaves the selection as it was.
    pub fn set_crew(&mut self, name: Option<&str>) -> Result<(), String> {
        let Some(name) = name else {
            self.active_crew = None;
            return Ok(());
        };
        let crews = forge_config::agents::crews::load_crews(self.workspace.root());
        if !crews.iter().any(|c| c.name == name) {
            let known: Vec<&str> = crews.iter().map(|c| c.name.as_str()).collect();
            return Err(format!(
                "no crew named '{name}'. Saved crews: [{}]",
                known.join(", ")
            ));
        }
        self.active_crew = Some(name.to_string());
        Ok(())
    }

    /// Every saved crew visible from this session's workspace.
    pub fn saved_crews(&self) -> Vec<forge_config::agents::crews::Crew> {
        forge_config::agents::crews::load_crews(self.workspace.root())
    }

    /// The crew selected for this session, if any.
    pub fn active_crew(&self) -> Option<&str> {
        self.active_crew.as_deref()
    }

    /// `guidance` plus a directive for every crew this turn names, and for the session's crew.
    /// A delegation-off thread gets none: the directive would tell the model to call a tool it
    /// does not have.
    pub(crate) fn guidance_with_crews(&self, prompt: &str, guidance: &[String]) -> Vec<String> {
        let mut out = guidance.to_vec();
        if self.delegation_off || !self.config.mesh.subagents.enabled {
            return out;
        }
        if self.active_crew.is_none() && !prompt.contains("@crew:") {
            return out;
        }
        let crews = forge_config::agents::crews::load_crews(self.workspace.root());
        let mut chosen = forge_config::agents::crews::mentioned(prompt, &crews);
        if chosen.is_empty() {
            chosen = crews
                .iter()
                .filter(|c| Some(c.name.as_str()) == self.active_crew.as_deref())
                .collect();
        }
        out.extend(
            chosen
                .into_iter()
                .map(forge_config::agents::crews::directive),
        );
        out
    }
}
