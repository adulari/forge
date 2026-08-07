//! Fleet agent-to-agent messaging: the `message_session` virtual tool a session's model calls to
//! message another daemon-hosted (fleet) session.
//!
//! The session core never talks to a live session registry (ADR-0004: it stays surface-agnostic)
//! — [`FleetMessaging`] is the callback the host process wires in. `forge serve`'s daemon driver
//! attaches an in-process implementation over its own `SessionRegistry` + `Store`; a bare
//! `forge run`/`forge chat` session never gets one, so the tool is simply not advertised there.
//! The CLI-bridge mirror (`forge mcp-serve`) and the `forge send` CLI are separate processes with
//! no registry access at all, so they implement the same shape over HTTP against a running
//! daemon instead — see `forge-cli`'s `mcp_serve::fleet` and `cli::commands::send`.
//!
//! Side-effect/permission treatment mirrors `send_to_agent` (ADR-0008): both are core-owned
//! virtual tools intercepted in `tool_dispatch` ahead of the registry/permission broker, and both
//! are always recorded as `"allowed"` — sending a message to a sibling fleet session is treated
//! the same as messaging an already-spawned child, not as a new side effect requiring a fresh
//! permission decision.

use forge_types::ToolCall;

use crate::{CoreError, Session};

/// How a fleet message is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageMode {
    /// Queued for delivery when the target session goes idle / at its current turn's end.
    FollowUp,
    /// Delivered at the target's very next turn boundary, ahead of anything already queued —
    /// but never injected into a turn already streaming.
    Steer,
}

impl MessageMode {
    /// Parse the wire/tool-argument spelling (`""`/`"follow_up"` → follow-up, `"steer"` → steer).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "" | "follow_up" => Some(Self::FollowUp),
            "steer" => Some(Self::Steer),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FollowUp => "follow_up",
            Self::Steer => "steer",
        }
    }
}

/// Where a message lands in a BUSY target's follow-up queue: `Steer` jumps to the front —
/// outranking the queued backlog, so it's delivered at the target's very next turn boundary —
/// while `FollowUp` joins the back, delivered after everything already queued. Idle targets never
/// call this (there's no backlog to outrank; the message just starts a turn immediately).
///
/// Pure so the steer-vs-follow-up ordering decision is unit-testable on its own, independent of
/// the async delivery plumbing that decides busy-vs-idle in the first place (`forge-cli`'s
/// `driver/submit.rs` for daemon-hosted sessions, `run.rs` for a host-terminal TUI with a remote
/// bridge attached).
pub fn insert_into_queue(queue: &mut Vec<String>, mode: MessageMode, text: String) {
    match mode {
        MessageMode::Steer => queue.insert(0, text),
        MessageMode::FollowUp => queue.push(text),
    }
}

/// One addressable fleet peer: a daemon-hosted session other than the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetPeer {
    pub id: String,
    pub title: Option<String>,
}

/// Host-provided fleet messaging capability. Async because the daemon's live registry sits
/// behind a `tokio::sync::Mutex` — see `forge-cli`'s `SessionRegistry`.
#[async_trait::async_trait]
pub trait FleetMessaging: Send + Sync {
    /// Every other live fleet session (never includes the caller's own session).
    async fn peers(&self) -> Vec<FleetPeer>;
    /// Send `text` to `target_session_id` (already resolved — see [`resolve_target`]).
    /// Persistence + delivery live entirely in the host; the `Err` string is shown to the model
    /// as the tool result.
    async fn send(
        &self,
        target_session_id: &str,
        mode: MessageMode,
        text: &str,
    ) -> Result<(), String>;
}

/// Why [`resolve_target`] could not resolve an address to exactly one peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveError {
    NotFound,
    Ambiguous(usize),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "no fleet session matches that address"),
            Self::Ambiguous(n) => write!(f, "address is ambiguous ({n} sessions match)"),
        }
    }
}

