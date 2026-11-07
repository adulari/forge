//! Scouts (docs/features/scouts.md): the `scout {question}` tool runs the built-in read-only
//! `scout` subagent in a throwaway context and only its cited brief reaches the main transcript.
//!
//! The tool is an alias for a one-agent `spawn_agents` call, so routing, the depth-1 guard,
//! permissions, persistence and the child rows on every surface are the ones subagents already
//! have. This module owns the alias, the advert, the brief cap and the "saved" line.

use forge_config::{HarnessConfig, ScoutsMode};
use forge_provider::ToolSpec;
use forge_types::ToolCall;
use serde_json::{json, Value};

use crate::subagent::{Lifecycle, SPAWN_AGENTS_TOOL};

pub(crate) const SCOUT_TOOL: &str = "scout";
const SCOUT_AGENT: &str = "scout";
/// The brief the parent keeps, in characters (about 3k tokens); the scout is asked for less.
const BRIEF_CAP_CHARS: usize = 12_000;
/// What a scout step costs in prompt tokens: its small fixed context, against the parent's.
const SCOUT_CONTEXT_TOKENS: u64 = 18_000;
/// The parent's fixed prompt (system, tools, pinned context) on top of its transcript.
const PARENT_OVERHEAD_TOKENS: u64 = 8_000;

pub(crate) fn specs(harness: &HarnessConfig) -> Vec<ToolSpec> {
    let lead = match harness.scouts {
        ScoutsMode::Off => return Vec::new(),
        ScoutsMode::Auto => {
            "Call this FIRST, before reading or searching code yourself, whenever the task needs \
             orientation in code you have not seen: where something lives, how a feature works, \
             which files a change touches. "
        }
        ScoutsMode::Tool => {
            "Use this instead of several reads and searches when you need to orient yourself in \
             code you have not seen. "
        }
    };
    vec![ToolSpec {
        name: SCOUT_TOOL.to_string(),
        description: format!(
            "{lead}A scout is a throwaway read-only agent on a cheap model: it explores in its own \
             small context and returns only a short brief of facts with `path:start-end` \
             citations, so none of its reading enters your context. Ask one specific question \
             (what you need to know, not how to look). Read the cited ranges yourself before you \
             edit them. Skip it when you already know the file to change."
        ),
        schema: json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "What to find out, e.g. 'where is the retry backoff for failed uploads decided and which tests cover it?'"
                }
            },
            "required": ["question"]
        }),
    }]
}

/// `scout {question}` as the one-agent `spawn_agents` call it runs as; any other call, and every
/// call while scouts are off, comes back unchanged.
pub(crate) fn rewrite(harness: &HarnessConfig, call: ToolCall) -> ToolCall {
    if call.name != SCOUT_TOOL || harness.scouts == ScoutsMode::Off {
        return call;
    }
    let question = call
        .args
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let args = if question.is_empty() {
        json!({ "agents": [] })
    } else {
        json!({ "agents": [{ "agent": SCOUT_AGENT, "task": question }] })
    };
    ToolCall {
        id: call.id,
        name: SPAWN_AGENTS_TOOL.to_string(),
        args,
    }
}

/// Whether a `spawn_agents` call is a scout run.
pub(crate) fn is_scout(args: &Value) -> bool {
    let agents = args.get("agents").and_then(Value::as_array);
    matches!(agents, Some(a) if a.len() == 1
        && a[0].get("agent").and_then(Value::as_str) == Some(SCOUT_AGENT))
}

/// The ids of the children that finished during one `spawn_agents` call.
#[derive(Default)]
pub(crate) struct Tally {
    done: Vec<String>,
}

impl Tally {
    pub(crate) fn note(&mut self, ev: &Lifecycle) {
        if let Lifecycle::Done { id, .. } = ev {
            self.done.push((*id).to_string());
        }
    }
}

impl crate::Session {
    /// The scout runs the open turn has made, for its stored detail.
    pub(crate) fn scouts_this_turn(&self) -> Option<u32> {
        self.store
            .count_scout_calls_since(&self.id, self.expert.prompt_seq)
            .ok()
            .filter(|n| *n > 0)
    }

