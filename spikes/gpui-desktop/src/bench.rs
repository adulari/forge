//! Frame/event recorder for the spike. A watcher thread dumps it when `$SPIKE_DIR/dump` appears,
//! so an external driver can inject real X11 input and read back what the app saw.
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static START: OnceLock<Instant> = OnceLock::new();
static FRAMES: Mutex<Vec<f64>> = Mutex::new(Vec::new());
static EVENTS: Mutex<Vec<(String, f64)>> = Mutex::new(Vec::new());

pub fn init() {
    START.get_or_init(Instant::now);
}

pub fn now_ms() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// One call per rendered frame of the root view.
pub fn frame() {
    FRAMES.lock().unwrap().push(now_ms());
}

pub fn event(name: &str) {
    EVENTS.lock().unwrap().push((name.to_string(), now_ms()));
}

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
        .unwrap_or(0)
}

pub fn watch() {
    let Ok(dir) = std::env::var("SPIKE_DIR") else { return };
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(40));
        if std::fs::remove_file(format!("{dir}/reset")).is_ok() {
            FRAMES.lock().unwrap().clear();
            EVENTS.lock().unwrap().clear();
        }
        if std::fs::remove_file(format!("{dir}/dump")).is_ok() {
            let out = serde_json::json!({
                "now_ms": now_ms(),
                "rss_kb": rss_kb(),
                "frames": *FRAMES.lock().unwrap(),
                "events": *EVENTS.lock().unwrap(),
            });
            let _ = std::fs::write(format!("{dir}/dump.tmp"), out.to_string());
            let _ = std::fs::rename(format!("{dir}/dump.tmp"), format!("{dir}/dump.json"));
        }
    });
}
