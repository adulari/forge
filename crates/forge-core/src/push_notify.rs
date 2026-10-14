//! The `push_notification` virtual tool: a model-callable "tell the user" that fans out to every
//! channel the host has (phone, web push, desktop).
//!
//! Like `message_session`, the session core never talks to a delivery backend: the process
//! installs a [`PushSink`] once ([`install_sink`]) — `forge serve` an in-process one that owns
//! Anywhere/APNs/VAPID state, everything else (`forge run`, the TUI, the `forge mcp-serve` bridge
//! child) one that routes through the running daemon and falls back to a desktop notification.
//! The same [`run`] entry point serves the direct path and the bridge, so permission, argument
//! validation and the anti-spam limiter cannot diverge between them.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use forge_types::{PermissionDecision, PermissionMode, SideEffect, ToolCall};

use crate::permission::decide_mode;
use crate::{CoreError, Session, ToolSpec};

pub const PUSH_NOTIFICATION_TOOL: &str = "push_notification";

const MAX_MESSAGE_CHARS: usize = 240;
const MAX_TITLE_CHARS: usize = 60;

/// Sends allowed per session inside [`WINDOW`].
pub const MAX_PER_WINDOW: usize = 4;
/// Sends allowed per session per hour, so a slow loop is still bounded.
pub const MAX_PER_HOUR: usize = 20;
/// An identical message from the same session is dropped for this long.
pub const DEDUPE_WINDOW: Duration = Duration::from_secs(600);
pub const WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Priority {
    #[default]
    Normal,
    High,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::High => "high",
        }
    }
}

/// A validated notification, ready for a sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRequest {
    pub message: String,
    pub title: Option<String>,
    pub priority: Priority,
    pub session_id: String,
}

/// What one channel did with a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelReport {
    pub channel: String,
    pub delivered: bool,
    pub detail: String,
}

impl ChannelReport {
    pub fn ok(channel: &str, detail: &str) -> Self {
        Self {
            channel: channel.into(),
            delivered: true,
            detail: detail.into(),
        }
    }

    pub fn failed(channel: &str, detail: &str) -> Self {
        Self {
            channel: channel.into(),
            delivered: false,
            detail: detail.into(),
        }
    }
}

#[async_trait::async_trait]
pub trait PushSink: Send + Sync {
    /// Fan `request` out. An empty list means no channel was available at all.
    async fn send(&self, request: &PushRequest) -> Vec<ChannelReport>;
}

static SINK: RwLock<Option<Arc<dyn PushSink>>> = RwLock::new(None);

/// Install (or replace) the process-wide sink.
pub fn install_sink(sink: Arc<dyn PushSink>) {
    *SINK.write().unwrap_or_else(|e| e.into_inner()) = Some(sink);
}

fn installed_sink() -> Option<Arc<dyn PushSink>> {
    SINK.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Permission for the tool. It reaches the user's own devices over the network, so it is
/// classified as [`SideEffect::Network`]: refused in Plan (no side effects at all), allowed in
/// Auto/AcceptEdits/Bypass. Guarded (`Default`) would ask for a network call, but a virtual tool
/// has no prompt to show and the limiter bounds the cost, so it proceeds there too.
pub fn permission_error(mode: PermissionMode) -> Option<String> {
    match decide_mode(mode, SideEffect::Network) {
        PermissionDecision::Deny => Some(format!(
            "push_notification is not available in {} mode (it is a side effect). Tell the user \
             in your reply instead.",
            mode.key()
        )),
        _ => None,
    }
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: PUSH_NOTIFICATION_TOOL.to_string(),
        description: format!(
            "Notify the user on their phone and desktop — for a long task that finished while \
             they are away, or a decision only they can make. Not for progress updates or \
             anything you could say in your normal reply. At most {MAX_PER_WINDOW} per minute; an \
             identical message is suppressed. Reports which channels delivered; when none is \
             configured, say it in your reply instead."
        ),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "what the user needs to know, one short sentence"
                },
                "title": {
                    "type": "string",
                    "description": "optional short heading (default \"Forge\")"
                },
                "priority": {
                    "type": "string",
                    "enum": ["normal", "high"],
                    "description": "high only when the user must act now"
                }
            },
            "required": ["message"]
        }),
    }
}

