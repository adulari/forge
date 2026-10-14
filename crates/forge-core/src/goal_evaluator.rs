//! The `/goal` progress evaluator (docs/features/goal-evaluator.md).
//!
//! After each goal turn one cheap side call judges the goal against what was actually done — the
//! task plan, the turn's final answer, commits, the diff, gate results and recorded checks — and
//! returns a structured verdict: achieved or not, what is still missing, the single next step.
//! The goal loop builds its next prompt from that verdict instead of a fixed "continue" string.
//!
//! Best-effort like every side call: when it is switched off, no model is usable, the call fails
//! or times out, or the reply is not a usable verdict, [`Session::evaluate_goal`] returns `None`
//! and the loop falls back to its fixed prompt. The evaluator can never stall a goal.

use super::*;
use crate::compaction_policy::record_model_failure_in;
use crate::spend_guard::priced;

/// Purpose key for completion options (output ceiling, prompt cache key).
pub(crate) const GOAL_EVALUATOR_PURPOSE: &str = "goal-evaluate";
/// Usage-row purpose, alongside `shell/diagnose` and the other side calls.
const USAGE_PURPOSE: &str = "goal/evaluate";
const GOAL_EVAL_MAX_SECS: u64 = 90;
const MAX_ITEMS: usize = 6;
const ITEM_CHARS: usize = 160;
const ANSWER_CHARS: usize = 3_000;
const GATE_CHARS: usize = 1_500;

/// The first words of the evaluator's system prompt. The offline mock provider keys on it.
pub const GOAL_EVALUATOR_MARKER: &str = "You are the goal progress evaluator";

const GOAL_EVALUATOR_SYSTEM: &str = "You are the goal progress evaluator for an autonomous coding \
agent. Judge how far the GOAL is from done using ONLY the evidence given: the task plan, the \
agent's final message, commits, the diff, quality-gate results and recorded checks. The agent's own \
claims are not proof: a requirement is met only when the commits or the diff show the work and \
nothing contradicts it. Reply with ONE JSON object and nothing else:\n\
{\"achieved\": true|false, \"remaining\": [\"concrete missing item\", ...], \"next_step\": \"the \
single highest-value next action\", \"confidence\": 0.0-1.0, \"progressed\": true|false, \
\"reason\": \"one line\"}\n\
achieved is true only when every part of the goal is met; then remaining is []. Each remaining \
item is a short, concrete deliverable (under 15 words), most important first, at most 6. \
progressed is false when the evidence shows no movement since the previous verdict.";

/// One evaluator verdict, parsed from the side call's JSON reply.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalVerdict {
    pub achieved: bool,
    pub remaining: Vec<String>,
    pub next_step: String,
    pub confidence: f64,
    /// `false` when the evaluator saw no movement since the previous verdict. Absent from the
    /// reply means `true`: only an explicit "no progress" counts toward a wedged goal.
    pub progressed: bool,
    pub reason: String,
    /// The model that answered (empty when parsed outside a live call).
    pub model: String,
}

/// What the goal loop knows that the session does not: the goal itself, when it started, the
/// previous verdict's list, and the last gate failure.
#[derive(Debug, Clone, Default)]
pub struct GoalEvalRequest {
    pub goal: String,
    /// Unix seconds at goal start; commits after it are this goal's work.
    pub started_unix: i64,
    pub iteration: usize,
    pub previous_remaining: Vec<String>,
    pub last_gate_failure: Option<String>,
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}

fn tail(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.trim().chars().collect();
    let start = chars.len().saturating_sub(max);
    let body: String = chars[start..].iter().collect();
    if start > 0 {
        format!("…{body}")
    } else {
        body
    }
}

