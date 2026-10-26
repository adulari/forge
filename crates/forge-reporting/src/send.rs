//! Background delivery. Nothing here is awaited by a session: reports are files first, sent by one
//! short-lived thread with short timeouts, and a 429/503 from the relay pauses all sending.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::store::{now_secs, Store};

pub const DEFAULT_ENDPOINT: &str = "https://forge.adulari.dev/relay/v1/reports";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MIN_BACKOFF_SECS: u64 = 60;
const MAX_BACKOFF_SECS: u64 = 24 * 3600;
const DEFAULT_BACKOFF_SECS: u64 = 3600;
const NETWORK_BACKOFF_SECS: u64 = 300;
const MAX_BODY: usize = 32 * 1024;

/// `FORGE_REPORTING_URL` or the public relay.
pub fn endpoint() -> String {
    std::env::var("FORGE_REPORTING_URL")
        .ok()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string())
}

#[derive(Debug, PartialEq, Eq)]
pub enum PostOutcome {
    Sent,
    Backoff(Duration),
    Rejected,
    Failed,
}

pub fn clamp_backoff(secs: u64) -> Duration {
    Duration::from_secs(secs.clamp(MIN_BACKOFF_SECS, MAX_BACKOFF_SECS))
}

pub async fn post(client: &reqwest::Client, url: &str, body: Vec<u8>) -> PostOutcome {
    let response = client
        .post(url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await;
    let Ok(response) = response else {
        return PostOutcome::Failed;
    };
    let status = response.status();
    if status.is_success() {
        return PostOutcome::Sent;
    }
    if status.as_u16() == 429 || status.as_u16() == 503 {
        let secs = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_BACKOFF_SECS);
        return PostOutcome::Backoff(clamp_backoff(secs));
    }
    if status.is_server_error() {
        return PostOutcome::Failed;
    }
    PostOutcome::Rejected
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct FlushStats {
    pub sent: usize,
    pub dropped: usize,
    pub paused: bool,
}

/// Send queued reports oldest-first until the queue is empty, `budget` runs out, or the relay says
/// to stop. Blocking; call it from a background thread (or from the panic hook, with a small
/// budget).
pub fn flush(store: &Store, url: &str, budget: Duration) -> FlushStats {
    let mut stats = FlushStats::default();
    if store.in_backoff() {
        stats.paused = true;
        return stats;
    }
    let queued = store.queued();
    if queued.is_empty() {
        return stats;
    }
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return stats;
    };
    let Ok(client) = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build() else {
        return stats;
    };
    let deadline = Instant::now() + budget;
    for (listed, report) in queued {
        if Instant::now() >= deadline {
            break;
        }
        let Some(path) = store.claim(&listed) else {
            continue;
        };
        let Ok(body) = serde_json::to_vec(&report) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if body.len() > MAX_BODY {
            let _ = std::fs::remove_file(&path);
            stats.dropped += 1;
            continue;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let outcome = runtime.block_on(async {
            tokio::time::timeout(
                remaining.max(Duration::from_millis(200)),
                post(&client, url, body),
            )
            .await
            .unwrap_or(PostOutcome::Failed)
        });
        match outcome {
            PostOutcome::Sent => {
                store.append_sent(&report);
                let _ = std::fs::remove_file(&path);
                stats.sent += 1;
            }
            PostOutcome::Rejected => {
                tracing::debug!("report rejected by relay; dropping it");
                let _ = std::fs::remove_file(&path);
                stats.dropped += 1;
            }
            PostOutcome::Backoff(wait) => {
                store.release(&path);
                store.set_backoff_until(now_secs() + wait.as_secs());
                stats.paused = true;
                break;
            }
            PostOutcome::Failed => {
                store.release(&path);
                store.set_backoff_until(now_secs() + NETWORK_BACKOFF_SECS);
                stats.paused = true;
                break;
            }
        }
    }
    stats
}

/// [`flush`] on its own thread, waiting at most `budget` for it. Safe to call from inside a tokio
/// runtime or a panic hook, where blocking on a nested runtime would itself panic.
pub fn flush_on_thread(store: &Store, url: &str, budget: Duration) -> FlushStats {
    let (tx, rx) = std::sync::mpsc::channel();
    let (store, url) = (store.clone(), url.to_string());
    let spawned = std::thread::Builder::new()
        .name("forge-report-flush".into())
        .spawn(move || {
            let _ = tx.send(flush(&store, &url, budget));
        });
    if spawned.is_err() {
        return FlushStats::default();
    }
    rx.recv_timeout(budget + Duration::from_millis(500))
        .unwrap_or_default()
}

static FLUSHING: AtomicBool = AtomicBool::new(false);

/// Flush on a background thread; at most one runs at a time.
pub fn spawn_flush(store: Store, url: String) {
    if FLUSHING.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("forge-report-send".into())
        .spawn(move || {
            let _ = flush(&store, &url, Duration::from_secs(20));
            FLUSHING.store(false, Ordering::Release);
        });
    if spawned.is_err() {
        FLUSHING.store(false, Ordering::Release);
    }
}
