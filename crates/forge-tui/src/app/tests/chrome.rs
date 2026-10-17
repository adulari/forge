use super::*;
use crate::theme::color;

#[test]
fn compact_band_hidden_until_context_fills_then_shows_hint() {
    assert_eq!(
        compact_band_height(&App::default()),
        0,
        "no signal → no band"
    );
    let low = App {
        context_tokens: 10_000,
        context_limit: Some(100_000), // 10%
        ..Default::default()
    };
    assert_eq!(compact_band_height(&low), 0, "plenty of room → no band");
    let near = App {
        context_tokens: 70_000, // 70% ≥ 65% hint threshold
        context_limit: Some(100_000),
        ..Default::default()
    };
    assert_eq!(compact_band_height(&near), 1, "approaching → hint shows");
    let out = screen_wh(&near, 80, LIVE_H);
    assert!(
        out.contains("auto-compact"),
        "hint names the trigger: {out:?}"
    );
}

#[test]
fn compact_band_shows_animated_bar_while_compacting() {
    let app = App {
        compaction: Some(CompactionState {
            start_tick: 0,
            auto: true,
        }),
        tick: 40, // ~2.4s elapsed → eased bar well underway
        ..Default::default()
    };
    assert_eq!(compact_band_height(&app), 1);
    let out = screen_wh(&app, 80, LIVE_H);
    assert!(out.contains("auto-compacting"), "shows the label: {out:?}");
    assert!(out.contains('%'), "shows a percentage: {out:?}");
    assert!(out.contains('▰'), "shows a filled bar: {out:?}");
}

#[test]
fn cost_cell_distinguishes_subscription_priced_free_and_untracked() {
    assert_eq!(cost_cell("claude-cli::", 0.0), "subscription");
    assert_eq!(cost_cell("codex-cli::gpt-5.5", 0.0), "subscription");
    assert_eq!(cost_cell("openai::gpt-4o-mini", 0.0123), "$0.01230");
    // Genuinely free: known-$0 provider, positive evidence.
    assert_eq!(cost_cell("ollama::llama3", 0.0), "free");
    assert_eq!(cost_cell("groq::llama-3.1-8b-instant", 0.0), "free");
    assert_eq!(
        cost_cell("openrouter::cohere/north-mini-code:free", 0.0),
        "free"
    );
    // OpenCode Go bills a flat subscription, so its $0 marginal cost is real rather than a
    // missing price. Distinct from OpenCode Zen (`opencode::`) below, despite the similar name.
    assert_eq!(cost_cell("opencode_go::glm-5.2", 0.0), "subscription");
    // Unpriced gateway/credit model: not a bridge, $0 only because we lack a price. Zen bills
    // premium and free models against one shared key balance, so unpriced must not read free.
    assert_eq!(cost_cell("opencode::glm-5.2", 0.0), "untracked");
    assert_eq!(
        cost_cell("openrouter::anthropic/claude-opus", 0.0),
        "untracked"
    );
    // Zen marks genuinely-free models with a `-free` suffix — positive evidence, so "free".
    assert_eq!(cost_cell("opencode::qwen3-coder-free", 0.0), "free");
}

/// Regression: an interrupted workflow (active=true, WorkflowFinished never arrived) used
/// to survive `on_turn_start`'s `if !active` guard forever — every later turn inherited the
/// dead run's state, and new spawn_agents batches were misrouted into its row list.
#[test]
fn turn_start_clears_a_stale_active_workflow() {
    let mut app = App::default();
    app.workflow.begin(None);
    app.workflow
        .on_agent_start("a".into(), "general".into(), "task".into(), None, None);
    assert!(app.workflow.active, "simulated stuck state");
    app.on_turn_start();
    assert!(!app.workflow.active, "stale active run cleared");
    assert!(app.workflow.rows.is_empty(), "dead run's rows gone");
}

