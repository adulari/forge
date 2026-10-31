//! The control toolset on the CLI bridge. A bridged model's calls take the same road as the
//! direct path's: the permission gate here (the bridge is where the user's prompt is relayed),
//! then the running daemon over HTTP, which executes the tool and writes the audit line.

use std::sync::Arc;

use forge_core::control;
use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};
use serde_json::Value;

use super::super::{gate, permission, ForgeMcp};
use crate::control_http::HttpControl;

pub(super) fn tools() -> Vec<Tool> {
    let disabled = crate::control_settings::disabled();
    control::specs(&disabled)
        .into_iter()
        .map(|spec| {
            let schema: JsonObject = spec.schema.as_object().cloned().unwrap_or_default();
            Tool::new(spec.name, spec.description, Arc::new(schema))
        })
        .collect()
}

fn fail(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text.into())])
}

impl ForgeMcp {
    pub(super) async fn handle_control(&self, name: &str, args: &Value) -> CallToolResult {
        if let Err(error) = control::validate(name, args) {
            return fail(error);
        }
        if crate::control_settings::disabled()
            .iter()
            .any(|n| n == name)
        {
            return fail(format!(
                "error: {name} is switched off - the user can turn it back on in Settings > Control tools"
            ));
        }
        let me = std::env::var(forge_core::snapshot::ENV_SESSION).unwrap_or_default();
        let side_effect =
            control::effective_side_effect(&HttpControl::local(), &me, name, args).await;
        let mode = self.effective_mode();
        let decision = permission::decide(mode, side_effect, name, args, &self.rules);
        let prompt = super::super::permission_relay::Prompt {
            args: Some(args),
            diff: None,
        };
        if let Err(refusal) = gate(decision, (name, side_effect, mode), prompt).await {
            return fail(refusal);
        }
        match HttpControl::local().run(&me, name, args).await {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Err(text) => fail(text),
        }
    }
}
