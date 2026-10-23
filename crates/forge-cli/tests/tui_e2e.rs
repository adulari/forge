//! On-demand TUI end-to-end smokes over a real PTY. Drives the actual `forge chat --mock` binary
//! through a pseudo-terminal — answering the terminal's cursor-position (DSR) query so the inline
//! viewport initializes (a CI runner's null terminal won't, hence `#[ignore]`).
//!
//! Run locally: `cargo test -p forge-agent --test tui_e2e -- --ignored --nocapture`

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// These smokes synchronise with the TUI by sleeping fixed intervals — 1500ms for init, then the
/// per-step `sleep_ms_after`. Those budgets hold when one test has the machine and do not when
/// eight of them each spawn a real `forge` binary on its own PTY at once: the assertion then reads
/// a capture containing only the startup banner, because the TUI has not got as far as rendering.
///
/// Measured on 2026-08-11 with `scripts/flake-hunt.sh 8`:
///   parallel (cargo's default):  6 of 8 runs failed — `tui_wrapper_command_delegates_to_its_skill`
///                                6/8, `tui_loop_starts_and_stops_on_interrupt` 5/8,
///                                `tui_remote_control_toggles...` 4/8, `tui_autocheckpoints...` 3/8
///   `--test-threads=1`:          8 of 8 runs clean
///
/// So the sleeps are adequate and the contention is the whole problem. Serialising here rather than
/// relying on `--test-threads=1` keeps that guarantee wherever the suite is invoked from — a CI job,
/// a local `cargo test`, or an IDE — instead of depending on every caller remembering the flag.
/// Poison is deliberately ignored: one panicking test must not cascade into failing the other seven.
static PTY_SERIAL: Mutex<()> = Mutex::new(());

/// Launch `forge chat --mock` on a PTY in a throwaway cwd, answer DSR queries, then feed the
/// `(keys, sleep_ms_after)` script. Returns `(clean_exit, plain_output)`.
fn drive_pty(script: &[(&str, u64)]) -> (bool, String) {
    drive_pty_env(script, &[])
}

/// A script step `"{WAIT_FOR}<marker>"` waits for `<marker>` to render instead of sending keys.
const WAIT_FOR: &str = "\u{0}wait:";

