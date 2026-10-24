//! Debug trace of what runs between `main` and the first frame (`RUST_LOG=forge::startup=debug`).
//! The session build has its own clock (`StartupClock`); this one starts at `main`, so the time
//! before the session exists is attributed to a phase instead of guessed at.

use std::sync::OnceLock;
use std::time::Instant;

static MAIN_ENTERED: OnceLock<Instant> = OnceLock::new();

pub(crate) fn begin() {
    let _ = MAIN_ENTERED.set(Instant::now());
}

pub(crate) fn mark(phase: &str) {
    if let Some(start) = MAIN_ENTERED.get() {
        tracing::debug!(
            target: "forge::startup",
            "{phase} (main+{} ms, rss {} MB)",
            start.elapsed().as_millis(),
            resident_mb()
        );
    }
}

/// With `RUST_LOG=forge::startup=debug`, log the resident set every 250 ms for the first 10 s, so a
/// step in memory can be lined up with whatever else the log says happened at that moment.
pub(crate) fn sample_memory_at_start() {
    if !tracing::enabled!(target: "forge::startup", tracing::Level::DEBUG) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("forge-rss-sampler".into())
        .spawn(|| {
            let mut last = 0;
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(250));
                let now = resident_mb();
                if now != last {
                    tracing::debug!(target: "forge::startup", "memory: {last} -> {now} MB");
                    last = now;
                }
            }
        });
}

/// Resident set of this process in MB (0 where `/proc` is not there); a few microseconds.
pub(crate) fn resident_mb() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .nth(1)
                .and_then(|p| p.parse::<u64>().ok())
        })
        .map_or(0, |pages| pages * 4096 / (1024 * 1024))
}

/// Logs how long its scope took when dropped (one debug line).
pub(crate) struct Span {
    phase: &'static str,
    started: Instant,
}

impl Span {
    pub(crate) fn new(phase: &'static str) -> Self {
        Self {
            phase,
            started: Instant::now(),
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        tracing::debug!(
            target: "forge::startup",
            "{} took {} ms (rss {} MB)",
            self.phase,
            self.started.elapsed().as_millis(),
            resident_mb()
        );
    }
}