#[test]
fn temper_shows_in_statusline_and_switching_does_not_spam_scrollback() {
    let mut app = App {
        temper: "Ask".into(),
        ..App::default()
    };
    // Wide enough that the temper segment renders.
    assert!(
        screen_wh(&app, 90, LIVE_H).contains("Ask"),
        "active temper shown in the statusline"
    );

    app.set_temper("Auto-edit");
    assert_eq!(app.temper, "Auto-edit");
    assert!(
        screen_wh(&app, 90, LIVE_H).contains("Auto-edit"),
        "statusline reflects the new temper"
    );
    // Switching updates the (colored) statusline indicator only — no per-switch scrollback
    // line (rapid SHIFT+TAB cycling used to flood the view).
    assert!(
        flush_text(&mut app).is_empty(),
        "switching the temper queues nothing to scrollback"
    );
}

#[test]
fn temper_indicator_is_color_coded_by_posture() {
    // Each temper renders in its own color so the current posture reads at a glance.
    use crate::theme::styles::temper_color;
    assert_eq!(temper_color("Read-only"), color::user());
    assert_eq!(temper_color("Ask"), color::fg_muted());
    assert_eq!(temper_color("Auto-edit"), color::brand());
    assert_eq!(temper_color("Full"), color::err());
}

#[test]
fn shift_tab_is_a_cycle_temper_key_not_an_edit() {
    let mut input = String::new();
    let mut cur = 0usize;
    assert_eq!(
        handle_key(&mut input, &mut cur, KeyKind::CycleTemper),
        InputOutcome::Editing
    );
    assert!(input.is_empty(), "temper key never edits the input line");
}

#[test]
fn statusline_hint_idle_empty_shows_cmds_and_help() {
    let app = App::default();
    let hint = hint_text(&app);
    assert!(hint.contains("? keys"), "idle-empty hint has keys: {hint}");
    assert!(
        hint.contains("Ctrl+K actions"),
        "idle-empty hint has actions: {hint}"
    );
}

#[test]
fn the_hint_row_while_busy_says_what_esc_and_enter_do() {
    let app = App {
        busy: true,
        ..Default::default()
    };
    let hint = hint_text(&app);
    assert!(hint.contains("Esc stop"), "busy hint: {hint}");
    assert!(hint.contains("Enter queue"), "busy hint: {hint}");
}

#[test]
fn the_hint_row_with_the_palette_open_shows_run() {
    let mut app = App::default();
    app.palette.open_with("");
    let hint = hint_text(&app);
    assert!(hint.contains("Enter run"), "palette hint: {hint}");
    assert!(hint.contains("Tab complete"), "palette hint: {hint}");
}

#[test]
fn the_hint_row_with_a_picker_open_shows_select() {
    use crate::commands::{PickerKind, PickerRow};
    let mut app = App::default();
    app.picker.open_with(
        PickerKind::Sessions,
        "resume",
        vec![PickerRow {
            id: "s1".into(),
            title: "session one".into(),
            subtitle: String::new(),
        }],
    );
    let hint = hint_text(&app);
    assert!(hint.contains("Enter select"), "picker hint: {hint}");
    assert!(hint.contains("type to filter"), "picker hint: {hint}");
}

#[test]
fn the_hint_row_after_a_finished_turn_is_the_idle_set() {
    let app = App {
        done: true,
        ..Default::default()
    };
    let hint = hint_text(&app);
    assert!(hint.contains("/ commands"), "done hint: {hint}");
    assert!(
        !hint.contains("Ctrl-C"),
        "Ctrl+C is not advertised as quit while it means stop: {hint}"
    );
}

#[test]
fn a_narrow_composer_border_keeps_the_stop_recovery() {
    let app = App {
        done: true,
        last_stop_reason: Some(forge_types::StopReason::MaxSteps),
        ..Default::default()
    };
    let screen = screen_wh(&app, 30, LIVE_H);
    assert!(
        screen.contains("send continue"),
        "recovery action remains visible: {screen}"
    );
    let wide = screen_wh(&app, 100, LIVE_H);
    assert!(
        wide.contains("Step limit reached · send `continue` to resume"),
        "the full sentence fits when there is room: {wide}"
    );
}