/// [`drive_pty`] with extra environment for the `forge` child (e.g. mock pacing).
fn drive_pty_env(script: &[(&str, u64)], env: &[(&str, &str)]) -> (bool, String) {
    let _serial = PTY_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let dir = std::env::temp_dir().join(format!("forge-e2e-{}", forge_id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sample.rs"), "fn main() { let x = 1; }\n").unwrap();

    // Seed a real project-scope skill + a wrapper command that delegates to it, so the
    // skill-invocation tests exercise the actual catalog → resolve → dispatch path end to end.
    // trust_project skips the first-use confirmation gate so the command runs on first try.
    std::fs::create_dir_all(dir.join(".forge")).unwrap();
    std::fs::write(
        dir.join(".forge/config.toml"),
        "[commands]\ntrust_project = true\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join(".forge/skills/demoskill")).unwrap();
    std::fs::write(
        dir.join(".forge/skills/demoskill/SKILL.md"),
        "---\nname: demoskill\ndescription: a demo skill for e2e\n---\nDEMO_METHODOLOGY_MARKER: do the demo steps.",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join(".forge/commands")).unwrap();
    std::fs::write(
        dir.join(".forge/commands/demo.md"),
        "Use the **demoskill** skill to handle this request.\n\nRequest: $ARGUMENTS",
    )
    .unwrap();
    // The project-trust gate ignores a project's `.forge/` settings (and asks on a TTY) until the
    // project is trusted. Trust the throwaway cwd in a store private to this run, so the seeded
    // skill and command load on the first try and the developer's real trust store is untouched.
    let xdg_data = dir.join("xdg-data");
    let xdg_config = dir.join("xdg-config");
    std::fs::create_dir_all(&xdg_data).unwrap();
    std::fs::create_dir_all(&xdg_config).unwrap();
    let trusted = std::process::Command::new(env!("CARGO_BIN_EXE_forge"))
        .arg("trust")
        .arg(&dir)
        .env("XDG_DATA_HOME", &xdg_data)
        .env("XDG_CONFIG_HOME", &xdg_config)
        .env("FORGE_DB", dir.join("forge.db"))
        .output()
        .expect("run forge trust");
    assert!(trusted.status.success(), "forge trust failed: {trusted:?}");
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_forge"));
    cmd.env("XDG_DATA_HOME", &xdg_data);
    cmd.env("XDG_CONFIG_HOME", &xdg_config);
    cmd.arg("chat");
    cmd.arg("--mock");
    // Inline viewport: drives the native-scrollback path so the PTY capture is deterministic
    // (the default full-screen mode renders on the alternate screen, which is harder to assert on).
    cmd.arg("--inline");
    cmd.cwd(&dir);
    // Isolate the store in the temp dir — the real store now lives in a per-user data dir, so
    // without this the e2e would read/write the developer's actual session+usage DB.
    cmd.env("FORGE_DB", dir.join("forge.db"));
    for (key, value) in env {
        cmd.env(key, value);
    }

    let mut child = pair.slave.spawn_command(cmd).expect("spawn forge");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let writer = Arc::new(Mutex::new(pair.master.take_writer().expect("writer")));
    let captured = Arc::new(Mutex::new(String::new()));

    let w_reader = Arc::clone(&writer);
    let cap = Arc::clone(&captured);
    let reader_thread = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    if chunk.contains("\u{1b}[6n") {
                        if let Ok(mut w) = w_reader.lock() {
                            let _ = w.write_all(b"\x1b[1;1R");
                            let _ = w.flush();
                        }
                    }
                    cap.lock().unwrap().push_str(&chunk);
                }
            }
        }
    });

    let send = |s: &str| {
        let mut w = writer.lock().unwrap();
        let _ = w.write_all(s.as_bytes());
        let _ = w.flush();
    };
    thread::sleep(Duration::from_millis(1500)); // init + first DSR round-trip
    for (keys, after_ms) in script {
        if let Some(marker) = keys.strip_prefix(WAIT_FOR) {
            // Paced steps: block until the (ANSI-stripped) capture shows `marker`, so a test can
            // key off what the TUI actually rendered instead of guessing how long it takes.
            let deadline = Instant::now() + Duration::from_secs(120);
            while !strip_ansi(&captured.lock().unwrap()).contains(marker) {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {marker:?}"
                );
                // Tight: a step that must land the instant a marker renders (Esc as an answer
                // ends) is lost to a 50 ms poll, which always arrives after the turn has settled.
                thread::sleep(Duration::from_millis(2));
            }
        } else {
            send(keys);
        }
        thread::sleep(Duration::from_millis(*after_ms));
    }

    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().expect("try_wait") {
            break Some(s);
        }
        if start.elapsed() > Duration::from_secs(8) {
            let _ = child.kill();
            break None;
        }
        thread::sleep(Duration::from_millis(50));
    };
    drop(writer);
    let _ = reader_thread.join();
    std::fs::remove_dir_all(&dir).ok();

    let plain = strip_ansi(&captured.lock().unwrap());
    let clean = status.map(|s| s.success()).unwrap_or(false);
    (clean, plain)
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_autocheckpoints_then_undo_picker_rewinds_over_a_pty() {
    // A real turn auto-checkpoints; /undo opens the rewind picker; Enter rewinds.
    let (clean, plain) = drive_pty(&[
        // Debug/CI builds can take longer than one second to finish the first mock tool round-trip.
        // Do not race the idle-only /undo command against the still-running turn.
        ("say hi\r", 2500),
        ("/undo\r", 800),
        ("\r", 800),
        // Rewinding puts the rewound prompt back in the composer; clear it (Ctrl+U) so `/quit`
        // is the whole line. Idle Esc only arms the rewind picker; /quit is what exits.
        ("\x15", 100),
        ("/quit\r", 0),
    ]);
    assert!(clean, "clean exit, no panic: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        plain.contains("rewound to that point"),
        "auto-checkpoint → /undo picker → rewind worked end to end: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_assay_mode_opens_choice_picker_and_runs_without_crashing() {
    // /assay opens the analysis-vs-cleanup picker; selecting a choice runs the flow. Under --mock
    // with no provider keys there are no live models, so it degrades gracefully (a note) rather
    // than crashing — this smoke proves the palette → AssayChoice picker → spawn_assay wiring.
    let (clean, plain) = drive_pty(&[
        ("/assay\r", 800),
        ("\r", 1200),
        ("\x03", 600),
        ("/quit\r", 600),
    ]);
    assert!(clean, "clean exit, no panic: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        plain.to_lowercase().contains("assay"),
        "the assay choice picker was reached: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_skill_invoked_directly_is_found_and_loads_methodology() {
    // Invoking a real skill by name (no args) must FIND it and prime its methodology — the
    // regression behind "unable to find skill". /quit exits.
    let (clean, plain) = drive_pty(&[("/demoskill\r", 1000), ("/quit\r", 600)]);
    assert!(clean, "clean exit: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        !plain.contains("unknown command"),
        "the skill was found, not reported unknown: {plain}"
    );
    assert!(
        plain.contains("methodology primed") || plain.contains("skill · demoskill"),
        "the skill loaded its methodology: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_wrapper_command_delegates_to_its_skill() {
    // A CC-style wrapper (/demo → **demoskill**) must resolve as a command and run — not
    // "unknown command". Esc stops the mock turn, /quit exits.
    let (clean, plain) = drive_pty(&[
        ("/demo do the thing\r", 1200),
        ("\x1b", 600),
        ("/quit\r", 600),
    ]);
    assert!(clean, "clean exit: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        !plain.contains("unknown command"),
        "the wrapper resolved, not unknown: {plain}"
    );
    assert!(
        plain.contains("command · /demo"),
        "the wrapper command ran: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_goal_sets_objective_and_runs() {
    // /goal pins the objective (note) and spawns a decomposition turn; Esc stops, /quit exits.
    let (clean, plain) = drive_pty(&[
        ("/goal ship the parser\r", 1500),
        ("\x1b", 600),
        ("/quit\r", 600),
    ]);
    assert!(clean, "clean exit: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        plain.contains("goal set"),
        "the /goal note was shown: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_loop_starts_and_stops_on_interrupt() {
    // /loop starts an autonomous loop (note), Esc interrupts it, then /quit exits cleanly.
    let (clean, plain) = drive_pty(&[
        ("/loop keep working\r", 1500),
        ("\x1b", 600),
        ("/quit\r", 600),
    ]);
    assert!(clean, "clean exit: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        plain.contains("loop started"),
        "the /loop note was shown: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_esc_sends_a_queued_prompt_the_turn_already_absorbed() {
    // Paced mock (500ms/word, 3s think pause before each reply). "hello" runs a read_file
    // round-trip; the prompt queued meanwhile is absorbed (steered) at that tool boundary, and the
    // mock then "thinks" for 3s before answering it. Esc lands inside that pause: the absorbed
    // prompt was never answered, so the interrupt must send it as the next turn. Before the fix
    // this printed "stopped responding" and the prompt was never answered.
    let wait_steer = format!("{WAIT_FOR}you · follow-up");
    let (clean, plain) = drive_pty_env(
        &[
            ("hello\r", 1000),
            ("followup marker\r", 0),
            (wait_steer.as_str(), 300),
            ("\x1b", 1500),
            ("\x1b", 600),
            ("/quit\r", 600),
        ],
        &[("FORGE_MOCK_STREAM_DELAY_MS", "500")],
    );
    assert!(clean, "clean exit: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        plain.contains("1 queued"),
        "the queue band showed the prompt mid-turn: {plain}"
    );
    assert!(
        plain.contains("followup marker"),
        "the band listed the queued text itself: {plain}"
    );
    // The notice reads `▲ interrupted — …` since the transcript re-glyphs legacy `⏹` notes.
    let first_esc = plain
        .find("interrupted — continuing")
        .expect("the first Esc interrupted the turn");
    assert!(
        plain[..first_esc].contains("you · follow-up"),
        "the turn absorbed the prompt before Esc (the case that used to stop): {plain}"
    );
    assert!(
        plain[first_esc..].starts_with("interrupted — continuing with the next queued prompt"),
        "Esc must send the absorbed prompt, not stop: {}",
        &plain[first_esc..]
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_esc_during_a_turn_says_what_to_do_next() {
    // The old notice, "interrupted — stopped responding", read like a hang. After a real
    // interrupt the person is told what happened and what comes next.
    let (clean, plain) = drive_pty_env(
        &[("hello\r", 1000), ("\x1b", 600), ("/quit\r", 600)],
        &[("FORGE_MOCK_STREAM_DELAY_MS", "500")],
    );
    assert!(clean, "clean exit: {plain}");
    assert!(
        plain.contains("Interrupted. Type what to do instead."),
        "the interrupt notice says what to do next: {plain}"
    );
    assert!(
        !plain.contains("stopped responding"),
        "no hang-sounding wording: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_esc_the_instant_an_answer_finishes_never_reports_an_interruption() {
    // The race: the turn's last events and its done signal are still queued when Esc arrives, and
    // the loop used to treat that Esc as an interrupt. It threw the queued events away and printed
    // an interruption under an answer whose result row was already on screen. Esc is sent the
    // moment the answer's last line renders, which is inside that window; a handful of rounds,
    // because landing in it depends on scheduling.
    //
    // The invariant: the turn's result row (`0.4s · ↑42 ↓18`) and an interruption notice never
    // appear together. An Esc that lands before the result event is a real interrupt (no row ever
    // follows); one that lands after it is no interrupt at all (no notice).
    for round in 0..6 {
        let (clean, plain) = drive_pty_env(
            &[
                ("mock:code a small question\r", 0),
                (&format!("{WAIT_FOR}That's it."), 0),
                ("\x1b", 900),
                ("/quit\r", 600),
            ],
            &[("FORGE_MOCK_STREAM_DELAY_MS", "25")],
        );
        assert!(clean, "round {round}: clean exit: {plain}");
        assert!(
            plain.contains("That's it."),
            "round {round}: the answer finished: {plain}"
        );
        assert!(
            !(plain.contains("nterrupted") && plain.contains("· ↑42")),
            "round {round}: Esc on a finished turn must not print an interruption: {plain}"
        );
    }
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_quit_prints_how_to_resume_once_something_was_said() {
    let (clean, plain) = drive_pty(&[("say hi\r", 2500), ("/quit\r", 800)]);
    assert!(clean, "clean exit: {plain}");
    let hint = plain
        .lines()
        .find(|line| line.starts_with("Resume this session: forge chat --resume "))
        .unwrap_or_else(|| panic!("no resume line: {plain}"));
    let id = hint
        .trim_start_matches("Resume this session: forge chat --resume ")
        .split_whitespace()
        .next()
        .unwrap();
    assert_eq!(id.len(), 8, "{hint}");
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "{hint}");
    assert!(
        hint.contains("\"say hi\""),
        "the session's name follows the command: {hint}"
    );
    assert_eq!(plain.matches("Resume this session").count(), 1, "{plain}");

    // A session where nothing was said has nothing to resume.
    let (clean, plain) = drive_pty(&[("/quit\r", 800)]);
    assert!(clean, "clean exit: {plain}");
    assert!(
        !plain.contains("Resume this session"),
        "an empty session is not offered back: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_btw_runs_and_compact_queues_while_a_turn_is_running() {
    // Paced mock so "hello" is still running when the commands are typed. `/btw` must run beside
    // the turn (no idle refusal), `/compact` must be queued rather than refused or steered, and
    // Esc must hand the queued command to dispatch instead of sending "/compact" to the model.
    let (clean, plain) = drive_pty_env(
        &[
            ("hello\r", 1000),
            ("/btw what is a mutex\r", 400),
            ("\x1b", 300), // closes the side-question card
            ("/compact\r", 400),
            ("\x1b", 2500),
            ("/quit\r", 600),
        ],
        &[("FORGE_MOCK_STREAM_DELAY_MS", "500")],
    );
    assert!(clean, "clean exit: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        !plain.contains("commands run when the turn is idle"),
        "neither command was refused: {plain}"
    );
    assert!(
        plain.contains("1 queued") && plain.contains("/compact"),
        "/compact waited in the queue, listed in the band: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_unknown_command_and_rename_mid_turn_leave_the_ui_responsive() {
    // Paced mock so "hello" is still running when the lines are typed. A `/name` that is not a
    // built-in used to await the session lock the running turn holds: the render loop stopped
    // drawing and reading keys (not even Esc) until the turn ended, and for good once the turn
    // asked the UI something. Esc must still interrupt the turn afterwards, and `/rename` must
    // report its result while the turn is running, not when it ends.
    let (clean, plain) = drive_pty_env(
        &[
            ("hello\r", 1000),
            ("/definitely-not-a-command\r", 400),
            ("/rename mid turn title\r", 600),
            ("\x1b", 800),
            ("/quit\r", 600),
        ],
        &[("FORGE_MOCK_STREAM_DELAY_MS", "500")],
    );
    assert!(clean, "clean exit: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    let unknown = plain
        .find("unknown command: /definitely-not-a-command")
        .unwrap_or_else(|| panic!("the unknown command was answered: {plain}"));
    let renamed = plain
        .find("session renamed to “mid turn title”")
        .unwrap_or_else(|| panic!("/rename reported its result: {plain}"));
    let interrupted = plain
        .find("Interrupted. Type what to do instead.")
        .unwrap_or_else(|| panic!("Esc reached the running turn: {plain}"));
    assert!(
        unknown < renamed && renamed < interrupted,
        "both lines were handled while the turn ran, before Esc stopped it: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_config_opens_wizard_fullscreen_and_returns_to_chat() {
    // /config takes over the screen with the setup wizard; Esc cancels it and the inline chat
    // loop must resume cleanly (raw mode re-enabled, viewport rebuilt) — then /quit exits. This
    // proves the alt-screen takeover + restore doesn't wedge the terminal.
    let (clean, plain) = drive_pty(&[
        ("/config\r", 1200),
        ("\x1b", 800),    // Esc cancels the wizard, back to chat
        ("/quit\r", 600), // chat still responsive → clean exit
    ]);
    assert!(clean, "clean exit after the config takeover: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        plain.contains("Forge setup") || plain.to_lowercase().contains("config"),
        "the config wizard was reached and chat resumed: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_remote_control_toggles_and_shows_statusline_indicator() {
    // /remote turns on the server: prints the connect URL + a QR code into scrollback and lights
    // the `◉ remote` statusline indicator. Running /remote again turns it off. No model keys are
    // needed (the server is pure UI infra), so this works under --mock.
    let (clean, plain) = drive_pty(&[
        ("/remote\r", 1200),
        ("/remote\r", 600), // toggle off again
        ("/quit\r", 600),
    ]);
    assert!(clean, "clean exit: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        plain.contains("remote control on"),
        "turning on printed the on-note: {plain}"
    );
    assert!(
        plain.contains("http://") || plain.contains("https://"),
        "the connect URL was printed: {plain}"
    );
    assert!(
        plain.contains("scan to connect") || plain.contains('█'),
        "a QR code was rendered into scrollback: {plain}"
    );
    assert!(
        plain.contains("remote control off"),
        "the second /remote turned it off: {plain}"
    );
    // The `◉ remote` segment appears in the statusline while on (and the frame is redrawn).
    assert!(
        plain.contains("remote"),
        "the statusline showed the remote indicator: {plain}"
    );
}

fn forge_id() -> String {
    format!("{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')', ' '], "")
}

/// Drop CSI/escape sequences so assertions match the visible text, not the control bytes.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_ctrl_c_clears_draft_then_needs_two_presses_to_quit() {
    // Ctrl-C on a draft clears it (saved for ↑); on an empty input it arms, and the second press
    // exits. No /quit: a clean exit proves the double press quit.
    let (clean, plain) = drive_pty(&[
        ("half typed", 400),
        ("\x03", 500),
        ("\x03", 500),
        ("\x03", 600),
    ]);
    assert!(clean, "clean exit via double Ctrl-C: {plain}");
    assert!(
        plain.contains("input cleared"),
        "draft clear noted: {plain}"
    );
    assert!(plain.contains("Ctrl+C again"), "arm hint shown: {plain}");
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_bang_line_runs_in_the_shell_and_the_model_responds() {
    // `!cmd` is shell mode: the line runs through the shell tool (a `Run` card with the exit code)
    // instead of reaching the model as text, and the model then answers the output.
    let (clean, plain) = drive_pty(&[("!echo bang-e2e-marker\r", 3500), ("/quit\r", 600)]);
    assert!(clean, "clean exit, no panic: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        plain.contains("exit 0"),
        "the command ran and its card reports the exit code: {plain}"
    );
    assert!(
        plain.contains("Let me inspect the project manifest"),
        "the model responded to the shell output: {plain}"
    );
}

#[test]
#[ignore = "needs a DSR-answering pty; run locally with --ignored"]
fn tui_unbound_ctrl_chords_never_type_and_ctrl_d_twice_quits() {
    // Ctrl+X / Ctrl+B / Ctrl+Q have no meaning in the composer and used to insert `x`, `b`, `q`.
    // Ctrl+D on an empty prompt arms a quit and a second press leaves, with no /quit needed.
    let (clean, plain) = drive_pty(&[
        ("abc\x18\x02\x11", 400),
        ("\x15", 200),
        ("\x04", 400),
        ("\x04", 0),
    ]);
    assert!(clean, "Ctrl+D twice exits cleanly, no panic: {plain}");
    assert!(!plain.to_lowercase().contains("panic"), "no panic: {plain}");
    assert!(
        !plain.contains("abcx") && !plain.contains("abcxb"),
        "an unbound Ctrl chord was typed into the draft: {plain}"
    );
    assert!(
        plain.contains("Ctrl+D again to quit"),
        "the first Ctrl+D on an empty prompt arms the quit and names its chord: {plain}"
    );
}