fn lenient_bool(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" => Some(true),
            "false" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn lenient_text(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn lenient_list(value: Option<&serde_json::Value>) -> Vec<String> {
    let items: Vec<String> = match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(o) => lenient_text(
                    o.get("item")
                        .or_else(|| o.get("title"))
                        .or_else(|| o.get("description")),
                ),
                other => other.to_string(),
            })
            .collect(),
        Some(serde_json::Value::String(s)) => s.lines().map(str::to_string).collect(),
        _ => Vec::new(),
    };
    items
        .into_iter()
        .map(|item| {
            clip(
                item.trim().trim_start_matches(['-', '*', '•']).trim(),
                ITEM_CHARS,
            )
        })
        .filter(|item| !item.is_empty())
        .take(MAX_ITEMS)
        .collect()
}

/// The first JSON object in `text` that carries an `achieved` key — tolerant of code fences and
/// prose around it, which small models add however firmly they are told not to.
fn verdict_object(text: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    text.char_indices()
        .filter(|(_, c)| *c == '{')
        .find_map(|(i, _)| {
            let mut stream =
                serde_json::Deserializer::from_str(&text[i..]).into_iter::<serde_json::Value>();
            match stream.next() {
                Some(Ok(serde_json::Value::Object(map))) if map.contains_key("achieved") => {
                    Some(map)
                }
                _ => None,
            }
        })
}

/// Parse an evaluator reply. `None` when it holds no verdict the loop can act on: no JSON object,
/// no readable `achieved`, or "not achieved" with neither a remaining item nor a next step.
pub fn parse_goal_verdict(text: &str) -> Option<GoalVerdict> {
    let map = verdict_object(text)?;
    let achieved = map.get("achieved").and_then(lenient_bool)?;
    let mut remaining = lenient_list(map.get("remaining"));
    let mut next_step = clip(
        &lenient_text(map.get("next_step").or_else(|| map.get("next"))),
        ITEM_CHARS,
    );
    if !achieved && remaining.is_empty() && next_step.is_empty() {
        return None;
    }
    if achieved {
        remaining.clear();
    } else if next_step.is_empty() {
        next_step = remaining[0].clone();
    }
    let confidence = match map.get("confidence") {
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        Some(serde_json::Value::String(s)) => s.trim().trim_end_matches('%').parse().ok(),
        _ => None,
    }
    .map(|c: f64| if c > 1.0 { c / 100.0 } else { c })
    .unwrap_or(0.5)
    .clamp(0.0, 1.0);
    Some(GoalVerdict {
        achieved,
        remaining,
        next_step,
        confidence,
        progressed: map.get("progressed").and_then(lenient_bool).unwrap_or(true),
        reason: clip(&lenient_text(map.get("reason")), ITEM_CHARS),
        model: String::new(),
    })
}

/// The compact line each verdict is shown as: `🎯 3 left · next: …` or `🎯 goal achieved: …`.
pub fn goal_progress_line(verdict: &GoalVerdict) -> String {
    if verdict.achieved {
        let reason = if verdict.reason.is_empty() {
            "every part of the goal is met"
        } else {
            verdict.reason.as_str()
        };
        return format!("🎯 goal achieved: {}", clip(reason, 120));
    }
    let left = match verdict.remaining.len() {
        0 => "not done".to_string(),
        n => format!("{n} left"),
    };
    format!("🎯 {left} · next: {}", clip(&verdict.next_step, 100))
}