    /// For a scout run: cap the brief the parent keeps and put the "saved" line in the
    /// scrollback. Any other `spawn_agents` result passes through untouched.
    pub(crate) fn finish_scout(
        &mut self,
        args: &Value,
        tally: &Tally,
        result: String,
        ok: bool,
    ) -> String {
        if !is_scout(args) {
            return result;
        }
        let result = cap(result);
        let calls: u64 = tally
            .done
            .iter()
            .filter_map(|id| self.store.session_tool_call_count(id).ok())
            .sum();
        let main_tokens = self
            .transcript
            .iter()
            .map(|m| m.content.chars().count() as u64)
            .sum::<u64>()
            / 4;
        let summary = summary_line(calls, result.chars().count() as u64 / 4, main_tokens);
        self.presenter.emit(crate::PresenterEvent::ToolResult {
            name: SCOUT_TOOL.to_string(),
            ok,
            summary,
            detail: crate::tool_detail(&result),
        });
        result
    }
}

fn summary_line(calls: u64, brief_tokens: u64, main_tokens: u64) -> String {
    let parent_step = main_tokens + PARENT_OVERHEAD_TOKENS;
    let saved = calls.saturating_mul(parent_step.saturating_sub(SCOUT_CONTEXT_TOKENS));
    format!(
        "scout: {calls} reads -> {} brief (saved ~{})",
        tokens(brief_tokens),
        tokens(saved)
    )
}

fn tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

fn cap(result: String) -> String {
    if result.chars().count() <= BRIEF_CAP_CHARS {
        return result;
    }
    let cut: String = result.chars().take(BRIEF_CAP_CHARS).collect();
    format!("{cut}\n[brief truncated at {BRIEF_CAP_CHARS} characters; ask a narrower question for the rest]")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harness(mode: ScoutsMode) -> HarnessConfig {
        HarnessConfig {
            scouts: mode,
            ..HarnessConfig::default()
        }
    }

    fn scout_call(question: &str) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            name: SCOUT_TOOL.into(),
            args: json!({ "question": question }),
        }
    }

    #[test]
    fn scout_becomes_a_one_agent_spawn_with_the_same_id() {
        let out = rewrite(&harness(ScoutsMode::Tool), scout_call("where is X?"));
        assert_eq!(out.id, "c1");
        assert_eq!(out.name, SPAWN_AGENTS_TOOL);
        assert_eq!(out.args["agents"][0]["agent"], "scout");
        assert_eq!(out.args["agents"][0]["task"], "where is X?");
        assert!(is_scout(&out.args));
    }

    #[test]
    fn off_leaves_the_call_and_advertises_nothing() {
        let cfg = harness(ScoutsMode::Off);
        assert_eq!(rewrite(&cfg, scout_call("q")).name, SCOUT_TOOL);
        assert!(specs(&cfg).is_empty());
    }

    #[test]
    fn an_empty_question_is_refused_by_the_spawn_parser() {
        let out = rewrite(&harness(ScoutsMode::Tool), scout_call("  "));
        assert!(crate::subagent::parse_requests(&out.args, 4).is_err());
    }

    #[test]
    fn auto_tells_the_model_to_scout_first_and_tool_does_not() {
        let auto = &specs(&harness(ScoutsMode::Auto))[0].description;
        let tool = &specs(&harness(ScoutsMode::Tool))[0].description;
        assert!(auto.contains("FIRST"));
        assert!(!tool.contains("FIRST"));
    }

    #[test]
    fn a_plain_spawn_is_not_a_scout() {
        let args = json!({ "agents": [{ "agent": "explore", "task": "t" }] });
        assert!(!is_scout(&args));
    }

    #[test]
    fn the_brief_is_capped_on_a_character_boundary() {
        let long = "é".repeat(BRIEF_CAP_CHARS + 50);
        let capped = cap(long);
        assert!(capped.contains("[brief truncated"));
        assert!(capped.chars().count() < BRIEF_CAP_CHARS + 150);
        assert_eq!(cap("short".into()), "short");
    }

    #[test]
    fn the_summary_line_reports_reads_brief_and_saving() {
        assert_eq!(
            summary_line(14, 1800, 100_000),
            "scout: 14 reads -> 1.8k brief (saved ~1.3M)"
        );
        assert_eq!(
            summary_line(2, 300, 5_000),
            "scout: 2 reads -> 300 brief (saved ~0)"
        );
    }
}