fn collapse(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// Validate tool arguments. `Err` is the text shown to the model.
pub fn parse_args(args: &serde_json::Value, session_id: &str) -> Result<PushRequest, String> {
    let message = collapse(
        args.get("message").and_then(|v| v.as_str()).unwrap_or(""),
        MAX_MESSAGE_CHARS,
    );
    if message.is_empty() {
        return Err("error: `message` is required".into());
    }
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .map(|t| collapse(t, MAX_TITLE_CHARS))
        .filter(|t| !t.is_empty());
    let priority = match args
        .get("priority")
        .or_else(|| args.get("urgency"))
        .and_then(|v| v.as_str())
        .map(|p| p.trim().to_lowercase())
        .as_deref()
    {
        None | Some("") | Some("normal") => Priority::Normal,
        Some("high") => Priority::High,
        Some(other) => {
            return Err(format!(
                "error: unknown priority '{other}' — use normal or high"
            ))
        }
    };
    Ok(PushRequest {
        message,
        title,
        priority,
        session_id: session_id.to_string(),
    })
}

/// Why a send was refused before reaching any channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Duplicate,
    RateLimited { retry_in: Duration },
}

impl Refusal {
    pub fn text(&self) -> String {
        match self {
            Self::Duplicate => "not sent: the same notification was already sent recently. Do \
                                not repeat it."
                .into(),
            Self::RateLimited { retry_in } => format!(
                "not sent: too many notifications from this session — wait {}s. Stop \
                 notifying and put it in your reply.",
                retry_in.as_secs().max(1)
            ),
        }
    }
}

/// Per-session send limiter: a burst cap, an hourly cap, and identical-message suppression.
#[derive(Default)]
pub struct Limiter {
    sessions: HashMap<String, SessionWindow>,
}

#[derive(Default)]
struct SessionWindow {
    sends: VecDeque<Instant>,
    recent: HashMap<String, Instant>,
}

impl Limiter {
    /// Record the send when allowed. Refused attempts are not recorded, so a looping model does
    /// not extend its own penalty indefinitely.
    pub fn check(&mut self, session: &str, message: &str, now: Instant) -> Result<(), Refusal> {
        let w = self.sessions.entry(session.to_string()).or_default();
        let hour = Duration::from_secs(3600);
        while w
            .sends
            .front()
            .is_some_and(|t| now.duration_since(*t) >= hour)
        {
            w.sends.pop_front();
        }
        w.recent
            .retain(|_, t| now.duration_since(*t) < DEDUPE_WINDOW);
        let key = message.to_lowercase();
        if w.recent.contains_key(&key) {
            return Err(Refusal::Duplicate);
        }
        let in_window: Vec<Instant> = w
            .sends
            .iter()
            .copied()
            .filter(|t| now.duration_since(*t) < WINDOW)
            .collect();
        if in_window.len() >= MAX_PER_WINDOW {
            let oldest = in_window[0];
            return Err(Refusal::RateLimited {
                retry_in: WINDOW.saturating_sub(now.duration_since(oldest)),
            });
        }
        if w.sends.len() >= MAX_PER_HOUR {
            return Err(Refusal::RateLimited {
                retry_in: hour.saturating_sub(now.duration_since(w.sends[0])),
            });
        }
        w.sends.push_back(now);
        w.recent.insert(key, now);
        Ok(())
    }
}

static LIMITER: Mutex<Option<Limiter>> = Mutex::new(None);

fn global_check(session: &str, message: &str) -> Result<(), Refusal> {
    let mut guard = LIMITER.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(Limiter::default)
        .check(session, message, Instant::now())
}

