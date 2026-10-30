//! Run an interactive CLI flow (a browser sign-in) inside a pseudo-terminal, so the person never
//! has to leave onboarding: the sign-in URL and any device code become job prompts, a code the
//! page shows can be typed back, and a token the CLI prints is captured without ever being
//! displayed. The same job events serve the terminal screen and the desktop app via the daemon.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::job::{JobHandle, PromptKind};

/// What a Claude Code long-lived token starts with.
const TOKEN_PREFIX: &str = "sk-ant-oat";
/// Shorter than this is prose, not a token.
const TOKEN_MIN_LEN: usize = 30;

/// One thing found in the child's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scan {
    /// A line for the log, with any token already replaced by `***`.
    Line(String),
    Url(String),
    Token(String),
    /// The flow is waiting for a pasted code or redirect URL.
    AskCode(String),
}

/// Turns raw PTY bytes into [`Scan`]s: strips terminal escapes, splits lines, and spots URLs,
/// tokens and "paste the code" prompts, including on a prompt line that never gets a newline.
#[derive(Default)]
pub(crate) struct Scanner {
    pending: Vec<u8>,
    seen_urls: HashSet<String>,
    tail_asked: bool,
}

impl Scanner {
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<Scan> {
        let mut out = Vec::new();
        for &b in bytes {
            if b == b'\n' || b == b'\r' {
                let line = std::mem::take(&mut self.pending);
                self.tail_asked = false;
                out.extend(self.line(&String::from_utf8_lossy(&line)));
            } else {
                self.pending.push(b);
            }
        }
        if !self.tail_asked && !self.pending.is_empty() {
            let tail = strip_ansi(&String::from_utf8_lossy(&self.pending));
            if asks_for_code(&tail) {
                self.tail_asked = true;
                out.push(Scan::AskCode(tail.trim().to_string()));
            }
        }
        out
    }

    /// The child is gone: whatever is left is a last line.
    pub(crate) fn finish(&mut self) -> Vec<Scan> {
        let line = std::mem::take(&mut self.pending);
        self.tail_asked = false;
        self.line(&String::from_utf8_lossy(&line))
    }

    fn line(&mut self, raw: &str) -> Vec<Scan> {
        let clean = strip_ansi(raw);
        let clean = clean.trim();
        if clean.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut shown = clean.to_string();
        if let Some(token) = find_token(clean) {
            shown = shown.replace(&token, "***");
            out.push(Scan::Token(token));
        }
        for url in find_urls(&shown) {
            if self.seen_urls.insert(url.clone()) {
                out.push(Scan::Url(url));
            }
        }
        if asks_for_code(&shown) {
            out.push(Scan::AskCode(shown.clone()));
        }
        out.push(Scan::Line(shown));
        out
    }
}

fn asks_for_code(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("paste") && (lower.contains("code") || lower.contains("url"))
}

/// Remove CSI and OSC escape sequences and other control characters.
pub(crate) fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            if !c.is_control() || c == '\t' {
                out.push(c);
            }
            continue;
        }
        match chars.next() {
            Some('[') => {
                let mut params = String::new();
                for n in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        // Full-screen CLIs (Claude Code's Ink UI) move the cursor right instead
                        // of printing spaces; keep the gap or words run together.
                        if n == 'C' {
                            let gap = params.parse::<usize>().unwrap_or(1).clamp(1, 512);
                            out.push_str(&" ".repeat(gap));
                        }
                        break;
                    }
                    params.push(n);
                }
            }
            Some(']') => {
                while let Some(n) = chars.next() {
                    if n == '\u{7}' {
                        break;
                    }
                    if n == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn find_token(text: &str) -> Option<String> {
    let start = text.find(TOKEN_PREFIX)?;
    let token: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (token.len() >= TOKEN_MIN_LEN).then_some(token)
}

fn find_urls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("https://") {
        let tail = &rest[i..];
        let end = tail
            .find(|c: char| {
                c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '<' | '>')
            })
            .unwrap_or(tail.len());
        let url = tail[..end].trim_end_matches(['.', ',', ';', ':', ')', ']']);
        if url.len() > "https://".len() {
            out.push(url.to_string());
        }
        rest = &tail[end..];
    }
    out
}