#[test]
fn the_hint_row_while_typing_shows_send_newline_and_mode() {
    let app = App {
        input: "hello".into(),
        ..Default::default()
    };
    let hint = hint_text(&app);
    assert!(hint.contains("Enter send"), "typing hint: {hint}");
    assert!(hint.contains("Ctrl+J newline"), "typing hint: {hint}");
    assert!(hint.contains("Shift+Tab mode"), "typing hint: {hint}");
}

#[test]
fn statusline_shows_model_and_cost() {
    let mut app = App {
        cost_usd: 0.42,
        ..Default::default()
    };
    app.apply(PresenterEvent::Routing {
        effort: None,
        tier: "standard".into(),
        model: "openai::gpt-4o-mini".into(),
        rationale: "x".into(),
    });
    let text = screen(&app);
    // In-band display uses the short tail (consistent with the panels/transcript), not the
    // full `provider::model` id.
    assert!(
        text.contains("gpt-4o-mini"),
        "short model in statusline: {text:?}"
    );
    assert!(
        !text.contains("openai::gpt-4o-mini"),
        "full id not shown in-band"
    );
    assert!(text.contains("$0.42"), "cost in statusline: {text}");
    assert!(
        text.contains("gpt-4o-mini · standard"),
        "tier joins the model: {text}"
    );
}

#[test]
fn statusline_shows_forge_effort_labelled_as_meshs_own() {
    // The cell is labelled "mesh" because the statusline now carries TWO efforts: this one,
    // which biases routing, and the rung glued to the model id, which is what the model runs
    // at. An unlabelled level would be read as the model's.
    let mut app = App::default();
    app.apply(PresenterEvent::Effort(Some(
        forge_types::EffortLevel::XHigh,
    )));
    let text = screen_wh(&app, 100, LIVE_H);
    assert!(
        text.contains("mesh xhigh"),
        "effort in statusline: {text:?}"
    );

    // Clearing the pin no longer hides the cell. It previously vanished, so the most common
    // state of all — mesh choosing — showed nothing where a level had been.
    app.apply(PresenterEvent::Effort(None));
    let text = screen_wh(&app, 100, LIVE_H);
    assert!(
        text.contains("mesh auto"),
        "unpinned effort still names who is choosing: {text:?}"
    );
}

#[test]
fn a_pin_shows_in_the_statusline_with_no_turn_having_run() {
    // Pinning a model used to change session state and emit nothing, so the status row kept
    // naming the PREVIOUSLY routed model until the next turn produced a Routing event. The
    // dispatch/picker paths now emit on pin; this asserts the consumer half — that such an
    // event alone, with no turn ever started, updates model and rung.
    let mut app = App::default();
    assert!(app.routing.is_none(), "nothing routed yet");
    assert!(!app.busy, "no turn is running");
    app.apply(PresenterEvent::Routing {
        tier: "pinned".into(),
        model: "codex-oauth::gpt-6-astra".into(),
        rationale: "pinned by /model".into(),
        effort: Some(forge_types::EffortLevel::High),
    });
    let text = screen_wh(&app, 120, LIVE_H);
    assert!(
        text.contains("codex-oauth::gpt-6-astra") && text.contains("⟨high⟩"),
        "the pin and its rung must be visible immediately: {text:?}"
    );
}

#[test]
fn the_rung_mesh_chose_is_what_the_statusline_reports() {
    // End of the chain: mesh picked medium from astra's measured ladder with nothing pinned,
    // and that is what the request sends — so that is what the marker must say. Reading the
    // session pin instead would render ⟨auto⟩ for a turn whose rung was in fact chosen.
    let mut app = App::default();
    assert_eq!(app.effort, None, "nothing pinned");
    app.apply(PresenterEvent::Routing {
        tier: "complex".into(),
        model: "codex-oauth::gpt-6-astra".into(),
        rationale: "auto-selected".into(),
        effort: Some(forge_types::EffortLevel::Medium),
    });
    let text = screen_wh(&app, 120, LIVE_H);
    assert!(
        text.contains("⟨med⟩"),
        "mesh's chosen rung, not the empty pin: {text:?}"
    );
}

