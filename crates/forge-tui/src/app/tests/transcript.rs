use super::*;
use crate::theme::color;

#[test]
fn error_event_renders_distinct_red_glyph_from_warning() {
    let mut app = App::default();
    app.apply(PresenterEvent::Error("provider hard-fail".into()));
    let lines = app.drain_flush();
    let line = lines.first().expect("error line flushed");
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.contains('✗'), "distinct error glyph: {text}");
    assert!(text.contains("provider hard-fail"));
    // Rendered in the error red, not the benign warning yellow.
    assert!(
        line.spans.iter().any(|s| s.style.fg == Some(color::err())),
        "error styled red"
    );

    // A Warning still uses the benign yellow ▲ — the two are visually distinct.
    let mut app2 = App::default();
    app2.apply(PresenterEvent::Warning("heads up".into()));
    let wlines = app2.drain_flush();
    let wtext: String = wlines[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(wtext.contains('▲') && !wtext.contains('✗'));
}

#[test]
fn reasoning_collapses_to_discoverable_marker_when_hidden() {
    let mut app = App::default();
    assert!(!app.show_thinking, "hidden by default");
    app.apply(PresenterEvent::Reasoning("deep thoughts".into()));
    app.apply(PresenterEvent::AssistantDone);
    let text = flush_text(&mut app);
    // Collapsed: a discoverability marker, not the raw reasoning text, not silence.
    assert!(text.contains("thinking"), "collapsed marker shown: {text}");
    assert!(
        text.contains("/thinking"),
        "tells the user how to expand: {text}"
    );
    assert!(
        !text.contains("deep thoughts"),
        "raw reasoning hidden when collapsed"
    );
}

#[test]
fn previous_message_jump_walks_user_messages_upward() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    for i in 0..3 {
        app.submit_user(&format!("prompt {i}"));
        app.apply(PresenterEvent::AssistantText("a b c d".into()));
        let _ = app.drain_flush();
    }
    // Render once (tail) so the geometry exists; the view is at the bottom.
    // A short frame (body 8 rows under the 5 rows of chrome) so the 18 transcript rows scroll and older prompts sit above.
    app.transcript_to_bottom();
    let _ = screen_wh(&app, 80, 13);
    assert!(
        app.has_user_message_above(),
        "older prompts sit above the tail view: geom={:?} rows={:?}",
        app.transcript_geom.get(),
        app.user_message_rows()
    );
    assert!(app.transcript_to_prev_user());
    let first = app.transcript_scroll;
    let _ = screen_wh(&app, 80, 13);
    assert!(app.transcript_to_prev_user(), "again → the one before");
    assert!(app.transcript_scroll < first);
    assert_eq!(
        app.transcript_scroll, 0,
        "two presses from the tail reach the first prompt (rows 0 and 6 are above; 12 is in view)"
    );
    let _ = screen_wh(&app, 80, 13);
    assert!(!app.has_user_message_above());
    assert!(
        !app.transcript_to_prev_user(),
        "nothing above the first prompt"
    );
    // The bar is drawn only while there is somewhere to go.
    app.transcript_to_bottom();
    let screen = screen_wh(&app, 80, 13);
    assert!(screen.contains("Previous message"), "{screen}");
    assert!(
        app.prev_bar_hit(79 - 10, 1),
        "the bar sits on the first row under the header"
    );
}

#[test]
fn user_message_is_queued_to_scrollback() {
    let mut app = App::default();
    app.submit_user("my own task");
    let text = flush_text(&mut app);
    assert!(text.contains("you"));
    assert!(text.contains("my own task"));
}

#[test]
fn job_notice_renders_as_a_harness_notice_not_a_user_message() {
    let mut app = App::default();
    app.submit_user(NOTICE);
    let text = drain_text_lines(&mut app);
    assert!(!text.contains("you"), "{text}");
    assert!(!text.contains("[background job"), "{text}");
    assert_eq!(
        text.trim_end(),
        "  ⚙ job 3429249 ✓ exited 0 · /tmp/batch.sh > /tmp/batch_result.log 2>&1"
    );
}

