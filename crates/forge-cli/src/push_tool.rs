//! Delivery half of the `push_notification` tool (`forge_core::push_notify`).
//!
//! Two sinks share one fan-out: the daemon's [`DaemonSink`] owns Anywhere, APNs and VAPID state
//! and delivers in-process; every other process (`forge run`, the TUI, the `forge mcp-serve`
//! bridge child) uses [`LocalSink`], which hands the request to the running daemon and only falls
//! back to a desktop notification when no daemon answers.
//!
//! Privacy follows ADR-0012 and docs/features/forge-anywhere.md: hosted services never see the
//! message in readable form. The Anywhere leg is the closed-vocabulary `attention_required` hint, APNs through the
//! public relay is rewritten to a generic alert (carrying, per device that registered a key, the text
//! sealed end-to-end so only its notification extension can open it), and the real text is kept on the host in the
//! [`notices`] log, which the app reads over the end-to-end encrypted bridge. Web push is
//! self-hosted (the subscription endpoint is the user's own browser vendor, payload encrypted per
//! RFC 8291), so it carries the text.

use std::sync::Arc;
use std::time::Duration;

use forge_core::push_notify::{ChannelReport, Priority, PushRequest, PushSink};

pub(crate) mod notices;
mod routes;

pub(crate) use routes::{channels_for, routes};

const DESKTOP_TIMEOUT: Duration = Duration::from_secs(4);
const DAEMON_TIMEOUT: Duration = Duration::from_secs(8);
/// Web push retention: a "build finished" is still worth seeing an hour later.
const WEB_TTL: u32 = 3600;

pub(crate) fn install_local_sink() {
    forge_core::push_notify::install_sink(Arc::new(LocalSink));
}

pub(crate) fn install_daemon_sink(channels: Channels) {
    forge_core::push_notify::install_sink(Arc::new(DaemonSink(Arc::new(channels))));
}

/// Everything the daemon can deliver through. `ambient` covers the channels that touch the
/// outside world (Anywhere service, desktop) and is off in tests.
pub(crate) struct Channels {
    pub(crate) store: Arc<forge_store::Store>,
    pub(crate) web: Option<Arc<crate::push::PushNotifier>>,
    pub(crate) apns: Option<Arc<crate::apns::ApnsNotifier>>,
    pub(crate) ambient: bool,
}

impl Channels {
    pub(crate) async fn deliver(&self, request: &PushRequest) -> Vec<ChannelReport> {
        let notice_seq = notices::record(request);
        let (web, apns, anywhere, desktop) = tokio::join!(
            self.web_push(request),
            self.native_push(request, notice_seq),
            self.anywhere(),
            self.desktop(request),
        );
        [web, apns, anywhere, desktop]
            .into_iter()
            .flatten()
            .collect()
    }

    fn message(request: &PushRequest, ttl: u32) -> crate::push::PushMessage {
        crate::push::PushMessage {
            kind: "notify",
            session: request.session_id.clone(),
            title: request.title.clone().unwrap_or_else(|| "Forge".into()),
            body: request.message.clone(),
            seq: 0,
            ttl,
        }
    }

    async fn web_push(&self, request: &PushRequest) -> Option<ChannelReport> {
        let notifier = self.web.as_ref()?;
        let store = self.store.clone();
        let subscribers = tokio::task::spawn_blocking(move || {
            store
                .list_push_subscriptions()
                .map(|s| s.len())
                .unwrap_or(0)
        })
        .await
        .unwrap_or(0);
        Some(if subscribers == 0 {
            ChannelReport::failed("web-push", "no browser or PWA has subscribed")
        } else {
            notifier.dispatch(Self::message(request, WEB_TTL));
            ChannelReport::ok(
                "web-push",
                &format!("queued for {subscribers} subscription(s)"),
            )
        })
    }

    async fn native_push(&self, request: &PushRequest, notice_seq: u64) -> Option<ChannelReport> {
        let notifier = self.apns.as_ref()?;
        let store = self.store.clone();
        let (devices, sealed) = tokio::task::spawn_blocking(move || {
            let subs = store.list_apns_subscriptions().unwrap_or_default();
            (subs.len(), subs.iter().filter(|s| s.e2e.is_some()).count())
        })
        .await
        .unwrap_or((0, 0));
        Some(if devices == 0 {
            ChannelReport::failed("phone-apns", "no iOS device has registered")
        } else {
            let mut message = Self::message(request, WEB_TTL);
            message.seq = notice_seq;
            notifier.dispatch_alert(message);
            let shown = if !notifier.alerts_are_generic() {
                "queued".to_string()
            } else if sealed == devices {
                "lock screen shows the real text, sealed end-to-end to the device".to_string()
            } else if sealed > 0 {
                format!(
                    "real text sealed end-to-end for {sealed} device(s); the rest show a generic alert (update the app)"
                )
            } else {
                "lock screen shows a generic alert; real text in the app (update the app for sealed text)"
                    .to_string()
            };
            ChannelReport::ok("phone-apns", &format!("{shown} ({devices} device(s))"))
        })
    }

