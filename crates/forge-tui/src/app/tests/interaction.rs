use super::*;

// ── Next-prompt suggestion: SuggestionReady + Tab-accept + turn/session clearing ──────────
#[test]
fn suggestion_ready_sets_the_field() {
    let mut app = App::default();
    assert_eq!(app.suggested_prompt, None);
    app.apply(PresenterEvent::SuggestionReady {
        text: "add a test for this".into(),
    });
    assert_eq!(app.suggested_prompt.as_deref(), Some("add a test for this"));
}

#[test]
fn suggestion_is_cleared_on_new_turn() {
    let mut app = App {
        suggested_prompt: Some("stale from the last turn".into()),
        ..Default::default()
    };
    app.on_turn_start();
    assert_eq!(app.suggested_prompt, None);
}

#[test]
fn suggestion_is_cleared_on_session_switch() {
    let mut app = App {
        suggested_prompt: Some("stale from the old session".into()),
        ..Default::default()
    };
    app.clear_transcript();
    assert_eq!(app.suggested_prompt, None);
}

#[test]
fn take_suggestion_for_tab_fills_empty_input_and_clears_the_suggestion() {
    let mut app = App {
        suggested_prompt: Some("commit these changes".into()),
        ..Default::default()
    };
    assert_eq!(
        app.take_suggestion_for_tab().as_deref(),
        Some("commit these changes")
    );
    assert_eq!(app.suggested_prompt, None, "taken, so consumed");
}

#[test]
fn take_suggestion_for_tab_is_a_no_op_when_input_is_not_empty() {
    let mut app = App {
        input: "already typing".into(),
        suggested_prompt: Some("commit these changes".into()),
        ..Default::default()
    };
    assert_eq!(app.take_suggestion_for_tab(), None);
    // A stray Tab mid-sentence must never clobber the pending suggestion either.
    assert_eq!(
        app.suggested_prompt.as_deref(),
        Some("commit these changes")
    );
}

#[test]
fn take_suggestion_for_tab_is_a_no_op_when_there_is_no_suggestion() {
    let mut app = App::default();
    assert_eq!(app.take_suggestion_for_tab(), None);
}

#[test]
fn stale_multibyte_cursor_does_not_panic_during_render() {
    let app = App {
        input: "é你".into(),
        input_cursor: 1,
        ..Default::default()
    };
    let _ = screen_wh(&app, 20, 5);
}

#[test]
fn input_cursor_up_recalls_on_first_row_moves_otherwise() {
    // Single-line / first-row cursor → None (caller recalls history).
    assert_eq!(input_cursor_up("hello", 3), None);
    assert_eq!(input_cursor_up("", 0), None);
    // Multiline, cursor on the second line → moves up to the same column on line 1.
    // "abc\nxy|z" : cursor at byte 6 (col 2 on line 2) → line 1 col 2 = byte 2.
    assert_eq!(input_cursor_up("abc\nxyz", 6), Some(2));
    // Column clamped when the previous line is shorter.
    // "ab\nlongline|" cursor at end (col 8) → clamp to line-1 len (2) = byte 2.
    assert_eq!(input_cursor_up("ab\nlongline", 11), Some(2));
    // A stale cursor inside a multibyte scalar is sanitized before slicing.
    assert_eq!(input_cursor_up("é\n你", 4), Some(0));
}

#[test]
fn voice_waveform_scrolls_once_full() {
    let mut v = VoiceOverlay::recording(false);
    for i in 0..VOICE_WAVEFORM_BARS {
        v.push_level(i as f32 / VOICE_WAVEFORM_BARS as f32);
    }
    assert_eq!(v.waveform.len(), VOICE_WAVEFORM_BARS);
    assert_eq!(v.waveform.front().copied(), Some(0.0));

    // One more sample past capacity: the oldest (front) sample drops off, the ring stays
    // bounded, and the new sample lands at the back.
    v.push_level(0.5);
    assert_eq!(v.waveform.len(), VOICE_WAVEFORM_BARS);
    assert_eq!(
        v.waveform.front().copied(),
        Some(1.0 / VOICE_WAVEFORM_BARS as f32)
    );
    assert_eq!(v.waveform.back().copied(), Some(0.5));
}

