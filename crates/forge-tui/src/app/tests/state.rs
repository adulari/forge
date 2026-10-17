use super::*;

#[test]
fn model_short_strips_provider_prefix() {
    assert_eq!(
        model_short(Some("anthropic::claude-opus-4-8")),
        "claude-opus-4-8"
    );
    assert_eq!(
        model_short(Some("groq::llama-3.1-8b-instant")),
        "llama-3.1-8b-instant"
    );
    // No prefix → unchanged; empty/None → placeholder.
    assert_eq!(model_short(Some("opus")), "opus");
    assert_eq!(model_short(Some("")), "…");
    assert_eq!(model_short(None), "…");
}

#[test]
fn width_cap_scales_but_floors_at_min() {
    // Wide terminal scales the budget above the old fixed min.
    assert!(width_cap(200, 6, 48) > 48);
    // Narrow terminal never drops below the min (old fixed cap preserved).
    assert_eq!(width_cap(40, 6, 48), 48);
    // Width 0 (pre-first-render) falls back to 80, not 0.
    assert!(width_cap(0, 6, 48) >= 48);
}

#[test]
fn a_model_mesh_has_no_ladder_for_falls_back_to_the_pin() {
    let mut app = App::default();
    app.apply(PresenterEvent::Effort(Some(forge_types::EffortLevel::High)));
    app.apply(PresenterEvent::Routing {
        tier: "complex".into(),
        model: "codex-oauth::gpt-6-astra".into(),
        rationale: "auto-selected".into(),
        effort: None,
    });
    let text = screen_wh(&app, 120, LIVE_H);
    assert!(
        text.contains("⟨high⟩"),
        "the pin still applies when mesh has no opinion: {text:?}"
    );
}

#[test]
fn insert_at_cursor_mid_string() {
    let mut buf = "ac".to_string();
    let mut cur = 1usize; // between 'a' and 'c'
    handle_key(&mut buf, &mut cur, KeyKind::Char('b'));
    assert_eq!(buf, "abc");
    assert_eq!(cur, 2);
}

#[test]
fn fullscreen_transcript_renders_log_tail_and_clears() {
    let mut app = App {
        fullscreen: true,
        transcript_follow: true,
        ..Default::default()
    };
    app.push_scrollback(
        (0..30)
            .map(|i| TextLine::from(format!("line {i}")))
            .collect(),
    );
    // Following → the tail is visible, the head scrolled off, in a 5-row body.
    let area = Rect::new(0, 0, 40, 5);
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 5)).unwrap();
    term.draw(|f| render_transcript_area(f, area, &app))
        .unwrap();
    let buf = term.backend().buffer().clone();
    let text: String = buf.content().iter().map(|c| c.symbol()).collect();
    assert!(text.contains("line 29"), "tail visible: {text:?}");
    assert!(!text.contains("line 0 "), "head scrolled off");
    // /clear empties the rendered transcript.
    app.clear_transcript();
    assert_eq!(app.transcript_total_rows(40), 0);
}

mod paste_cap {
    use super::*;

    #[test]
    fn a_huge_single_line_paste_collapses_to_a_chip() {
        let mut app = App::default();
        app.handle_paste("A".repeat(5000));
        assert_eq!(app.input, "[pasted text (5000 chars)]");
        app.handle_paste("short".into());
        assert!(app.input.ends_with("short"));
    }
}
