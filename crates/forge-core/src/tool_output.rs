//! A tool call's complete output, kept where a person can open it.
//!
//! The same output has two consumers that want different things. The model gets a budgeted copy
//! (the shell tool keeps 64 KB, head and tail) and the chat's tool card gets a short preview of
//! that. Neither is the whole output, and a head-only preview dropped exactly the part a person
//! opens a failing call to read: the last lines, where a test run prints its error.
//!
//! So the preview keeps both ends, and the complete output is written to a file beside the session
//! store, one per call. Surfaces are handed a reference to it ([`forge_types::ToolOutputRef`])
//! rather than the text: holding it in memory would pin every megabyte a session's shell calls
//! printed for the life of the process. An in-memory store has no directory, so tests never spool.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use forge_types::{ToolOutputRef, FULL_OUTPUT_HINT};

use super::*;

const PREVIEW_HEAD_LINES: usize = 120;
const PREVIEW_TAIL_LINES: usize = 60;
const PREVIEW_MAX_CHARS: usize = 8_000;
/// Kept outputs older than this are removed the first time a process keeps anything.
const RETAIN_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The bounded preview a tool card shows inline: the start AND the end of the output, with the
/// middle folded into a marker that points at the full view. `None` when there is nothing beyond
/// the summary line.
pub(crate) fn tool_detail(result: &str) -> Option<String> {
    let trimmed = result.trim_end();
    if trimmed.is_empty() || trimmed == summarize(result) {
        return None;
    }
    if !exceeds_preview(trimmed) {
        return Some(trimmed.to_string());
    }
    let lines: Vec<&str> = trimmed.lines().collect();
    let head = take_within(
        lines.iter().copied(),
        PREVIEW_HEAD_LINES,
        PREVIEW_MAX_CHARS * 2 / 3,
    );
    let rest = &lines[head.len()..];
    let mut tail = take_within(
        rest.iter().rev().copied(),
        PREVIEW_TAIL_LINES,
        PREVIEW_MAX_CHARS / 3,
    );
    tail.reverse();
    let hidden = lines.len() - head.len() - tail.len();
    let marker = match hidden {
        0 => format!("… long lines shortened · {FULL_OUTPUT_HINT}"),
        1 => format!("… 1 line hidden · {FULL_OUTPUT_HINT}"),
        n => format!("… {n} lines hidden · {FULL_OUTPUT_HINT}"),
    };
    // Lines hidden from the middle are marked where they were. Lines that were only shortened are
    // marked after everything, so the marker never splits output that is all still there.
    let mut parts = head;
    if hidden > 0 {
        parts.push(marker);
        parts.extend(tail);
    } else {
        parts.extend(tail);
        parts.push(marker);
    }
    Some(parts.join("\n"))
}

fn exceeds_preview(text: &str) -> bool {
    text.lines().count() > PREVIEW_HEAD_LINES + PREVIEW_TAIL_LINES
        || text.chars().count() > PREVIEW_MAX_CHARS
}

/// Whole lines from `lines` until either budget runs out. A single line longer than the whole
/// character budget is cut rather than skipped, so one enormous line still previews as something.
fn take_within<'a>(
    lines: impl Iterator<Item = &'a str>,
    max_lines: usize,
    max_chars: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for line in lines.take(max_lines) {
        let len = line.chars().count();
        if used + len <= max_chars {
            used += len;
            out.push(line.to_string());
        } else {
            if out.is_empty() {
                let cut: String = line.chars().take(max_chars).collect();
                out.push(format!("{cut}…"));
            }
            break;
        }
    }
    out
}

impl Session {
    /// Keep a call's complete output for the full-output view and tell the surfaces where it is.
    /// `full` is the tool's uncut output when the model's copy was cut (the shell tool); otherwise
    /// the result itself is kept, but only when the inline preview cannot show all of it.
    pub(crate) fn surface_full_output(
        &mut self,
        name: &str,
        call_id: &str,
        full: Option<&str>,
        result: &str,
    ) {
        let text = match full {
            Some(full) => full,
            None if exceeds_preview(result.trim_end()) => result,
            None => return,
        };
        let Some(root) = self
            .store
            .db_path()
            .and_then(Path::parent)
            .map(|dir| dir.join("tool-output"))
        else {
            return;
        };
        prune_expired_once(&root);
        match spool(&root, &self.id, call_id, text) {
            Ok(output) => self.presenter.emit(PresenterEvent::ToolOutput {
                name: name.to_string(),
                output,
            }),
            Err(error) => {
                tracing::warn!(%error, tool = name, "could not keep the full tool output")
            }
        }
    }
}