#[test]
fn voice_waveform_clamps_out_of_range_levels() {
    let mut v = VoiceOverlay::recording(false);
    v.push_level(-0.5);
    v.push_level(1.7);
    assert_eq!(
        v.waveform.iter().copied().collect::<Vec<_>>(),
        vec![0.0, 1.0]
    );
}

#[test]
fn voice_mmss_formats_minutes_and_seconds() {
    let mut v = VoiceOverlay::recording(false);
    assert_eq!(v.mmss(), "00:00");
    v.elapsed_secs = 65;
    assert_eq!(v.mmss(), "01:05");
    v.elapsed_secs = 3661; // over an hour: minutes keep rolling, not clamped to 59
    assert_eq!(v.mmss(), "61:01");
}

#[test]
fn insert_voice_transcript_into_empty_input() {
    let mut input = String::new();
    let mut cursor = 0;
    let n = insert_voice_transcript(&mut input, &mut cursor, "hello world");
    assert_eq!(input, "hello world");
    assert_eq!(cursor, input.len());
    assert_eq!(n, 11);
}

#[test]
fn insert_voice_transcript_pads_a_joining_space_on_both_sides() {
    let mut input = "foobar".to_string();
    let mut cursor = 3; // between "foo" and "bar"
    let n = insert_voice_transcript(&mut input, &mut cursor, "hello");
    assert_eq!(input, "foo hello bar");
    assert_eq!(n, 5);
    // Cursor lands right after the inserted text (including its trailing pad space), i.e.
    // right before "bar" — not right after "hello".
    assert_eq!(cursor, "foo hello ".len());
}

#[test]
fn insert_voice_transcript_skips_padding_next_to_existing_whitespace() {
    // Cursor right after a trailing space: no double space on the left.
    let mut input = "hello ".to_string();
    let mut cursor = input.len();
    insert_voice_transcript(&mut input, &mut cursor, "world");
    assert_eq!(input, "hello world");

    // Cursor right before a leading space: no double space on the right.
    let mut input = " world".to_string();
    let mut cursor = 0;
    insert_voice_transcript(&mut input, &mut cursor, "hello");
    assert_eq!(input, "hello world");
}

#[test]
fn insert_voice_transcript_trims_and_ignores_blank_text() {
    let mut input = "abc".to_string();
    let mut cursor = 3;
    let n = insert_voice_transcript(&mut input, &mut cursor, "   ");
    assert_eq!(input, "abc");
    assert_eq!(cursor, 3);
    assert_eq!(n, 0);

    let mut input = String::new();
    let mut cursor = 0;
    let n = insert_voice_transcript(&mut input, &mut cursor, "  hi  ");
    assert_eq!(input, "hi");
    assert_eq!(n, 2);
}

#[test]
fn insert_voice_transcript_clamps_a_stale_out_of_bounds_cursor() {
    let mut input = "abc".to_string();
    let mut cursor = 99;
    insert_voice_transcript(&mut input, &mut cursor, "x");
    assert_eq!(input, "abc x");
    assert_eq!(cursor, input.len());
}

#[test]
fn permission_controls_stay_visible_on_a_narrow_terminal() {
    let app = App {
        prompt: Some("allow write_file (FileWrite) [y/n/a=always]".into()),
        ..Default::default()
    };
    let screen = screen_wh(&app, 30, LIVE_H);
    assert_eq!(prompt_height(&app), PERMISSION_H);
    assert!(screen.contains("[y]es"), "allow stays visible: {screen}");
    assert!(
        screen.contains("[a]lways"),
        "always stays visible: {screen}"
    );
    assert!(screen.contains("[n]o"), "deny stays visible: {screen}");
}