#[test]
fn a_resumed_fullscreen_transcript_shows_tool_cards_not_raw_json() {
    let mut app = App {
        fullscreen: true,
        ..App::default()
    };
    app.replay_history(&[
        ReplayItem::Tool {
            name: "read_file".into(),
            args: r#"{"path":"a.rs"}"#.into(),
        },
        ReplayItem::ToolResult {
            name: "read_file".into(),
            ok: true,
            summary: "12 lines".into(),
        },
    ]);
    assert_eq!(app.tool_cards.len(), 1);
    assert_eq!(app.tool_cards[0].status, CardStatus::Ok);
}

#[test]
fn a_steer_closes_the_text_streamed_before_it() {
    let mut app = App::default();
    app.apply(PresenterEvent::AssistantDelta("first answer".into()));
    app.apply(PresenterEvent::Steered("also rename it".into()));
    assert!(!app.streaming_active && app.streaming.is_empty());
    app.apply(PresenterEvent::AssistantDelta("second answer".into()));
    let text = drain_text_lines(&mut app);
    let (first, steer, second) = (
        text.find("first answer").unwrap(),
        text.find("also rename it").unwrap(),
        text.find("second answer"),
    );
    assert!(first < steer && second.is_none());
}

#[test]
fn failed_coalesced_job_notices_render_one_header_each_with_dim_tail() {
    let mut app = App::default();
    let prompt = format!(
        "[2 earlier background-job notice(s) were dropped; check `shell_job` list]\n\n[background job 9 `make` exited 2]\nlast lines:\nerror: boom\n\n{NOTICE}"
    );
    app.apply(PresenterEvent::Steered(prompt));
    let text = drain_text_lines(&mut app);
    assert!(!text.contains("steer") && !text.contains("you"), "{text}");
    assert!(text.contains("⚙ 2 earlier job notice(s) dropped"), "{text}");
    assert!(
        text.contains("⚙ job 9 ✗ exited 2 · make\n      error: boom"),
        "{text}"
    );
    assert!(text.contains("⚙ job 3429249 ✓ exited 0"), "{text}");
}

#[test]
fn wakeup_renders_as_a_dim_harness_row_with_the_reason_underneath() {
    let mut app = App::default();
    let stored = forge_types::format_wakeup_prompt(
        "Check the probe file\nsecond line\nthird line\nfourth line",
        "Waiting for the probe answer",
    );
    app.submit_user(&forge_types::format_wakeup_delivery(&stored));
    let text = drain_text_lines(&mut app);
    assert!(
        !text.contains("you") && !text.contains("[scheduled wakeup]"),
        "{text}"
    );
    assert!(text.contains("⏰ wakeup · Check the probe file"), "{text}");
    assert!(text.contains("second line"), "{text}");
    assert!(!text.contains("third line") && text.contains("…"), "{text}");
    assert!(
        text.contains("because: Waiting for the probe answer"),
        "{text}"
    );
}

#[test]
fn heartbeat_renders_with_its_label_via_the_steer_path() {
    let mut app = App::default();
    app.apply(PresenterEvent::Steered(
        forge_types::format_heartbeat_delivery(Some("ci-watch"), "check CI"),
    ));
    let text = drain_text_lines(&mut app);
    assert!(text.contains("♻ heartbeat ci-watch · check CI"), "{text}");
    assert!(!text.contains("steer") && !text.contains("you"), "{text}");
}

#[test]
fn assistant_text_is_queued_to_scrollback() {
    let mut app = App::default();
    app.apply(PresenterEvent::AssistantText(
        "the workspace looks healthy".into(),
    ));
    assert!(flush_text(&mut app).contains("the workspace looks healthy"));
}