#[test]
fn statusline_names_the_provider_and_the_models_own_rung() {
    // The same model over codex-oauth, codex-cli and explabs is three routes with three costs,
    // quotas and effort ladders, so the provider is part of "what am I talking to".
    let mut app = App::default();
    app.apply(PresenterEvent::Effort(Some(
        forge_types::EffortLevel::Medium,
    )));
    app.apply(PresenterEvent::Routing {
        effort: None,
        tier: "standard".into(),
        model: "codex-oauth::gpt-6-astra".into(),
        rationale: "x".into(),
    });
    let text = screen_wh(&app, 120, LIVE_H);
    assert!(
        text.contains("codex-oauth::gpt-6-astra"),
        "provider-qualified model: {text:?}"
    );
    assert!(
        text.contains("⟨med⟩"),
        "the rung the model actually runs at, glued to the id: {text:?}"
    );
    assert!(text.contains("mesh medium"), "mesh's own effort: {text:?}");
}

#[test]
fn whitehot_effort_has_its_own_statusline_segment_and_slider_stop() {
    let mut app = App::default();
    app.apply(PresenterEvent::Effort(Some(
        forge_types::EffortLevel::WhiteHot,
    )));
    let text = screen_wh(&app, 100, LIVE_H);
    assert!(
        text.contains("mesh white-hot"),
        "statusline segment: {text}"
    );

    app.effort_slider = true;
    let text = screen_wh(&app, 100, LIVE_H);
    assert!(
        text.contains("WHITE·HOT"),
        "fifth slider stop label: {text}"
    );
    assert!(text.contains("white-hot"), "forge-themed title: {text}");
}

#[test]
fn cost_meter_shows_running_total() {
    let mut app = App::default();
    app.apply(PresenterEvent::Cost {
        session_total_usd: 0.0033,
        session_in: 0,
        session_cached_in: Some(0),
        session_out: 0,
        context_tokens: 0,
        context_limit: None,
    });
    assert!(screen(&app).contains("<$0.01"));
    app.cost_usd = 3.456;
    assert!(screen(&app).contains("$3.46"));
}

#[test]
fn humanizes_token_counts() {
    assert_eq!(human(0), "0");
    assert_eq!(human(999), "999");
    assert_eq!(human(12_345), "12.3k");
    assert_eq!(human(1_100_000), "1.1M");
}

#[test]
fn statusline_shows_token_counter_and_context_gauge() {
    let mut app = App::default();
    app.apply(PresenterEvent::Cost {
        session_total_usd: 0.01,
        session_in: 12_300,
        session_cached_in: Some(6_000),
        session_out: 4_100,
        context_tokens: 18_200,
        context_limit: Some(200_000),
    });
    let wide = screen_wh(&app, 120, LIVE_H);
    assert!(wide.contains("ctx"), "context gauge shown: {wide}");
    assert!(
        wide.contains("9% of 200k"),
        "context percentage and window: {wide}"
    );
    // Session totals no longer crowd the status row; `/status` lists them.
    app.status_open = true;
    let status = screen_wh(&app, 120, 30);
    assert!(
        status.contains("↑12.3k (6.0k cached) ↓4.1k"),
        "session totals: {status}"
    );
    assert!(
        status.contains("18.2k / 200.0k (9%)"),
        "context row: {status}"
    );
}