/// Resolve a `target`/id/id-prefix address against the live fleet: an exact id match wins
/// outright; otherwise an exact (and unique) title match; otherwise a unique id prefix. Pure and
/// host-independent so both the direct-path tool and the CLI/bridge transports share one
/// resolution rule.
pub fn resolve_target<'a>(
    peers: &'a [FleetPeer],
    address: &str,
) -> Result<&'a FleetPeer, ResolveError> {
    let address = address.trim();
    if address.is_empty() {
        return Err(ResolveError::NotFound);
    }
    if let Some(p) = peers.iter().find(|p| p.id == address) {
        return Ok(p);
    }
    let title_matches: Vec<&FleetPeer> = peers
        .iter()
        .filter(|p| p.title.as_deref() == Some(address))
        .collect();
    match title_matches.len() {
        1 => return Ok(title_matches[0]),
        n if n > 1 => return Err(ResolveError::Ambiguous(n)),
        _ => {}
    }
    let prefix_matches: Vec<&FleetPeer> =
        peers.iter().filter(|p| p.id.starts_with(address)).collect();
    match prefix_matches.len() {
        1 => Ok(prefix_matches[0]),
        0 => Err(ResolveError::NotFound),
        n => Err(ResolveError::Ambiguous(n)),
    }
}

/// A short label for an unresolved address error: `"<title> (<id-prefix>)"` per known peer.
pub fn describe_peers(peers: &[FleetPeer]) -> String {
    peers
        .iter()
        .map(|p| {
            format!(
                "{} ({})",
                p.title.as_deref().unwrap_or("unnamed"),
                &p.id[..p.id.len().min(8)]
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The fleet-messaging virtual tool name.
pub const MESSAGE_SESSION_TOOL: &str = "message_session";

/// The `ToolSpec` advertised to the model for [`MESSAGE_SESSION_TOOL`], only when this session has
/// a [`FleetMessaging`] attached.
pub fn message_session_spec() -> forge_provider::ToolSpec {
    forge_provider::ToolSpec {
        name: MESSAGE_SESSION_TOOL.to_string(),
        description: "Send a message to another daemon-hosted (fleet) session — a sibling agent \
            running elsewhere under this same `forge serve` daemon, addressed by its name or a \
            unique id prefix. `follow_up` (default) queues the message for delivery when the \
            target session goes idle or finishes its current turn. `steer` jumps the queue: it \
            is delivered at the target's very next turn boundary, ahead of anything already \
            queued, but never interrupts a turn already streaming. Message text is capped at \
            16KB."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "target session name or a unique id prefix"
                },
                "message": {
                    "type": "string",
                    "description": "message text (max 16KB)"
                },
                "mode": {
                    "type": "string",
                    "enum": ["follow_up", "steer"],
                    "description": "delivery mode (default follow_up)"
                }
            },
            "required": ["target", "message"]
        }),
    }
}

impl Session {
    /// Handle a `message_session` call: resolve the target against the live fleet and hand the
    /// text to the host's [`FleetMessaging`] callback.
    pub(crate) async fn message_session(
        &mut self,
        msg_id: &str,
        call: &ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let target = call
            .args
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let message = call
            .args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let mode_raw = call
            .args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("follow_up");

        let (result, ok) = match &self.fleet {
            None => (
                "error: message_session is not available — this session is not hosted by \
                 forge serve"
                    .to_string(),
                false,
            ),
            Some(_) if target.is_empty() || message.is_empty() => (
                "error: message_session needs both `target` and `message`".to_string(),
                false,
            ),
            Some(_) if message.len() > 16 * 1024 => (
                format!(
                    "error: message is {} bytes, exceeds the 16KB limit",
                    message.len()
                ),
                false,
            ),
            Some(fleet) => match MessageMode::parse(mode_raw) {
                None => (
                    format!("error: unknown mode '{mode_raw}' — use follow_up or steer"),
                    false,
                ),
                Some(mode) => {
                    let peers = fleet.peers().await;
                    match resolve_target(&peers, &target) {
                        Ok(peer) => match fleet.send(&peer.id, mode, &message).await {
                            Ok(()) => (
                                format!(
                                    "message sent to {} ({}) [{}]",
                                    peer.title.as_deref().unwrap_or("unnamed"),
                                    &peer.id[..peer.id.len().min(8)],
                                    mode.as_str()
                                ),
                                true,
                            ),
                            Err(e) => (format!("error: {e}"), false),
                        },
                        Err(e) => (
                            format!(
                                "error: {e} for '{target}'. Live fleet sessions: [{}]",
                                describe_peers(&peers)
                            ),
                            false,
                        ),
                    }
                }
            },
        };
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            "allowed",
            if ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: &str, title: Option<&str>) -> FleetPeer {
        FleetPeer {
            id: id.to_string(),
            title: title.map(str::to_string),
        }
    }