#[test]
fn question_response_never_looks_like_a_permission_prompt() {
    let mut app = App::default();
    app.set_question(
        "Which migration should Forge run?",
        &[QChoice {
            label: "Safe migration".into(),
            description: "Keeps existing session data.".into(),
        }],
        false,
    );
    let screen = screen_wh(&app, 80, LIVE_H);
    assert_eq!(prompt_height(&app), 0, "the form is not a permission bar");
    assert!(
        screen.contains("Which migration should Forge run?"),
        "question rendered in the live region: {screen}"
    );
    assert!(
        screen.contains("Safe migration"),
        "options rendered as rows: {screen}"
    );
    assert!(
        screen.contains("answer"),
        "composer has answer affordance: {screen}"
    );
    assert!(
        !screen.contains("[y]es"),
        "no false permission key: {screen}"
    );
    assert!(
        !screen.contains("[a]lways"),
        "no false always key: {screen}"
    );
    assert!(!screen.contains("[n]o"), "no false deny key: {screen}");
}

#[test]
fn queued_images_follow_their_prompt_and_vanish_with_it() {
    let img = |d: &str| forge_types::ImageAttachment {
        media_type: "image/png".to_string(),
        data_base64: d.to_string(),
    };
    let mut app = App::default();
    app.stash_queued_images("look at this", vec![img("a")]);
    app.stash_queued_images("and this", vec![img("b")]);
    app.set_queued(&["look at this".to_string(), "and this".to_string()]);
    app.promote_queued_images("and this");
    assert_eq!(app.take_ready_images(), vec![img("b")]);
    assert!(app.take_ready_images().is_empty());
    // A prompt that left the queue without running drops its images.
    app.set_queued(&[]);
    app.promote_queued_images("look at this");
    assert!(app.take_ready_images().is_empty());
}

#[test]
fn resolve_paste_blocks_substitutes_text_and_extracts_images() {
    let mut app = App::default();
    app.input.clear();
    app.input_cursor = 0;
    // Type "see ", paste a multiline text block, type " and ", attach an image.
    for c in "see ".chars() {
        handle_key(&mut app.input, &mut app.input_cursor, KeyKind::Char(c));
    }
    app.handle_paste("line1\nline2\nline3".to_string());
    for c in " and ".chars() {
        handle_key(&mut app.input, &mut app.input_cursor, KeyKind::Char(c));
    }
    app.attach_image(
        forge_types::ImageAttachment {
            media_type: "image/png".to_string(),
            data_base64: "Zm9v".to_string(),
        },
        "PNG 2x2",
    );
    let raw = std::mem::take(&mut app.input);
    let (resolved, images) = app.resolve_paste_blocks(raw);
    // Text block restored inline; image placeholder stripped; image returned out-of-band.
    assert_eq!(resolved, "see line1\nline2\nline3 and ");
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].data_base64, "Zm9v");
    // Blocks are drained.
    let (again, imgs2) = app.resolve_paste_blocks("nothing".to_string());
    assert_eq!(again, "nothing");
    assert!(imgs2.is_empty());
}

#[test]
fn submitted_image_shows_a_marker_in_history() {
    let mut app = App::default();
    app.attach_image(
        forge_types::ImageAttachment {
            media_type: "image/png".to_string(),
            data_base64: "Zm9vYmFy".to_string(),
        },
        "PNG 4x4",
    );
    let raw = std::mem::take(&mut app.input);
    let (_text, images) = app.resolve_paste_blocks(raw);
    assert_eq!(images.len(), 1);
    app.submit_user("look at this");
    let echoed = flush_text(&mut app);
    assert!(echoed.contains("look at this"));
    assert!(
        echoed.contains("🖼 image attached"),
        "history should mark the attached image, got: {echoed}"
    );
    // Marker is one-shot: a later plain turn doesn't repeat it.
    app.submit_user("next");
    let again = flush_text(&mut app);
    assert!(!again.contains("🖼"));
}

