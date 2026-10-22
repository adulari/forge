//! What a CLI-bridge turn did inside the bridged CLI.
//!
//! The bridge runs the whole agent loop inside one `complete()`: Claude Code makes its model calls
//! and runs its tools in a subprocess, and Forge sees them only as stream events. The response
//! carries no tool calls and the usage is one row, so a session that only reads its own records
//! counts one model call and no tool calls for a turn that made dozens. This collects the events
//! while the response streams and writes them down once it is recorded.

use super::*;

/// A tool the bridged CLI ran: the call, then its outcome once the result arrived.
#[derive(Debug)]
struct BridgedCall {
    name: String,
    args: String,
    outcome: Option<(bool, String)>,
}

#[derive(Debug, Default)]
pub(crate) struct BridgeTrace {
    calls: Vec<BridgedCall>,
    model_calls: Option<u64>,
}

impl BridgeTrace {
    pub(crate) fn started(&mut self, name: &str, args: &str) {
        self.calls.push(BridgedCall {
            name: name.to_string(),
            args: args.to_string(),
            outcome: None,
        });
    }

    /// A result closes the oldest unfinished call of that name: the CLI runs same-named tools in
    /// the order it started them.
    pub(crate) fn finished(&mut self, name: &str, ok: bool, summary: &str) {
        if let Some(call) = self
            .calls
            .iter_mut()
            .find(|c| c.outcome.is_none() && c.name == name)
        {
            call.outcome = Some((ok, summary.to_string()));
        }
    }

    pub(crate) fn add_model_calls(&mut self, calls: u64) {
        self.model_calls = Some(self.model_calls.unwrap_or(0) + calls);
    }
}

impl Session {
    /// Persist what the bridged CLI did during the response stored as `message_id`: its tool calls
    /// into the audit ledger (where replay, export and the tool-call counts read them) and its
    /// model-call count onto the usage row. Always empties the trace; anything the active model
    /// did not run through a bridge is dropped.
    pub(crate) fn record_bridge_trace(&mut self, message_id: &str, model: &str) {
        let trace = std::mem::take(&mut *self.bridge_trace.lock().unwrap());
        if !forge_provider::is_cli_bridge(model) {
            return;
        }
        for call in &trace.calls {
            let (status, result) = match &call.outcome {
                Some((true, summary)) => ("ok", summary.as_str()),
                Some((false, summary)) => ("error", summary.as_str()),
                None => ("unfinished", ""),
            };
            // The permission decision was made by the bridge's own `forge mcp-serve`, not here.
            let _ = self
                .store
                .record_tool_call(message_id, &call.name, &call.args, result, "bridge", status);
        }
        if let Some(calls) = trace.model_calls {
            let _ = self.store.set_usage_api_calls(message_id, calls);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_close_the_oldest_unfinished_call_of_the_same_name() {
        let mut trace = BridgeTrace::default();
        trace.started("read", "{\"a\":1}");
        trace.started("shell", "{}");
        trace.started("read", "{\"a\":2}");
        trace.finished("read", true, "first");
        trace.finished("read", false, "second");
        trace.finished("never-started", true, "ignored");
        let outcomes: Vec<_> = trace.calls.iter().map(|c| c.outcome.clone()).collect();
        assert_eq!(
            outcomes,
            [
                Some((true, "first".to_string())),
                None,
                Some((false, "second".to_string()))
            ]
        );
    }

    #[test]
    fn model_calls_from_several_results_add_up() {
        let mut trace = BridgeTrace::default();
        assert_eq!(trace.model_calls, None);
        trace.add_model_calls(3);
        trace.add_model_calls(2);
        assert_eq!(trace.model_calls, Some(5));
    }
}