/// Human-readable tool result for a set of channel reports.
pub fn format_result(reports: &[ChannelReport]) -> (String, bool) {
    if reports.is_empty() {
        return (NO_CHANNEL.to_string(), false);
    }
    let delivered: Vec<&ChannelReport> = reports.iter().filter(|r| r.delivered).collect();
    let lines = reports
        .iter()
        .map(|r| {
            format!(
                "- {}: {}{}",
                r.channel,
                if r.delivered { "delivered" } else { "failed" },
                if r.detail.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", r.detail)
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if delivered.is_empty() {
        (
            format!("no channel delivered the notification:\n{lines}"),
            false,
        )
    } else {
        (
            format!(
                "notification sent via {} channel(s):\n{lines}",
                delivered.len()
            ),
            true,
        )
    }
}

pub const NO_CHANNEL: &str = "no notification channel available — nothing was sent. The user can \
    configure one: enroll this host with `forge anywhere enable` (phone), open the web app and \
    allow notifications (web push), or run on a machine with a graphical session (desktop). Put \
    the message in your reply instead.";

/// The whole tool, minus transcript recording: permission, validation, limiter, delivery. The
/// `bool` is "tool succeeded"; a missing channel is reported but never an error that fails a turn.
pub async fn run(
    args: &serde_json::Value,
    session_id: &str,
    mode: PermissionMode,
) -> (String, bool) {
    if let Some(denied) = permission_error(mode) {
        return (denied, false);
    }
    let request = match parse_args(args, session_id) {
        Ok(r) => r,
        Err(e) => return (e, false),
    };
    if let Err(refusal) = global_check(session_id, &request.message) {
        return (refusal.text(), false);
    }
    let Some(sink) = installed_sink() else {
        return (NO_CHANNEL.to_string(), true);
    };
    let reports = sink.send(&request).await;
    // "Nothing is configured" is information for the model, not a failure of its turn.
    let (text, _) = format_result(&reports);
    (text, true)
}

impl Session {
    pub(crate) async fn push_notification(
        &mut self,
        msg_id: &str,
        call: &ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let (result, ok) = run(&call.args, &self.id, self.mode).await;
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
    use serde_json::json;

    fn t(secs: u64) -> Instant {
        // A fixed base so tests control elapsed time without sleeping.
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(Instant::now) + Duration::from_secs(secs)
    }

    #[test]
    fn args_require_a_message_and_normalise_whitespace() {
        assert!(parse_args(&json!({}), "s").is_err());
        assert!(parse_args(&json!({"message": "   "}), "s").is_err());
        let r = parse_args(&json!({"message": " build\n  done ", "title": " CI "}), "s").unwrap();
        assert_eq!(r.message, "build done");
        assert_eq!(r.title.as_deref(), Some("CI"));
        assert_eq!(r.priority, Priority::Normal);
    }

    #[test]
    fn priority_accepts_urgency_alias_and_rejects_unknown() {
        let hi = parse_args(&json!({"message": "x", "urgency": "HIGH"}), "s").unwrap();
        assert_eq!(hi.priority, Priority::High);
        let err = parse_args(&json!({"message": "x", "priority": "urgent"}), "s").unwrap_err();
        assert!(err.contains("unknown priority"), "{err}");
    }

    #[test]
    fn long_messages_are_truncated() {
        let r = parse_args(&json!({"message": "a".repeat(1000)}), "s").unwrap();
        assert_eq!(r.message.chars().count(), MAX_MESSAGE_CHARS);
        assert!(r.message.ends_with('…'));
    }

    #[test]
    fn plan_mode_denies_everything_else_proceeds() {
        assert!(permission_error(PermissionMode::Plan).is_some());
        for m in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ] {
            assert!(permission_error(m).is_none(), "{m:?}");
        }
    }

    #[test]
    fn identical_messages_are_suppressed_until_the_window_passes() {
        let mut l = Limiter::default();
        assert!(l.check("s", "Build done", t(0)).is_ok());
        assert_eq!(l.check("s", "build DONE", t(5)), Err(Refusal::Duplicate));
        assert!(l.check("other", "Build done", t(5)).is_ok());
        assert!(l.check("s", "Build done", t(601)).is_ok());
    }

    #[test]
    fn burst_is_capped_per_minute_and_recovers() {
        let mut l = Limiter::default();
        for i in 0..MAX_PER_WINDOW {
            assert!(l.check("s", &format!("m{i}"), t(i as u64)).is_ok());
        }
        match l.check("s", "one more", t(10)) {
            Err(Refusal::RateLimited { retry_in }) => assert!(retry_in <= WINDOW),
            other => panic!("{other:?}"),
        }
        assert!(l.check("s", "one more", t(61)).is_ok());
    }

    #[test]
    fn hourly_cap_bounds_a_slow_loop() {
        let mut l = Limiter::default();
        for i in 0..MAX_PER_HOUR {
            assert!(
                l.check("s", &format!("m{i}"), t(i as u64 * 100)).is_ok(),
                "{i}"
            );
        }
        assert!(matches!(
            l.check("s", "late", t(MAX_PER_HOUR as u64 * 100)),
            Err(Refusal::RateLimited { .. })
        ));
    }

    #[test]
    fn refused_attempts_do_not_extend_the_penalty() {
        let mut l = Limiter::default();
        for i in 0..MAX_PER_WINDOW {
            l.check("s", &format!("m{i}"), t(i as u64)).unwrap();
        }
        for _ in 0..50 {
            assert!(l.check("s", "spam", t(20)).is_err());
        }
        assert!(l.check("s", "later", t(61)).is_ok());
    }

    #[test]
    fn result_reports_each_channel_and_the_empty_case() {
        let (text, ok) = format_result(&[]);
        assert!(!ok && text.contains("no notification channel available"));
        let (text, ok) = format_result(&[
            ChannelReport::ok("desktop", ""),
            ChannelReport::failed("web-push", "no subscriptions"),
        ]);
        assert!(ok);
        assert!(text.contains("desktop: delivered") && text.contains("web-push: failed"));
        let (text, ok) = format_result(&[ChannelReport::failed("desktop", "no display")]);
        assert!(!ok && text.contains("no channel delivered"));
    }

    fn test_session() -> Session {
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config = forge_config::Config::default();
        Session::start(
            Arc::new(forge_store::Store::open_in_memory().unwrap()),
            Arc::new(forge_provider::MockProvider),
            Arc::new(crate::HeuristicRouter::new(config.clone())),
            forge_tools::ToolRegistry::with_core_tools_in(workspace),
            Box::new(forge_tui::HeadlessPresenter::new(false)),
            config,
            workspace.to_str().unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn direct_path_advertises_the_tool_records_the_call_and_denies_in_plan() {
        let mut s = test_session();
        // Deferred on the lean surface: reachable through tool_search/tool_call.
        assert!(s
            .full_tool_specs()
            .iter()
            .any(|t| t.name == PUSH_NOTIFICATION_TOOL));
        let msg_id = s
            .store
            .add_message(s.id(), 0, forge_types::Role::User, "hi", None)
            .unwrap();
        let call = ToolCall {
            id: "tc1".into(),
            name: PUSH_NOTIFICATION_TOOL.into(),
            args: json!({"message": "direct-path-test"}),
        };
        s.mode = PermissionMode::Plan;
        let denied = s.push_notification(&msg_id, &call).await.unwrap();
        assert!(denied.contains("plan"), "{denied}");
        s.mode = PermissionMode::Auto;
        let sent = s.push_notification(&msg_id, &call).await.unwrap();
        assert!(!sent.starts_with("error"), "{sent}");
    }

    struct Recorder(Mutex<Vec<PushRequest>>);

    #[async_trait::async_trait]
    impl PushSink for Recorder {
        async fn send(&self, request: &PushRequest) -> Vec<ChannelReport> {
            self.0.lock().unwrap().push(request.clone());
            vec![ChannelReport::ok("test", "")]
        }
    }

    #[tokio::test]
    async fn run_validates_then_dedupes_then_delivers() {
        let rec = Arc::new(Recorder(Mutex::new(Vec::new())));
        install_sink(rec.clone());
        let session = "run-test-session";
        let ok_args = json!({"message": "deploy finished", "priority": "high"});

        let (text, ok) = run(&ok_args, session, PermissionMode::Plan).await;
        assert!(!ok && text.contains("plan"), "{text}");
        let (_, ok) = run(&json!({}), session, PermissionMode::Auto).await;
        assert!(!ok);

        let (text, ok) = run(&ok_args, session, PermissionMode::Auto).await;
        assert!(ok && text.contains("test: delivered"), "{text}");
        let (text, ok) = run(&ok_args, session, PermissionMode::Auto).await;
        assert!(!ok && text.contains("already sent"), "{text}");

        let seen = rec.0.lock().unwrap();
        let mine: Vec<_> = seen.iter().filter(|r| r.session_id == session).collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].priority, Priority::High);
    }
}