#[test]
fn paste_strips_terminal_controls_and_normalizes_carriage_returns() {
    let mut app = App::default();
    app.handle_paste("safe\x1b[31m\rnext\x07".into());
    assert_eq!(app.input, "[pasted text (2 lines)]");
    let placeholder = app.input.clone();
    let (resolved, _) = app.resolve_paste_blocks(placeholder);
    assert_eq!(resolved, "safe[31m\nnext");
    assert!(app.input.is_char_boundary(app.input_cursor));
}

#[test]
fn paste_collapses_crlf_instead_of_doubling_newlines() {
    // A Windows-origin "\r\n" must become a single "\n", not "\n\n" (a spurious blank line).
    let mut app = App::default();
    app.handle_paste("foo\r\nbar".into());
    assert_eq!(app.input, "[pasted text (2 lines)]");
    let (resolved, _) = app.resolve_paste_blocks(app.input.clone());
    assert_eq!(resolved, "foo\nbar");
}

#[test]
fn streamed_text_cannot_inject_terminal_controls() {
    let mut app = App::default();
    app.apply(PresenterEvent::AssistantDelta(
        "hello\x1b[2J\x07world".into(),
    ));
    assert_eq!(app.streaming, "hello[2Jworld");
    app.apply(PresenterEvent::Warning("warn\x1b[31m".into()));
    let rendered: String = app
        .drain_flush()
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect();
    assert!(!rendered.contains('\x1b'));
}

#[test]
fn paste_into_empty_input_with_stale_cursor_does_not_panic() {
    // Regression: input_cursor could outlive the input contents (e.g. after a submit clears
    // input but a key path left the cursor > 0), making a `&input[..cursor]` slice panic.
    let mut app = App::default();
    app.input.clear();
    app.input_cursor = 5; // stale: past end of empty input
    app.handle_paste("x".to_string());
    app.input_cursor = 5;
    app.handle_paste("a\nb\nc".to_string());
    let _ = app.try_delete_paste_block(KeyKind::Backspace);
    let _ = app.try_delete_paste_block(KeyKind::DeleteForward);
}

#[test]
fn input_handling_never_panics_and_keeps_cursor_valid() {
    // Deterministic fuzz: drive the input line with a long pseudo-random sequence of edits,
    // pastes (single- and multi-line, multibyte) and paste-block deletes, asserting after
    // every op that the cursor stays in-bounds and on a char boundary. No rand crate — a
    // small LCG keeps it reproducible. Guards the class of panic the paste crash belonged to.
    let mut app = App::default();
    let chars = ['a', 'é', '你', '🦀', ' ', '\n', '/'];
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut next = |n: u64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) % n
    };
    for _ in 0..20_000 {
        match next(10) {
            0..=4 => {
                let c = chars[next(chars.len() as u64) as usize];
                let key = if c == '\n' {
                    KeyKind::InsertNewline
                } else {
                    KeyKind::Char(c)
                };
                let _ = handle_key(&mut app.input, &mut app.input_cursor, key);
            }
            5 => {
                let key = match next(9) {
                    0 => KeyKind::Backspace,
                    1 => KeyKind::DeleteForward,
                    2 => KeyKind::Left,
                    3 => KeyKind::Right,
                    4 => KeyKind::Home,
                    5 => KeyKind::End,
                    6 => KeyKind::DeleteWordBack,
                    7 => KeyKind::KillLineBack,
                    _ => KeyKind::KillLineForward,
                };
                let _ = handle_key(&mut app.input, &mut app.input_cursor, key);
            }
            6 => app.handle_paste("single line paste".to_string()),
            7 => app.handle_paste("multi\nline\né你🦀\npaste".to_string()),
            8 => {
                let _ = app.try_delete_paste_block(KeyKind::Backspace);
            }
            _ => {
                // Occasionally desync the cursor to simulate the bug's precondition.
                app.input_cursor = app.input_cursor.wrapping_add(next(4) as usize);
                let _ = app.try_delete_paste_block(KeyKind::DeleteForward);
            }
        }
        assert!(
            app.input_cursor <= app.input.len(),
            "cursor {} > len {}",
            app.input_cursor,
            app.input.len()
        );
        assert!(
            app.input.is_char_boundary(app.input_cursor),
            "cursor {} not on a char boundary in {:?}",
            app.input_cursor,
            app.input
        );
    }
}