fn task_plan(tasks: &[forge_types::TodoItem]) -> String {
    if tasks.is_empty() {
        return "(no task plan recorded)".to_string();
    }
    tasks
        .iter()
        .map(|t| {
            let status = match t.status {
                forge_types::TodoStatus::Done => "done",
                forge_types::TodoStatus::InProgress => "in progress",
                forge_types::TodoStatus::Pending => "pending",
            };
            format!("- [{status}] {}", clip(&t.title, ITEM_CHARS))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn evaluator_input(
    request: &GoalEvalRequest,
    tasks: &[forge_types::TodoItem],
    final_answer: Option<&str>,
    evidence: &str,
) -> String {
    let previous = if request.previous_remaining.is_empty() {
        "(first evaluation)".to_string()
    } else {
        request
            .previous_remaining
            .iter()
            .map(|r| format!("- {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let gate = request
        .last_gate_failure
        .as_deref()
        .map(|g| tail(g, GATE_CHARS))
        .unwrap_or_else(|| "(none)".to_string());
    format!(
        "GOAL:\n{}\n\nTurn: {}\n\nTask plan:\n{}\n\nAgent's final message this turn:\n{}\n\n{}\n\
         Last quality-gate failure:\n{}\n\nPrevious verdict's remaining items:\n{}",
        request.goal.trim(),
        request.iteration,
        task_plan(tasks),
        final_answer
            .filter(|a| !a.trim().is_empty())
            .map(|a| tail(a, ANSWER_CHARS))
            .unwrap_or_else(|| "(no final message)".to_string()),
        evidence,
        gate,
        previous
    )
}

impl Session {
    /// The evaluator's model: `[goal] evaluator_model` when it is safe to use, else the ordinary
    /// side-call model (pin semantics, no foreign subscription, zero-retention, no CLI bridge).
    fn goal_evaluator_model(&self, routed: &forge_mesh::RoutingDecision) -> Option<String> {
        let configured = self
            .config
            .goal
            .evaluator_model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty());
        if let Some(model) = configured {
            let own = self
                .effective_pin()
                .and_then(|pin| pin.first().cloned())
                .unwrap_or_else(|| routed.model.clone());
            let foreign_subscription = forge_mesh::catalog::is_subscription(model) && model != own;
            if !forge_provider::is_cli_bridge(model)
                && !foreign_subscription
                && forge_mesh::privacy::allows(&self.config, model)
            {
                return Some(model.to_string());
            }
            tracing::warn!(model, "ignoring [goal] evaluator_model for this session");
        }
        self.post_turn_auxiliary_model(routed)
    }

    /// `[goal] evaluator`: whether the goal loop should run the evaluator after each turn.
    pub fn goal_evaluator_enabled(&self) -> bool {
        self.config.goal.evaluator
    }

    fn goal_evaluator_fallback(&mut self, why: &str) {
        self.presenter.emit(PresenterEvent::Warning(format!(
            "goal evaluator {why} — continuing with the standard prompt"
        )));
    }

    /// Judge the goal after a turn. `None` means "no verdict, use the fixed prompt": the
    /// evaluator is off (silently), or the call could not produce a usable verdict (with one
    /// warning). A verdict is also shown to the user as a [`PresenterEvent::Progress`] line and
    /// its cost is recorded as a `goal/evaluate` side call.
    pub async fn evaluate_goal(&mut self, request: &GoalEvalRequest) -> Option<GoalVerdict> {
        if !self.config.goal.evaluator {
            return None;
        }
        let budget = self.budget_snapshot();
        if budget.status() == BudgetStatus::Exhausted {
            self.goal_evaluator_fallback("skipped (budget exhausted)");
            return None;
        }
        let readiness = self.provider_readiness();
        let decision = self
            .router
            .route_hinted(
                "evaluate goal progress",
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
            self.goal_evaluator_fallback("skipped (no side-call model available)");
            return None;
        };
        let evidence =
            crate::goal_evidence::collect(self.workspace.root(), &self.id, request.started_unix)
                .await;
        let input = evaluator_input(
            request,
            self.tasks(),
            self.last_assistant_text(),
            &evidence.render(),
        );
        let messages = [Message::system(GOAL_EVALUATOR_SYSTEM), Message::user(input)];
        self.presenter.emit(PresenterEvent::AuxiliaryRequest {
            model: model.clone(),
            purpose: "evaluating goal progress".to_string(),
        });
        let provider = self.provider.clone();
        let options = Self::auxiliary_completion_options(&self.id, GOAL_EVALUATOR_PURPOSE);
        let mut sink = |_: StreamEvent| {};
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(GOAL_EVAL_MAX_SECS),
            provider.complete_with(&model, &messages, &[], &options, &mut sink),
        )
        .await;
        let reply = match response {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) => {
                record_model_failure_in(
                    &self.store,
                    &model,
                    &error,
                    std::time::Duration::from_secs(self.config.mesh.failover_cooldown_secs),
                );
                self.goal_evaluator_fallback(&format!("failed ({})", clip(&error.to_string(), 80)));
                return None;
            }
            Err(_) => {
                self.goal_evaluator_fallback(&format!("timed out after {GOAL_EVAL_MAX_SECS}s"));
                return None;
            }
        };
        if let Err(error) = self.store.record_side_call_usage_for(
            &self.id,
            USAGE_PURPOSE,
            Some(&model),
            &priced(&self.pricing, &model, &reply.usage),
        ) {
            tracing::warn!(%error, "goal evaluator usage was not recorded");
        }
        let Some(mut verdict) = parse_goal_verdict(&reply.content) else {
            self.goal_evaluator_fallback("returned no usable verdict");
            return None;
        };
        verdict.model = model;
        self.presenter
            .emit(PresenterEvent::Progress(goal_progress_line(&verdict)));
        Some(verdict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fenced_reply_with_prose_parses() {
        let reply = "Sure! Here is my verdict:\n```json\n{\"achieved\": false, \"remaining\": \
                     [\"- add the CLI flag\", \"write tests\"], \"next_step\": \"add the CLI \
                     flag\", \"confidence\": 0.8, \"reason\": \"flag missing\"}\n```";
        let v = parse_goal_verdict(reply).unwrap();
        assert!(!v.achieved);
        assert_eq!(v.remaining, vec!["add the CLI flag", "write tests"]);
        assert_eq!(v.next_step, "add the CLI flag");
        assert!((v.confidence - 0.8).abs() < 1e-9);
        assert!(v.progressed);
        assert_eq!(goal_progress_line(&v), "🎯 2 left · next: add the CLI flag");
    }

    #[test]
    fn lenient_fields_are_accepted() {
        let v = parse_goal_verdict(
            "{\"achieved\": \"no\", \"remaining\": \"one\\ntwo\", \"confidence\": \"85%\", \
             \"progressed\": false}",
        )
        .unwrap();
        assert_eq!(v.remaining, vec!["one", "two"]);
        assert_eq!(v.next_step, "one", "next step falls back to the first item");
        assert!((v.confidence - 0.85).abs() < 1e-9);
        assert!(!v.progressed);
    }

    #[test]
    fn an_achieved_verdict_drops_leftover_items() {
        let v = parse_goal_verdict(
            "{\"achieved\": true, \"remaining\": [\"x\"], \"reason\": \"all three parts land\"}",
        )
        .unwrap();
        assert!(v.achieved);
        assert!(v.remaining.is_empty());
        assert_eq!(
            goal_progress_line(&v),
            "🎯 goal achieved: all three parts land"
        );
    }

    #[test]
    fn malformed_replies_are_rejected() {
        for reply in [
            "",
            "I think the goal is mostly done.",
            "{\"remaining\": [\"x\"]}",
            "{\"achieved\": maybe}",
            "{\"achieved\": false, \"remaining\": []}",
            "{\"achieved\": false, \"remaining\": [",
        ] {
            assert!(parse_goal_verdict(reply).is_none(), "{reply:?}");
        }
    }

    #[test]
    fn the_input_carries_goal_plan_answer_and_evidence() {
        let request = GoalEvalRequest {
            goal: "ship three endpoints".into(),
            started_unix: 0,
            iteration: 3,
            previous_remaining: vec!["add /c".into()],
            last_gate_failure: Some("pytest: 1 failed".into()),
        };
        let tasks = vec![forge_types::TodoItem {
            title: "add /a".into(),
            status: forge_types::TodoStatus::Done,
            assignee: None,
        }];
        let input = evaluator_input(&request, &tasks, Some("added /a"), "Commits: abc");
        for needle in [
            "ship three endpoints",
            "- [done] add /a",
            "added /a",
            "Commits: abc",
            "pytest: 1 failed",
            "- add /c",
        ] {
            assert!(input.contains(needle), "missing {needle:?} in {input}");
        }
    }
}
