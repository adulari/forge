//! The per-connection gate on inbound WebSocket frames.

use crate::serve_access::{access, AuditEntry};
use crate::serve_clients::ClientAuth;
use crate::serve_clients::Scope;

/// Filters inbound WebSocket frames for one connection: the scope each input kind needs, and an
/// audit line for every approval.
pub(crate) struct InputGate {
    auth: ClientAuth,
    session: String,
}

pub(crate) fn input_scope(kind: &str) -> Scope {
    match kind {
        "allow" | "answer" => Scope::Approve,
        _ => Scope::Prompt,
    }
}

impl InputGate {
    pub(crate) fn new(auth: ClientAuth, session: &str) -> Self {
        Self {
            auth,
            session: session.to_string(),
        }
    }

    /// `Some(reason)` when the frame must be dropped; the reason is shown to the client.
    pub(crate) fn check(&self, frame: &str) -> Option<String> {
        // Fail closed: a frame whose `kind` cannot be read is never forwarded. The consumer parses
        // the same text as `RemoteInput` and ignores stray fields, so a gate that skipped
        // unparseable frames let a View-only client smuggle prompts past it.
        let value = serde_json::from_str::<serde_json::Value>(frame).ok();
        let Some(kind) = value
            .as_ref()
            .and_then(|v| v.get("kind"))
            .and_then(|k| k.as_str())
        else {
            return Some("scope_denied:malformed".to_string());
        };
        let mut scope = input_scope(kind);
        if matches!(
            serde_json::from_str::<crate::remote::RemoteInput>(frame),
            Ok(crate::remote::RemoteInput::Allow { .. }
                | crate::remote::RemoteInput::Answer { .. })
        ) {
            scope = Scope::Approve;
        }
        let allowed = self.auth.allows(scope);
        if scope == Scope::Approve || !allowed {
            let detail = value
                .as_ref()
                .and_then(|v| v.get("yes"))
                .and_then(|y| y.as_bool())
                .map(|yes| if yes { "approve" } else { "deny" });
            access().record(AuditEntry {
                client: self.auth.name(),
                label: &self.auth.label,
                route: format!("WS input {kind}"),
                session: Some(self.session.clone()),
                decision: if allowed { "allow" } else { "deny" },
                detail,
                status: None,
                ip: None,
            });
        }
        (!allowed).then(|| format!("scope_denied:{}", scope.as_str()))
    }
}
