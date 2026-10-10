use std::sync::{Arc, Mutex, OnceLock};

use super::Snapshot;

/// How many broadcast snapshots the per-server [`EventLog`] retains for reconnect replay. One
/// entry per *changed* frame covers minutes of activity; a client that was away longer gets a
/// full-snapshot resync instead (plus `GET /api/history` pagination for the scrollback it wants).
pub const EVENT_LOG_CAP: usize = 512;

/// Byte ceiling on what the [`EventLog`] retains, counted over each frame's struct and serialized
/// forms. The frame count alone does not bound memory: a frame is as big as the transcript tail,
/// the streaming edge and the overlay it carries, so a session with long tool lines kept 512 of
/// them per session for the life of the daemon.
pub const EVENT_LOG_MAX_BYTES: usize = 8 * 1024 * 1024;

/// A bounded ring of every broadcast snapshot keyed by its [`Snapshot::revision`], so a
/// reconnecting client (`?rev=<last seen>` on the WS handshake) replays exactly the frames it
/// missed instead of flickering through a from-scratch rebuild. Revisions are consecutive (one
/// bump per actually-broadcast frame), so "everything after rev N" is answerable precisely — or
/// not at all (evicted / unknown / foreign counter), which forces a full-snapshot resync.
pub struct SnapshotFrame {
    pub snapshot: Snapshot,
    json: Arc<str>,
    resync_json: OnceLock<Arc<str>>,
}

impl SnapshotFrame {
    pub fn new(snapshot: Snapshot) -> Self {
        let json = serde_json::to_string(&snapshot)
            .unwrap_or_else(|_| "{}".into())
            .into();
        Self {
            snapshot,
            json,
            resync_json: OnceLock::new(),
        }
    }

    pub(super) fn text(&self, resync: bool) -> Arc<str> {
        if !resync {
            return self.json.clone();
        }
        self.resync_json
            .get_or_init(|| {
                let mut snapshot = self.snapshot.clone();
                snapshot.resync = true;
                serde_json::to_string(&snapshot)
                    .unwrap_or_else(|_| "{}".into())
                    .into()
            })
            .clone()
    }

    /// What this frame costs the log: the struct is about as large as its JSON, and the resync
    /// copy is built on demand, so the serialized form counts twice.
    fn retained_bytes(&self) -> usize {
        self.json.len() * 2
    }
}

pub struct EventLog {
    ring: std::collections::VecDeque<(u64, Arc<SnapshotFrame>)>,
    cap: usize,
    max_bytes: usize,
    bytes: usize,
}

pub(super) fn lock_event_log(events: &Mutex<EventLog>) -> std::sync::MutexGuard<'_, EventLog> {
    events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl EventLog {
    pub fn new(cap: usize) -> Self {
        Self::with_byte_cap(cap, EVENT_LOG_MAX_BYTES)
    }

    pub fn with_byte_cap(cap: usize, max_bytes: usize) -> Self {
        Self {
            ring: std::collections::VecDeque::new(),
            cap,
            max_bytes,
            bytes: 0,
        }
    }

    /// Record a broadcast frame, evicting the oldest beyond the count or byte cap (memory stays
    /// bounded no matter how long the session runs). The newest frame is always kept, even when it
    /// alone exceeds the byte cap.
    pub fn push(&mut self, rev: u64, snap: Arc<SnapshotFrame>) {
        self.bytes += snap.retained_bytes();
        self.ring.push_back((rev, snap));
        while self.ring.len() > self.cap || (self.bytes > self.max_bytes && self.ring.len() > 1) {
            if let Some((_, evicted)) = self.ring.pop_front() {
                self.bytes -= evicted.retained_bytes();
            }
        }
    }

    /// Every retained snapshot with `revision > since`, oldest first — `Some(vec![])` when the
    /// client is already current. `None` when the gap can't be filled faithfully (the log is
    /// empty, `since` predates the oldest retained entry, or `since` is from a future/foreign
    /// counter): the caller must then resync with one full snapshot instead of replaying a hole.
    pub fn replay_after(&self, since: u64) -> Option<Vec<Arc<SnapshotFrame>>> {
        let (front, _) = self.ring.front()?;
        let (back, _) = self.ring.back()?;
        if since.checked_add(1).is_none_or(|next| next < *front) || since > *back {
            return None;
        }
        Some(
            self.ring
                .iter()
                .filter(|(rev, _)| *rev > since)
                .map(|(_, s)| s.clone())
                .collect(),
        )
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.ring.len()
    }

    #[cfg(test)]
    pub(super) fn retained_bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(rev: u64, tail_bytes: usize) -> Arc<SnapshotFrame> {
        let snapshot = Snapshot {
            revision: rev,
            streaming: "x".repeat(tail_bytes),
            ..Snapshot::default()
        };
        Arc::new(SnapshotFrame::new(snapshot))
    }

    #[test]
    fn large_frames_cannot_grow_the_log_past_its_byte_cap() {
        let cap = 64 * 1024;
        let mut log = EventLog::with_byte_cap(EVENT_LOG_CAP, cap);
        for rev in 1..=400 {
            log.push(rev, frame(rev, 20 * 1024));
            assert!(
                log.retained_bytes() <= cap + 2 * 21 * 1024,
                "retained {} bytes after {rev} frames",
                log.retained_bytes()
            );
        }
        assert!(log.len() < 8, "kept {} frames of ~40 KiB", log.len());
        let (newest, _) = log.ring.back().unwrap();
        assert_eq!(*newest, 400, "the newest frame survives eviction");
        assert!(
            log.replay_after(1).is_none(),
            "an evicted gap forces a resync"
        );
        assert!(log.replay_after(399).is_some_and(|f| f.len() == 1));
    }

    #[test]
    fn one_oversized_frame_is_still_kept() {
        let mut log = EventLog::with_byte_cap(8, 1024);
        log.push(1, frame(1, 64 * 1024));
        assert_eq!(log.len(), 1);
        log.push(2, frame(2, 64 * 1024));
        assert_eq!(log.len(), 1);
        assert_eq!(
            log.retained_bytes(),
            log.ring.back().unwrap().1.retained_bytes()
        );
    }

    #[test]
    fn small_frames_stay_bound_by_the_count_cap() {
        let mut log = EventLog::new(16);
        for rev in 1..=100 {
            log.push(rev, frame(rev, 8));
        }
        assert_eq!(log.len(), 16);
    }
}
