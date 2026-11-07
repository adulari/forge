//! The `/goal --contract` flow (docs/features/goal-contract.md): before any work starts, one side
//! call drafts a short validation contract, the person approves it as a card, and after that an
//! independent validator runs every check itself and decides whether the goal is done.
//!
//! The validator makes no model call. It runs each assertion's command through the session's own
//! `shell` tool, after the permission broker has judged the line in plan mode, which lets a
//! command through only when it is provably read-only. A check that is refused, times out, exits
//! non-zero or misses its `contains` text fails its assertion, with the output tail as proof.

use super::*;
use crate::spend_guard::priced;
use forge_types::goal_contract::{
    card_steps, parse_contract, tail, worker_guidance, Assertion, AssertionResult, GoalContract,
    MAX_ASSERTIONS, MIN_ASSERTIONS, PROOF_CHARS,
};
use forge_types::SideEffect;

/// Purpose key for completion options (output ceiling, prompt cache key).
pub(crate) const GOAL_CONTRACT_PURPOSE: &str = "goal-contract";
const USAGE_PURPOSE: &str = "goal/contract";
const DRAFT_MAX_SECS: u64 = 120;
const MAX_REVISIONS: usize = 3;
const LISTING_ENTRIES: usize = 60;

/// The first words of the drafter's system prompt. The offline mock provider keys on it.
pub const GOAL_CONTRACT_MARKER: &str = "You are the goal contract drafter";

const GOAL_CONTRACT_SYSTEM: &str = "You are the goal contract drafter for an autonomous coding \
agent. Write the validation contract for the GOAL: 3 to 12 numbered assertions that must ALL be \
true when the goal is done, each with a concrete check an independent validator will run itself. \
Reply with ONE JSON object and nothing else:\n\
{\"assertions\": [{\"assertion\": \"what must be true\", \"check\": \"a read-only shell command\", \
\"expect\": \"the outcome in words, e.g. exit 0\", \"contains\": \"optional text the output must \
contain\"}]}\n\
Rules: the check must exit 0 exactly when the assertion holds. It must be READ-ONLY (ls, cat, \
head, tail, wc, grep, test, diff, jq, git status/log/diff/show, stat, ...): no builds, no tests \
that write, no redirects to files, no network. A behaviour or file statement becomes a command \
(`test -f path`, `grep -q 'text' path`, `git log --oneline | grep -q 'subject'`). Prefer checks \
that fail now and pass only when the work is really done. One fact per assertion, no duplicates.";

/// What the contract flow settled on, left for the goal loop.
#[derive(Debug, Clone, PartialEq)]
pub enum ContractOutcome {
    /// The person approved it; the goal runs against it.
    Approved(GoalContract),
    /// The person cancelled; the goal must not start.
    Declined,
    /// No usable contract could be drafted; the goal must not start.
    Failed(String),
}

fn listing(root: &std::path::Path) -> String {
    let Ok(entries) = std::fs::read_dir(root) else {
        return "(unreadable)".to_string();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() {
                format!("{name}/")
            } else {
                name
            }
        })
        .filter(|n| !n.starts_with(".git") || n == ".github/")
        .collect();
    names.sort();
    names.truncate(LISTING_ENTRIES);
    names.join("\n")
}

fn draft_input(goal: &str, listing: &str, feedback: &[String], rejected: &[String]) -> String {
    let mut out = format!(
        "GOAL:\n{}\n\nWorkspace top level:\n{listing}\n",
        goal.trim()
    );
    for f in feedback {
        out.push_str(&format!("\nThe person asked for these changes: {f}\n"));
    }
    if !rejected.is_empty() {
        out.push_str(
            "\nThese checks were refused because they are not read-only; replace them with \
             read-only checks:\n",
        );
        for r in rejected {
            out.push_str(&format!("- {r}\n"));
        }
    }
    out
}

impl Session {
    /// A `/goal --contract` is starting: the next turn drafts and asks for approval before it
    /// does anything else.
    pub fn request_goal_contract(&mut self, goal: &str) {
        self.pending_contract_goal = Some(goal.trim().to_string());
        self.contract_outcome = None;
    }

    /// `[goal] contract`: every goal drafts a contract, as if started with `--contract`.
    pub fn goal_contract_default(&self) -> bool {
        self.config.goal.contract
    }

    pub fn goal_contract_requested(&self) -> bool {
        self.pending_contract_goal.is_some()
    }

    /// The decision the contract flow left, once.
    pub fn take_goal_contract_outcome(&mut self) -> Option<ContractOutcome> {
        self.contract_outcome.take()
    }

