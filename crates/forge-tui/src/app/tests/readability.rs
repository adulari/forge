//! The transcript of a real tool-heavy turn, collapsed and with every group open: scripts with
//! calls inside them, long commands under `$HOME`, a failed command with its diagnosis, a
//! silence-guard progress note and a background job's exit. `print_captures` dumps the screens
//! (`cargo test -p forge-agent-tui readability::print_captures -- --ignored --nocapture`).

use super::*;

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/home/u".into())
}

fn start(app: &mut App, name: &str, args: &str) {
    app.apply(PresenterEvent::ToolStart {
        name: name.into(),
        args: args.into(),
    });
}

fn finish(app: &mut App, name: &str, ok: bool, summary: &str, detail: Option<&str>) {
    app.apply(PresenterEvent::ToolResult {
        name: name.into(),
        ok,
        summary: summary.into(),
        detail: detail.map(str::to_string),
    });
}

fn shell_args(cmd: &str) -> String {
    serde_json::json!({ "command": cmd }).to_string()
}

fn script(app: &mut App, code: &str, inner: &[(&str, bool)]) {
    start(app, "run_code", code);
    for (cmd, ok) in inner {
        start(app, "shell", &shell_args(cmd));
        let code = if *ok { 0 } else { 1 };
        finish(
            app,
            "shell",
            *ok,
            &format!("shell: exit {code}"),
            Some("out"),
        );
    }
    finish(
        app,
        "run_code",
        true,
        "shell: exit 0",
        Some("shell: exit 0\nout"),
    );
}

/// A full-screen app part way through a long turn, with every kind of row the screenshots show.
pub(super) fn long_turn() -> App {
    let h = home();
    let mut app = App {
        fullscreen: true,
        fold_tool_groups: true,
        transcript_follow: true,
        ..App::default()
    };
    app.submit_user("get the solver login working again");
    app.ensure_assistant_label();
    app.apply(PresenterEvent::AssistantText(
        "I'll check which keys the environment has first.".into(),
    ));
    let env = "env | grep -i 'INFERHUB\\|OPENAI\\|V2_' | sed 's/=.*/=<set>/'; echo ---; ls ~/.config | grep -i 'solver\\|vm\\|infer'";
    script(
        &mut app,
        &format!(
            "const r = await tools.shell({{command: \"{}\"}});\nreturn r;",
            env.replace('"', "\\\"")
        ),
        &[(env, false)],
    );
    app.apply(PresenterEvent::ShellDiagnosis {
        command: env.into(),
        diagnosis: "The exit 1 comes from the final `grep` matching nothing in `~/.config`, a normal no-match result rather than a real failure.".into(),
        fix: Some("ls ~/.config; true".into()),
    });
    app.apply(PresenterEvent::Progress(
        "4 steps since the last update: 4 run_code".into(),
    ));
    app.apply(PresenterEvent::AssistantText(
        "No key in the environment. Reading the login test and the substrate notes next.".into(),
    ));
    start(
        &mut app,
        "read_file",
        &serde_json::json!({ "path": format!("{h}/wt/vm-solver/rust/crates/app/examples/android_login_test.rs") }).to_string(),
    );
    finish(
        &mut app,
        "read_file",
        true,
        "//! Test bot password login for a just-created account",
        Some("//! Test bot"),
    );
    let tail = format!("tail -6 {h}/.local/share/forge/workspaces/ssu-dashboard-92fa779cb121c072/jobs/2809333.log 2>/dev/null; echo \"exit: $?\"");
    script(
        &mut app,
        &format!("const r = await tools.shell({{command: \"{tail}\"}});\nreturn r;")
            .replace("\"exit", "\\\"exit")
            .replace("$?\"", "$?\\\""),
        &[(&tail, true)],
    );
    start(
        &mut app,
        "write_file",
        &serde_json::json!({ "path": format!("{h}/re-work/vm-ab/run_cohort.py"), "content": "x" })
            .to_string(),
    );
    app.apply(PresenterEvent::Diff(forge_types::FileDiff {
        path: format!("{h}/re-work/vm-ab/run_cohort.py"),
        kind: forge_types::DiffKind::Created,
        old: None,
        new: Some((1..=12).map(|i| format!("line {i}\n")).collect()),
        lang: Some("python".into()),
        binary: false,
    }));
    finish(&mut app, "write_file", true, "wrote 12 lines", None);
    let sed =
        format!("cd {h}/wt/vm-solver && sed -n '1,60p' docs/VM-CHROMIUM-SUBSTRATE-2026-10-18.md");
    let rg = format!("cd {h}/wt/vm-solver && rg -n 'fn chromium_challenge_with_key' -A 30 rust/crates/spotify/src/captcha.rs | head -80");
    script(
        &mut app,
        "const r = await Promise.all([\n  tools.shell({command: \"a\"}),\n  tools.shell({command: \"b\"}),\n]);\nreturn r.join('\\n');",
        &[(&sed, true), (&rg, true)],
    );
    app.apply(PresenterEvent::AssistantText(
        "Kicked off the build in the background.".into(),
    ));
    app.submit_user(&format!(
        "[background job 2809333 `exec {h}/bin/agentlive-exec /mnt/work/agentlive/logs/2026-10-24/cc98a0c86fde4adf.log -- 'cargo build -p ss-app'` exited 0]\nlast lines:\n   Compiling ss-api v4.5.146 ({h}/wt/vm-solver/rust/crates/api)\nwarning: `ss-core` (lib) generated 2 warnings\n   Compiling ss-app v4.5.146 ({h}/wt/vm-solver/rust/crates/app)\n    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 28s\n"
    ));
    app.ensure_assistant_label();
    app.apply(PresenterEvent::AssistantText(
        "Waiting for the cohort run.".into(),
    ));
    let sleep = format!("sleep 120; tail -5 {h}/.local/share/forge/workspaces/ssu-dashboard-92fa779cb121c072/jobs/2809340.log 2>/dev/null; echo '--- running:'; pgrep -c -f 'cohort'");
    start(
        &mut app,
        "run_code",
        "const r = await tools.shell({command: \"sleep 120\"});\nreturn r;",
    );
    start(&mut app, "shell", &shell_args(&sleep));
    app.busy = true;
    let _ = app.drain_flush();
    app
}

