//! The `/goal --mission` flow (docs/features/goal-contract.md, "Missions (milestones)"): one side
//! call drafts an ordered list of milestones, each with its own read-only assertions, the person
//! approves the whole mission on one card, and from then on it is an ordinary approved
//! [`GoalContract`] whose `mission` field says which milestone is current. The validator, the
//! relay and every surface work on that contract; this module only drafts and asks.

use super::*;
use crate::goal_contract::{
    draft_input, listing, parse_answer, ContractAnswer, ContractOutcome, MAX_REVISIONS,
};
use forge_types::goal_contract::mission::{
    parse_mission, MissionDraft, MAX_MILESTONES_HARD, MAX_MILESTONE_ASSERTIONS, MIN_MILESTONES,
    MIN_MILESTONE_ASSERTIONS,
};
use forge_types::goal_contract::worker_guidance;

/// The first words of the planner's system prompt. The offline mock provider keys on it.
pub const GOAL_MISSION_MARKER: &str = "You are the goal mission planner";

fn system_prompt(max_milestones: usize) -> String {
    format!(
        "{GOAL_MISSION_MARKER} for an autonomous coding agent. Split the GOAL into \
{MIN_MILESTONES} to {max_milestones} MILESTONES done strictly in order, each by a fresh worker \
with a narrow context. Each milestone gets {MIN_MILESTONE_ASSERTIONS} to \
{MAX_MILESTONE_ASSERTIONS} assertions that must ALL be true when that milestone is done, each \
with a concrete check an independent validator will run itself. Reply with ONE JSON object and \
nothing else:\n\
{{\"milestones\": [{{\"title\": \"what this milestone delivers\", \"assertions\": \
[{{\"assertion\": \"what must be true\", \"check\": \"a read-only shell command\", \
\"expect\": \"the outcome in words, e.g. exit 0\", \"contains\": \"optional text the output must \
contain\"}}]}}]}}\n\
Rules: the check must exit 0 exactly when the assertion holds. It must be READ-ONLY (ls, cat, \
head, tail, wc, grep, test, diff, jq, git status/log/diff/show, stat, ...): no builds, no tests \
that write, no redirects to files, no network. A behaviour or file statement becomes a command \
(`test -f path`, `grep -q 'text' path`, `git log --oneline | grep -q 'subject'`). Later \
milestones build on earlier ones, and earlier milestones' checks are re-run at every boundary, \
so never write a check a later milestone will break. Prefer checks that fail now and pass only \
when the work is really done. One fact per assertion, no duplicates."
    )
}

impl Session {
    /// A `/goal --mission` is starting: the next turn drafts the milestones and asks for approval
    /// before it does anything else.
    pub fn request_goal_mission(&mut self, goal: &str) {
        self.request_goal_contract(goal);
        self.pending_mission = true;
    }

    /// `[goal] mission_max_milestones`, clamped to what a mission can hold.
    pub fn mission_milestone_cap(&self) -> usize {
        self.config
            .goal
            .mission_max_milestones
            .clamp(MIN_MILESTONES, MAX_MILESTONES_HARD)
    }

