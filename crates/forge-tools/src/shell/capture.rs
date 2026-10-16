//! Bounded capture of a child process stream.

use super::*;

/// Read an async stream, keeping at most [`CAPTURE_CAP`] bytes: the first half and the LAST half
/// (a ring), so a long run's final lines — the totals, the error — survive. Returns the bytes and
/// whether anything in the middle was dropped.
pub(super) async fn read_capped<R: AsyncRead + Unpin>(mut r: R) -> (Vec<u8>, bool) {
    const HEAD: usize = CAPTURE_CAP / 2;
    const TAIL: usize = CAPTURE_CAP - HEAD;
    let mut head = Vec::new();
    let mut tail: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
    let mut dropped = 0usize;
    let mut chunk = [0u8; 8192];
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut data = &chunk[..n];
                if head.len() < HEAD {
                    let take = data.len().min(HEAD - head.len());
                    head.extend_from_slice(&data[..take]);
                    data = &data[take..];
                }
                tail.extend(data.iter().copied());
                if tail.len() > TAIL {
                    let excess = tail.len() - TAIL;
                    tail.drain(..excess);
                    dropped += excess;
                }
            }
        }
    }
    if dropped == 0 {
        head.extend(tail);
        return (head, false);
    }
    // Start the kept tail on a line boundary when one is near, so it doesn't open mid-line.
    let tail: Vec<u8> = tail.into_iter().collect();
    let skip = tail
        .iter()
        .take(512)
        .position(|b| *b == b'\n')
        .map_or(0, |i| i + 1);
    head.extend_from_slice(format!("\n[… {dropped} bytes omitted …]\n").as_bytes());
    head.extend_from_slice(&tail[skip..]);
    (head, true)
}
