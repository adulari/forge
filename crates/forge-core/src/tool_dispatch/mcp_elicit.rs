//! Serving MCP elicitation while an MCP tool call is in flight.
//!
//! A server may stop mid-call and ask the user for input. The call future and the user prompt
//! must progress together, so [`Session::call_mcp_serving_elicitation`] polls both: the call, and
//! the requests the server's client handler forwards through `forge_mcp::elicit`.

use forge_mcp::elicit::{ElicitPrompt, ElicitReply, ElicitRequest};
use forge_types::{Answer, QChoice, Question};

use super::*;

/// The server a call targets, so a request from another server is not shown under this call.
fn target_server(name: &str, args: &serde_json::Value) -> Option<String> {
    let qualified = if name == forge_mcp::MCP_CALL {
        ["name", "qualified_name", "tool"]
            .iter()
            .find_map(|key| args.get(*key).and_then(|v| v.as_str()))?
    } else {
        name
    };
    qualified
        .split_once("__")
        .map(|(server, _)| server.to_string())
}

/// Times a rejected value is re-asked before the elicitation is declined.
const MAX_REASKS: usize = 3;

impl Session {
    pub(super) async fn call_mcp_serving_elicitation(
        &mut self,
        mcp: &Arc<forge_mcp::McpManager>,
        name: &str,
        args: &serde_json::Value,
    ) -> forge_mcp::McpCallOutcome {
        if !self.presenter.is_attended() {
            return mcp.call(name, args).await;
        }
        let (scope, mut requests) =
            forge_mcp::elicit::open_scope(target_server(name, args).as_deref());
        let call = mcp.call(name, args);
        tokio::pin!(call);
        let outcome = loop {
            tokio::select! {
                outcome = &mut call => break outcome,
                Some(request) = requests.recv() => self.answer_elicitation(request),
            }
        };
        drop(scope);
        outcome
    }

    /// Put one elicitation to the user. Nothing the user types is recorded: the values go to the
    /// server and the scrollback shows only what was asked.
    fn answer_elicitation(&mut self, request: ElicitRequest) {
        let reply = match &request.prompt {
            ElicitPrompt::Url(url) => self.ask_elicitation_url(&request, url),
            ElicitPrompt::Form(form) => {
                let mut errors: Vec<Option<String>> = Vec::new();
                let mut reply = ElicitReply::Decline;
                for _ in 0..MAX_REASKS {
                    let mut questions = form.questions(&errors);
                    if let Some(first) = questions.first_mut() {
                        first.text =
                            format!("{}: {}\n{}", request.server, request.message, first.text);
                    }
                    let answers = self.presenter.ask_form(&questions);
                    if answers.iter().all(Answer::is_empty) {
                        break;
                    }
                    match form.resolve(&answers) {
                        Ok(values) => {
                            reply = ElicitReply::Accept(values);
                            break;
                        }
                        Err(found) => errors = found,
                    }
                }
                reply
            }
        };
        let _ = request.reply.send(reply);
    }

    fn ask_elicitation_url(&mut self, request: &ElicitRequest, url: &str) -> ElicitReply {
        let choice = |label: &str, description: &str| QChoice {
            label: label.to_string(),
            description: description.to_string(),
        };
        let question = Question {
            header: "Open link".into(),
            text: format!("{}: {}\n{url}", request.server, request.message),
            options: vec![
                choice("Continue", "I opened the link and finished the step"),
                choice("Decline", "skip this step"),
                choice("Cancel", "abandon the whole call"),
            ],
            multi: false,
            allow_other: false,
            allow_note: false,
            secret: false,
        };
        let answers = self.presenter.ask_form(&[question]);
        match answers
            .first()
            .and_then(|a| a.selected.first())
            .map(String::as_str)
        {
            Some("Continue") => ElicitReply::Accept(Default::default()),
            Some("Cancel") => ElicitReply::Cancel,
            _ => ElicitReply::Decline,
        }
    }
}
