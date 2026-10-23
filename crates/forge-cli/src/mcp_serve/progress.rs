//! `forge mcp-serve`'s share of progress supervision: judge each bridged call as it lands, put a
//! nudge in front of the result that earned it, and refuse every call once the turn is stopped.
//!
//! The judgement is `forge_core::BridgeProgress` (the same ledger the direct loop uses, minus the
//! narration and idle lanes). A stop is also reported to the parent through the sink so it ends
//! the turn as stalled instead of re-driving a plan that is going nowhere.

use super::{report_to_sink, ForgeMcp};
use forge_core::BridgeVerdict;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;

impl ForgeMcp {
    /// The refusal for a call made after this turn was stopped, if it was.
    pub(super) fn progress_refusal(&self) -> Option<CallToolResult> {
        let turn = forge_core::snapshot::current_seq();
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        progress
            .stopped_in(turn)
            .map(|refusal| CallToolResult::error(vec![ContentBlock::text(refusal.to_string())]))
    }

    /// Judge one finished call and shape the result text the model reads. `tasks_done` is the
    /// number of Done tasks after an `update_tasks` call.
    pub(super) fn judge_call(
        &self,
        tool: &str,
        args: &Value,
        outcome: (String, bool),
        mutating: bool,
        tasks_done: Option<usize>,
    ) -> (String, bool) {
        let (text, ok) = outcome;
        let root = std::env::current_dir().unwrap_or_default();
        let verdict = {
            let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
            progress.observe_call(
                forge_core::snapshot::current_seq(),
                tool,
                args,
                ok,
                mutating,
                &text,
                tasks_done,
                &root,
            )
        };
        match verdict {
            BridgeVerdict::Fine => (text, ok),
            BridgeVerdict::Nudge(note) => (format!("{note}\n\n{text}"), ok),
            BridgeVerdict::Stop { refusal, headline } => {
                report_to_sink(serde_json::json!({ "k": "stall", "report": headline }));
                (refusal, false)
            }
        }
    }
}