#[test]
fn statusline_shows_live_turn_timer_and_per_turn_tokens() {
    let mut app = App {
        session_in: 1_000,
        session_cached_in: Some(400),
        session_out: 200,
        ..Default::default()
    };
    // A turn starts: baseline snapshot taken, the I/O shell ticks the timer, usage reported.
    app.on_turn_start();
    app.busy = true;
    app.turn_elapsed_secs = 73; // 1m13s
    app.apply(PresenterEvent::Cost {
        session_total_usd: 0.02,
        session_in: 2_200,            // +1.2k this turn
        session_cached_in: Some(700), // +300 cached this turn
        session_out: 540,             // +340 this turn
        context_tokens: 0,
        context_limit: None,
    });
    let s = screen_wh(&app, 120, LIVE_H);
    // The composer's live line names the phase and carries the one clock.
    assert!(s.contains("Preparing"), "live line names the phase: {s}");
    assert!(
        s.contains("1m13s"),
        "single turn timer on the live line: {s}"
    );
    assert!(
        s.contains("↓340 tok"),
        "streamed tokens on the live line: {s}"
    );
    assert!(
        s.contains("↑1.2k ↓340"),
        "per-turn token deltas in the status row: {s}"
    );
    app.status_open = true;
    let status = screen_wh(&app, 120, 30);
    assert!(
        status.contains("↑1.2k (300 cached) ↓340"),
        "the turn row keeps the cache delta: {status}"
    );
    assert!(
        status.contains("↑2.2k (700 cached) ↓540"),
        "session totals: {status}"
    );
    app.on_turn_start();
    assert_eq!(
        app.turn_cached_in, None,
        "a new turn clears the prior cache delta"
    );
}

#[test]
fn turn_timer_does_not_hide_the_compaction_band() {
    // The turn timer lives on the composer's live line; the compaction band is its own row.
    // Both must show at once (the regression report was "the timer hides compaction").
    let mut app = App {
        session_in: 100,
        session_out: 50,
        ..Default::default()
    };
    app.on_turn_start();
    app.busy = true;
    app.turn_elapsed_secs = 5;
    app.compaction = Some(CompactionState {
        start_tick: 0,
        auto: false,
    });
    let s = screen_wh(&app, 120, LIVE_H);
    assert!(
        s.contains("compacting"),
        "compaction band still visible: {s}"
    );
    assert!(s.contains("5s"), "turn timer also visible: {s}");
}

#[test]
fn fmt_dur_is_compact_across_scales() {
    assert_eq!(fmt_dur(0), "0s");
    assert_eq!(fmt_dur(45), "45s");
    assert_eq!(fmt_dur(73), "1m13s");
    assert_eq!(fmt_dur(3_661), "1h01m");
}

#[test]
fn context_gauge_uses_fallback_limit_when_unknown() {
    let mut app = App::default();
    app.apply(PresenterEvent::Cost {
        session_total_usd: 0.01,
        session_in: 5_000,
        session_cached_in: Some(0),
        session_out: 1_000,
        context_tokens: 6_000,
        context_limit: None,
    });
    let wide = screen_wh(&app, 120, LIVE_H);
    // Unknown limit → conservative fallback so a % + bar still show, marked approximate.
    assert!(
        wide.contains("5% of ~128k"),
        "fallback denominator shown and marked approximate: {wide}"
    );
}

#[test]
fn the_gauge_survives_a_narrow_status_line() {
    let mut app = App::default();
    app.apply(PresenterEvent::Cost {
        session_total_usd: 0.0033,
        session_in: 12_300,
        session_cached_in: Some(0),
        session_out: 4_100,
        context_tokens: 18_200,
        context_limit: Some(200_000),
    });
    // One row at any width: widgets drop in priority order and the gauge stays.
    let narrow = screen_wh(&app, 40, LIVE_H);
    assert!(narrow.contains("ctx"), "gauge stays: {narrow}");
    assert!(narrow.contains("9%"), "gauge percentage stays: {narrow}");
    let wide = screen_wh(&app, 100, LIVE_H);
    assert!(
        wide.contains("<$0.01"),
        "cost shows when there is room: {wide}"
    );
    assert_eq!(
        crate::app::render::statusline_height(&app),
        1,
        "the status line is one row"
    );
}

#[test]
fn input_bar_renders_when_present() {
    let input = "fix the bug".to_string();
    let app = App {
        input_cursor: input.len(),
        input,
        ..Default::default()
    };
    assert!(screen(&app).contains("❯ fix the bug"));
}

