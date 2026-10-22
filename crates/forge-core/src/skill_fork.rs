//! `context: fork`: a skill or command that runs as one subagent instead of in the main loop.
//!
//! The expanded skill text (methodology plus the typed task) is the subagent's task; this
//! session's context gets only the line the user typed and the subagent's answer, so a large
//! skill does not fill the main conversation (Claude Code's "Run skills in a subagent"). The
//! subagent is an ordinary `spawn_agents` child — same routing, permission gate, worktree
//! isolation and cost roll-up — of the skill's `agent:` type, with the skill's `model:` applied.
//! Unlike Claude Code it runs in the foreground: the answer ends the turn.

use std::collections::HashMap;

use forge_config::AgentDef;

use super::turn_grant::ForkTurn;
use super::*;

impl Session {
    pub(crate) async fn run_forked_turn(
        &mut self,
        prompt: &str,
        guidance: &[String],
        tier_override: Option<TaskTier>,
        fork: ForkTurn,
    ) -> Result<LoopOutcome, CoreError> {
        if !self.config.mesh.subagents.enabled {
            self.presenter.emit(PresenterEvent::Warning(
                "this skill asks to run in a subagent (`context: fork`), but subagents are \
                 disabled ([mesh.subagents] enabled = false) — running it inline"
                    .to_string(),
            ));
            return Box::pin(self.run_turn_inner(prompt, guidance, tier_override)).await;
        }
        let task = fork_task(prompt, guidance);
        let label = if fork.label.trim().is_empty() {
            prompt.to_string()
        } else {
            fork.label.clone()
        };
        let mode = format!("{:?}", self.mode);
        self.store
            .ensure_session(&self.id, &self.workspace.display(), &mode)?;
        let seq = self.next_seq();
        self.store
            .add_message(&self.id, seq, Role::User, &label, None)?;
        self.transcript.push(Message::user(&label));

        let mut agents = forge_config::load_agents_layered(
            self.workspace.root(),
            &self.config.mesh.subagents.agents_dir,
        );
        apply_skill_model(&mut agents, &fork);
        let ctx = subagent::AgentCtx {
            provider: Arc::clone(&self.provider),
            router: Arc::clone(&self.router),
            store: Arc::clone(&self.store),
            config: self.config.clone(),
            pricing: self.pricing.clone(),
            mode: self.mode,
            rules: self.rules.clone(),
            depth: 0,
            max_depth: self.config.mesh.subagents.max_depth,
            agents: Arc::new(agents),
            worktree_root: None,
            repo_root: self.workspace.root().to_path_buf(),
            effective_pin: self.effective_pin(),
            inherit_pin: !self.subagents_free(),
        };
        let request = subagent::AgentRequest {
            agent: fork.agent.clone(),
            task,
        };
        let budget = self.budget_snapshot();
        let parent_id = self.id.clone();
        let mut child_model: Option<String> = None;
        let presenter = &mut self.presenter;
        let mut on_event = |ev: subagent::Lifecycle| match ev {
            subagent::Lifecycle::Start {
                id,
                agent,
                task,
                model,
            } => {
                child_model = Some(model.to_string());
                presenter.emit(PresenterEvent::SubagentStart {
                    id: id.to_string(),
                    agent: agent.to_string(),
                    task: task.to_string(),
                    model: Some(model.to_string()),
                    phase: None,
                });
            }
            subagent::Lifecycle::Progress { id, snippet } => {
                presenter.emit(PresenterEvent::SubagentProgress {
                    id: id.to_string(),
                    snippet: snippet.to_string(),
                })
            }
            subagent::Lifecycle::Done {
                id,
                agent,
                ok,
                summary,
                cost_usd,
            } => presenter.emit(PresenterEvent::SubagentResult {
                id: id.to_string(),
                agent: agent.to_string(),
                ok,
                summary: summary.to_string(),
                cost_usd,
            }),
        };
        let (combined, ok) =
            subagent::orchestrate(&ctx, &parent_id, vec![request], budget, 1, &mut on_event)
                .await?;
        let _ = self
            .fire_lifecycle(
                forge_config::HookEvent::SubagentStop,
                serde_json::json!({ "ok": ok }),
            )
            .await;

        let text = strip_agent_label(&combined).to_string();
        let seq = self.next_seq();
        self.store.add_message(
            &self.id,
            seq,
            Role::Assistant,
            &text,
            child_model.as_deref(),
        )?;
        self.transcript.push(Message::assistant(&text));
        self.presenter
            .emit(PresenterEvent::AssistantDelta(text.clone()));
        self.presenter.emit(PresenterEvent::AssistantDone);
        let outcome = if ok {
            LoopOutcome::final_answer(text)
        } else {
            LoopOutcome::no_output(text)
        };
        self.presenter.emit(PresenterEvent::Done {
            final_text: outcome.text.clone(),
            stop_reason: outcome.stop_reason,
        });
        Ok(outcome)
    }
}

/// The subagent's task: the skill's methodology, then the typed task.
fn fork_task(prompt: &str, guidance: &[String]) -> String {
    guidance
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(prompt))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The skill's `model:` overrides the agent type's model for this one run.
fn apply_skill_model(agents: &mut HashMap<String, AgentDef>, fork: &ForkTurn) {
    let Some(model) = fork.model.as_deref() else {
        return;
    };
    let (tier, pin) = forge_config::agents::parse_model(model);
    if tier.is_none() && pin.is_none() {
        return;
    }
    let mut def = subagent::find_agent(agents, &fork.agent)
        .cloned()
        .unwrap_or_else(|| AgentDef {
            name: fork.agent.clone(),
            ..AgentDef::default()
        });
    def.tier = tier.or(def.tier);
    def.pinned_model = pin.or(def.pinned_model);
    // Keyed by the requested spelling so resolution finds this copy first.
    agents.insert(fork.agent.clone(), def);
}

/// `orchestrate` labels each child's answer `[agent 1: name]`; one forked child needs no label.
fn strip_agent_label(combined: &str) -> &str {
    match combined.strip_prefix("[agent 1: ") {
        Some(rest) => rest.split_once("]\n").map(|(_, text)| text).unwrap_or(rest),
        None => combined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_puts_methodology_before_the_typed_task() {
        assert_eq!(
            fork_task("PR 12", &["Summarize PRs.".into(), " ".into()]),
            "Summarize PRs.\n\nPR 12"
        );
        assert_eq!(fork_task("", &["Only body".into()]), "Only body");
    }

    #[test]
    fn the_single_child_label_is_stripped() {
        assert_eq!(
            strip_agent_label("[agent 1: explore]\nfound it"),
            "found it"
        );
        assert_eq!(strip_agent_label("plain"), "plain");
    }

    #[test]
    fn skill_model_pins_or_tiers_the_requested_agent() {
        let mut agents = forge_config::load_agents(std::path::Path::new("/nonexistent"));
        let fork = |agent: &str, model: &str| ForkTurn {
            agent: agent.into(),
            model: Some(model.into()),
            label: String::new(),
        };
        apply_skill_model(&mut agents, &fork("general-purpose", "haiku"));
        assert_eq!(agents["general-purpose"].tier, Some(TaskTier::Trivial));
        apply_skill_model(&mut agents, &fork("x", "inherit"));
        assert!(!agents.contains_key("x"), "inherit changes nothing");
        apply_skill_model(&mut agents, &fork("y", "p::m"));
        assert_eq!(agents["y"].pinned_model.as_deref(), Some("p::m"));
    }
}
