//! Full-screen PTY smoke for folded tool groups: drive `forge chat --mock` through the
//! `mock:tools` scenario, click a group's summary row with a real SGR mouse report, and read the
//! screen back through a small terminal model (cursor moves + printed cells), because the
//! alternate screen is redrawn in place and a byte stream alone cannot say what is on it.
//!
//! Run locally: `cargo test -p forge-agent --test tui_fold_e2e -- --ignored --nocapture`

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

const COLS: usize = 110;
const ROWS: usize = 34;

/// The visible grid of a terminal fed the raw output: enough of VT100 for ratatui's crossterm
/// backend (absolute cursor moves, erase, printed text; colours and modes are ignored).
struct Screen {
    cells: Vec<Vec<char>>,
    row: usize,
    col: usize,
    /// The tail of the last read when it ended inside an escape sequence or a UTF-8 character:
    /// a read boundary can fall anywhere, and feeding half a sequence prints its bytes.
    pending: Vec<u8>,
}

impl Screen {
    fn new() -> Self {
        Self {
            cells: vec![vec![' '; COLS]; ROWS],
            row: 0,
            col: 0,
            pending: Vec::new(),
        }
    }

    /// Feed raw output, holding back an incomplete trailing escape or UTF-8 sequence.
    fn feed_bytes(&mut self, bytes: &[u8]) {
        let mut all = std::mem::take(&mut self.pending);
        all.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&all) {
            Ok(_) => all.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => all.len(),
        };
        let text = String::from_utf8_lossy(&all[..valid]).to_string();
        let cut = incomplete_escape(&text);
        self.pending = text.as_bytes()[cut..].to_vec();
        self.pending.extend_from_slice(&all[valid..]);
        self.feed(&text[..cut]);
    }

    fn feed(&mut self, text: &str) {
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\u{1b}' => match chars.next() {
                    Some('[') => {
                        let mut params = String::new();
                        for d in chars.by_ref() {
                            if ('@'..='~').contains(&d) {
                                self.csi(&params, d);
                                break;
                            }
                            params.push(d);
                        }
                    }
                    Some(']') => {
                        // OSC: skip to BEL or ST.
                        while let Some(d) = chars.next() {
                            if d == '\u{7}' || (d == '\u{1b}' && chars.peek() == Some(&'\\')) {
                                break;
                            }
                        }
                    }
                    _ => {}
                },
                '\r' => self.col = 0,
                '\n' => self.row = (self.row + 1).min(ROWS - 1),
                c if c.is_control() => {}
                c => {
                    if self.row < ROWS && self.col < COLS {
                        self.cells[self.row][self.col] = c;
                    }
                    self.col += 1;
                }
            }
        }
    }

    fn csi(&mut self, params: &str, op: char) {
        if params.starts_with('?') {
            return;
        }
        let nums: Vec<usize> = params.split(';').map(|p| p.parse().unwrap_or(0)).collect();
        let n =
            |i: usize, default: usize| nums.get(i).copied().filter(|v| *v > 0).unwrap_or(default);
        match op {
            'H' | 'f' => {
                self.row = n(0, 1).saturating_sub(1).min(ROWS - 1);
                self.col = n(1, 1).saturating_sub(1).min(COLS - 1);
            }
            'G' => self.col = n(0, 1).saturating_sub(1).min(COLS - 1),
            'J' if nums.first() == Some(&2) || nums.first() == Some(&3) => {
                self.cells = vec![vec![' '; COLS]; ROWS];
            }
            'K' => {
                for c in self.col..COLS {
                    self.cells[self.row][c] = ' ';
                }
            }
            _ => {}
        }
    }

    fn rows(&self) -> Vec<String> {
        self.cells.iter().map(|r| r.iter().collect()).collect()
    }

    fn text(&self) -> String {
        self.rows().join("\n")
    }
}

struct Session {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    screen: Arc<Mutex<Screen>>,
    reader: Option<thread::JoinHandle<()>>,
    dir: std::path::PathBuf,
}