#[test]
fn question_renders_options_to_scrollback_and_resolves_an_answer() {
    let mut app = App::default();
    let options = vec![
        QChoice {
            label: "Postgres".into(),
            description: "relational".into(),
        },
        QChoice {
            label: "SQLite".into(),
            description: String::new(),
        },
    ];
    app.set_question("which database?", &options, true);
    assert!(app.awaiting_question());
    let screen = screen_wh(&app, 80, LIVE_H);
    assert!(
        screen.contains("which database?"),
        "question shown in the live region: {screen}"
    );
    assert!(
        screen.contains("1 Postgres") && screen.contains("2 SQLite"),
        "options numbered: {screen}"
    );
    assert!(
        screen.contains("Other"),
        "free text allowed → Other row: {screen}"
    );

    // A number selects (remote/legacy text path); the question clears and the record lands
    // in scrollback.
    assert_eq!(app.resolve_question("2").as_deref(), Some("SQLite"));
    assert!(!app.awaiting_question());
    let sb = flush_text(&mut app);
    assert!(
        sb.contains("which database?") && sb.contains("SQLite"),
        "answered question recorded: {sb}"
    );

    // Invalid input keeps the question open (None).
    app.set_question("again?", &options, false);
    assert_eq!(app.resolve_question("not-a-number"), None);
    assert!(
        app.awaiting_question(),
        "invalid answer keeps the question open"
    );
}

#[test]
fn question_form_renders_tabs_checkboxes_other_and_note_rows() {
    let mut app = App::default();
    let opt = |l: &str, d: &str| QChoice {
        label: l.into(),
        description: d.into(),
    };
    app.open_form(vec![
        forge_types::Question {
            header: "Database".into(),
            ..forge_types::Question::single(
                "Which database should sessions use?",
                &[opt("Postgres", "recommended"), opt("SQLite", "single file")],
                true,
            )
        },
        forge_types::Question {
            header: "Scope".into(),
            multi: true,
            allow_note: true,
            ..forge_types::Question::single(
                "Which features go in the MVP?",
                &[opt("Auth", ""), opt("Billing", "stripe"), opt("Search", "")],
                true,
            )
        },
    ]);
    // Answer question 1 with Enter, toggle two boxes on question 2, start a note.
    assert_eq!(app.form_key(KeyKind::Enter), Some(crate::FormOutcome::Open));
    app.form_key(KeyKind::Char('1'));
    app.form_key(KeyKind::Char('3'));
    app.form_key(KeyKind::Char('n'));
    for c in "later".chars() {
        app.form_key(KeyKind::Char(c));
    }
    let screen = screen_wh(&app, 90, LIVE_H);
    println!("{screen}");
    assert!(screen.contains("question 2 of 2"), "{screen}");
    assert!(
        screen.contains("✓ Database") && screen.contains("✓ Scope"),
        "tab strip: {screen}"
    );
    assert!(
        screen.contains("☑ 1 Auth") && screen.contains("☐ 2 Billing"),
        "{screen}"
    );
    assert!(screen.contains("☑ 3 Search"), "{screen}");
    assert!(screen.contains("Other…"), "{screen}");
    assert!(screen.contains("note later"), "inline note field: {screen}");
    assert!(screen.contains("choose all that apply"), "{screen}");
    // Enter saves the note; Enter again (on the note row, answered) submits.
    assert_eq!(app.form_key(KeyKind::Enter), Some(crate::FormOutcome::Open));
    match app.form_key(KeyKind::Enter) {
        Some(crate::FormOutcome::Submit(a)) => {
            assert_eq!(a[0].selected, ["Postgres"]);
            assert_eq!(a[1].selected, ["Auth", "Search"]);
            assert_eq!(a[1].note.as_deref(), Some("later"));
        }
        other => panic!("{other:?}"),
    }
    assert!(!app.awaiting_question());
    let sb = flush_text(&mut app);
    assert!(sb.contains("Auth, Search — note: later"), "{sb}");
}