#[test]
fn a_finished_tool_call_occupies_one_transcript_row_not_two() {
    // The old rendering emitted a `↳ name {json…}` line and, later, a separate `✓ name …`
    // line: the reader had to pair them up by eye. One card owns both halves.
    let mut app = card_app();
    run_a_tool(&mut app, "shell", r#"{"command":"cargo test"}"#, None);
    assert_eq!(app.main_log.len(), 1, "{:?}", main_log_text(&app));
    let text = main_log_text(&app);
    assert!(
        text.contains("cargo test") && text.contains("exit 0 in 12ms"),
        "{text}"
    );
}

#[test]
fn clicking_a_card_expands_it_in_place_and_clicking_again_collapses_it() {
    let mut app = card_app();
    run_a_tool(
        &mut app,
        "shell",
        r#"{"command":"cargo test","cwd":"/tmp/x"}"#,
        Some("running 3 tests\nall passed"),
    );
    let collapsed = app.main_log.len();

    assert!(app.toggle_tool_card_at(4, 0), "the row is a click target");
    assert!(app.main_log.len() > collapsed, "expanding adds rows");
    let text = main_log_text(&app);
    assert!(text.contains("all passed"), "output is revealed: {text}");
    assert!(text.contains("/tmp/x"), "arguments are revealed: {text}");

    assert!(app.toggle_tool_card_at(4, 0), "clicking again toggles back");
    assert_eq!(app.main_log.len(), collapsed, "collapsing restores the row");
    assert!(!main_log_text(&app).contains("all passed"));
}

#[test]
fn a_tool_that_finishes_inside_one_frame_updates_its_own_row() {
    // Found live: a fast tool starts and finishes between two frames, so its card is still
    // queued in the flush when the result lands. Rewriting `main_log` at the card's unset
    // position inserted a SECOND copy of the row at the very top of the transcript while the
    // queued row stayed "running" for the rest of the session.
    let mut app = card_app();
    app.apply(PresenterEvent::ToolStart {
        name: "write_file".into(),
        args: r#"{"path":"note.txt"}"#.into(),
    });
    app.apply(PresenterEvent::ToolResult {
        name: "write_file".into(),
        ok: true,
        summary: "wrote 26 bytes".into(),
        detail: None,
    });
    let flushed = flush_text(&mut app);
    assert!(flushed.contains("wrote 26 bytes"), "{flushed}");
    assert!(
        !flushed.contains("running"),
        "the row is not left mid-call: {flushed}"
    );
    assert_eq!(
        app.main_log.len(),
        1,
        "one row, not a duplicate: {:?}",
        main_log_text(&app)
    );
    // And the card is still anchored correctly afterwards, so it stays clickable.
    assert!(app.toggle_tool_card_at(4, 0));
    assert!(app.tool_cards[0].expanded);
}

#[test]
fn a_click_outside_any_card_is_not_swallowed() {
    // The shell treats a `false` return as "start a text selection here" — a click on
    // ordinary transcript text must keep selecting, not silently do nothing.
    let mut app = card_app();
    app.apply(PresenterEvent::AssistantText("just prose".into()));
    let _ = app.drain_flush();
    assert!(!app.toggle_tool_card_at(4, 0));
}

#[test]
fn expanding_an_earlier_card_leaves_the_later_ones_clickable() {
    // Cards track their own position in the log; splicing a taller card in must shift the
    // ones after it, or every later click lands on the wrong card (or on nothing).
    let mut app = card_app();
    run_a_tool(&mut app, "shell", r#"{"command":"first"}"#, Some("out one"));
    run_a_tool(
        &mut app,
        "shell",
        r#"{"command":"second"}"#,
        Some("out two"),
    );
    let second_row = (app.tool_cards[1].start) as u16;

    assert!(app.toggle_tool_card_at(4, 0), "expand the first card");
    let grew = app.main_log.len() - 2;
    let moved = app.tool_cards[1].start as u16;
    assert_eq!(moved, second_row + grew as u16, "the later card moved down");

    assert!(app.toggle_tool_card_at(4, moved), "and is still clickable");
    assert!(app.tool_cards[1].expanded);
    assert!(main_log_text(&app).contains("out two"));
}

#[test]
fn ctrl_t_expands_the_most_recent_call_without_a_mouse() {
    let mut app = card_app();
    run_a_tool(&mut app, "shell", r#"{"command":"first"}"#, Some("out one"));
    run_a_tool(
        &mut app,
        "shell",
        r#"{"command":"second"}"#,
        Some("out two"),
    );
    assert!(app.toggle_last_tool_card());
    assert!(app.tool_cards[1].expanded && !app.tool_cards[0].expanded);
    // Nothing to expand on a fresh app — the shell shows a note instead of doing nothing.
    assert!(!App::default().toggle_last_tool_card());
}

#[test]
fn inline_mode_keeps_the_two_line_rendering() {
    // Inline prints into the terminal's own scrollback, which cannot be rewritten later, so
    // a card there would be a control that never responds. It must stay plain lines.
    let mut app = App::default(); // fullscreen defaults to false
    app.apply(PresenterEvent::ToolStart {
        name: "shell".into(),
        args: r#"{"command":"ls"}"#.into(),
    });
    app.apply(PresenterEvent::ToolResult {
        name: "shell".into(),
        ok: true,
        summary: "shell: exit 0".into(),
        detail: Some("a.txt".into()),
    });
    let text = flush_text(&mut app);
    assert!(app.tool_cards.is_empty(), "no cards in inline mode");
    assert!(text.contains("↳"), "the classic call line: {text}");
    assert!(text.contains("exit 0"));
}

#[test]
fn a_result_with_no_matching_start_still_renders() {
    // Some refusals emit a result without a start (the call never ran). Falling back to the
    // plain line keeps the outcome visible instead of dropping it on the floor.
    let mut app = card_app();
    app.apply(PresenterEvent::ToolResult {
        name: "shell".into(),
        ok: false,
        summary: "permission denied by policy".into(),
        detail: None,
    });
    assert!(flush_text(&mut app).contains("permission denied by policy"));
}

#[test]
fn tool_invocation_is_queued_to_scrollback() {
    let mut app = App::default();
    app.apply(PresenterEvent::ToolStart {
        name: "read_file".into(),
        args: "{\"path\":\"Cargo.toml\"}".into(),
    });
    app.apply(PresenterEvent::ToolResult {
        name: "read_file".into(),
        ok: true,
        summary: "[workspace]".into(),
        detail: None,
    });
    assert!(flush_text(&mut app).contains("read_file"));
}

#[test]
fn budget_warning_is_queued_to_scrollback() {
    let mut app = App::default();
    app.apply(PresenterEvent::Warning(
        "approaching daily budget cap".into(),
    ));
    assert!(flush_text(&mut app).contains("approaching daily budget cap"));
}

#[test]
fn streaming_accumulates_and_shows_live_until_done() {
    let mut app = App::default();
    app.apply(PresenterEvent::AssistantDelta("first line\nsecond ".into()));
    // The header is queued on the first delta; the body accumulates live (rendered as
    // markdown only on Done, so multi-line blocks stay whole).
    assert!(
        flush_text(&mut app).contains("⚒ forge"),
        "header flushed on first delta"
    );
    assert_eq!(
        app.streaming, "first line\nsecond ",
        "reply accumulates live"
    );
    assert!(screen(&app).contains("second"), "tail shown in preview");
    assert!(screen(&app).contains('▌'), "cursor shown while streaming");
}

#[test]
fn assistant_done_renders_reply_to_scrollback() {
    let mut app = App::default();
    app.apply(PresenterEvent::AssistantDelta("committed text".into()));
    app.apply(PresenterEvent::AssistantDone);
    assert!(app.streaming.is_empty(), "streaming buffer cleared");
    assert!(flush_text(&mut app).contains("committed text"));
}

#[test]
fn model_search_shows_one_indicator_and_clears_on_output() {
    // Failover must drive a SINGLE animated live indicator, not one scrollback warning per
    // hop. The indicator clears the moment real output begins.
    let mut app = App::default();
    app.apply(PresenterEvent::ModelSearch {
        model: "groq::llama-3.3-70b-versatile".into(),
        retrying: false,
    });
    assert!(app.model_search.is_some());
    assert!(
        screen(&app).contains("Finding a model"),
        "animated search indicator shown on the composer border"
    );
    // The failed model id is NOT spammed into scrollback.
    assert!(
        !flush_text(&mut app).contains("groq::llama-3.3-70b-versatile"),
        "no per-hop failover line flushed to scrollback"
    );
    // Output settles it.
    app.apply(PresenterEvent::AssistantDelta("hello".into()));
    assert!(
        app.model_search.is_none(),
        "indicator cleared once output begins"
    );
    assert!(!screen(&app).contains("Finding a model"));
}

#[test]
fn assistant_markdown_is_rendered_not_literal() {
    let mut app = App::default();
    app.apply(PresenterEvent::AssistantDelta(
        "## Plan\n\n- do **it**\n".into(),
    ));
    app.apply(PresenterEvent::AssistantDone);
    let text = flush_text(&mut app);
    assert!(
        text.contains("Plan") && !text.contains("##"),
        "heading rendered: {text:?}"
    );
    assert!(text.contains("• do it"), "bullet + stripped bold: {text:?}");
}

#[test]
fn shell_diagnosis_card_shows_the_fix_without_a_key_hint() {
    let text: String = shell_diagnosis_lines("cargo bild", "typo", Some("cargo build"), 100)
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("fix: cargo build"), "{text}");
    assert!(!text.contains("press F"), "{text}");
}

#[test]
fn viewer_down_at_tail_reenables_follow() {
    // Bug-hunt 6 (deferred): the inline viewer never re-armed follow on a downward scroll.
    let mut app = App {
        viewer: Some(ViewerState {
            selected: 0,
            scroll: 79,
            follow: false,
        }),
        ..Default::default()
    };
    app.viewer_geom.set(Some((100, 20))); // wrapped_len=100, body_h=20 → max_scroll=80
    app.viewer_key(KeyKind::Down); // 79 → 80 reaches the tail
    let v = app.viewer.as_ref().unwrap();
    assert!(v.follow, "reaching the tail must re-arm follow");
    assert_eq!(v.scroll, 80, "scroll clamps to the tail");
    app.viewer_key(KeyKind::Up);
    assert!(
        !app.viewer.as_ref().unwrap().follow,
        "scrolling up pauses follow again"
    );
}

#[test]
fn viewer_up_from_a_freshly_opened_viewer_moves_one_line() {
    // Bug: `ViewerState::default()` inits `scroll` to a `usize::MAX / 2` tail sentinel, and
    // only a DOWNWARD scroll (`viewer_refollow_at_tail`) ever clamped it back into range.
    // `render_live` substitutes its own `usize::MAX / 2` for display while `follow` is true
    // and clamps THAT local copy — never writing the clamped value back to `v.scroll` — so
    // Up just decremented the still-untouched sentinel. The render kept showing the same
    // tail row (clamped to `max_scroll` every frame regardless), so Up appeared to do
    // nothing for roughly `usize::MAX / 2 - max_scroll` presses.
    let mut app = App {
        viewer: Some(ViewerState::default()), // opens at the tail: follow = true
        ..Default::default()
    };
    app.viewer_geom.set(Some((100, 20))); // wrapped_len=100, body_h=20 → max_scroll=80
    app.viewer_key(KeyKind::Up);
    let v = app.viewer.as_ref().unwrap();
    assert_eq!(
        v.scroll, 79,
        "one Up press from a freshly opened (tailing) viewer must move up exactly one line"
    );
    assert!(!v.follow, "scrolling up pauses follow");
}

#[test]
fn selection_spans_committed_and_streaming_rows() {
    // Bug-hunt 6 (deferred): a copy that ran into the live streaming reply was cut at the
    // committed/stream boundary because `selection_text` only read the wrap cache.
    let app = App {
        main_log: vec![TextLine::from("AAA")],
        streaming: "BBB".to_string(),
        streaming_active: true,
        // Select committed row 0 ("AAA") through the whole streaming row 1.
        selection: Some(((0, 0), (1, 100))),
        ..Default::default()
    };
    app.transcript_geom.set(Some(TranscriptGeom {
        col0: 0,
        row0: 0,
        width: 80,
        height: 24,
        scroll: 0,
    }));
    let sel = app
        .selection_text()
        .expect("a selection spanning the boundary yields text");
    assert!(
        sel.contains("AAA") && sel.contains("BBB"),
        "selection must include the streaming tail, got: {sel:?}"
    );
}

#[test]
fn streaming_edge_renders_markdown_not_a_raw_blob() {
    let app = App {
        streaming_active: true,
        streaming: "# Heading\n\n- first point\n- second point\n\nA closing paragraph.".into(),
        ..Default::default()
    };
    let rows = app.streaming_edge(80, 0, usize::MAX);
    let text: Vec<String> = rows
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    // The old code dumped the whole reply (newlines and all) as ONE wrapped span. Markdown
    // rendering must split it across rows for the heading, each bullet, and the paragraph.
    assert!(
        rows.len() >= 4,
        "partial markdown split into rows: {text:?}"
    );
    assert!(
        text.iter().any(|l| l.contains("first point"))
            && text.iter().any(|l| l.contains("second point")),
        "bullets on their own rows: {text:?}"
    );
    // The blinking cursor block rides the last row.
    assert!(
        text.last().unwrap().contains('▌'),
        "cursor on last row: {text:?}"
    );
    // streaming_edge_len agrees with the rendered row count (no cursor-only extra row here).
    assert_eq!(app.streaming_edge_len(80), rows.len());
}

/// The in-loop viewer memoizes the wrap of the entry it is showing, so its cache must invalidate
/// on every input that changes those rows: the content underneath, the terminal width, and the
/// selected entry. Scroll must NOT invalidate it (it only moves the window) but must still move
/// the visible slice — a cache that outlived a content change would freeze a streaming turn's
/// viewer, and one keyed too coarsely would re-wrap the whole transcript every frame again.
#[test]
fn viewer_wrap_cache_tracks_content_width_and_selection() {
    let mut app = App::default();
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "general".into(),
        task: "find call sites".into(),
        model: None,
        phase: None,
    });
    app.push_scrollback(
        (0..40)
            .map(|i| TextLine::from(Span::raw(format!("chat line {i}"))))
            .collect(),
    );
    app.open_viewer(0);

    let first = screen_wh(&app, 40, 12);
    assert!(first.contains("chat line 39"), "tail shown: {first}");
    let key_after_first = app.viewer_wrap.borrow().key;
    assert!(key_after_first.is_some(), "the wrap was cached");

    // Proof the rows are memoized and not silently rebuilt every frame: rewrite a cached line
    // WITHOUT bumping the revision (only reachable from inside this module) and the frame is
    // byte-identical, i.e. nothing re-read `main_log`.
    app.main_log[39] = TextLine::from(Span::raw("rewritten behind the cache"));
    assert_eq!(
        screen_wh(&app, 40, 12),
        first,
        "an unchanged revision must reuse the wrapped rows"
    );
    assert_eq!(app.viewer_wrap.borrow().key, key_after_first);

    // Content change → new rows.
    app.push_scrollback(vec![TextLine::from(Span::raw("chat line 40"))]);
    let grown = screen_wh(&app, 40, 12);
    assert!(
        grown.contains("chat line 40"),
        "appended line is picked up: {grown}"
    );

    // Width change → re-wrap at the new width (this line only wraps at 40 columns).
    app.push_scrollback(vec![TextLine::from(Span::raw("w".repeat(60)))]);
    let narrow_rows = {
        let _ = screen_wh(&app, 40, 12);
        app.viewer_wrap.borrow().rows.len()
    };
    let wide_rows = {
        let _ = screen_wh(&app, 100, 12);
        app.viewer_wrap.borrow().rows.len()
    };
    assert!(
        narrow_rows > wide_rows,
        "narrow terminal wraps into more rows: {narrow_rows} vs {wide_rows}"
    );

    // Scrolling back to the top reveals earlier rows without rebuilding the wrap.
    let cached = app.viewer_wrap.borrow().key;
    app.viewer_key(KeyKind::Home);
    let scrolled = screen_wh(&app, 100, 12);
    assert_eq!(
        app.viewer_wrap.borrow().key,
        cached,
        "scrolling must not invalidate the wrap"
    );
    assert!(
        scrolled.contains("chat line 0") && !scrolled.contains("chat line 40"),
        "the window moved: {scrolled}"
    );

    // Switching entry → the subagent's own transcript, then closing releases the cache.
    app.viewer_key(KeyKind::Right);
    let subagent = screen_wh(&app, 100, 12);
    assert!(
        subagent.contains("general") && !subagent.contains("chat line"),
        "selection change re-wraps the new entry: {subagent}"
    );
    app.viewer_key(KeyKind::Esc);
    let _ = screen_wh(&app, 100, 12);
    assert!(
        app.viewer_wrap.borrow().key.is_none(),
        "closing the viewer releases its transcript copy"
    );
}

