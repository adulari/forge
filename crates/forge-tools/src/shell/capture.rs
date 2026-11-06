//! Bounded capture of a child process stream.

use super::*;

/// The bytes read so far: at most [`CAPTURE_CAP`], the first half and the LAST half (a ring), so
/// a long run's final lines — the totals, the error — survive.
#[derive(Default)]
pub(super) struct Captured {
    head: Vec<u8>,
    tail: std::collections::VecDeque<u8>,
    dropped: usize,
}

/// Shared with the reader task, so whatever arrived before a drain timeout is still there to show.
pub(super) type SharedCapture = std::sync::Arc<std::sync::Mutex<Captured>>;

const HEAD: usize = CAPTURE_CAP / 2;
const TAIL: usize = CAPTURE_CAP - HEAD;

impl Captured {
    pub(super) fn push(&mut self, mut data: &[u8]) {
        if self.head.len() < HEAD {
            let take = data.len().min(HEAD - self.head.len());
            self.head.extend_from_slice(&data[..take]);
            data = &data[take..];
        }
        self.tail.extend(data.iter().copied());
        if self.tail.len() > TAIL {
            let excess = self.tail.len() - TAIL;
            self.tail.drain(..excess);
            self.dropped += excess;
        }
    }

    /// The captured bytes and whether anything in the middle was dropped.
    pub(super) fn snapshot(&self) -> (Vec<u8>, bool) {
        let mut out = self.head.clone();
        if self.dropped == 0 {
            out.extend(self.tail.iter().copied());
            return (out, false);
        }
        // Start the kept tail on a line boundary when one is near, so it doesn't open mid-line.
        let skip = self
            .tail
            .iter()
            .take(512)
            .position(|b| *b == b'\n')
            .map_or(0, |i| i + 1);
        out.extend_from_slice(format!("\n[… {} bytes omitted …]\n", self.dropped).as_bytes());
        out.extend(self.tail.iter().skip(skip).copied());
        (out, true)
    }
}

pub(super) fn lock(capture: &SharedCapture) -> std::sync::MutexGuard<'_, Captured> {
    capture
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Read an async stream to EOF into `into`.
pub(super) async fn read_capped<R: AsyncRead + Unpin>(
    mut r: R,
    into: SharedCapture,
    live: Option<super::live::LiveTail>,
) {
    let mut chunk = [0u8; 8192];
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                lock(&into).push(&chunk[..n]);
                if let Some(live) = &live {
                    live.push(&chunk[..n]);
                }
            }
        }
    }
}