#[test]
fn command_palette_renders_filtered_commands() {
    let mut app = App::default();
    app.palette.open_with("");
    app.palette.anim = 1.0; // fully revealed
    let text = screen(&app);
    assert!(text.contains("/help"), "palette shows commands: {text}");
    assert!(text.contains("▸"), "selected row marked");
}

#[test]
fn picker_renders_heading_rows_and_selection() {
    use crate::commands::{PickerKind, PickerRow};
    let mut app = App::default();
    app.picker.open_with(
        PickerKind::Sessions,
        "resume a session",
        vec![PickerRow {
            id: "aaa".into(),
            title: "aaa  $0.01  2 msgs".into(),
            subtitle: "fix the auth bug".into(),
        }],
    );
    app.picker.anim = 1.0;
    let text = screen(&app);
    assert!(text.contains("resume a session"), "heading shown: {text}");
    assert!(text.contains("fix the auth bug"), "row subtitle shown");
    assert!(text.contains('▸'), "selected row marked");
}

#[test]
fn command_center_and_empty_chat_are_discoverable() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    let idle = screen_wh(&app, 100, 30);
    assert!(
        idle.contains("Explain this codebase"),
        "starter shown: {idle}"
    );
    assert!(
        idle.contains("Ctrl+K for all actions"),
        "command hint shown: {idle}"
    );
    assert!(
        idle.contains("Check readiness with forge doctor"),
        "setup recovery action shown: {idle}"
    );
    assert!(
        idle.contains("Preview routing with /mesh"),
        "routing discovery action shown: {idle}"
    );

    app.command_center.open();
    let center = screen_wh(&app, 100, 30);
    assert!(center.contains("Command center"), "title shown: {center}");
    assert!(center.contains("Start work"), "categories shown: {center}");
    assert!(center.contains("/plan"), "commands shown: {center}");
}

#[test]
fn picker_zero_height_does_not_panic() {
    use crate::commands::PickerKind;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let mut app = App::default();
    app.picker.open_with(PickerKind::Sessions, "resume", vec![]);
    let mut term = Terminal::new(TestBackend::new(80, 0)).unwrap();
    let _ = term.draw(|f| render_live(f, &app));
}

#[test]
fn command_palette_zero_height_does_not_panic() {
    // Regression: clamp(1, 0) panicked on a 0-height viewport.
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let mut app = App::default();
    app.palette.open_with("");
    let mut term = Terminal::new(TestBackend::new(80, 0)).unwrap();
    // Must not panic.
    let _ = term.draw(|f| render_live(f, &app));
}

#[test]
fn input_box_grows_with_wrapped_content_then_caps() {
    // Empty input → minimum height. A very long single line → wraps and grows, capped.
    assert_eq!(input_box_height("", 80), INPUT_H);
    let long = "x".repeat(80 * 10); // far exceeds the cap
    assert_eq!(input_box_height(&long, 80), INPUT_MAX_H);
    // A short line stays at the minimum.
    assert_eq!(input_box_height("hello", 80), INPUT_H);
}

#[test]
fn input_text_rows_counts_cell_width_not_chars() {
    // 40 CJK glyphs are 80 terminal cells, so they wrap to more rows than 40 ascii chars (40
    // cells) at the same width — counting `chars()` would tie them and hide the cursor.
    let wide: String = "世".repeat(40);
    let narrow = "x".repeat(40);
    assert!(
        input_text_rows(&wide, 80) > input_text_rows(&narrow, 80),
        "wide glyphs must occupy more wrapped rows than the same count of ascii"
    );
}

#[test]
fn typing_a_char_appends_and_keeps_editing() {
    let mut buf = String::new();
    let mut cur = 0usize;
    assert_eq!(
        handle_key(&mut buf, &mut cur, KeyKind::Char('h')),
        InputOutcome::Editing
    );
    assert_eq!(
        handle_key(&mut buf, &mut cur, KeyKind::Char('i')),
        InputOutcome::Editing
    );
    assert_eq!(buf, "hi");
    assert_eq!(cur, 2);
}