impl Session {
    fn start() -> Self {
        let dir = std::env::temp_dir().join(format!("forge-fold-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("sample.rs"), "fn main() { let x = 1; }\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
        let (data, config) = (dir.join("xdg-data"), dir.join("xdg-config"));
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        let trusted = std::process::Command::new(env!("CARGO_BIN_EXE_forge"))
            .arg("trust")
            .arg(&dir)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", &config)
            .env("FORGE_DB", dir.join("forge.db"))
            .output()
            .expect("run forge trust");
        assert!(trusted.status.success(), "forge trust failed: {trusted:?}");

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS as u16,
                cols: COLS as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_forge"));
        cmd.args(["chat", "--mock", "--mode", "bypass", "--fullscreen"]);
        cmd.cwd(&dir);
        cmd.env("XDG_DATA_HOME", &data);
        cmd.env("XDG_CONFIG_HOME", &config);
        cmd.env("FORGE_DB", dir.join("forge.db"));
        cmd.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(cmd).expect("spawn forge");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("reader");
        let writer = Arc::new(Mutex::new(pair.master.take_writer().expect("writer")));
        let screen = Arc::new(Mutex::new(Screen::new()));
        let (w, s) = (Arc::clone(&writer), Arc::clone(&screen));
        let reader = thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        if chunk.contains("\u{1b}[6n") {
                            if let Ok(mut w) = w.lock() {
                                let _ = w.write_all(b"\x1b[1;1R");
                                let _ = w.flush();
                            }
                        }
                        s.lock().unwrap().feed_bytes(&buf[..n]);
                    }
                }
            }
        });
        // The master must outlive the child; leak it into the writer's lifetime.
        std::mem::forget(pair.master);
        Self {
            child,
            writer,
            screen,
            reader: Some(reader),
            dir,
        }
    }

    fn send(&self, bytes: &str) {
        let mut w = self.writer.lock().unwrap();
        let _ = w.write_all(bytes.as_bytes());
        let _ = w.flush();
    }

    /// Type a line and submit it as a separate keystroke, the way a person does.
    fn submit(&self, line: &str) {
        self.send(line);
        thread::sleep(Duration::from_millis(300));
        self.send("\r");
    }

    fn click(&self, col: usize, row: usize) {
        self.send(&format!("\x1b[<0;{};{}M", col + 1, row + 1));
        thread::sleep(Duration::from_millis(60));
        self.send(&format!("\x1b[<0;{};{}m", col + 1, row + 1));
    }

    fn rows(&self) -> Vec<String> {
        self.screen.lock().unwrap().rows()
    }

    fn wait_until(&self, what: &str, done: impl Fn(&[String]) -> bool) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let rows = self.rows();
            if done(&rows) {
                return rows;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}:\n{}",
                self.screen.lock().unwrap().text()
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn quit(mut self) -> bool {
        self.send("\x15");
        thread::sleep(Duration::from_millis(100));
        self.submit("/quit");
        let start = Instant::now();
        let ok = loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                break status.success();
            }
            if start.elapsed() > Duration::from_secs(10) {
                let _ = self.child.kill();
                break false;
            }
            thread::sleep(Duration::from_millis(50));
        };
        drop(self.reader.take());
        let _ = std::fs::remove_dir_all(&self.dir);
        ok
    }
}

/// Where an escape sequence that has not ended yet starts in `text`, or its length.
fn incomplete_escape(text: &str) -> usize {
    let Some(at) = text.rfind('\u{1b}') else {
        return text.len();
    };
    let tail = &text[at + 1..];
    let done = match tail.chars().next() {
        None => false,
        Some('[') => tail[1..].chars().any(|d| ('@'..='~').contains(&d)),
        Some(']') => tail.contains('\u{7}') || tail.contains("\u{1b}\\"),
        Some(_) => true,
    };
    if done {
        text.len()
    } else {
        at
    }
}

fn has(rows: &[String], needle: &str) -> bool {
    rows.iter().any(|r| r.contains(needle))
}

fn row_with(rows: &[String], needle: &str) -> Option<usize> {
    rows.iter().position(|r| r.contains(needle))
}

#[test]
#[ignore = "drives a real full-screen forge on a pty; run locally with --ignored"]
fn clicking_a_folded_tool_group_opens_it_and_clicking_again_folds_it() {
    let s = Session::start();
    s.wait_until("the composer", |r| has(r, "Message Forge"));
    s.submit("mock:tools tour the project");
    let rows = s.wait_until("the turn to finish", |r| has(r, "Done. I explored"));

    // Folded: one summary row for the exploring calls, the calls themselves out of sight.
    let summary = "Listed 1 directory, searched for 2 patterns, read 1 file, ran 2 shell commands";
    let at =
        row_with(&rows, summary).unwrap_or_else(|| panic!("summary row:\n{}", rows.join("\n")));
    assert!(rows[at].contains('▸'), "{}", rows.join("\n"));
    assert!(rows[at].contains("1 failed"), "{}", rows.join("\n"));
    assert!(!has(&rows, "✓ Find"), "{}", rows.join("\n"));
    assert!(
        !has(&rows, "Previous message"),
        "no navigation pill at the bottom:\n{}",
        rows.join("\n")
    );

    // A real mouse click on the summary opens the group in place.
    s.click(6, at);
    let open = s.wait_until("the group to open", |r| {
        has(r, &format!("▾ {summary}")) && has(r, "✓ Find")
    });
    assert_eq!(
        row_with(&open, summary),
        Some(at),
        "the summary stays under the pointer:\n{}",
        open.join("\n")
    );
    assert!(
        has(&open, "✓ List") && has(&open, "echo 'build ok'"),
        "{}",
        open.join("\n")
    );

    // And folds it again.
    s.click(6, at);
    let closed = s.wait_until("the group to fold", |r| {
        has(r, &format!("▸ {summary}")) && !has(r, "✓ Find")
    });
    assert!(!has(&closed, "echo 'build ok'"), "{}", closed.join("\n"));

    // Alt+O opens every group from the keyboard.
    s.send("\x1bo");
    s.wait_until("every group to open", |r| {
        has(r, "▾ Created mock-summary.md") && has(r, "✓ Write") && has(r, "✓ Edit")
    });
    assert!(s.quit(), "clean exit");
}
