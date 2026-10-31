//! Rendering of a kernel cell for the model: the status header first (so `summarize()` and the
//! tool cards show it), then the notice, the cell output, the displayed value and the traceback.

use serde_json::Value;

use crate::shell::output::{strip_ansi, truncate_for_model};
use crate::ToolRun;

pub(super) const RESULT_MAX_CHARS: usize = 16_000;
pub(super) const CAPTURE_HEAD: usize = 64 * 1024;
pub(super) const CAPTURE_TAIL: usize = 64 * 1024;

/// Head plus tail of one cell's raw output, so a runaway print cannot grow without bound.
#[derive(Default)]
pub(super) struct HeadTail {
    head: Vec<u8>,
    tail: Vec<u8>,
    total: usize,
}

impl HeadTail {
    pub(super) fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len();
        let mut rest = bytes;
        if self.head.len() < CAPTURE_HEAD {
            let take = (CAPTURE_HEAD - self.head.len()).min(rest.len());
            self.head.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
        }
        if rest.is_empty() {
            return;
        }
        self.tail.extend_from_slice(rest);
        if self.tail.len() > CAPTURE_TAIL {
            let drop = self.tail.len() - CAPTURE_TAIL;
            self.tail.drain(..drop);
        }
    }

    /// The captured text and whether bytes were dropped between head and tail.
    pub(super) fn into_text(self) -> (String, bool) {
        let dropped = self.total - self.head.len() - self.tail.len();
        let mut text = String::from_utf8_lossy(&self.head).into_owned();
        if dropped > 0 {
            text.push_str(&format!("\n… {dropped} bytes omitted …\n"));
        }
        text.push_str(&String::from_utf8_lossy(&self.tail));
        (text, dropped > 0)
    }
}

/// A line's visible text is what follows its last carriage return, so progress bars that redraw
/// in place (tqdm, pip) collapse to their final state.
pub(super) fn collapse_cr(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            line.rsplit('\r').next().unwrap_or(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn clean_output(raw: &str) -> String {
    collapse_cr(&strip_ansi(raw))
}

pub(super) fn ok_header(cell: u64, elapsed_ms: u64, bang_exit: Option<i64>) -> String {
    let secs = format!("{:.1}s", elapsed_ms as f64 / 1000.0);
    match bang_exit {
        Some(code) => format!("python: ok (cell {cell}, {secs}; a ! command exited {code})"),
        None => format!("python: ok (cell {cell}, {secs})"),
    }
}

pub(super) fn error_header(cell: u64, ename: &str) -> String {
    format!("python: error {ename} (cell {cell})")
}

pub(super) fn interrupted_header(secs: u64) -> String {
    format!("python: interrupted after {secs}s; kernel state kept")
}

pub(super) const KILLED_HEADER: &str = "python: kernel killed after ignoring interrupt; state lost";

pub(super) fn died_header(status: &str) -> String {
    format!("python: kernel died ({status}); state lost")
}

pub(super) fn failed_header(tried: &[String], detail: &str) -> String {
    format!(
        "python: failed to start (tried {}): {detail}; use shell",
        tried.join(", ")
    )
}

pub(super) struct Rendered {
    pub header: String,
    pub notice: Option<String>,
    pub output: String,
    pub value: Option<String>,
    pub traceback: Option<String>,
    pub capture_truncated: bool,
}

/// Assemble the model copy (body head-plus-tail truncated) and, when anything was cut, the
/// untruncated text for `/output`.
pub(super) fn assemble(r: Rendered) -> ToolRun {
    let mut body = String::new();
    if let Some(notice) = &r.notice {
        body.push_str(&format!("[{notice}]\n"));
    }
    let output = clean_output(&r.output);
    if !output.trim().is_empty() {
        body.push_str(output.trim_end());
        body.push('\n');
    }
    if let Some(value) = &r.value {
        body.push_str(&format!("Out: {value}\n"));
    }
    if let Some(tb) = &r.traceback {
        body.push_str(strip_ansi(tb).trim_end());
        body.push('\n');
    }
    let body = body.trim_end().to_string();
    let (model_body, cut) = truncate_for_model(&body, RESULT_MAX_CHARS);
    let join = |b: &str| {
        if b.is_empty() {
            r.header.clone()
        } else {
            format!("{}\n{b}", r.header)
        }
    };
    ToolRun {
        model: join(&model_body),
        full: (cut || r.capture_truncated).then(|| join(&body)),
    }
}

pub(super) fn reply_str<'a>(reply: &'a Value, key: &str) -> Option<&'a str> {
    reply.get(key).and_then(Value::as_str)
}
