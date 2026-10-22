//! `PreModelSwitch` / `PostModelSwitch` hooks around a model switch the user asked for
//! (Claude-Code parity, docs/features/hooks.md).
//!
//! `PreModelSwitch` may refuse: exit 2, `decision: "block"`, `permissionDecision: "deny"`, a
//! timeout, or `permissionDecision: "ask"` (Forge has no confirmation step on this path, so an ask
//! refuses, as Claude Code does everywhere but its interactive `/model`). After the pin changes,
//! `PostModelSwitch` runs and what it prints joins the model's context. The matcher of both is
//! tested against the model the session switches to, with or without its `provider::` prefix.
//! Clearing a pin switches to `mesh` (per-turn routing).

use forge_config::HookEvent;

use crate::hooks_events::{run_event_hooks, EventMatch};
use crate::{PresenterEvent, Session};

/// The name a cleared pin switches to.
const MESH: &str = "mesh";

impl Session {
    /// Switch the pinned model (`None` returns to mesh routing) the way the user asked for it from
    /// `source` (`command` for `/model <id>`, `picker`, `sdk` for a remote client). `Err` carries
    /// the reason a `PreModelSwitch` hook refused (also shown as a warning); the pin is then
    /// unchanged.
    pub async fn request_model_switch(
        &mut self,
        to: Option<String>,
        source: &str,
    ) -> Result<(), String> {
        let from = self
            .restored_turn_state()
            .model
            .unwrap_or_else(|| MESH.to_string());
        let target = to.clone().unwrap_or_else(|| MESH.to_string());
        let fields = serde_json::json!({
            "from_model": from,
            "to_model": target,
            "requested_model": to,
            "source": source,
            "cwd": self.workspace.display(),
        });
        if self.has_hooks_for(HookEvent::PreModelSwitch) {
            let out = run_event_hooks(
                &self.config.hooks,
                HookEvent::PreModelSwitch,
                &self.id,
                EventMatch::Model(&target),
                fields.clone(),
            )
            .await;
            for note in &out.notes {
                self.presenter
                    .emit(PresenterEvent::Warning(format!("⎇ hook: {note}")));
            }
            let refusal = out
                .blocked
                .or(out.halt)
                .or_else(|| out.timed_out.then(|| "the hook timed out".to_string()))
                .or_else(|| {
                    out.ask
                        .map(|ask| format!("{ask} (asked to confirm; Forge treats that as no)"))
                });
            if let Some(why) = refusal {
                let refused = format!(
                    "a PreModelSwitch hook kept the current model ({from}) instead of {target}: {why}"
                );
                self.presenter
                    .emit(PresenterEvent::Warning(format!("⎇ {refused}")));
                return Err(refused);
            }
        }
        self.pin_model(to);
        self.fire_post_model_switch(&target, fields).await;
        Ok(())
    }

    async fn fire_post_model_switch(&mut self, target: &str, fields: serde_json::Value) {
        if !self.has_hooks_for(HookEvent::PostModelSwitch) {
            return;
        }
        let out = run_event_hooks(
            &self.config.hooks,
            HookEvent::PostModelSwitch,
            &self.id,
            EventMatch::Model(target),
            fields,
        )
        .await;
        for failure in out.failures {
            self.presenter
                .emit(PresenterEvent::Warning(format!("⎇ hook error: {failure}")));
        }
        for text in out.context {
            self.inject_hook_context(&text);
        }
    }
}
