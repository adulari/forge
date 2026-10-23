//! A running `forge chat` must come through a daemon restart and the replacement of its own
//! binary — the deploy sequence (`install` the new build, `systemctl --user restart forge-serve`)
//! that left terminals dead or unusable.
//!
//! Everything is private to the test: its own HOME/XDG dirs, store and daemon port, `--mock`
//! models, and copies of the `forge` binary in a temp dir. Nothing touches a real daemon.
//!
//! Like the other PTY smokes this needs a pseudo-terminal that answers the cursor-position query,
//! so it is `#[ignore]`d from the default run:
//! `cargo test -p forge-agent --test tui_daemon_restart_e2e -- --ignored --nocapture`

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

const FORGE: &str = env!("CARGO_BIN_EXE_forge");
const WAIT: Duration = Duration::from_secs(60);

struct Sandbox {
    root: PathBuf,
    port: u16,
}

impl Sandbox {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("forge-daemon-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in [
            "home", "config", "data", "state", "cache", "run", "bin", "ws",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        // A port nothing else holds right now; the daemon is told to listen on it explicitly.
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        Self { root, port }
    }

    fn env(&self) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("HOME", self.root.join("home")),
            ("XDG_CONFIG_HOME", self.root.join("config")),
            ("XDG_DATA_HOME", self.root.join("data")),
            ("XDG_STATE_HOME", self.root.join("state")),
            ("XDG_CACHE_HOME", self.root.join("cache")),
            ("XDG_RUNTIME_DIR", self.root.join("run")),
            ("FORGE_DB", self.root.join("forge.db")),
        ]
    }

    fn workspace(&self) -> PathBuf {
        self.root.join("ws")
    }

    fn install(&self, name: &str) -> PathBuf {
        let path = self.root.join("bin").join(name);
        std::fs::copy(FORGE, &path).unwrap();
        path
    }

    fn start_daemon(&self, binary: &Path) -> Child {
        let log = std::fs::File::create(self.root.join("daemon.log")).unwrap();
        let mut command = Command::new(binary);
        command
            .args([
                "serve",
                "--local",
                "--mock",
                "--port",
                &self.port.to_string(),
            ])
            .current_dir(self.workspace())
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log);
        for (key, value) in self.env() {
            command.env(key, value);
        }
        let child = command.spawn().expect("start the private daemon");
        let deadline = Instant::now() + WAIT;
        while TcpStream::connect(("127.0.0.1", self.port)).is_err() {
            assert!(
                Instant::now() < deadline,
                "the private daemon never listened"
            );
            thread::sleep(Duration::from_millis(100));
        }
        child
    }

    /// What `systemctl --user restart` does to the daemon: SIGTERM, wait for it to drain.
    fn stop_daemon(child: &mut Child) {
        let status = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let deadline = Instant::now() + WAIT;
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the daemon did not stop");
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn query<T>(
        &self,
        sql: &str,
        row: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Option<T> {
        let conn = rusqlite::Connection::open_with_flags(
            self.root.join("forge.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .ok()?;
        conn.query_row(sql, [], row).ok()
    }

    fn exec(&self, sql: &str) {
        let conn = rusqlite::Connection::open(self.root.join("forge.db")).unwrap();
        conn.busy_timeout(Duration::from_secs(10)).unwrap();
        conn.execute(sql, []).unwrap();
    }

    fn assistant_messages(&self) -> i64 {
        self.query(
            "SELECT COUNT(*) FROM message WHERE role = 'assistant'",
            |r| r.get(0),
        )
        .unwrap_or(0)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "drives a real PTY; run with --ignored"]
fn a_running_chat_survives_a_daemon_restart_and_the_replacement_of_its_own_binary() {
    let sandbox = Sandbox::new();
    let daemon_binary = sandbox.install("forge-daemon");
    let chat_binary = sandbox.install("forge-chat");
    let mut daemon = Daemon(sandbox.start_daemon(&daemon_binary));

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let mut cmd = CommandBuilder::new(&chat_binary);
    cmd.args(["chat", "--mock", "--inline"]);
    cmd.cwd(sandbox.workspace());
    for (key, value) in sandbox.env() {
        cmd.env(key, value);
    }
    let mut chat = pair.slave.spawn_command(cmd).expect("spawn forge chat");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
    let answer = Arc::clone(&writer);
    let transcript = Arc::new(Mutex::new(String::new()));
    let capture = Arc::clone(&transcript);
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
            if chunk.contains("\u{1b}[6n") {
                let mut w = answer.lock().unwrap();
                let _ = w.write_all(b"\x1b[1;1R");
                let _ = w.flush();
            }
            capture.lock().unwrap().push_str(&chunk);
        }
    });
    let send = |text: &str| {
        let mut w = writer.lock().unwrap();
        w.write_all(text.as_bytes()).unwrap();
        w.flush().unwrap();
    };

    wait_until("the chat to draw its prompt", || {
        transcript.lock().unwrap().contains("Message Forge")
    });
    send("first prompt before the restart\r");
    wait_until("the first answer", || sandbox.assistant_messages() >= 1);
    let before = sandbox.assistant_messages();
    let session: String = sandbox
        .query("SELECT id FROM session WHERE local_live = 1", |r| r.get(0))
        .expect("the chat publishes presence for its session");

    // The deploy: replace the chat's binary file (a new inode, as `install` and `cargo build`
    // leave it), then restart the daemon.
    let replacement = sandbox.install("forge-chat.new");
    std::fs::rename(&replacement, &chat_binary).unwrap();
    #[cfg(target_os = "linux")]
    {
        let chat_pid = chat.process_id().expect("chat pid");
        let exe = std::fs::read_link(format!("/proc/{chat_pid}/exe")).unwrap();
        assert!(
            exe.to_string_lossy().ends_with(" (deleted)"),
            "the chat must be running an unlinked inode for this to test anything: {exe:?}"
        );
    }
    Sandbox::stop_daemon(&mut daemon.0);
    // The state a stalled terminal leaves behind: the session is still in the daemon's fleet
    // (it was created through the daemon, or the daemon hosted it before the chat took over), and
    // the chat's heartbeat is older than the one-minute staleness window although the process and
    // its control channel are alive. The restarted daemon used to resume it beside the chat.
    sandbox.exec(&format!(
        "UPDATE session SET daemon_live = 1, local_last_seen = strftime('%s','now') - 300 \
         WHERE id = '{session}'"
    ));
    daemon = Daemon(sandbox.start_daemon(&daemon_binary));
    wait_until("the daemon to decide about the session", || {
        std::fs::read_to_string(sandbox.root.join("daemon.log"))
            .is_ok_and(|log| log.contains("leaving it there") || log.contains("could not restore"))
    });

    // Still alive, still the owner of its session, and still takes turns.
    assert!(chat.try_wait().unwrap().is_none(), "the chat exited");
    send("second prompt after the restart\r");
    wait_until("an answer after the restart", || {
        sandbox.assistant_messages() > before
    });
    assert!(chat.try_wait().unwrap().is_none(), "the chat exited");
    let live: Option<i64> = sandbox.query(
        &format!("SELECT local_live FROM session WHERE id = '{session}'"),
        |r| r.get(0),
    );
    assert_eq!(live, Some(1), "the chat is still the session's owner");
    let daemon_log = std::fs::read_to_string(sandbox.root.join("daemon.log")).unwrap_or_default();
    assert!(
        daemon_log.contains(&format!("session {} is open in a terminal", &session[..8])),
        "the restarted daemon must leave the chat's session with the chat: {daemon_log}"
    );
    let still_in_daemon_fleet: Option<i64> = sandbox.query(
        &format!("SELECT daemon_live FROM session WHERE id = '{session}'"),
        |r| r.get(0),
    );
    assert_eq!(
        still_in_daemon_fleet,
        Some(0),
        "a session that lives in a terminal is not the daemon's to restore"
    );

    send("\x03\x03");
    drop(daemon);
}