fn capture(app: &App, w: u16, h: u16) -> String {
    screen_rows(app, w, h)
        .into_iter()
        .map(|r| r.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
#[ignore = "prints captures for a human to read"]
fn print_captures() {
    for (open, label) in [(false, "collapsed"), (true, "Alt+O open")] {
        for w in [185u16, 150, 100] {
            let mut app = long_turn();
            if open {
                app.fold_ui.toggle_all(true);
            }
            println!("──── {label} · {w} cols ────\n{}\n", capture(&app, w, 60));
        }
    }
}

fn screen_of(open: bool, w: u16) -> Vec<String> {
    let mut app = long_turn();
    if open {
        app.fold_ui.toggle_all(true);
    }
    screen_rows(&app, w, 70)
}

fn count(screen: &[String], needle: &str) -> usize {
    screen.iter().filter(|r| r.contains(needle)).count()
}

#[test]
fn a_failed_command_shows_once_with_its_diagnosis_under_it() {
    for open in [false, true] {
        for w in [150, 100] {
            let screen = screen_of(open, w);
            assert_eq!(count(&screen, "env | grep -i"), 1, "{screen:#?}");
            assert!(
                !screen.iter().any(|r| r.contains("shell failed")),
                "{screen:#?}"
            );
            let cause = screen
                .iter()
                .position(|r| r.contains("The exit 1 comes from"))
                .expect("the diagnosis");
            let failed = screen
                .iter()
                .position(|r| r.contains("✗ Run"))
                .unwrap_or(cause);
            assert!(failed <= cause, "{screen:#?}");
            assert!(
                screen.iter().any(|r| r.contains("fix ls ~/.config; true")),
                "{screen:#?}"
            );
        }
    }
}

#[test]
fn the_silence_guard_note_stays_out_of_a_folded_transcript() {
    let screen = screen_of(false, 150);
    assert!(
        !screen
            .iter()
            .any(|r| r.contains("steps since the last update")),
        "{screen:#?}"
    );
}

#[test]
fn a_script_is_counted_by_the_calls_it_made() {
    let screen = screen_of(false, 150);
    assert!(
        screen.iter().any(|r| r.contains(
            "▸ Read 1 file, ran 3 shell commands, created ~/re-work/vm-ab/run_cohort.py (+12)"
        )),
        "{screen:#?}"
    );
    assert!(!screen.iter().any(|r| r.contains("script")), "{screen:#?}");
}

#[test]
fn no_row_spells_out_the_home_directory() {
    let home = home();
    for open in [false, true] {
        let screen = screen_of(open, 185);
        assert!(!screen.iter().any(|r| r.contains(&home)), "{screen:#?}");
    }
    let screen = screen_of(false, 150);
    assert!(
        screen
            .iter()
            .any(|r| r.contains("✓ job 2809333 exited 0 · ~/bin/agentlive-exec")),
        "{screen:#?}"
    );
}

#[test]
fn the_composer_border_does_not_repeat_the_running_command() {
    for open in [false, true] {
        let screen = screen_of(open, 150);
        assert!(
            screen.iter().any(|r| r.contains("Running a command")),
            "{screen:#?}"
        );
        assert_eq!(count(&screen, "sleep 120"), 1, "{screen:#?}");
        let row = screen.iter().find(|r| r.contains("sleep 120")).unwrap();
        assert!(row.trim_end().ends_with("running"), "{screen:#?}");
    }
}

#[test]
fn an_open_group_nests_a_scripts_calls_and_wraps_their_commands() {
    for w in [185u16, 150, 100, 60] {
        let screen = screen_of(true, w);
        assert!(
            !screen
                .iter()
                .any(|r| r.contains("await") || r.contains("tools.shell")),
            "{screen:#?}"
        );
        let script = screen
            .iter()
            .position(|r| r.contains("Script 2 calls: run"))
            .unwrap_or_else(|| panic!("{w}: {screen:#?}"));
        let indent = |r: &str| r.chars().position(|c| c == '✓' || c == '✗').unwrap_or(0);
        let child = screen[script + 1..]
            .iter()
            .find(|r| r.contains("✓ Run"))
            .expect("the script's call");
        assert_eq!(indent(child), indent(&screen[script]) + 2, "{screen:#?}");
        let text: String = screen
            .iter()
            .map(|r| r.trim().trim_start_matches('▏').trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains("captcha.rs | head -80"),
            "{w}: the command wraps whole: {screen:#?}"
        );
        for r in &screen {
            if r.contains("✓ Run") {
                assert!(r.contains("exit 0"), "{w}: the status is never cut: {r:?}");
            }
        }
    }
}

#[test]
fn a_tool_group_is_set_apart_from_the_prose_around_it() {
    let screen = screen_of(false, 150);
    let at = screen
        .iter()
        .position(|r| r.contains("▸ Read 1 file"))
        .expect("the group");
    assert_eq!(screen[at - 1].trim(), "▏", "{screen:#?}");
    assert_eq!(screen[at + 1].trim(), "▏", "{screen:#?}");
    assert!(
        screen[at - 2].contains("Reading the login test"),
        "{screen:#?}"
    );
}

#[test]
fn a_read_after_a_script_does_not_fold_into_the_scripts_own_read() {
    let mut app = App {
        fullscreen: true,
        fold_tool_groups: true,
        ..App::default()
    };
    app.ensure_assistant_label();
    start(
        &mut app,
        "run_code",
        "await tools.read_file({path: 'a.rs'});",
    );
    start(&mut app, "read_file", r#"{"path":"a.rs"}"#);
    finish(&mut app, "read_file", true, "1 line", Some("x"));
    finish(&mut app, "run_code", true, "x", None);
    start(&mut app, "read_file", r#"{"path":"b.rs"}"#);
    finish(&mut app, "read_file", true, "1 line", Some("y"));
    let reads: Vec<&tool_cards::ToolCard> = app
        .tool_cards
        .iter()
        .filter(|c| c.name == "read_file")
        .collect();
    assert_eq!(reads.len(), 2, "{:#?}", app.tool_cards);
    assert_eq!(reads[0].parent, Some(app.tool_cards[0].id));
    assert_eq!(reads[1].parent, None);
    assert!(reads.iter().all(|c| c.group.is_empty()));
}
