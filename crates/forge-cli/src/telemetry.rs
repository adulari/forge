//! Anonymous, content-free product counters.
//!
//! The wire schema is intentionally closed: callers can select a surface, but cannot attach
//! arbitrary properties. Every event uses the same constant PostHog distinct id, so events cannot
//! be joined into a device or person history. Local period markers let event counts represent
//! active installations without transmitting an installation identifier.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{Datelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::Command;

const DEFAULT_HOST: &str = "https://eu.i.posthog.com";
const DISTINCT_ID: &str = "forge-anonymous";
const STATE_FILE: &str = "anonymous-telemetry.json";
static LOCAL_EVENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    Cli,
    Tui,
}

impl Surface {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Tui => "tui",
        }
    }
}

/// Opaque launch context carried until command completion.
pub(crate) struct TelemetryRun {
    show_notice: bool,
    surface: Surface,
    tracks_run: bool,
}

impl TelemetryRun {
    pub(crate) const fn show_notice(&self) -> bool {
        self.show_notice
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingEvent {
    event: String,
    period: String,
    #[serde(default)]
    local_id: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TelemetryState {
    #[serde(default)]
    installed: bool,
    #[serde(default)]
    activated: bool,
    #[serde(default)]
    notice_shown: bool,
    #[serde(default)]
    day: String,
    #[serde(default)]
    week: String,
    #[serde(default)]
    month: String,
    #[serde(default)]
    window: String,
    #[serde(default)]
    pending: Vec<PendingEvent>,
}

/// Queue anonymous launch/activity counters and send them in the background.
///
/// Returns `true` once per installation so the composition root can disclose the behaviour. All
/// local and network failures degrade silently: analytics must never prevent Forge from starting.
pub(crate) fn start(command: &Command) -> TelemetryRun {
    let surface = surface(command);
    let tracks_run = matches!(command, Command::Chat { .. } | Command::Run { .. });
    let disabled = TelemetryRun {
        show_notice: false,
        surface,
        tracks_run,
    };
    if !is_enabled() {
        return disabled;
    }
    let Some(api_key) = project_key() else {
        return disabled;
    };
    let Some(path) = state_path() else {
        return disabled;
    };

    let now = Utc::now();
    let iso_week = now.iso_week();
    let day = now.format("%Y-%m-%d").to_string();
    let week = format!("{}-W{:02}", iso_week.year(), iso_week.week());
    let month = now.format("%Y-%m").to_string();
    let window = (now.timestamp() / (30 * 60)).to_string();

    let mut state = read_state(&path);
    let show_notice = !state.notice_shown && surface == Surface::Tui;
    if show_notice {
        state.notice_shown = true;
    }
    let first_install = !state.installed;
    queue_period(&mut state, "forge_installed", "once", first_install);
    state.installed = true;
    queue_changed(&mut state, "forge_active_month", &month, Marker::Month);
    queue_changed(&mut state, "forge_active_week", &week, Marker::Week);
    queue_changed(&mut state, "forge_active_day", &day, Marker::Day);
    queue_changed(&mut state, "forge_active_window", &window, Marker::Window);
    if let Some(event) = feature_event(command) {
        state.pending.push(pending_event(event, &day));
    }

    if write_state(&path, &state).is_err() || state.pending.is_empty() {
        return TelemetryRun {
            show_notice,
            surface,
            tracks_run,
        };
    }

    let pending = state.pending.clone();
    let host = posthog_host();
    let heartbeat_key = api_key.clone();
    let heartbeat_host = host.clone();
    let heartbeat_path = path.clone();
    tokio::spawn(async move {
        if send(&host, &api_key, surface, &pending).await {
            remove_sent(&path, &pending);
        }
    });
    if surface == Surface::Tui {
        tokio::spawn(tui_heartbeat(heartbeat_host, heartbeat_key, heartbeat_path));
    }
    TelemetryRun {
        show_notice,
        surface,
        tracks_run,
    }
}

/// Queue the outcome of an interactive Forge run. It is retried on the next launch if needed.
pub(crate) fn finish(run: TelemetryRun, succeeded: bool) {
    if !run.tracks_run || !is_enabled() {
        return;
    }
    let (Some(api_key), Some(path)) = (project_key(), state_path()) else {
        return;
    };
    let mut state = read_state(&path);
    if succeeded && !state.activated {
        state.activated = true;
        state.pending.push(pending_event("forge_activated", "once"));
    }
    state.pending.push(pending_event(
        if succeeded {
            "forge_run_succeeded"
        } else {
            "forge_run_failed"
        },
        &Utc::now().format("%Y-%m-%d").to_string(),
    ));
    if write_state(&path, &state).is_err() {
        return;
    }
    let pending = state.pending.clone();
    let host = posthog_host();
    tokio::spawn(async move {
        if send(&host, &api_key, run.surface, &pending).await {
            remove_sent(&path, &pending);
        }
    });
}

fn surface(command: &Command) -> Surface {
    match command {
        Command::Chat { plain: false, .. } | Command::Run { tui: true, .. } => Surface::Tui,
        _ => Surface::Cli,
    }
}

fn feature_event(command: &Command) -> Option<&'static str> {
    match command {
        Command::Mesh { .. } => Some("forge_feature_mesh"),
        Command::Voice { .. } => Some("forge_feature_voice"),
        Command::Serve { .. } | Command::Attach { .. } => Some("forge_feature_remote"),
        Command::Mcp { .. } | Command::McpServe { .. } => Some("forge_feature_mcp"),
        Command::Lattice { .. } => Some("forge_feature_lattice"),
        Command::Assay { .. } => Some("forge_feature_assay"),
        Command::Bench { .. } | Command::Benchmarks { .. } => Some("forge_feature_bench"),
        Command::Schedule { .. } | Command::Queue { .. } => Some("forge_feature_automation"),
        Command::Plugin { .. } | Command::Skill { .. } => Some("forge_feature_extensibility"),
        _ => None,
    }
}

fn is_enabled() -> bool {
    if cfg!(debug_assertions) && !env_truthy("FORGE_TELEMETRY_FORCE") {
        return false;
    }
    if env_truthy("DO_NOT_TRACK") {
        return false;
    }
    if let Ok(value) = std::env::var("FORGE_TELEMETRY") {
        if is_false(&value) {
            return false;
        }
    }
    forge_config::load()
        .map(|config| config.telemetry.enabled)
        .unwrap_or(true)
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|value| !is_false(&value))
        .unwrap_or(false)
}

fn is_false(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

fn project_key() -> Option<String> {
    std::env::var("FORGE_POSTHOG_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .or_else(|| {
            option_env!("FORGE_POSTHOG_KEY")
                .filter(|key| !key.trim().is_empty())
                .map(str::to_owned)
        })
}

fn posthog_host() -> String {
    std::env::var("FORGE_POSTHOG_HOST")
        .ok()
        .filter(|host| !host.trim().is_empty())
        .or_else(|| {
            option_env!("FORGE_POSTHOG_HOST")
                .filter(|host| !host.trim().is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| DEFAULT_HOST.to_owned())
        .trim_end_matches('/')
        .to_owned()
}

fn state_path() -> Option<PathBuf> {
    forge_config::data_dir().map(|dir| dir.join(STATE_FILE))
}

fn read_state(path: &Path) -> TelemetryState {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_state(path: &Path, state: &TelemetryState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(state).map_err(std::io::Error::other)?;
    std::fs::write(path, bytes)
}

#[derive(Debug, Clone, Copy)]
enum Marker {
    Day,
    Week,
    Month,
    Window,
}

fn queue_changed(state: &mut TelemetryState, event: &str, period: &str, marker: Marker) {
    let old = match marker {
        Marker::Day => &mut state.day,
        Marker::Week => &mut state.week,
        Marker::Month => &mut state.month,
        Marker::Window => &mut state.window,
    };
    if old == period {
        return;
    }
    old.clone_from(&period.to_owned());
    queue_period(state, event, period, true);
}

fn queue_period(state: &mut TelemetryState, event: &str, period: &str, should_queue: bool) {
    if should_queue
        && !state
            .pending
            .iter()
            .any(|queued| queued.event == event && queued.period == period)
    {
        state.pending.push(pending_event(event, period));
    }
}

fn pending_event(event: &str, period: &str) -> PendingEvent {
    let nanos = Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_default()
        .unsigned_abs();
    let sequence = LOCAL_EVENT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    PendingEvent {
        event: event.to_owned(),
        period: period.to_owned(),
        local_id: nanos ^ sequence ^ u64::from(std::process::id()),
    }
}

async fn send(host: &str, api_key: &str, surface: Surface, pending: &[PendingEvent]) -> bool {
    let batch: Vec<Value> = pending
        .iter()
        .filter(|queued| is_allowed_event(&queued.event))
        .map(|queued| {
            json!({
                "event": queued.event,
                "properties": properties(surface, &queued.period),
            })
        })
        .collect();
    if batch.is_empty() {
        return true;
    }
    let body = json!({ "api_key": api_key, "batch": batch });
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return false;
    };
    client
        .post(format!("{host}/batch/"))
        .json(&body)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

fn is_allowed_event(event: &str) -> bool {
    matches!(
        event,
        "forge_installed"
            | "forge_active_month"
            | "forge_active_week"
            | "forge_active_day"
            | "forge_active_window"
            | "forge_run_succeeded"
            | "forge_run_failed"
            | "forge_activated"
            | "forge_feature_mesh"
            | "forge_feature_voice"
            | "forge_feature_remote"
            | "forge_feature_mcp"
            | "forge_feature_lattice"
            | "forge_feature_assay"
            | "forge_feature_bench"
            | "forge_feature_automation"
            | "forge_feature_extensibility"
    )
}

async fn tui_heartbeat(host: String, api_key: String, path: PathBuf) {
    let mut interval = tokio::time::interval(Duration::from_secs(30 * 60));
    interval.tick().await;
    loop {
        interval.tick().await;
        if !is_enabled() {
            return;
        }
        let window = (Utc::now().timestamp() / (30 * 60)).to_string();
        let mut state = read_state(&path);
        queue_changed(&mut state, "forge_active_window", &window, Marker::Window);
        if state.pending.is_empty() || write_state(&path, &state).is_err() {
            continue;
        }
        let pending = state.pending.clone();
        if send(&host, &api_key, Surface::Tui, &pending).await {
            remove_sent(&path, &pending);
        }
    }
}

fn properties(surface: Surface, period: &str) -> Value {
    json!({
        "distinct_id": DISTINCT_ID,
        "$process_person_profile": false,
        "$geoip_disable": true,
        "surface": surface.as_str(),
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "distribution": option_env!("FORGE_DISTRIBUTION").unwrap_or("unknown"),
        "period": period,
        "schema": 1,
    })
}

fn remove_sent(path: &Path, sent: &[PendingEvent]) {
    let mut state = read_state(path);
    state.pending.retain(|queued| !sent.contains(queued));
    let _ = write_state(path, &state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn properties_have_only_the_public_anonymous_schema() {
        let value = properties(Surface::Tui, "2026-07");
        let object = value.as_object().expect("properties object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "$geoip_disable",
                "$process_person_profile",
                "arch",
                "distinct_id",
                "distribution",
                "os",
                "period",
                "schema",
                "surface",
                "version",
            ]
        );
        assert_eq!(object["distinct_id"], DISTINCT_ID);
        assert_eq!(object["$process_person_profile"], false);
        assert_eq!(object["$geoip_disable"], true);
    }

    #[test]
    fn period_events_are_locally_deduplicated() {
        let mut state = TelemetryState::default();
        queue_changed(&mut state, "forge_active_day", "2026-07-16", Marker::Day);
        queue_changed(&mut state, "forge_active_day", "2026-07-16", Marker::Day);
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.day, "2026-07-16");
    }

    #[test]
    fn false_environment_values_are_recognized() {
        for value in ["0", "false", "NO", "Off"] {
            assert!(is_false(value));
        }
        assert!(!is_false("1"));
    }

    #[test]
    fn wire_event_names_are_closed() {
        assert!(is_allowed_event("forge_active_month"));
        assert!(is_allowed_event("forge_feature_mesh"));
        assert!(is_allowed_event("forge_activated"));
        assert!(!is_allowed_event("prompt_contents"));
        assert!(!is_allowed_event("arbitrary_local_state"));
    }

    #[tokio::test]
    async fn capture_request_contains_only_the_closed_anonymous_payload() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            loop {
                let read = socket.read(&mut buffer).await.expect("read request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or_default();
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .expect("write response");
            String::from_utf8(request).expect("utf-8 request")
        });

        let event = pending_event("forge_active_day", "2026-07-16");
        assert!(
            send(
                &format!("http://{address}"),
                "phc_public_test_key",
                Surface::Tui,
                &[event],
            )
            .await
        );
        let request = server.await.expect("test server task");
        assert!(request.contains("forge_active_day"));
        assert!(request.contains("forge-anonymous"));
        assert!(request.contains("$geoip_disable"));
        assert!(!request.contains("local_id"));
        assert!(!request.contains("prompt"));
        assert!(!request.contains("repository"));
    }
}
