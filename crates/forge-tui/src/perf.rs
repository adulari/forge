//! Opt-in frame-time histogram for the interactive TUI. Set `FORGE_TUI_PERF=<path>` and the
//! render loop records, per metric, how long each frame/iteration/keystroke took; a summary
//! (count, p50, p95, p99, max in microseconds) is rewritten to `<path>` a few times a second and on
//! exit; creating the file `<path>` with its extension changed to `.reset` starts a fresh
//! measurement window. Unset, every entry point is a single relaxed atomic load.

use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    /// `Terminal::draw` end to end: build the frame, diff it, flush to the tty.
    Draw,
    /// Just the widget tree (`render_live`), without the terminal diff/flush.
    Render,
    /// A keystroke handled by the loop until the frame that shows it has been flushed.
    Echo,
    /// One whole iteration of the render loop, excluding its sleep.
    Iter,
    /// Folding a batch of queued presenter events into the app state.
    Apply,
}

const METRICS: [(Metric, &str); 5] = [
    (Metric::Draw, "draw"),
    (Metric::Render, "render"),
    (Metric::Echo, "echo"),
    (Metric::Iter, "iter"),
    (Metric::Apply, "apply"),
];

const MAX_SAMPLES: usize = 200_000;

struct Recorder {
    path: std::path::PathBuf,
    samples: [Vec<u32>; 5],
    last_flush: Instant,
}

static STATE: AtomicU8 = AtomicU8::new(0);
static RECORDER: Mutex<Option<Recorder>> = Mutex::new(None);

fn init() -> bool {
    let rec = std::env::var_os("FORGE_TUI_PERF")
        .filter(|p| !p.is_empty())
        .map(|path| Recorder {
            path: path.into(),
            samples: Default::default(),
            last_flush: Instant::now(),
        });
    let on = rec.is_some();
    if let Ok(mut slot) = RECORDER.lock() {
        *slot = rec;
    }
    STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
    on
}

/// Whether the histogram is collecting. Callers use it to skip taking timestamps at all.
pub fn enabled() -> bool {
    match STATE.load(Ordering::Relaxed) {
        0 => init(),
        s => s == 2,
    }
}

pub fn record(metric: Metric, took: Duration) {
    if !enabled() {
        return;
    }
    let Ok(mut guard) = RECORDER.lock() else {
        return;
    };
    let Some(rec) = guard.as_mut() else { return };
    let slot = &mut rec.samples[metric as usize];
    if slot.len() < MAX_SAMPLES {
        slot.push(u32::try_from(took.as_micros()).unwrap_or(u32::MAX));
    }
    if rec.last_flush.elapsed() >= Duration::from_millis(250) {
        rec.last_flush = Instant::now();
        let reset_marker = rec.path.with_extension("reset");
        if reset_marker.exists() {
            let _ = std::fs::remove_file(&reset_marker);
            rec.samples = Default::default();
        }
        write_summary(rec);
    }
}

/// Write the summary now (the loop calls this on exit so the final numbers are never lost).
pub fn flush() {
    if !enabled() {
        return;
    }
    if let Ok(mut guard) = RECORDER.lock() {
        if let Some(rec) = guard.as_mut() {
            write_summary(rec);
        }
    }
}

/// Start a fresh measurement window: drop everything recorded so far.
pub fn reset() {
    if !enabled() {
        return;
    }
    if let Ok(mut guard) = RECORDER.lock() {
        if let Some(rec) = guard.as_mut() {
            rec.samples = Default::default();
        }
    }
}

/// `(count, p50, p95, p99, max)` in microseconds. Nearest-rank percentiles.
pub fn summarize(samples: &[u32]) -> (usize, u32, u32, u32, u32) {
    if samples.is_empty() {
        return (0, 0, 0, 0, 0);
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = |p: usize| sorted[((sorted.len() * p).div_ceil(100)).clamp(1, sorted.len()) - 1];
    (
        sorted.len(),
        rank(50),
        rank(95),
        rank(99),
        *sorted.last().unwrap_or(&0),
    )
}

fn write_summary(rec: &Recorder) {
    let mut out = String::from("{");
    for (i, (metric, name)) in METRICS.iter().enumerate() {
        let (n, p50, p95, p99, max) = summarize(&rec.samples[*metric as usize]);
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "\"{name}\":{{\"n\":{n},\"p50_us\":{p50},\"p95_us\":{p95},\"p99_us\":{p99},\"max_us\":{max}}}"
        ));
    }
    out.push_str("}\n");
    let tmp = rec.path.with_extension("tmp");
    if let Ok(mut f) = std::fs::File::create(&tmp) {
        if f.write_all(out.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, &rec.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_nearest_rank() {
        let samples: Vec<u32> = (1..=100).collect();
        assert_eq!(summarize(&samples), (100, 50, 95, 99, 100));
        assert_eq!(summarize(&[7]), (1, 7, 7, 7, 7));
        assert_eq!(summarize(&[]), (0, 0, 0, 0, 0));
    }
}