    /// Draft, show the card, ask; revisions loop until the person approves or cancels. Leaves the
    /// result in [`Session::take_goal_contract_outcome`], like a flat contract.
    pub(crate) async fn resolve_goal_mission(&mut self) {
        let Some(goal) = self.pending_contract_goal.take() else {
            return;
        };
        let listing = listing(self.workspace.root());
        let mut feedback: Vec<String> = Vec::new();
        let outcome = loop {
            let draft = match self.draft_mission(&goal, &listing, &feedback).await {
                Ok(draft) => draft,
                Err(why) => break ContractOutcome::Failed(why),
            };
            let steps = draft.card_steps();
            let Some(mut contract) = draft.into_contract() else {
                break ContractOutcome::Failed("the mission draft was not usable".to_string());
            };
            self.presenter
                .emit(PresenterEvent::PlanProposed(forge_types::PlanProposal {
                    title: format!(
                    "Mission - {} milestones, {} assertions an independent validator will check",
                    steps.len(),
                    contract.assertions.len()
                ),
                    steps: steps
                        .into_iter()
                        .map(|(title, detail)| forge_types::PlanStep { title, detail })
                        .collect(),
                    notes: Some(
                        "Milestones run in order, each in a fresh worker; earlier milestones' \
                         checks are re-run at every boundary. Checks are read-only."
                            .to_string(),
                    ),
                }));
            if self.mode == PermissionMode::Bypass || !self.presenter.is_attended() {
                contract.approved = true;
                break ContractOutcome::Approved(contract);
            }
            match self.ask_mission_approval(&contract) {
                ContractAnswer::Approve => {
                    contract.approved = true;
                    break ContractOutcome::Approved(contract);
                }
                ContractAnswer::Cancel => break ContractOutcome::Declined,
                ContractAnswer::Revise(text) => {
                    feedback.push(text);
                    if feedback.len() > MAX_REVISIONS {
                        break ContractOutcome::Failed(
                            "the mission was revised too many times".to_string(),
                        );
                    }
                }
            }
        };
        match &outcome {
            ContractOutcome::Approved(contract) => {
                if let Err(error) = self.prime_guidance(&[worker_guidance(contract)]) {
                    tracing::warn!(%error, "mission guidance was not recorded");
                }
                self.presenter.emit(PresenterEvent::Progress(format!(
                    "🧭 mission approved - {}",
                    contract.headline()
                )));
            }
            ContractOutcome::Declined => {
                self.presenter.emit(PresenterEvent::Warning(
                    "mission declined - the goal was not started".to_string(),
                ));
            }
            ContractOutcome::Failed(why) => {
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "mission could not be drafted ({why}) - the goal was not started"
                )));
            }
        }
        self.contract_outcome = Some(outcome);
    }

    fn ask_mission_approval(
        &mut self,
        contract: &forge_types::goal_contract::GoalContract,
    ) -> ContractAnswer {
        let milestones = contract.mission_ref().map_or(0, |m| m.total());
        let question = format!(
            "Approve this mission? - {milestones} milestones, {} assertions; each milestone runs \
             in a fresh worker once the one before it passes. Choose Approve / Cancel, or type \
             changes to revise.",
            contract.assertions.len()
        );
        let options = [
            forge_types::QChoice {
                label: "Approve".into(),
                description: "Start the mission; an independent validator checks every milestone"
                    .into(),
            },
            forge_types::QChoice {
                label: "Cancel".into(),
                description: "Do not start the goal".into(),
            },
        ];
        parse_answer(&self.presenter.ask(&question, &options, true))
    }

    /// One drafting round with a retry when the reply holds fewer than two milestones of
    /// read-only checks.
    async fn draft_mission(
        &mut self,
        goal: &str,
        listing: &str,
        feedback: &[String],
    ) -> Result<MissionDraft, String> {
        let cap = self.mission_milestone_cap();
        let mut rejected: Vec<String> = Vec::new();
        for _ in 0..2 {
            let input = draft_input(goal, listing, feedback, &rejected);
            let reply = self.drafter_side_call(&system_prompt(cap), &input).await?;
            let Some(mut draft) = parse_mission(&reply, cap) else {
                rejected.clear();
                continue;
            };
            let refused: Vec<String> = draft
                .milestones
                .iter()
                .flat_map(|m| m.assertions.iter())
                .filter(|a| !crate::permission::is_read_only_shell(&a.command))
                .map(|a| a.command.clone())
                .collect();
            draft.retain(|a| crate::permission::is_read_only_shell(&a.command));
            if draft.is_usable() {
                return Ok(draft);
            }
            rejected = refused;
        }
        Err(format!(
            "the model did not return {MIN_MILESTONES} to {cap} milestones of runnable read-only assertions"
        ))
    }
}
