//! On-disk state under `<data dir>/reports/`: the install id, the bounded offline queue, reports
//! waiting for an ask-mode decision, the log of what was sent, and the relay backoff marker.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::report::Report;

pub const MAX_QUEUE: usize = 20;
pub const MAX_PENDING: usize = 10;
pub const MAX_SENT_LOG: usize = 50;
/// A problem already reported from this install in this window is not sent again.
pub const RESEND_AFTER_SECS: u64 = 24 * 3600;

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SentEntry {
    pub sent_at: u64,
    pub report: Report,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BackoffState {
    #[serde(default)]
    backoff_until: u64,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Store { dir }
    }

    /// The default location: `FORGE_REPORTING_DIR`, else `<data dir>/reports`.
    pub fn default_location() -> Option<Store> {
        if let Ok(dir) = std::env::var("FORGE_REPORTING_DIR") {
            if !dir.trim().is_empty() {
                return Some(Store::new(PathBuf::from(dir)));
            }
        }
        forge_config::data_dir().map(|d| Store::new(d.join("reports")))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn queue_dir(&self) -> PathBuf {
        self.dir.join("queue")
    }

    fn pending_dir(&self) -> PathBuf {
        self.dir.join("pending")
    }

    fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        }
        fs::rename(&tmp, path)
    }

    /// The anonymous install id: random, created on first use, never derived from the machine.
    pub fn install_id(&self) -> String {
        let path = self.dir.join("install-id");
        if let Ok(existing) = fs::read_to_string(&path) {
            let id = existing.trim();
            if id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return id.to_ascii_lowercase();
            }
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let _ = Self::write_private(&path, id.as_bytes());
        id
    }

    pub fn write_last(&self, report: &Report) {
        if let Ok(json) = serde_json::to_vec_pretty(report) {
            let _ = Self::write_private(&self.dir.join("last.json"), &json);
        }
    }

    pub fn last(&self) -> Option<Report> {
        serde_json::from_slice(&fs::read(self.dir.join("last.json")).ok()?).ok()
    }

    fn sorted_json(dir: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        files
    }

    fn put_bounded(dir: &Path, report: &Report, max: usize) -> io::Result<PathBuf> {
        let name = format!(
            "{:012}-{}-{}.json",
            report.occurred_at,
            report.fingerprint,
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        );
        let path = dir.join(name);
        let json = serde_json::to_vec(report).map_err(io::Error::other)?;
        Self::write_private(&path, &json)?;
        let files = Self::sorted_json(dir);
        for old in files.iter().take(files.len().saturating_sub(max)) {
            let _ = fs::remove_file(old);
        }
        Ok(path)
    }

    pub fn enqueue(&self, report: &Report) -> io::Result<PathBuf> {
        Self::put_bounded(&self.queue_dir(), report, MAX_QUEUE)
    }

    /// Take exclusive ownership of a queued report by renaming it. A second flusher (another thread
    /// or another forge process) loses the race and skips it, so nothing is ever sent twice.
    pub fn claim(&self, path: &Path) -> Option<PathBuf> {
        let claimed = path.with_extension("sending");
        fs::rename(path, &claimed).ok().map(|()| claimed)
    }

    /// Give a claimed report back to the queue (the send did not complete).
    pub fn release(&self, claimed: &Path) {
        let _ = fs::rename(claimed, claimed.with_extension("json"));
    }

    /// A claim older than this belongs to a flusher that died mid-send.
    fn recover_stale_claims(&self) {
        for entry in fs::read_dir(self.queue_dir())
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if path.extension().is_some_and(|x| x == "sending") {
                let stale = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age.as_secs() > 60);
                if stale {
                    self.release(&path);
                }
            }
        }
    }

    pub fn queued(&self) -> Vec<(PathBuf, Report)> {
        self.recover_stale_claims();
        Self::sorted_json(&self.queue_dir())
            .into_iter()
            .filter_map(|p| {
                let report = serde_json::from_slice(&fs::read(&p).ok()?).ok();
                if report.is_none() {
                    let _ = fs::remove_file(&p);
                }
                report.map(|r| (p, r))
            })
            .collect()
    }

    /// Delete every queued and held report (reporting was switched off).
    pub fn clear_waiting(&self) {
        for dir in [self.queue_dir(), self.pending_dir()] {
            for file in Self::sorted_json(&dir) {
                let _ = fs::remove_file(file);
            }
        }
    }

    pub fn hold_pending(&self, report: &Report) -> io::Result<PathBuf> {
        Self::put_bounded(&self.pending_dir(), report, MAX_PENDING)
    }

    pub fn pending(&self) -> Vec<(String, Report)> {
        Self::sorted_json(&self.pending_dir())
            .into_iter()
            .filter_map(|p| {
                let id = p.file_stem()?.to_string_lossy().to_string();
                let report = serde_json::from_slice(&fs::read(&p).ok()?).ok();
                if report.is_none() {
                    let _ = fs::remove_file(&p);
                }
                report.map(|r| (id, r))
            })
            .collect()
    }

    /// Remove and return one held report.
    pub fn take_pending(&self, id: &str) -> Option<Report> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return None;
        }
        let path = self.pending_dir().join(format!("{id}.json"));
        let report = serde_json::from_slice(&fs::read(&path).ok()?).ok();
        let _ = fs::remove_file(&path);
        report
    }

    pub fn append_sent(&self, report: &Report) {
        let entry = SentEntry {
            sent_at: now_secs(),
            report: report.clone(),
        };
        let Ok(line) = serde_json::to_string(&entry) else {
            return;
        };
        let path = self.dir.join("sent.jsonl");
        let mut lines: Vec<String> = fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        lines.push(line);
        let keep = lines.len().saturating_sub(MAX_SENT_LOG);
        let body = lines[keep..].join("\n") + "\n";
        let _ = Self::write_private(&path, body.as_bytes());
    }

    /// The log of what was sent, oldest first.
    pub fn sent(&self) -> Vec<SentEntry> {
        fs::read_to_string(self.dir.join("sent.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    pub fn recently_sent(&self, fingerprint: &str) -> bool {
        let cutoff = now_secs().saturating_sub(RESEND_AFTER_SECS);
        self.sent()
            .iter()
            .any(|e| e.report.fingerprint == fingerprint && e.sent_at >= cutoff)
    }

    pub fn backoff_until(&self) -> u64 {
        fs::read(self.dir.join("state.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<BackoffState>(&b).ok())
            .map(|s| s.backoff_until)
            .unwrap_or(0)
    }

    pub fn set_backoff_until(&self, until: u64) {
        if let Ok(json) = serde_json::to_vec(&BackoffState {
            backoff_until: until,
        }) {
            let _ = Self::write_private(&self.dir.join("state.json"), &json);
        }
    }

    pub fn in_backoff(&self) -> bool {
        self.backoff_until() > now_secs()
    }
}