#[test]
fn wrap_cache_invalidates_on_append_and_at_the_cap() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.push_scrollback(vec![TextLine::from("one")]);
    assert_eq!(app.transcript_total_rows(40), 1);
    // Appending more must invalidate the memoized wrap (not serve a stale 1-row count).
    app.push_scrollback(vec![TextLine::from("two"), TextLine::from("three")]);
    assert_eq!(app.transcript_total_rows(40), 3);
    // At the MAIN_LOG_MAX cap the len stays constant while lines are replaced — the rev-based
    // key (not len) must still invalidate, so the newest line is reflected.
    for i in 0..MAIN_LOG_MAX + 50 {
        app.push_scrollback(vec![TextLine::from(format!("fill {i}"))]);
    }
    let _ = app.transcript_total_rows(40); // prime the cache at the cap
    app.push_scrollback(vec![TextLine::from("NEWEST")]);
    app.ensure_wrapped_main(40);
    let cache = app.wrap_cache.borrow();
    let last: String = cache
        .rows
        .last()
        .unwrap()
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(last.contains("NEWEST"), "cap append reflected: {last:?}");
}

#[test]
fn mouse_selection_extracts_text_within_and_across_wrapped_rows() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.push_scrollback(vec![TextLine::from("alpha"), TextLine::from("bravo")]);
    app.ensure_wrapped_main(80);
    app.transcript_geom.set(Some(TranscriptGeom {
        col0: 0,
        row0: 0,
        width: 80,
        height: 10,
        scroll: 0,
    }));
    // A cell outside the area is not a selection start.
    assert!(
        !app.selection_begin(0, 50),
        "row below the area → no selection"
    );

    // Single-row: select all of "alpha".
    assert!(app.selection_begin(0, 0));
    app.selection_extend(5, 0);
    assert_eq!(app.selection_text().as_deref(), Some("alpha"));

    // Across two rows: "pha" + newline + "bra".
    assert!(app.selection_begin(2, 0));
    app.selection_extend(3, 1);
    assert_eq!(app.selection_text().as_deref(), Some("pha\nbra"));

    // Clearing drops the selection (no text to copy).
    app.clear_selection();
    assert!(!app.has_selection());
    assert!(app.selection_text().is_none());
}