    async fn anywhere(&self) -> Option<ChannelReport> {
        if !self.ambient {
            return None;
        }
        let outcome = crate::anywhere::push::tool_hint().await?;
        Some(match outcome {
            Ok(()) => ChannelReport::ok(
                "phone-anywhere",
                "generic alert sent; real text is end-to-end encrypted and shown in the app",
            ),
            Err(e) => ChannelReport::failed("phone-anywhere", &e),
        })
    }

    async fn desktop(&self, request: &PushRequest) -> Option<ChannelReport> {
        if !self.ambient {
            return None;
        }
        Some(desktop_notify(request).await)
    }
}

pub(crate) struct DaemonSink(Arc<Channels>);

#[async_trait::async_trait]
impl PushSink for DaemonSink {
    async fn send(&self, request: &PushRequest) -> Vec<ChannelReport> {
        self.0.deliver(request).await
    }
}

/// Non-daemon processes: ask the running daemon, else notify the desktop directly.
pub(crate) struct LocalSink;

enum DaemonAnswer {
    Delivered(Vec<ChannelReport>),
    Refused(String),
    Unreachable,
}

#[async_trait::async_trait]
impl PushSink for LocalSink {
    async fn send(&self, request: &PushRequest) -> Vec<ChannelReport> {
        match ask_daemon(request).await {
            DaemonAnswer::Delivered(reports) => reports,
            DaemonAnswer::Refused(why) => vec![ChannelReport::failed("daemon", &why)],
            DaemonAnswer::Unreachable => vec![desktop_notify(request).await],
        }
    }
}

async fn ask_daemon(request: &PushRequest) -> DaemonAnswer {
    let base = crate::attach::resolve_base_url(None);
    let Ok(token) = crate::attach::resolve_token(None) else {
        return DaemonAnswer::Unreachable;
    };
    let Ok(http) = reqwest::Client::builder().timeout(DAEMON_TIMEOUT).build() else {
        return DaemonAnswer::Unreachable;
    };
    let response = http
        .post(format!("{base}/{token}/api/notify"))
        .json(&serde_json::json!({
            "message": request.message,
            "title": request.title,
            "priority": request.priority.as_str(),
            "session_id": request.session_id,
        }))
        .send()
        .await;
    let Ok(response) = response else {
        return DaemonAnswer::Unreachable;
    };
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    if !status.is_success() {
        let why = body["error"].as_str().unwrap_or("rejected").to_string();
        return DaemonAnswer::Refused(format!("daemon answered {status}: {why}"));
    }
    DaemonAnswer::Delivered(
        body["channels"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|c| ChannelReport {
                        channel: c["channel"].as_str().unwrap_or("?").into(),
                        delivered: c["delivered"].as_bool().unwrap_or(false),
                        detail: c["detail"].as_str().unwrap_or("").into(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// The program and arguments that show `title`/`body`, or `None` where this OS has no notifier
/// Forge knows.
pub(crate) fn desktop_command(
    title: &str,
    body: &str,
    priority: Priority,
) -> Option<(&'static str, Vec<String>)> {
    if cfg!(target_os = "macos") {
        let source = crate::cli::commands::run::notify::osascript_source(title, body);
        Some(("osascript", vec!["-e".into(), source]))
    } else if cfg!(target_os = "linux") {
        let urgency = match priority {
            Priority::Normal => "normal",
            Priority::High => "critical",
        };
        Some((
            "notify-send",
            vec![
                "--app-name=Forge".into(),
                format!("--urgency={urgency}"),
                "--".into(),
                title.into(),
                body.into(),
            ],
        ))
    } else {
        None
    }
}

async fn desktop_notify(request: &PushRequest) -> ChannelReport {
    let title = request.title.as_deref().unwrap_or("Forge");
    let Some((program, args)) = desktop_command(title, &request.message, request.priority) else {
        return ChannelReport::failed("desktop", "no notifier for this OS");
    };
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    // A daemon started by systemd may lack the bus address a desktop login exports.
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
            let bus = std::path::Path::new(&dir).join("bus");
            if bus.exists() {
                command.env(
                    "DBUS_SESSION_BUS_ADDRESS",
                    format!("unix:path={}", bus.display()),
                );
            }
        }
    }
    match tokio::time::timeout(DESKTOP_TIMEOUT, command.status()).await {
        Ok(Ok(status)) if status.success() => ChannelReport::ok("desktop", program),
        Ok(Ok(_)) => {
            ChannelReport::failed("desktop", "no graphical session or notification daemon")
        }
        Ok(Err(e)) => ChannelReport::failed("desktop", &format!("{program} unavailable: {e}")),
        Err(_) => ChannelReport::failed("desktop", "notifier timed out"),
    }
}

#[cfg(test)]
mod tests;
