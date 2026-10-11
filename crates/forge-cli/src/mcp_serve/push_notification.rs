//! `push_notification` on the CLI bridge. Same entry point as the direct path
//! (`forge_core::push_notify::run`), so permission, validation and the limiter are shared; the
//! installed `LocalSink` hands delivery to the running daemon.

use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};
use serde_json::Value;

use super::ForgeMcp;

pub(super) fn tool() -> Tool {
    let spec = forge_core::push_notify::spec();
    let schema: JsonObject = spec.schema.as_object().cloned().unwrap_or_default();
    Tool::new(spec.name, spec.description, Arc::new(schema))
}

impl ForgeMcp {
    pub(super) async fn handle_push_notification(&self, args: &Value) -> CallToolResult {
        let session = std::env::var(forge_core::snapshot::ENV_SESSION)
            .unwrap_or_else(|_| "bridge".to_string());
        let (text, ok) = forge_core::push_notify::run(args, &session, self.effective_mode()).await;
        let content = vec![ContentBlock::text(text)];
        if ok {
            CallToolResult::success(content)
        } else {
            CallToolResult::error(content)
        }
    }
}