#[test]
fn jump_bar_hit_tests_only_its_own_row_and_span() {
    let app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.jump_bar_geom.set(Some((20, 10, 8))); // row 20, cols [10, 18)
    assert!(app.jump_bar_hit(10, 20));
    assert!(app.jump_bar_hit(17, 20));
    assert!(!app.jump_bar_hit(18, 20), "past the right edge");
    assert!(!app.jump_bar_hit(12, 19), "wrong row");
}

#[test]
fn in_loop_viewer_opens_navigates_and_closes() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    // No activity → open is a no-op (nothing to view).
    app.open_viewer(0);
    assert!(app.viewer.is_none());

    // Two subagents → activity list is [main, sub, sub] (len 3).
    app.apply(crate::PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "general".into(),
        task: "x".into(),
        model: Some("m".into()),
        phase: None,
    });
    app.apply(crate::PresenterEvent::SubagentStart {
        id: "b".into(),
        agent: "general".into(),
        task: "y".into(),
        model: Some("m".into()),
        phase: None,
    });
    app.open_viewer(1);
    assert_eq!(app.viewer.as_ref().unwrap().selected, 1);

    // Right/Left switch entries (wrapping); a modal key is always consumed.
    assert!(app.viewer_key(KeyKind::Right));
    assert_eq!(app.viewer.as_ref().unwrap().selected, 2);
    assert!(app.viewer_key(KeyKind::Right)); // wraps 2 → 0
    assert_eq!(app.viewer.as_ref().unwrap().selected, 0);

    // Up pauses follow; Esc closes.
    app.viewer_key(KeyKind::Up);
    assert!(!app.viewer.as_ref().unwrap().follow);
    app.viewer_key(KeyKind::Esc);
    assert!(app.viewer.is_none());
    // Closed viewer ignores keys (not consumed).
    assert!(!app.viewer_key(KeyKind::Down));
}

