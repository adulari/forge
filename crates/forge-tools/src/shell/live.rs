//! A bounded rolling tail of a running foreground command's output, for UI surfaces only.
//!
//! The session scopes a [`LiveTail`] around a shell call; the stdout/stderr readers feed it as
//! bytes arrive and the session samples it. Nothing here reaches the model-facing result.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Lines kept (the in-progress line counts as one).
pub const LIVE_TAIL_LINES: usize = 10;
/// Chars kept per line; a longer line keeps its end, where progress and errors are.
const LINE_CHARS: usize = 512;
/// Bytes held for the line still being written (a `\r`-less progress stream could be endless).
const PENDING_CAP: usize = 64 * 1024;

#[derive(Default)]
struct State {
    lines: VecDeque<String>,
    pending: Vec<u8>,
    version: u64,
}

/// Shared handle: clone it into the readers, sample it from the session.
#[derive(Clone, Default)]
pub struct LiveTail(Arc<Mutex<State>>);

tokio::task_local! {
    static LIVE_TAIL: LiveTail;
}

/// Run `fut` so every foreground command it starts feeds `tail`.
pub async fn scope_live_tail<F: std::future::Future>(tail: LiveTail, fut: F) -> F::Output {
    LIVE_TAIL.scope(tail, fut).await
}

pub(super) fn current() -> Option<LiveTail> {
    LIVE_TAIL.try_with(Clone::clone).ok()
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn finish_line(bytes: &[u8]) -> String {
    let text = if bytes.contains(&0) {
        format!("<binary output: {} bytes, not shown>", bytes.len())
    } else {
        super::output::strip_ansi(&String::from_utf8_lossy(bytes))
    };
    let text = text.trim_end_matches('\r');
    let count = text.chars().count();
    if count > LINE_CHARS {
        text.chars().skip(count - LINE_CHARS).collect()
    } else {
        text.to_string()
    }
}

impl LiveTail {
    pub fn push(&self, data: &[u8]) {
        let mut state = lock(&self.0);
        for &byte in data {
            match byte {
                b'\n' => {
                    let line = finish_line(&state.pending);
                    state.pending.clear();
                    state.lines.push_back(line);
                    if state.lines.len() > LIVE_TAIL_LINES {
                        state.lines.pop_front();
                    }
                }
                // A bare carriage return redraws the line (progress bars): keep only the latest.
                b'\r' => state.pending.clear(),
                _ => {
                    if state.pending.len() >= PENDING_CAP {
                        state.pending.drain(..PENDING_CAP / 2);
                    }
                    state.pending.push(byte);
                }
            }
        }
        state.version += 1;
    }

    /// The last [`LIVE_TAIL_LINES`] lines (the line still being written last, if any) and a
    /// counter that changes whenever output arrived.
    pub fn sample(&self) -> (Vec<String>, u64) {
        let state = lock(&self.0);
        let mut lines: Vec<String> = state.lines.iter().cloned().collect();
        if !state.pending.is_empty() {
            let line = finish_line(&state.pending);
            if !line.is_empty() {
                lines.push(line);
            }
        }
        let skip = lines.len().saturating_sub(LIVE_TAIL_LINES);
        (lines.split_off(skip), state.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_last_lines_and_strips_ansi() {
        let tail = LiveTail::default();
        for i in 0..40 {
            tail.push(format!("\x1b[32mline {i}\x1b[0m\n").as_bytes());
        }
        tail.push(b"partial");
        let (lines, _) = tail.sample();
        assert_eq!(lines.len(), LIVE_TAIL_LINES);
        assert_eq!(lines.first().unwrap(), "line 31");
        assert_eq!(lines.last().unwrap(), "partial");
    }

    #[test]
    fn carriage_return_redraws_and_memory_is_bounded() {
        let tail = LiveTail::default();
        tail.push(b"10%\r50%\r99%");
        assert_eq!(tail.sample().0, vec!["99%".to_string()]);
        tail.push(&vec![b'x'; 10 * PENDING_CAP]);
        let state = lock(&tail.0);
        assert!(state.pending.len() <= PENDING_CAP);
    }

    #[test]
    fn splits_multibyte_chars_across_chunks_losslessly() {
        let tail = LiveTail::default();
        let bytes = "héllo\n".as_bytes();
        tail.push(&bytes[..2]);
        tail.push(&bytes[2..]);
        assert_eq!(tail.sample().0, vec!["héllo".to_string()]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_running_command_feeds_the_tail_without_changing_its_result() {
        let command = "echo one; echo two >&2; echo three";
        let policy = crate::SandboxPolicy::default();
        let strip = |text: String| {
            text.split_once(" in ")
                .map(|(_, rest)| rest.split_once("ms").unwrap().1.to_string())
                .unwrap()
        };
        let plain = strip(super::super::run_command(command, ".", 10, &policy).await);
        let tail = LiveTail::default();
        let streamed = scope_live_tail(
            tail.clone(),
            super::super::run_command(command, ".", 10, &policy),
        )
        .await;
        assert_eq!(strip(streamed), plain);
        let (lines, version) = tail.sample();
        assert!(version > 0);
        for want in ["one", "two", "three"] {
            assert!(lines.iter().any(|l| l == want), "{lines:?}");
        }
    }
}