    #[test]
    fn exact_id_wins_over_everything_else() {
        let peers = vec![
            peer("abc123", Some("worker")),
            peer("abc999", Some("abc123")),
        ];
        // "abc123" is both a real id AND another peer's title — the id match wins.
        assert_eq!(resolve_target(&peers, "abc123").unwrap().id, "abc123");
    }

    #[test]
    fn exact_unique_title_match_wins_over_prefix() {
        let peers = vec![
            peer("aaa111", Some("worker")),
            peer("aaa222", Some("other")),
        ];
        assert_eq!(resolve_target(&peers, "worker").unwrap().id, "aaa111");
    }

    #[test]
    fn duplicate_title_is_ambiguous() {
        let peers = vec![
            peer("aaa111", Some("worker")),
            peer("bbb222", Some("worker")),
        ];
        assert_eq!(
            resolve_target(&peers, "worker").unwrap_err(),
            ResolveError::Ambiguous(2)
        );
    }

    #[test]
    fn unique_id_prefix_resolves() {
        let peers = vec![peer("aaa111", None), peer("bbb222", None)];
        assert_eq!(resolve_target(&peers, "aaa").unwrap().id, "aaa111");
    }

    #[test]
    fn ambiguous_id_prefix_is_reported_with_a_count() {
        let peers = vec![peer("aaa111", None), peer("aaa222", None)];
        assert_eq!(
            resolve_target(&peers, "aaa").unwrap_err(),
            ResolveError::Ambiguous(2)
        );
    }

    #[test]
    fn unknown_address_is_not_found() {
        let peers = vec![peer("aaa111", Some("worker"))];
        assert_eq!(
            resolve_target(&peers, "nope").unwrap_err(),
            ResolveError::NotFound
        );
        assert_eq!(
            resolve_target(&peers, "").unwrap_err(),
            ResolveError::NotFound
        );
    }

    #[test]
    fn steer_jumps_the_queue_follow_up_joins_the_back() {
        let mut queue = vec!["first".to_string(), "second".to_string()];
        insert_into_queue(&mut queue, MessageMode::FollowUp, "third".to_string());
        assert_eq!(queue, vec!["first", "second", "third"]);

        insert_into_queue(&mut queue, MessageMode::Steer, "urgent".to_string());
        assert_eq!(queue, vec!["urgent", "first", "second", "third"]);
    }

    #[test]
    fn steer_on_an_empty_queue_is_just_the_one_entry() {
        let mut queue = Vec::new();
        insert_into_queue(&mut queue, MessageMode::Steer, "only".to_string());
        assert_eq!(queue, vec!["only"]);
    }

    #[test]
    fn repeated_steers_stack_most_recent_first() {
        let mut queue = vec!["backlog".to_string()];
        insert_into_queue(&mut queue, MessageMode::Steer, "first-steer".to_string());
        insert_into_queue(&mut queue, MessageMode::Steer, "second-steer".to_string());
        // The LATEST steer wins the very next turn boundary — it lands at the front.
        assert_eq!(queue, vec!["second-steer", "first-steer", "backlog"]);
    }

    #[test]
    fn message_mode_parses_and_round_trips() {
        assert_eq!(MessageMode::parse(""), Some(MessageMode::FollowUp));
        assert_eq!(MessageMode::parse("follow_up"), Some(MessageMode::FollowUp));
        assert_eq!(MessageMode::parse("steer"), Some(MessageMode::Steer));
        assert_eq!(MessageMode::parse("bogus"), None);
        assert_eq!(MessageMode::FollowUp.as_str(), "follow_up");
        assert_eq!(MessageMode::Steer.as_str(), "steer");
    }
}
