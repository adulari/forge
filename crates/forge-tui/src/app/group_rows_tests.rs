use super::*;

fn card(name: &str, args: &str, ok: Option<bool>, summary: &str, ms: u128) -> ToolCard {
    let mut c = ToolCard::new(1, name.into(), args.into());
    if let Some(ok) = ok {
        c.status = if ok {
            CardStatus::Ok
        } else {
            CardStatus::Failed
        };
        c.summary = summary.into();
        c.ms = Some(ms);
        c.detail = Some("out".into());
    }
    c
}

fn shell(cmd: &str, ok: Option<bool>) -> ToolCard {
    let args = serde_json::json!({ "command": cmd }).to_string();
    card("shell", &args, ok, "shell: exit 0", 182)
}

fn rows(c: &ToolCard, children: &[&ToolCard], width: usize) -> Vec<String> {
    let r = CallRow {
        card: c,
        depth: 0,
        children,
        width,
        tick: None,
        awaiting: false,
        disclosure: true,
    };
    call_rows(&r)
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

fn long_command() -> String {
    "cd /srv/wt/vm-solver && rg -n 'fn chromium_challenge_with_key' -A 30 rust/crates/spotify/src/captcha.rs | head -80".into()
}

#[test]
fn a_long_command_wraps_under_its_text_column_instead_of_being_cut() {
    let c = shell(&long_command(), Some(true));
    let out = rows(&c, &[], 80);
    assert!(out.len() > 1, "{out:#?}");
    for r in &out {
        assert!(cells(r) <= 80, "row overflows: {r:?}");
        assert!(!r.contains('…'), "nothing is cut: {out:#?}");
    }
    let joined: String = out.iter().map(|r| r.trim()).collect::<Vec<_>>().join(" ");
    assert!(joined.contains("captcha.rs | head -80"), "{out:#?}");
    let col = out[0]
        .chars()
        .position(|c| c == 'c')
        .expect("text on the first row");
    let cont = 3 + out[1].chars().skip(3).take_while(|c| *c == ' ').count();
    assert_eq!(cont, col, "the continuation hangs under the text: {out:#?}");
    assert!(out[0].trim_end().ends_with("exit 0 · 182ms ▸"), "{out:#?}");
}

#[test]
fn a_very_long_command_stops_at_four_rows_and_says_how_many_more() {
    let cmd = (0..40)
        .map(|i| format!("step{i}"))
        .collect::<Vec<_>>()
        .join(" && ");
    let c = shell(&cmd, Some(true));
    let out = rows(&c, &[], 70);
    assert_eq!(out.len(), MAX_TEXT_ROWS, "{out:#?}");
    assert!(out[3].contains("… 5 more lines"), "{out:#?}");
}

#[test]
fn a_tight_row_drops_the_duration_and_never_cuts_the_status() {
    let c = shell(&long_command(), Some(false));
    let mut c = c;
    c.summary = "shell: exit 101".into();
    let out = rows(&c, &[], 42);
    assert!(out[0].trim_end().ends_with("exit 101 ▸"), "{out:#?}");
    assert!(!out[0].contains("182ms"), "{out:#?}");
    let wide = rows(&c, &[], 120);
    assert!(wide[0].contains("exit 101 · 182ms"), "{wide:#?}");
}

#[test]
fn a_running_call_says_running_in_full() {
    let c = shell(&long_command(), None);
    for width in [40, 60, 100] {
        let out = rows(&c, &[], width);
        assert!(out[0].trim_end().ends_with("running"), "{width}: {out:#?}");
    }
}

#[test]
fn a_script_with_calls_says_what_it_called_never_its_source() {
    let code = "const r = await Promise.all([\n  tools.shell({command: \"a\"}),\n  tools.read_file({path: \"b\"}),\n]);\nreturn r;";
    let script = card("run_code", code, Some(true), "shell: exit 0", 590);
    let (a, b) = (
        shell("a", Some(true)),
        card("read_file", "{\"path\":\"b\"}", Some(true), "1 line", 3),
    );
    let out = rows(&script, &[&a, &b], 100);
    assert_eq!(out.len(), 1, "{out:#?}");
    assert!(out[0].contains("Script 2 calls: run, read"), "{out:#?}");
    assert!(out[0].contains("done · 590ms"), "{out:#?}");
    assert!(
        !out[0].contains("await") && !out[0].contains("shell: exit"),
        "{out:#?}"
    );
}

#[test]
fn a_script_without_calls_shows_its_first_lines_not_comments() {
    let code = "// compute it\n\nconst n = [1, 2, 3].reduce((a, b) => a + b, 0);\nreturn n;";
    let script = card("run_code", code, Some(true), "6", 4);
    let out = rows(&script, &[], 100);
    assert!(out[0].contains("const n = [1, 2, 3]"), "{out:#?}");
    assert!(out[1].ends_with("            return n;"), "{out:#?}");
    assert!(!out.iter().any(|r| r.contains("compute it")), "{out:#?}");
}