#[test]
fn backspace_removes_last_char() {
    let mut buf = "abc".to_string();
    let mut cur = 3usize;
    assert_eq!(
        handle_key(&mut buf, &mut cur, KeyKind::Backspace),
        InputOutcome::Editing
    );
    assert_eq!(buf, "ab");
    assert_eq!(cur, 2);
}

#[test]
fn enter_submits_and_clears_buffer() {
    let mut buf = "do it".to_string();
    let mut cur = buf.len();
    assert_eq!(
        handle_key(&mut buf, &mut cur, KeyKind::Enter),
        InputOutcome::Submit("do it".into())
    );
    assert_eq!(buf, "", "buffer cleared after submit");
    assert_eq!(cur, 0);
}

#[test]
fn enter_on_empty_buffer_keeps_editing() {
    let mut buf = "   ".to_string();
    let mut cur = buf.len();
    assert_eq!(
        handle_key(&mut buf, &mut cur, KeyKind::Enter),
        InputOutcome::Editing
    );
}

#[test]
fn esc_quits() {
    let mut buf = "whatever".to_string();
    let mut cur = buf.len();
    assert_eq!(
        handle_key(&mut buf, &mut cur, KeyKind::Esc),
        InputOutcome::Quit
    );
}

#[test]
fn left_right_move_cursor() {
    let mut buf = "abc".to_string();
    let mut cur = 3usize;
    handle_key(&mut buf, &mut cur, KeyKind::Left);
    assert_eq!(cur, 2);
    handle_key(&mut buf, &mut cur, KeyKind::Left);
    assert_eq!(cur, 1);
    handle_key(&mut buf, &mut cur, KeyKind::Right);
    assert_eq!(cur, 2);
}

#[test]
fn ctrl_j_inserts_newline_without_submit() {
    let mut buf = "hello".to_string();
    let mut cur = buf.len();
    assert_eq!(
        handle_key(&mut buf, &mut cur, KeyKind::InsertNewline),
        InputOutcome::Editing
    );
    assert_eq!(buf, "hello\n");
    assert_eq!(cur, 6);
}

#[test]
fn input_highlights_slash_command_mid_line() {
    let app = App {
        input: "please run /orchestrate scan".to_string(),
        ..Default::default()
    };
    let spans = input_spans(&app.input);
    // The `/orchestrate` token is its own bold-orange span.
    let hi = spans
        .iter()
        .find(|s| s.content.contains("/orchestrate"))
        .expect("slash token has its own span");
    assert!(
        hi.style
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD),
        "command token is highlighted bold"
    );
    // The preceding prose is a separate, unstyled span.
    assert!(
        spans.iter().any(|s| s.content == "please run "),
        "prose kept verbatim before the token"
    );
}

#[test]
fn input_does_not_highlight_double_slash_escape() {
    let spans = input_spans("//literal text");
    // No bold command span — the whole thing is plain (escape preserved).
    assert!(
        !spans.iter().any(|s| s
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD)),
        "// escape is not highlighted as a command"
    );
}

#[test]
fn composer_cursor_moves_by_grapheme_cluster() {
    let s = "a👨‍👩‍👧e\u{301}b";
    let family_end = "a👨‍👩‍👧".len();
    assert_eq!(crate::app::next_char_boundary(s, 1), family_end);
    assert_eq!(crate::app::prev_char_boundary(s, family_end), 1);
    let accent_end = family_end + "e\u{301}".len();
    assert_eq!(crate::app::next_char_boundary(s, family_end), accent_end);
    assert_eq!(crate::app::prev_char_boundary(s, accent_end), family_end);
    assert_eq!(crate::app::prev_char_boundary(s, 0), 0);
    assert_eq!(crate::app::next_char_boundary(s, s.len()), s.len());
}
