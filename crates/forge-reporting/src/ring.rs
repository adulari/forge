//! A short ring buffer of what the harness was just doing: event kinds and durations only. Nothing
//! passed to [`event`] can carry content because the kind is reduced to a lowercase slug.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};

const CAPACITY: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEntry {
    pub k: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    pub ago: u64,
}

static RING: Mutex<VecDeque<(Instant, String, Option<u64>)>> = Mutex::new(VecDeque::new());

fn kind_slug(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-'))
        .take(32)
        .collect()
}

/// Record that something happened (`turn_start`, `tool_call`, `compaction`...) and optionally how
/// long it took. Cheap and lock-light; safe on hot paths.
pub fn event(kind: &str, ms: Option<u64>) {
    let slug = kind_slug(kind);
    if slug.is_empty() {
        return;
    }
    let Ok(mut ring) = RING.lock() else { return };
    if ring.len() == CAPACITY {
        ring.pop_front();
    }
    ring.push_back((Instant::now(), slug, ms));
}

pub fn snapshot() -> Vec<EventEntry> {
    let now = Instant::now();
    let Ok(ring) = RING.lock() else {
        return Vec::new();
    };
    ring.iter()
        .map(|(at, k, ms)| EventEntry {
            k: k.clone(),
            ms: *ms,
            ago: now.duration_since(*at).as_secs(),
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn clear() {
    if let Ok(mut ring) = RING.lock() {
        ring.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_slugs_and_is_bounded() {
        assert_eq!(
            kind_slug("turn start /home/alice/secret.txt"),
            "turnstarthomealicesecret.txt"
        );
        assert_eq!(kind_slug("x".repeat(80).as_str()).len(), 32);
        clear();
        for _ in 0..100 {
            event("tool_call", Some(3));
        }
        let snap = snapshot();
        assert!(snap.len() <= CAPACITY);
        assert!(snap
            .iter()
            .all(|e| !e.k.contains('/') && !e.k.contains(' ')));
    }
}
