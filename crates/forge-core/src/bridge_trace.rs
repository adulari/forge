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

/// How long a bridged model may keep calling tools after `mcp-serve` told it to stop before the
/// process is killed: a model that obeys answers within seconds, so this is only the bound for
/// one that does not.
const STALL_KILL_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Default)]
pub(crate) struct BridgeTrace {
    calls: Vec<BridgedCall>,
    model_calls: Option<u64>,
    /// `mcp-serve` stopped the turn: what the person is told, and when.
    stall: Option<(String, std::time::Instant)>,
    /// The model kept starting tools well after the stop.
    kill: bool,
}

impl BridgeTrace {
    pub(crate) fn stalled(&mut self, headline: String) {
        self.stall
            .get_or_insert((headline, std::time::Instant::now()));
    }

    pub(crate) fn started(&mut self, name: &str, args: &str) {
        if self
            .stall
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > STALL_KILL_AFTER)
        {
            self.kill = true;
        }
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

/// Run a completion, but give up on it if the bridged model ignores a stop (see
/// [`kill_requested`]): dropping it kills the bridge process group, exactly as the idle watchdog
/// does, and the turn ends as stalled on what the model already did. The response is empty and
/// that call's usage is lost.
pub(crate) async fn until_killed<F>(
    completion: F,
    trace: &std::sync::Arc<std::sync::Mutex<BridgeTrace>>,
    bridge: bool,
) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError>
where
    F: std::future::Future<
        Output = Result<forge_provider::ModelResponse, forge_provider::ProviderError>,
    >,
{
    tokio::select! {
        result = completion => result,
        () = kill_requested(trace, bridge) => Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: String::new(),
            tool_calls: Vec::new(),
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        }),
    }
}

/// Resolves once the bridged model has ignored a stop for [`STALL_KILL_AFTER`] and started another
/// tool; never for a model that is not a bridge.
async fn kill_requested(trace: &std::sync::Arc<std::sync::Mutex<BridgeTrace>>, bridge: bool) {
    if !bridge {
        return std::future::pending().await;
    }
    while !trace.lock().unwrap().kill {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
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
        if let Some((headline, _)) = trace.stall {
            self.bridge_stall = Some(headline);
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

    #[test]
    fn only_a_tool_started_long_after_the_stop_asks_for_the_kill() {
        let mut trace = BridgeTrace::default();
        trace.stalled("Stopped: no progress.".into());
        trace.started("shell", "{}");
        assert!(
            !trace.kill,
            "a call right after the stop is the model finishing up"
        );
        trace.stall = Some((
            "Stopped: no progress.".into(),
            std::time::Instant::now() - STALL_KILL_AFTER - std::time::Duration::from_secs(1),
        ));
        trace.started("shell", "{}");
        assert!(
            trace.kill,
            "a model that keeps working 30 s after the stop is killed"
        );
    }

    #[tokio::test]
    async fn a_killed_completion_resolves_empty_and_a_quiet_one_is_left_alone() {
        let trace = std::sync::Arc::new(std::sync::Mutex::new(BridgeTrace::default()));
        {
            let mut t = trace.lock().unwrap();
            t.stall = Some((
                "Stopped: no progress.".into(),
                std::time::Instant::now() - STALL_KILL_AFTER - std::time::Duration::from_secs(1),
            ));
            t.started("shell", "{}");
        }
        let never = std::future::pending::<
            Result<forge_provider::ModelResponse, forge_provider::ProviderError>,
        >();
        let response = until_killed(never, &trace, true).await.unwrap();
        assert!(response.content.is_empty() && response.tool_calls.is_empty());

        let quiet = std::sync::Arc::new(std::sync::Mutex::new(BridgeTrace::default()));
        let done = async {
            Ok(forge_provider::ModelResponse {
                reasoning: String::new(),
                reasoning_items: Vec::new(),
                content: "finished".into(),
                tool_calls: Vec::new(),
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        };
        assert_eq!(
            until_killed(done, &quiet, true).await.unwrap().content,
            "finished"
        );
    }
}