const ENTER_DELAY: Duration = Duration::from_millis(300);

pub(crate) struct PtySpec<'a> {
    pub program: &'a str,
    pub args: &'a [String],
    pub timeout: Duration,
    pub env_remove: &'a [&'a str],
}

pub(crate) struct PtyRun {
    pub success: bool,
    /// A token the child printed. Already registered as a job secret.
    pub token: Option<String>,
}

/// Run `spec` in a PTY, reporting its output, URLs and prompts on `job`.
pub(crate) fn run_pty(job: &JobHandle, spec: PtySpec<'_>) -> Result<PtyRun, String> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 50,
            // Wide enough that a long sign-in URL is never wrapped across lines.
            cols: 1000,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("could not open a terminal for the sign-in: {e}"))?;
    let mut cmd = CommandBuilder::new(spec.program);
    cmd.args(spec.args);
    for var in spec.env_remove {
        cmd.env_remove(var);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("NO_COLOR", "1");
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("could not start {}: {e}", spec.program))?;
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("could not read the sign-in terminal: {e}"))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("could not write to the sign-in terminal: {e}"))?;

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let mut scanner = Scanner::default();
    let mut token = None;
    let started = Instant::now();
    let mut exited = None;
    let handle_scans = |scans: Vec<Scan>, token: &mut Option<String>| {
        for scan in scans {
            match scan {
                Scan::Token(t) => {
                    job.add_secret(t.clone());
                    *token = Some(t);
                }
                Scan::Url(url) => {
                    job.prompt(PromptKind::OpenUrl { url }, "Open this page to sign in.")
                }
                Scan::AskCode(line) => job.prompt(
                    PromptKind::Code {
                        label: "Paste the code from your browser".into(),
                    },
                    line,
                ),
                Scan::Line(line) => job.output(line),
            }
        }
    };

    let mut enter_at: Option<Instant> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => handle_scans(scanner.feed(&bytes), &mut token),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if let Some(text) = job.try_input() {
            job.add_secret(text.clone());
            let _ = writer.write_all(text.as_bytes());
            let _ = writer.flush();
            enter_at = Some(Instant::now() + ENTER_DELAY);
        }
        // Enter goes in its own write, after a pause: Ink (Claude Code's UI) reads a multi-byte chunk
        // as pasted text, so a trailing "\r" in the same chunk never submits the field.
        if enter_at.is_some_and(|at| Instant::now() >= at) {
            enter_at = None;
            let _ = writer.write_all(b"\r");
            let _ = writer.flush();
        }
        if job.cancelled() || started.elapsed() > spec.timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if job.cancelled() {
                "Cancelled.".to_string()
            } else {
                "The sign-in took too long and was stopped.".to_string()
            });
        }
        if exited.is_none() {
            if let Ok(Some(status)) = child.try_wait() {
                exited = Some(status.success());
            }
        }
        if exited.is_some() {
            // Drain what the child wrote just before it left.
            while let Ok(bytes) = rx.recv_timeout(Duration::from_millis(200)) {
                handle_scans(scanner.feed(&bytes), &mut token);
            }
            break;
        }
    }
    handle_scans(scanner.finish(), &mut token);
    let success = match exited {
        Some(ok) => ok,
        None => child.wait().map(|s| s.success()).unwrap_or(false),
    };
    Ok(PtyRun { success, token })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "sk-ant-oat01-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-abcdef";

    fn transcript() -> Vec<u8> {
        "\u{1b}[2J\u{1b}[HOpening browser to sign in\u{2026}\r\n\
             If it did not open, visit:\r\n\
             \u{1b}]8;;x\u{7}https://claude.ai/oauth/authorize?code=true&client_id=abc&state=xyz\u{1b}]8;;\u{7}\r\n\
             Paste code here if prompted > \u{1b}[?25h"
            .as_bytes()
            .to_vec()
    }

    #[test]
    fn cursor_forward_moves_become_spaces() {
        assert_eq!(
            strip_ansi("Paste\u{1b}[1Ccode\u{1b}[Chere\u{1b}[3Cif"),
            "Paste code here   if"
        );
    }

    #[test]
    fn escapes_are_stripped_and_lines_split() {
        assert_eq!(
            strip_ansi("\u{1b}[1;31mred\u{1b}[0m ok\u{1b}]0;title\u{7}"),
            "red ok"
        );
    }

    #[test]
    fn the_sign_in_url_and_the_code_prompt_are_found_even_without_a_trailing_newline() {
        let mut s = Scanner::default();
        let scans = s.feed(&transcript());
        assert!(scans.contains(&Scan::Url(
            "https://claude.ai/oauth/authorize?code=true&client_id=abc&state=xyz".into()
        )));
        assert!(
            scans
                .iter()
                .any(|x| matches!(x, Scan::AskCode(l) if l.contains("Paste code here"))),
            "{scans:?}"
        );
        let again = s.feed(b"");
        assert!(
            again.is_empty(),
            "the unterminated prompt is announced once"
        );
    }

    #[test]
    fn a_urls_is_reported_once_per_distinct_address() {
        let mut s = Scanner::default();
        let first = s.feed(b"visit https://a.test/x now\r\n");
        let second = s.feed(b"visit https://a.test/x now\r\n");
        assert_eq!(
            first.iter().filter(|x| matches!(x, Scan::Url(_))).count(),
            1
        );
        assert_eq!(
            second.iter().filter(|x| matches!(x, Scan::Url(_))).count(),
            0
        );
    }

    #[test]
    fn the_token_is_captured_and_never_appears_in_a_line() {
        let mut s = Scanner::default();
        let mut all = s.feed(&transcript());
        all.extend(s.feed(format!("Your token:\r\n  {TOKEN}\r\nStore it safely.\r\n").as_bytes()));
        all.extend(s.finish());
        assert!(all.contains(&Scan::Token(TOKEN.to_string())));
        for scan in &all {
            if let Scan::Line(l) | Scan::AskCode(l) = scan {
                assert!(!l.contains("sk-ant-oat"), "token leaked into a line: {l}");
            }
            if let Scan::Url(u) = scan {
                assert!(!u.contains("sk-ant-oat"));
            }
        }
        assert!(all.iter().any(|x| matches!(x, Scan::Line(l) if l == "***")));
    }

    #[test]
    fn a_token_split_across_reads_is_still_captured() {
        let mut s = Scanner::default();
        let (a, b) = TOKEN.split_at(20);
        let mut all = s.feed(format!("token: {a}").as_bytes());
        all.extend(s.feed(format!("{b}\r\n").as_bytes()));
        assert!(all.contains(&Scan::Token(TOKEN.to_string())));
    }

    #[test]
    fn a_token_on_the_last_line_without_a_newline_is_flushed_on_finish() {
        let mut s = Scanner::default();
        let mut all = s.feed(TOKEN.as_bytes());
        all.extend(s.finish());
        assert!(all.contains(&Scan::Token(TOKEN.to_string())));
    }

    #[test]
    fn prose_that_mentions_the_prefix_is_not_a_token() {
        let mut s = Scanner::default();
        let scans = s.feed(b"tokens look like sk-ant-oat01-... in docs\r\n");
        assert!(!scans.iter().any(|x| matches!(x, Scan::Token(_))));
    }

    #[test]
    fn multibyte_text_split_across_reads_survives() {
        let mut s = Scanner::default();
        let bytes = "caf\u{e9} ready\r\n".as_bytes();
        let mut all = s.feed(&bytes[..4]);
        all.extend(s.feed(&bytes[4..]));
        assert!(all.contains(&Scan::Line("caf\u{e9} ready".into())));
    }
}