#[test]
fn busy_shows_a_spinner_frame() {
    // SPINNER[2] == "⠹"; the statusline animates while a turn runs.
    let app = App {
        busy: true,
        tick: 2,
        ..Default::default()
    };
    assert!(screen(&app).contains('⠹'), "spinner frame shown when busy");
}

#[test]
fn busy_provider_wait_shows_model_phase_and_stale_heartbeat() {
    let mut app = App::default();
    app.on_turn_start();
    app.busy = true;
    app.apply(PresenterEvent::Routing {
        effort: None,
        tier: "complex".into(),
        model: "qwencloud::qwen3.8-max-preview".into(),
        rationale: "pinned".into(),
    });
    app.apply(PresenterEvent::ProviderRequest {
        model: "qwencloud::qwen3.8-max-preview".into(),
        step: 0,
    });
    // 1,100 render ticks × 60ms = 66s without a presenter event.
    app.tick = 1_100;
    app.turn_elapsed_secs = 66;

    let text = screen_wh(&app, 150, LIVE_H);
    assert!(text.contains("Routing"), "phase is explicit: {text}");
    assert!(
        text.contains("qwen3.8-max-preview"),
        "model is visible: {text}"
    );
    assert!(
        text.contains("last event 1m06s ago"),
        "provider silence is explicit: {text}"
    );
}

#[test]
fn hidden_reasoning_reports_real_streamed_progress() {
    let mut app = App::default();
    app.on_turn_start();
    app.busy = true;
    app.apply(PresenterEvent::Routing {
        effort: None,
        tier: "complex".into(),
        model: "qwencloud::qwen3.8-max-preview".into(),
        rationale: "pinned".into(),
    });
    app.apply(PresenterEvent::Reasoning("abc".repeat(1_000)));

    let text = screen_wh(&app, 150, LIVE_H);
    assert!(text.contains("Thinking"), "reasoning phase: {text}");
    assert!(
        !text.contains("last event"),
        "a live stream shows no silence notice: {text}"
    );
    // The hidden-reasoning counter lives in the inspector, not on the one live line.
    app.status_open = true;
    let status = screen_wh(&app, 150, 30);
    assert!(
        status.contains("thought 3.0k chars"),
        "hidden reasoning has a progress counter: {status}"
    );
    assert!(
        status.contains("event now"),
        "stream heartbeat is live: {status}"
    );
}

#[test]
fn buffered_provider_events_keep_large_tool_stream_visibly_alive() {
    let mut app = App::default();
    app.on_turn_start();
    app.busy = true;
    app.apply(PresenterEvent::ProviderRequest {
        model: "qwencloud::qwen3.8-max-preview".into(),
        step: 1,
    });
    app.apply(PresenterEvent::ProviderProgress);
    app.apply(PresenterEvent::ProviderProgress);

    let text = screen_wh(&app, 150, LIVE_H);
    assert!(
        text.contains("Waiting · for the model"),
        "buffered stream phase: {text}"
    );
    app.status_open = true;
    let status = screen_wh(&app, 150, 30);
    assert!(
        status.contains("2 buffered events"),
        "provider heartbeat count: {status}"
    );
    assert!(
        status.contains("event now"),
        "stream is visibly live: {status}"
    );
}