    /// Run the pending request: draft, show the card, ask. Revisions loop until the person
    /// approves or cancels. Leaves the result in [`Session::take_goal_contract_outcome`].
    pub(crate) async fn resolve_goal_contract(&mut self) {
        let Some(goal) = self.pending_contract_goal.take() else {
            return;
        };
        let listing = listing(self.workspace.root());
        let mut feedback: Vec<String> = Vec::new();
        let outcome = loop {
            let assertions = match self.draft_assertions(&goal, &listing, &feedback).await {
                Ok(assertions) => assertions,
                Err(why) => break ContractOutcome::Failed(why),
            };
            let mut contract = GoalContract::new(assertions);
            self.presenter
                .emit(PresenterEvent::PlanProposed(forge_types::PlanProposal {
                    title: format!(
                        "Goal contract — {} assertions an independent validator will check",
                        contract.assertions.len()
                    ),
                    steps: card_steps(&contract.assertions)
                        .into_iter()
                        .map(|(title, detail)| forge_types::PlanStep { title, detail })
                        .collect(),
                    notes: Some(
                        "The goal is done only when every check passes. Checks are read-only."
                            .to_string(),
                    ),
                }));
            // Nobody to ask (a headless run, a daemon goal with no client attached) or standing
            // authorization (Bypass): the card above is the record, the run goes on.
            if self.mode == PermissionMode::Bypass || !self.presenter.is_attended() {
                contract.approved = true;
                break ContractOutcome::Approved(contract);
            }
            match self.ask_contract_approval(&contract) {
                ContractAnswer::Approve => {
                    contract.approved = true;
                    break ContractOutcome::Approved(contract);
                }
                ContractAnswer::Cancel => break ContractOutcome::Declined,
                ContractAnswer::Revise(text) => {
                    feedback.push(text);
                    if feedback.len() > MAX_REVISIONS {
                        break ContractOutcome::Failed(
                            "the contract was revised too many times".to_string(),
                        );
                    }
                }
            }
        };
        match &outcome {
            ContractOutcome::Approved(contract) => {
                if let Err(error) = self.prime_guidance(&[worker_guidance(contract)]) {
                    tracing::warn!(%error, "goal contract guidance was not recorded");
                }
                self.presenter.emit(PresenterEvent::Progress(format!(
                    "📜 goal contract approved — {} assertions",
                    contract.assertions.len()
                )));
            }
            ContractOutcome::Declined => {
                self.presenter.emit(PresenterEvent::Warning(
                    "goal contract declined — the goal was not started".to_string(),
                ));
            }
            ContractOutcome::Failed(why) => {
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "goal contract could not be drafted ({why}) — the goal was not started"
                )));
            }
        }
        self.contract_outcome = Some(outcome);
    }

    fn ask_contract_approval(&mut self, contract: &GoalContract) -> ContractAnswer {
        let n = contract.assertions.len();
        let question = format!(
            "Approve this goal contract? — {n} assertion{}; the goal is done only when every \
             check passes. Choose Approve / Cancel, or type changes to revise.",
            if n == 1 { "" } else { "s" }
        );
        let options = [
            forge_types::QChoice {
                label: "Approve".into(),
                description: "Start the goal; an independent validator checks every assertion"
                    .into(),
            },
            forge_types::QChoice {
                label: "Cancel".into(),
                description: "Do not start the goal".into(),
            },
        ];
        let answer = self.presenter.ask(&question, &options, true);
        let a = answer.trim();
        if a.eq_ignore_ascii_case("approve") || a.eq_ignore_ascii_case("yes") {
            ContractAnswer::Approve
        } else if a.is_empty()
            || a == forge_types::NO_ANSWER
            || a.eq_ignore_ascii_case("cancel")
            || a.eq_ignore_ascii_case("no")
        {
            ContractAnswer::Cancel
        } else {
            ContractAnswer::Revise(a.to_string())
        }
    }

    /// One drafting round with a retry when the reply holds fewer than three read-only checks.
    async fn draft_assertions(
        &mut self,
        goal: &str,
        listing: &str,
        feedback: &[String],
    ) -> Result<Vec<Assertion>, String> {
        let mut rejected: Vec<String> = Vec::new();
        for _ in 0..2 {
            let reply = self
                .contract_side_call(&draft_input(goal, listing, feedback, &rejected))
                .await?;
            let Some(parsed) = parse_contract(&reply) else {
                rejected.clear();
                continue;
            };
            let (kept, refused): (Vec<Assertion>, Vec<Assertion>) = parsed
                .into_iter()
                .partition(|a| crate::permission::is_read_only_shell(&a.command));
            if kept.len() >= MIN_ASSERTIONS {
                return Ok(renumber(kept));
            }
            rejected = refused.into_iter().map(|a| a.command).collect();
        }
        Err(format!(
            "the model did not return {MIN_ASSERTIONS} to {MAX_ASSERTIONS} runnable read-only assertions"
        ))
    }

    async fn contract_side_call(&mut self, input: &str) -> Result<String, String> {
        let budget = self.budget_snapshot();
        if budget.status() == BudgetStatus::Exhausted {
            return Err("budget exhausted".to_string());
        }
        let readiness = self.provider_readiness();
        let decision = self
            .router
            .route_hinted(
                "draft goal contract",
                false,
                budget,
                &readiness.health,
                &readiness.quota,
                Some(TaskTier::Trivial),
                self.pinned_effort,
                &self.project,
            )
            .await;
        let Some(model) = self.goal_evaluator_model(&decision) else {
            return Err("no side-call model available".to_string());
        };
        let messages = [
            Message::system(GOAL_CONTRACT_SYSTEM),
            Message::user(input.to_string()),
        ];
        self.presenter.emit(PresenterEvent::AuxiliaryRequest {
            model: model.clone(),
            purpose: "drafting goal contract".to_string(),
        });
        let provider = self.provider.clone();
        let options = Self::auxiliary_completion_options(&self.id, GOAL_CONTRACT_PURPOSE);
        let mut sink = |_: StreamEvent| {};
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(DRAFT_MAX_SECS),
            provider.complete_with(&model, &messages, &[], &options, &mut sink),
        )
        .await;
        let reply = match response {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) => return Err(forge_types::goal_contract::clip(&error.to_string(), 80)),
            Err(_) => return Err(format!("timed out after {DRAFT_MAX_SECS}s")),
        };
        if let Err(error) = self.store.record_side_call_usage_for(
            &self.id,
            USAGE_PURPOSE,
            Some(&model),
            &priced(&self.pricing, &model, &reply.usage),
        ) {
            tracing::warn!(%error, "goal contract usage was not recorded");
        }
        Ok(reply.content)
    }

    /// Run every check of an approved contract and return one result per assertion, in order.
    /// The goal loop records them and decides; nothing here changes the workspace.
    pub async fn validate_goal_contract(
        &mut self,
        contract: &GoalContract,
    ) -> Vec<AssertionResult> {
        let timeout = self.config.goal.contract_check_timeout_secs.clamp(1, 600);
        let mut results = Vec::with_capacity(contract.assertions.len());
        for assertion in &contract.assertions {
            results.push(self.run_contract_check(assertion, timeout).await);
        }
        let failing: Vec<String> = results
            .iter()
            .filter(|r| !r.passed)
            .map(|r| r.id.to_string())
            .collect();
        let passed = results.len() - failing.len();
        self.presenter
            .emit(PresenterEvent::Progress(if failing.is_empty() {
                format!("📜 contract {passed}/{passed} pass")
            } else {
                format!(
                    "📜 contract {passed}/{} pass · failing: {}",
                    results.len(),
                    failing.join(", ")
                )
            }));
        results
    }

    async fn run_contract_check(&mut self, assertion: &Assertion, timeout: u64) -> AssertionResult {
        let fail = |reason: String, proof: String| AssertionResult {
            id: assertion.id,
            passed: false,
            proof,
            reason,
        };
        let args = serde_json::json!({
            "command": assertion.command,
            "cwd": self.workspace.root(),
            "timeout_secs": timeout,
        });
        let workspace = self.workspace_binding.read().ok().map(|g| g.clone());
        let decision = crate::permission::decide_in(
            PermissionMode::Plan,
            SideEffect::Shell,
            "shell",
            &args,
            &self.rules,
            workspace.as_deref(),
        );
        if decision != PermissionDecision::Allow {
            return fail(
                "refused: the validator only runs commands it can prove read-only".to_string(),
                String::new(),
            );
        }
        let Some(tool) = self.tools.get("shell") else {
            return fail("the shell tool is not available".to_string(), String::new());
        };
        let run = tokio::time::timeout(
            std::time::Duration::from_secs(timeout + 15),
            tool.run_full(&args),
        )
        .await;
        let output = match run {
            Ok(Ok(run)) => run.model,
            Ok(Err(error)) => return fail(format!("could not run: {error}"), String::new()),
            Err(_) => return fail(format!("timed out after {timeout}s"), String::new()),
        };
        let first = output.lines().next().unwrap_or("").to_string();
        let body = output
            .split_once('\n')
            .map_or(String::new(), |(_, rest)| rest.to_string());
        let proof = tail(
            if body.trim().is_empty() {
                &first
            } else {
                &body
            },
            PROOF_CHARS,
        );
        if first.starts_with("shell: timed out") {
            return fail(format!("timed out after {timeout}s"), proof);
        }
        if forge_types::shell_result_failed(&output) {
            let reason = first.trim_start_matches("shell: ").to_string();
            return fail(reason, proof);
        }
        if let Some(needle) = assertion.contains.as_deref() {
            if !output.contains(needle) {
                return fail(format!("output does not contain \"{needle}\""), proof);
            }
        }
        AssertionResult {
            id: assertion.id,
            passed: true,
            proof,
            reason: String::new(),
        }
    }
}

enum ContractAnswer {
    Approve,
    Cancel,
    Revise(String),
}

fn renumber(mut assertions: Vec<Assertion>) -> Vec<Assertion> {
    for (i, a) in assertions.iter_mut().enumerate() {
        a.id = i as u32 + 1;
    }
    assertions
}