fn spool(
    root: &Path,
    session_id: &str,
    call_id: &str,
    text: &str,
) -> std::io::Result<ToolOutputRef> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = root.join(file_safe(session_id));
    std::fs::create_dir_all(&dir)?;
    // Providers reuse call ids (some send `call_0` on every turn), so the id alone would let a
    // later call overwrite the output an earlier card points at.
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("{stamp:x}-{seq}-{}.log", file_safe(call_id)));
    std::fs::write(&path, text)?;
    Ok(ToolOutputRef {
        path: path.display().to_string(),
        lines: text.lines().count(),
        bytes: text.len(),
    })
}

/// A path component that cannot climb out of the spool directory, whatever the provider sent.
fn file_safe(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .take(96)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "call".to_string()
    } else {
        cleaned
    }
}

fn prune_expired_once(root: &Path) {
    static PRUNED: OnceLock<()> = OnceLock::new();
    if PRUNED.set(()).is_err() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let expired = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|modified| is_expired(modified, now));
        if expired {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn is_expired(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified)
        .is_ok_and(|age| age > RETAIN_FOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(n: usize) -> String {
        (0..n)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn nothing_beyond_the_summary_has_no_preview() {
        assert_eq!(tool_detail(""), None);
        assert_eq!(tool_detail("one line"), None);
        let two = tool_detail("first\nsecond").expect("multi-line output is worth keeping");
        assert_eq!(two, "first\nsecond");
    }

    #[test]
    fn output_that_fits_is_previewed_whole() {
        let small = numbered(PREVIEW_HEAD_LINES + PREVIEW_TAIL_LINES);
        assert_eq!(tool_detail(&small).as_deref(), Some(small.as_str()));
    }

    #[test]
    fn a_long_output_keeps_its_start_and_its_end() {
        // A failing test run prints its error LAST. The old head-only preview dropped exactly that.
        let preview = tool_detail(&numbered(5_000)).unwrap();
        assert!(preview.starts_with("line 0\n"), "{preview}");
        assert!(preview.ends_with("line 4999"), "the tail survives");
        assert!(preview.contains("lines hidden · full output: /output"));
        assert!(preview.lines().count() <= PREVIEW_HEAD_LINES + PREVIEW_TAIL_LINES + 1);
        assert!(preview.chars().count() <= PREVIEW_MAX_CHARS + 100);
    }

    #[test]
    fn one_enormous_line_is_shortened_rather_than_dropped() {
        let wide = format!("head\n{}", "x".repeat(50_000));
        let preview = tool_detail(&wide).unwrap();
        assert!(preview.starts_with("head\nxxx"), "{}", &preview[..40]);
        assert!(preview.chars().count() <= PREVIEW_MAX_CHARS + 100);
    }

    #[test]
    fn a_reused_call_id_never_overwrites_an_earlier_calls_output() {
        let root = std::env::temp_dir().join(format!(
            "forge-tool-output-test-{}-{}",
            std::process::id(),
            SEQ_FOR_TESTS.fetch_add(1, Ordering::Relaxed)
        ));
        let first = spool(&root, "session", "call_0", "first").unwrap();
        let second = spool(&root, "session", "call_0", "second\nrun").unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(std::fs::read_to_string(&first.path).unwrap(), "first");
        assert_eq!((second.lines, second.bytes), (2, 10));
        let _ = std::fs::remove_dir_all(&root);
    }

    static SEQ_FOR_TESTS: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn an_id_cannot_climb_out_of_the_spool_directory() {
        assert_eq!(file_safe("../../etc/passwd"), "______etc_passwd");
        assert_eq!(file_safe(""), "call");
    }

    #[test]
    fn only_old_outputs_expire() {
        let now = SystemTime::now();
        assert!(!is_expired(now - Duration::from_secs(60), now));
        assert!(is_expired(now - RETAIN_FOR - Duration::from_secs(1), now));
    }
}
