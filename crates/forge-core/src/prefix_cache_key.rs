//! The provider prompt-cache key a session's main-loop requests carry.
//!
//! With `[mesh] cross_session_cache_key` on (the default), a session whose first request goes to a
//! provider that treats `prompt_cache_key` as a pure routing hint gets a key derived from what
//! every session in this workspace sends first: model, tool surface, system preamble and workspace
//! root. A new session then lands on the cache shard an earlier one warmed with the identical
//! fixed prefix instead of paying for it uncached. Otherwise the key is the session id, as before.
//! Either way the key is chosen once, at the first request, and never changes for the rest of the
//! session: a key that moved mid-session would send the next request to a cold shard.

use sha2::{Digest, Sha256};

use crate::Session;
use forge_provider::ToolSpec;
use forge_types::Message;

impl Session {
    pub(crate) fn main_loop_cache_key(
        &mut self,
        model: &str,
        specs: &[ToolSpec],
        session_key: &str,
    ) -> String {
        if let Some(key) = &self.frozen_cache_key {
            return key.clone();
        }
        let key = if self.config.mesh.cross_session_cache_key
            && forge_provider::prompt_cache::shares_prompt_cache_key_across_sessions(model)
        {
            let mut preamble = self.system_preamble();
            if let Some(prompt) = self.system_prompt_override(model) {
                preamble[0].content.clone_from(prompt);
            }
            cross_session_key(model, specs, &preamble, &self.workspace.display())
        } else {
            session_key.to_string()
        };
        self.frozen_cache_key = Some(key.clone());
        key
    }
}

/// A short, stable digest of the fixed request prefix. Every field is length-framed so two
/// different inputs can never concatenate to the same bytes.
pub(crate) fn cross_session_key(
    model: &str,
    specs: &[ToolSpec],
    preamble: &[Message],
    workspace: &str,
) -> String {
    let mut tools = Sha256::new();
    for spec in specs {
        frame(&mut tools, spec.name.as_bytes());
        frame(&mut tools, spec.description.as_bytes());
        frame(&mut tools, spec.schema.to_string().as_bytes());
    }
    let mut system = Sha256::new();
    for message in preamble {
        frame(&mut system, message.content.as_bytes());
    }
    let mut key = Sha256::new();
    frame(&mut key, b"forge-prefix-v1");
    frame(&mut key, model.as_bytes());
    frame(&mut key, &tools.finalize());
    frame(&mut key, &system.finalize());
    frame(&mut key, workspace.as_bytes());
    format!("forge-prefix-{}", &hex::encode(key.finalize())[..32])
}

fn frame(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: "d".into(),
            schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn the_key_is_stable_and_moves_with_every_prefix_input() {
        let tools = [spec("read_file"), spec("shell")];
        let system = [Message::system("You are Forge.")];
        let base = cross_session_key("openai::gpt-5.4", &tools, &system, "/repo");
        assert_eq!(
            base,
            cross_session_key("openai::gpt-5.4", &tools, &system, "/repo")
        );
        assert!(base.starts_with("forge-prefix-"));
        assert!(base.len() <= 64, "OpenAI caps prompt_cache_key length");
        for other in [
            cross_session_key("openai::gpt-5.5", &tools, &system, "/repo"),
            cross_session_key("openai::gpt-5.4", &tools[..1], &system, "/repo"),
            cross_session_key(
                "openai::gpt-5.4",
                &tools,
                &[Message::system("You are Forge!")],
                "/repo",
            ),
            cross_session_key("openai::gpt-5.4", &tools, &system, "/other"),
        ] {
            assert_ne!(base, other);
        }
    }
}
