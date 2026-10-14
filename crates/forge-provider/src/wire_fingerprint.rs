//! A per-message fingerprint of what a request puts on the wire, for prompt-cache attribution.
//!
//! Provider prompt caches key on a prefix: a request reuses the cache up to the first message that
//! differs from the previous request. Aggregate hit ratios cannot say WHY a call missed, so this
//! hashes every message after the same transforms the genai path applies (system placement and
//! the latest-image-turn rule), which means a rewrite those transforms introduce is visible too.
//! Comparing two fingerprints gives how many leading messages the new request shares with the old.

use forge_types::{Message, Role};
use sha2::{Digest as _, Sha256};

use crate::system_placement::{last_image_turn, place_system_messages};
use crate::ToolSpec;

/// Truncated SHA-256: collisions are irrelevant at a few thousand messages per session.
pub type MessageDigest = [u8; 16];

/// Hashes of one request: each wire message in order, plus the whole tool list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireFingerprint {
    messages: Vec<MessageDigest>,
    kinds: Vec<&'static str>,
    tools: MessageDigest,
}

/// How a request relates to the one sent before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefixReuse {
    /// Leading messages identical to the previous request.
    pub reused: usize,
    /// Messages in this request.
    pub total: usize,
    /// Messages in the previous request. `reused < previous_total` means it was rewritten.
    pub previous_total: usize,
    pub tools_changed: bool,
}

impl PrefixReuse {
    /// The previous request survives whole at the front of this one.
    pub fn is_append(&self) -> bool {
        self.reused == self.previous_total && !self.tools_changed
    }
}

impl WireFingerprint {
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// The kind of wire message at `index` (`system`, `user`, `reminder`, `assistant`, `tool`).
    pub fn kind_at(&self, index: usize) -> &'static str {
        self.kinds.get(index).copied().unwrap_or("none")
    }

    pub fn reuse_against(&self, previous: &WireFingerprint) -> PrefixReuse {
        let reused = self
            .messages
            .iter()
            .zip(&previous.messages)
            .take_while(|(a, b)| a == b)
            .count();
        PrefixReuse {
            reused,
            total: self.messages.len(),
            previous_total: previous.messages.len(),
            tools_changed: self.tools != previous.tools,
        }
    }
}

/// Fingerprint `messages` and `tools` exactly as a direct provider would send them.
pub fn fingerprint(messages: &[Message], tools: &[ToolSpec]) -> WireFingerprint {
    let placed = place_system_messages(messages);
    let image_turn = last_image_turn(&placed);
    let mut digests = Vec::with_capacity(placed.len());
    let mut kinds = Vec::with_capacity(placed.len());
    for (index, message) in placed.iter().enumerate() {
        digests.push(message_digest(message, Some(index) == image_turn));
        kinds.push(kind_of(message));
    }
    let mut tools_hash = Sha256::new();
    for spec in tools {
        field(&mut tools_hash, spec.name.as_bytes());
        field(&mut tools_hash, spec.description.as_bytes());
        field(&mut tools_hash, spec.schema.to_string().as_bytes());
    }
    WireFingerprint {
        messages: digests,
        kinds,
        tools: truncate(tools_hash),
    }
}

fn kind_of(message: &Message) -> &'static str {
    match message.role {
        Role::System => "system",
        Role::User if message.content.starts_with("<system-reminder>") => "reminder",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn message_digest(message: &Message, keeps_images: bool) -> MessageDigest {
    let mut hash = Sha256::new();
    field(&mut hash, message.role.as_str().as_bytes());
    field(&mut hash, message.content.as_bytes());
    for call in &message.tool_calls {
        field(&mut hash, call.id.as_bytes());
        field(&mut hash, call.name.as_bytes());
        field(&mut hash, call.args.to_string().as_bytes());
    }
    field(
        &mut hash,
        message.tool_call_id.as_deref().unwrap_or("").as_bytes(),
    );
    if keeps_images {
        for image in &message.images {
            field(&mut hash, image.media_type.as_bytes());
            field(&mut hash, image.data_base64.as_bytes());
        }
    }
    truncate(hash)
}

/// Length-prefixed so `("ab", "c")` and `("a", "bc")` hash differently.
fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn truncate(hash: Sha256) -> MessageDigest {
    let full = hash.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&full[..16]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::ImageAttachment;

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.to_string(),
            description: "d".to_string(),
            schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn an_append_only_transcript_reuses_the_whole_previous_request() {
        let mut messages = vec![
            Message::system("sys"),
            Message::user("task"),
            Message::assistant("answer"),
        ];
        let before = fingerprint(&messages, &[spec("read_file")]);
        messages.push(Message::system("nudge"));
        messages.push(Message::user("next"));
        let after = fingerprint(&messages, &[spec("read_file")]);

        let reuse = after.reuse_against(&before);
        assert!(reuse.is_append(), "{reuse:?}");
        assert_eq!(reuse.reused, 3);
        assert_eq!(reuse.total, 5);
        assert_eq!(after.kind_at(3), "reminder");
    }

    #[test]
    fn a_rewritten_earlier_message_is_located() {
        let mut messages = vec![
            Message::system("sys"),
            Message::user("task"),
            Message::tool_result("c1", "long output"),
            Message::user("more"),
        ];
        let before = fingerprint(&messages, &[]);
        messages[2].content = "long…[pruned]".to_string();
        let after = fingerprint(&messages, &[]);

        let reuse = after.reuse_against(&before);
        assert!(!reuse.is_append());
        assert_eq!(reuse.reused, 2);
        assert_eq!(after.kind_at(reuse.reused), "tool");
    }

    #[test]
    fn a_changed_tool_list_is_reported_separately_from_the_messages() {
        let messages = vec![Message::user("task")];
        let before = fingerprint(&messages, &[spec("a")]);
        let after = fingerprint(&messages, &[spec("a"), spec("b")]);
        let reuse = after.reuse_against(&before);
        assert_eq!(reuse.reused, 1);
        assert!(reuse.tools_changed);
        assert!(!reuse.is_append());
    }

    /// The genai path strips images from every image turn but the newest, so a new image turn
    /// rewrites the earlier one on the wire. Hashing before that transform would miss it.
    #[test]
    fn a_new_image_turn_rewrites_the_previous_image_turn() {
        let image = ImageAttachment {
            media_type: "image/png".to_string(),
            data_base64: "AAAA".to_string(),
        };
        let mut messages = vec![
            Message::system("sys"),
            Message::user_with_images("look", vec![image.clone()]),
            Message::assistant("seen"),
        ];
        let before = fingerprint(&messages, &[]);
        messages.push(Message::user_with_images("and this", vec![image]));
        let after = fingerprint(&messages, &[]);
        assert_eq!(after.reuse_against(&before).reused, 1);
    }
}