#[test]
fn streaming_tool_input_and_hidden_thinking_replace_the_buffered_event_count() {
    let mut app = App::default();
    app.on_turn_start();
    app.busy = true;
    app.apply(PresenterEvent::ProviderRequest {
        model: "claude-cli::claude-opus-5-5".into(),
        step: 0,
    });
    app.apply(PresenterEvent::ProviderThinking { tokens: 2_100 });
    let text = screen_wh(&app, 150, LIVE_H);
    assert!(text.contains("Thinking · ~2.1k tokens"), "thinking: {text}");
    app.status_open = true;
    assert!(!screen_wh(&app, 150, 30).contains("buffered events"));
    app.status_open = false;

    app.apply(PresenterEvent::ProviderToolInput {
        name: "mcp__forge__write_file".into(),
        chars: 12_300,
    });
    let text = screen_wh(&app, 150, LIVE_H);
    assert!(
        text.contains("Writing write_file call"),
        "tool input: {text}"
    );
    assert!(!text.contains("mcp__forge__"), "{text}");
    app.status_open = true;
    assert!(
        screen_wh(&app, 150, 30).contains("writing write_file call, 12.3k chars"),
        "the char count lives in the inspector"
    );
    app.status_open = false;

    app.apply(PresenterEvent::ToolStart {
        name: "mcp__forge__write_file".into(),
        args: "{}".into(),
    });
    let text = screen_wh(&app, 150, LIVE_H);
    assert!(!text.contains("Writing write_file call"), "{text}");
    assert!(text.contains("Running write_file"), "{text}");
    assert!(!text.contains("mcp__forge__"), "{text}");
}

#[test]
fn auxiliary_model_work_replaces_the_stale_tool_result_phase() {
    let mut app = App::default();
    app.on_turn_start();
    app.busy = true;
    app.apply(PresenterEvent::ToolResult {
        name: "shell".into(),
        ok: false,
        summary: "exit 1".into(),
        detail: None,
    });
    app.apply(PresenterEvent::AuxiliaryRequest {
        model: "codex-oauth::gpt-5.6-luna".into(),
        purpose: "diagnosing the failed shell command".into(),
    });
    let started = screen_wh(&app, 150, LIVE_H);
    assert!(started.contains("Working"), "phase: {started}");
    assert!(
        started.contains("diagnosing the failed shell command"),
        "purpose: {started}"
    );
    app.status_open = true;
    let status = screen_wh(&app, 150, 30);
    assert!(status.contains("gpt-5.6-luna"), "side-call model: {status}");
    app.status_open = false;

    app.apply(PresenterEvent::AuxiliaryProgress { chars: 1_250 });
    let progressing = screen_wh(&app, 150, LIVE_H);
    assert!(
        progressing.contains("internal model streamed 1.2k chars"),
        "side-call heartbeat: {progressing}"
    );
}

#[test]
fn idle_shows_no_spinner() {
    let text = screen(&App::default());
    assert!(!text.contains('⠹') && !text.contains('⠙'));
}

#[test]
fn a_backgrounded_live_workflow_shows_the_status_band_until_it_finishes() {
    let mut app = App::default();
    app.apply(PresenterEvent::WorkflowStarted {
        name: Some("audit".into()),
    });
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "general".into(),
        task: "x".into(),
        model: Some("m".into()),
        phase: None,
    });
    app.workflow.open = false; // Esc → background
    assert!(app.workflow.band_visible());
    let out = screen_wh(&app, 100, LIVE_H);
    assert!(out.contains("workflow"), "band names the run: {out:?}");
    assert!(
        out.contains("Ctrl+O view"),
        "band offers the reopen key: {out:?}"
    );
    app.apply(PresenterEvent::SubagentResult {
        id: "a".into(),
        agent: "general".into(),
        ok: true,
        summary: "done".into(),
        cost_usd: 0.0,
    });
    app.apply(PresenterEvent::WorkflowFinished {
        ok: true,
        summary: "all good".into(),
    });
    assert!(!app.workflow.band_visible(), "no band once the run is over");
}

#[test]
fn statusline_config_default_shows_model_and_cost() {
    let app = App::default();
    let out = screen(&app);
    // Default config has Model + SessionCost widgets on the left.
    // Model widget shows "—" when no routing; SessionCost shows "untracked" for no-model.
    assert!(
        out.contains('—') || out.contains("untracked"),
        "default statusline renders: {out:?}"
    );
}