#[test]
fn view_snapshot_round_trips_activity_and_viewer() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.apply(crate::PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "general".into(),
        task: "scan".into(),
        model: Some("opus".into()),
        phase: None,
    });
    app.apply(crate::PresenterEvent::SubagentProgress {
        id: "a".into(),
        snippet: "working\n".into(),
    });
    app.open_viewer(1);
    let json = app.view_snapshot_json().expect("activity → snapshot");

    // A fresh app restores the same activity list + open viewer.
    let mut restored = App::default();
    restored.restore_view_json(&json);
    assert_eq!(restored.activity_len(), 2, "main + 1 subagent");
    assert_eq!(restored.viewer.as_ref().unwrap().selected, 1);
    let views = restored.activity_views();
    assert!(views[1]
        .lines
        .iter()
        .any(|l| l.spans.iter().any(|s| s.content.contains("working"))));

    // A plain session (no activity / viewer / tasks) writes nothing.
    assert!(App::default().view_snapshot_json().is_none());
}

#[test]
fn mouse_selection_over_a_code_block_copies_no_frame() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    let code = "fn f() {\n    let v = \"a │ b\";\n}";
    app.push_scrollback(crate::render::markdown_to_lines(&format!(
        "```rust\n{code}\n```"
    )));
    app.ensure_wrapped_main(80);
    app.transcript_geom.set(Some(TranscriptGeom {
        col0: 0,
        row0: 0,
        width: 80,
        height: 10,
        scroll: 0,
    }));
    assert!(app.selection_begin(0, 0));
    app.selection_extend(79, 4);
    assert_eq!(app.selection_text().as_deref(), Some(code));
}

#[test]
fn wrap_words_measures_wide_glyphs_in_cells() {
    use unicode_width::UnicodeWidthStr;
    // A long unbreakable run of CJK (each 2 cells) must hard-split so no row exceeds the width in
    // CELLS — a char-count wrapper would double-fill each row.
    let rows = crate::app::lines::turn::wrap_words(&"語".repeat(20), 8);
    for r in &rows {
        assert!(
            UnicodeWidthStr::width(r.as_str()) <= 8,
            "row '{r}' is {} cells > 8",
            UnicodeWidthStr::width(r.as_str())
        );
    }
}