#[test]
fn statusline_shows_live_throughput_while_the_model_generates() {
    // The meter has to be readable from streamed characters alone: usage lands only when a
    // call finishes, which is exactly when the user no longer needs a speed readout.
    let base = std::time::Instant::now();
    let mut app = App {
        busy: true,
        turn_ran: true,
        ..Default::default()
    };
    app.throughput.on_turn_start(base);
    for step in 1..=8u64 {
        app.throughput
            .on_delta(50, base + std::time::Duration::from_millis(step * 125));
    }
    let out = screen(&app);
    assert!(
        out.contains("tok/s"),
        "throughput missing from a streaming statusline: {out:?}"
    );

    // Idle with nothing measured: no glyph at all rather than a zero.
    let idle = App::default();
    let out = screen(&idle);
    assert!(
        !out.contains("tok/s"),
        "throughput shown with nothing measured: {out:?}"
    );
}

#[test]
fn statusline_repo_name_widget_renders_when_set() {
    let mut app = App {
        repo_name: Some("forge".to_string()),
        ..Default::default()
    };
    app.statusline_config
        .left
        .push(forge_config::StatuslineWidget::RepoName);
    let out = screen(&app);
    assert!(out.contains("forge"), "repo name not shown: {out:?}");

    // Absent entirely when unset (widget returns None, doesn't render an empty tag).
    app.repo_name = None;
    let out = screen(&app);
    assert!(
        !out.contains('⚑'),
        "repo glyph shown with no repo name: {out:?}"
    );
}

#[test]
fn statusline_extra_rows_render_below_the_built_in_two() {
    let mut app = App {
        repo_name: Some("myrepo".to_string()),
        ..Default::default()
    };
    app.statusline_config.extra_rows = vec![vec![forge_config::StatuslineWidget::RepoName]];
    let out = screen_wh(&app, 80, LIVE_H);
    assert!(
        out.contains("myrepo"),
        "extra row's widget not rendered: {out:?}"
    );
}

#[test]
fn statusline_custom_shell_widget_shows_fallback_then_cached_output() {
    let mut app = App::default();
    app.statusline_config.left = vec![forge_config::StatuslineWidget::Custom {
        text: "loading…".to_string(),
        shell: Some("git rev-parse --short HEAD".to_string()),
        refresh_secs: 5,
    }];
    let out = screen(&app);
    assert!(out.contains("loading…"), "fallback text not shown: {out:?}");

    app.custom_widget_cache.insert(
        "git rev-parse --short HEAD".to_string(),
        "a1b2c3d".to_string(),
    );
    let out = screen(&app);
    assert!(out.contains("a1b2c3d"), "cached output not shown: {out:?}");
    assert!(
        !out.contains("loading…"),
        "stale fallback still shown once cached: {out:?}"
    );
}

#[test]
fn custom_widget_output_event_populates_cache() {
    let mut app = App::default();
    app.apply(PresenterEvent::CustomWidgetOutput {
        id: "echo hi".to_string(),
        text: "hi".to_string(),
    });
    assert_eq!(
        app.custom_widget_cache.get("echo hi").map(String::as_str),
        Some("hi")
    );
}

#[test]
fn statusline_command_output_is_an_extra_row_with_colours_stripped() {
    let mut app = App::default();
    app.statusline_config.command = Some("~/.claude/statusline.sh".to_string());
    let base_h = crate::app::render::statusline_height(&app);
    app.custom_widget_cache.insert(
        "~/.claude/statusline.sh".to_string(),
        "\u{1b}[32mMY-STATUS\u{1b}[0m ok".to_string(),
    );
    assert_eq!(crate::app::render::statusline_height(&app), base_h + 1);
    let out = screen_wh(&app, 80, LIVE_H);
    assert!(out.contains("MY-STATUS ok"), "command row missing: {out:?}");
    assert!(!out.contains('\u{1b}'), "escape leaked into the buffer");
}

#[test]
fn statusline_command_without_output_reserves_no_row() {
    let mut app = App::default();
    let h = crate::app::render::statusline_height(&app);
    app.statusline_config.command = Some("x".to_string());
    assert_eq!(crate::app::render::statusline_height(&app), h);
    app.custom_widget_cache
        .insert("x".to_string(), String::new());
    assert_eq!(crate::app::render::statusline_height(&app), h);
}
