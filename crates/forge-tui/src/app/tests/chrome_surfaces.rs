//! The new chrome: header, composer border, one-row status line, `/status`, and how all of it
//! degrades across colour tiers, glyph sets and widths.

use super::*;
use crate::app::render::chrome::{fit_hints, hint};
use crate::app::render::status_line::dropped_widgets;
use crate::git_location::{GitHead, GitLocation};
use crate::theme::{scoped, ColorTier, GlyphTier, Mode, Motion, Theme};
use ratatui::style::Color;

fn located() -> App {
    App {
        fullscreen: true,
        git_location: Some(GitLocation {
            repo: "proj".into(),
            head: GitHead::Branch("master".into()),
            worktree: false,
        }),
        ..App::default()
    }
}

fn routed(mut app: App) -> App {
    app.apply(PresenterEvent::Routing {
        effort: None,
        tier: "trivial".into(),
        model: "ollama::llama3.2".into(),
        rationale: "x".into(),
    });
    app
}

fn render_buffer(app: &App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| render_live(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn the_header_names_the_repo_and_branch_and_shows_the_version_only_when_wide() {
    let app = located();
    let narrow = screen_rows(&app, 100, 24).remove(0);
    assert!(narrow.contains("✦ forge"), "{narrow}");
    assert!(narrow.contains("proj ⑂ master"), "{narrow}");
    assert!(
        !narrow.contains(env!("CARGO_PKG_VERSION")),
        "no version under 120 cols: {narrow}"
    );
    let wide = screen_rows(&app, 160, 24).remove(0);
    assert!(
        wide.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))),
        "{wide}"
    );
    assert!(
        wide.trim_end().ends_with(env!("CARGO_PKG_VERSION")),
        "right-aligned: {wide}"
    );
}

#[test]
fn the_header_marks_worktrees_and_detached_heads_and_truncates_long_branches() {
    let mut app = located();
    app.git_location = Some(GitLocation {
        repo: "proj".into(),
        head: GitHead::Detached("e50b52d".into()),
        worktree: true,
    });
    let header = screen_rows(&app, 100, 24).remove(0);
    assert!(header.contains("⑂ @e50b52d worktree"), "{header}");
    app.git_location = Some(GitLocation {
        repo: "proj".into(),
        head: GitHead::Branch("feat/tool-output-viewer-with-a-very-long-name".into()),
        worktree: false,
    });
    let header = screen_rows(&app, 50, 24).remove(0);
    assert!(
        header.contains('…'),
        "a tight header keeps both ends of the branch: {header}"
    );
    assert!(header.contains("long-name"), "{header}");
}

#[test]
fn inline_has_no_header_and_the_region_stays_fourteen_rows() {
    let mut app = located();
    app.fullscreen = false;
    let rows = screen_rows(&app, 80, LIVE_H);
    assert_eq!(rows.len(), LIVE_H as usize);
    assert!(rows.iter().all(|r| !r.contains("✦ forge")), "{rows:?}");
}

#[test]
fn fullscreen_chrome_is_five_rows_at_80_by_24() {
    let rows = screen_rows(&located(), 80, 24);
    assert!(rows[0].contains("✦ forge"));
    // Composer (3 rows) and status line (1 row) are the last four rows.
    assert!(rows[20].starts_with('╭'), "{}", rows[20]);
    assert!(rows[21].starts_with('│'));
    assert!(rows[22].starts_with('╰'));
    assert!(rows[23].contains("ctx"), "{}", rows[23]);
    // Rows 1..=19 are the transcript: 19 rows under five rows of chrome.
    assert_eq!(rows.len(), 24);
}

#[test]
fn the_composer_border_carries_the_mode_chip_and_hints() {
    let mut app = located();
    app.temper = "Auto-edit".into();
    let rows = screen_rows(&app, 80, 24);
    let top = &rows[20];
    assert!(
        top.contains("Auto-edit ─╮"),
        "mode chip sits on the top right: {top}"
    );
    let bottom = &rows[22];
    assert!(
        bottom.contains("/ commands · @ files · Ctrl+K actions · Shift+Tab mode · ? keys"),
        "{bottom}"
    );
    assert!(bottom.ends_with("─╯"), "{bottom}");
}

#[test]
fn queue_and_multiline_chips_join_the_mode_chip() {
    let mut app = located();
    app.temper = "Ask".into();
    app.queued = vec!["next".into(), "after".into()];
    app.input = "one\ntwo".into();
    app.input_cursor = app.input.len();
    let rows = screen_rows(&app, 100, 24);
    let top = rows
        .iter()
        .find(|r| r.starts_with('╭'))
        .expect("composer top border");
    assert!(top.contains("2 lines · 2 queued · Ask"), "{top}");
}

#[test]
fn the_live_line_shows_spinner_verb_elapsed_and_streamed_tokens() {
    let mut app = routed(located());
    app.on_turn_start();
    app.busy = true;
    app.turn_elapsed_secs = 14;
    app.turn_out = 1_200;
    app.apply(PresenterEvent::AssistantDelta("hi".into()));
    app.tick = 2;
    let top = screen_rows(&app, 100, 24)
        .into_iter()
        .find(|r| r.starts_with('╭'))
        .expect("composer top border");
    assert!(top.contains("⠹ Writing · 14s · ↓1.2k tok"), "{top}");
}

#[test]
fn quit_and_esc_arming_show_in_the_border_not_the_status_line() {
    let mut app = located();
    app.esc_tap();
    let rows = screen_rows(&app, 80, 24);
    assert!(rows[20].contains("▲ Esc again to rewind"), "{}", rows[20]);
    assert!(!rows[23].contains("rewind"), "{}", rows[23]);
    let mut app = located();
    let mut history = crate::prompt_history::PromptHistory::default();
    app.idle_interrupt(&mut history);
    let rows = screen_rows(&app, 80, 24);
    assert!(rows[20].contains("▲ Ctrl+C again to quit"), "{}", rows[20]);
    assert!(
        rows[22].contains("Ctrl+C quit · any key cancels · ? keys"),
        "{}",
        rows[22]
    );
}

#[test]
fn a_stop_reason_moves_to_the_border_and_leads_the_hints_with_recovery() {
    let mut app = routed(located());
    app.done = true;
    app.last_stop_reason = Some(forge_types::StopReason::MaxSteps);
    let rows = screen_rows(&app, 100, 24);
    let top = rows.iter().find(|r| r.starts_with('╭')).unwrap();
    assert!(
        top.contains("▲ Step limit reached · send `continue` to resume"),
        "{top}"
    );
    let status = rows.last().unwrap();
    assert!(
        !status.contains("step limit"),
        "the status line carries no stop notice: {status}"
    );
    let bottom = rows.iter().find(|r| r.starts_with('╰')).unwrap();
    assert!(
        bottom.contains("continue resume · Ctrl+O tasks · Esc Esc rewind · ? keys"),
        "{bottom}"
    );
}

#[test]
fn hints_drop_from_the_second_to_last_backwards_and_keep_keys_last() {
    let all = vec![
        hint("/", "commands"),
        hint("@", "files"),
        hint("Ctrl+K", "actions"),
        hint("Shift+Tab", "mode"),
        hint("?", "keys"),
    ];
    let labels = |hints: &[crate::app::render::chrome::Hint]| {
        hints.iter().map(|h| h.key.clone()).collect::<Vec<_>>()
    };
    assert_eq!(
        labels(&fit_hints(all.clone(), 200)),
        ["/", "@", "Ctrl+K", "Shift+Tab", "?"]
    );
    assert_eq!(
        labels(&fit_hints(all.clone(), 52)),
        ["/", "@", "Ctrl+K", "?"]
    );
    assert_eq!(labels(&fit_hints(all.clone(), 30)), ["/", "@", "?"]);
    assert_eq!(labels(&fit_hints(all.clone(), 20)), ["/", "?"]);
    assert_eq!(labels(&fit_hints(all.clone(), 12)), ["?"]);
    assert!(fit_hints(all, 3).is_empty());
}

#[test]
fn the_status_line_is_one_row_with_the_model_cost_tokens_and_gauge() {
    let mut app = routed(located());
    app.cost_usd = 0.0;
    app.turn_ran = true;
    app.turn_in = 72;
    app.turn_out = 30;
    app.context_tokens = 42;
    app.context_limit = Some(32_000);
    assert_eq!(crate::app::render::statusline_height(&app), 1);
    let rows = screen_rows(&app, 100, 24);
    let status = rows.last().unwrap();
    assert!(status.contains("llama3.2 · trivial"), "{status}");
    assert!(status.contains("↑72 ↓30"), "{status}");
    assert!(status.contains("ctx ▱▱▱▱▱▱▱▱ 0% of 32k"), "{status}");
    assert!(status.contains("mesh auto"), "{status}");
}

#[test]
fn status_widgets_drop_in_priority_order_and_a_marker_counts_them() {
    let mut app = routed(located());
    app.turn_ran = true;
    app.turn_in = 72;
    app.turn_out = 30;
    app.context_tokens = 1_000;
    app.context_limit = Some(32_000);
    app.mcp_count = 3;
    app.statusline_config
        .left
        .push(forge_config::StatuslineWidget::McpStatus);
    let wide = screen_rows(&app, 140, 24).pop().unwrap();
    assert!(
        wide.contains("3 mcp") && wide.contains("↑72 ↓30") && wide.contains("mesh auto"),
        "{wide}"
    );
    assert_eq!(dropped_widgets(&app, 140), 0);
    // Extras (mcp) go before tokens, tokens before mesh; the gauge and model stay.
    let mid = screen_rows(&app, 74, 24).pop().unwrap();
    assert!(!mid.contains("3 mcp"), "{mid}");
    assert!(mid.contains("↑72 ↓30"), "{mid}");
    assert_eq!(dropped_widgets(&app, 74), 1);
    let tight = screen_rows(&app, 40, 24).pop().unwrap();
    assert!(!tight.contains("↑72"), "{tight}");
    assert!(!tight.contains("mesh auto"), "{tight}");
    assert!(
        tight.contains("llama3.2"),
        "the model is the last widget standing: {tight}"
    );
    assert!(tight.contains("ctx"), "the gauge is never dropped: {tight}");
    assert!(
        tight.contains('+'),
        "a +N marker counts the dropped widgets: {tight}"
    );
}

#[test]
fn every_status_widget_renders_in_the_status_line_or_in_status() {
    use forge_config::StatuslineWidget as W;
    let mut app = routed(located());
    app.fullscreen = true;
    app.temper = "Auto-edit".into();
    app.cost_usd = 0.5;
    app.turn_ran = true;
    app.turn_elapsed_secs = 9;
    app.turn_in = 10;
    app.turn_out = 20;
    app.session_in = 100;
    app.session_out = 50;
    app.git_branch = Some("master".into());
    app.repo_name = Some("proj".into());
    app.mcp_count = 2;
    app.usage_overlay.claude_5h_pct = Some(61.0);
    app.usage_overlay.codex_5h_pct = Some(18.0);
    app.effort = Some(forge_types::EffortLevel::High);
    app.statusline_config.left = vec![
        W::Model,
        W::SessionCost,
        W::Effort,
        W::Mode,
        W::TurnElapsed,
        W::TokensIn,
        W::TokensOut,
        W::SessionTokens,
        W::GitBranch,
        W::RepoName,
        W::QuotaClaude,
        W::QuotaCodex,
        W::McpStatus,
        W::Tier,
        W::Custom {
            text: "custom-bit".into(),
            shell: None,
            refresh_secs: 5,
        },
    ];
    let line = screen_rows(&app, 240, 24).pop().unwrap();
    for needle in [
        "llama3.2",
        "$0.50",
        "mesh high",
        "9s",
        "↑10",
        "↓20",
        "Σ ↑100 ↓50",
        "⑂ master",
        "proj",
        "claude 61%",
        "codex 18%",
        "2 mcp",
        "custom-bit",
    ] {
        assert!(
            line.contains(needle),
            "{needle} missing from the status line: {line}"
        );
    }
    // The mode is the composer chip, and `/status` lists it too.
    let screen = screen_wh(&app, 240, 24);
    assert!(screen.contains("Auto-edit"));
    app.status_open = true;
    let status = screen_wh(&app, 120, 30);
    assert!(status.contains("Auto-edit (Shift+Tab)"), "{status}");
}

#[test]
fn status_lists_every_field_of_the_inspector() {
    let mut app = routed(located());
    app.temper = "Auto-edit".into();
    app.session_id = "d0ff778e-0000".into();
    app.turns_finished = 2;
    app.session_in = 132;
    app.session_out = 54;
    app.context_tokens = 42;
    app.context_limit = Some(32_000);
    app.turn_ran = true;
    app.turn_elapsed_secs = 1;
    app.turn_in = 72;
    app.turn_out = 30;
    app.usage_overlay.claude_5h_pct = Some(61.0);
    app.mcp_count = 2;
    app.status_open = true;
    let screen = screen_wh(&app, 160, 40);
    for label in [
        "Status", "MODEL", "MODE", "WHERE", "CONTEXT", "TURN", "SESSION", "QUOTAS", "MCP",
        "VERSION",
    ] {
        assert!(screen.contains(label), "{label} missing: {screen}");
    }
    for needle in [
        "ollama::llama3.2",
        "trivial tier",
        "mesh auto",
        "effort",
        "Auto-edit (Shift+Tab)",
        "thinking hidden",
        "not a worktree",
        "remote control off",
        "42 / 32.0k (0%)",
        "auto-compact at 80%",
        "d0ff778e",
        "2 turns",
        "claude 5h 61%",
        "2 connected",
        "Esc close",
        env!("CARGO_PKG_VERSION"),
    ] {
        assert!(screen.contains(needle), "{needle} missing: {screen}");
    }
}

#[test]
fn status_is_a_registered_command_and_parses() {
    assert!(crate::commands::COMMANDS.iter().any(|c| c.name == "status"));
    assert_eq!(
        crate::commands::parse_command("/status"),
        crate::commands::CommandAction::Status
    );
    assert_eq!(crate::commands::command_category("status"), "Session");
}

fn tier_theme(tier: ColorTier, mode: Mode, glyphs: GlyphTier) -> Theme {
    Theme::new(tier, mode, glyphs, Motion::Reduced)
}

#[test]
fn ascii_glyphs_leave_no_unicode_in_the_chrome_or_notices() {
    // Lines are built (and their glyphs baked in) at the moment an event is applied, so the whole
    // exchange happens inside the scoped theme, exactly as it does in a session.
    scoped(
        tier_theme(ColorTier::Mono, Mode::Dark, GlyphTier::Ascii),
        || {
            let mut app = routed(located());
            app.temper = "Auto-edit".into();
            app.remote_active = true;
            app.done = true;
            app.last_stop_reason = Some(forge_types::StopReason::BudgetExhausted);
            app.apply(PresenterEvent::Warning("careful".into()));
            app.apply(PresenterEvent::Error("broke".into()));
            let notices: Vec<String> = app
                .drain_flush()
                .iter()
                .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect();
            assert!(notices.iter().all(|n| n.is_ascii()), "{notices:?}");
            for row in screen_rows(&app, 100, 30) {
                assert!(row.is_ascii(), "non-ASCII glyph in the chrome: {row}");
            }
            app.status_open = true;
            for row in screen_rows(&app, 100, 30) {
                assert!(row.is_ascii(), "non-ASCII glyph in /status: {row}");
            }
        },
    );
}

#[test]
fn mono_and_sixteen_colour_paint_no_backgrounds_and_light_has_no_dark_slab() {
    let mut app = routed(located());
    app.temper = "Auto-edit".into();
    app.status_open = true;
    for (tier, mode, paints) in [
        (ColorTier::Mono, Mode::Dark, false),
        (ColorTier::Ansi16, Mode::Dark, false),
        (ColorTier::Ansi16, Mode::Light, false),
        (ColorTier::TrueColor, Mode::Light, true),
    ] {
        scoped(tier_theme(tier, mode, GlyphTier::Unicode), || {
            let buffer = render_buffer(&app, 100, 30);
            let filled: Vec<_> = buffer
                .content
                .iter()
                .filter(|c| c.bg != Color::Reset)
                .collect();
            if paints {
                // Light truecolor: only the modal's light `surface` fill and the cursor block.
                for cell in &filled {
                    let [r, g, b] = crate::theme::degrade::color_rgb(cell.bg)
                        .unwrap_or_else(|| panic!("unexpected bg {:?}", cell.bg));
                    assert!(
                        r.min(g).min(b) > 0xA0 || cell.bg == color::brand(),
                        "dark slab painted in light mode: {:?}",
                        cell.bg
                    );
                }
            } else {
                assert!(
                    filled.is_empty(),
                    "{tier:?} {mode:?} painted {} backgrounds",
                    filled.len()
                );
            }
        });
    }
}

#[test]
fn light_mode_modals_do_not_fill_with_the_old_dark_slab() {
    let mut app = routed(located());
    app.command_center.open();
    scoped(
        tier_theme(ColorTier::TrueColor, Mode::Light, GlyphTier::Unicode),
        || {
            let buffer = render_buffer(&app, 100, 30);
            for cell in buffer.content.iter().filter(|c| c.bg != Color::Reset) {
                let [r, g, b] = crate::theme::degrade::color_rgb(cell.bg).expect("rgb fill");
                assert!(
                    r.min(g).min(b) > 0x40 || cell.bg == color::brand(),
                    "the SURFACE_BG / STATUS_BG slab is gone: {:?}",
                    cell.bg
                );
            }
        },
    );
}

#[test]
fn reduced_motion_freezes_the_composer_spinner() {
    let mut app = routed(located());
    app.on_turn_start();
    app.busy = true;
    let live = |tick: usize, theme: Theme| {
        let mut app = app.clone();
        app.tick = tick;
        scoped(theme, || {
            screen_rows(&app, 100, 24)
                .into_iter()
                .find(|r| r.starts_with('╭'))
                .unwrap()
        })
    };
    let reduced = Theme::new(
        ColorTier::TrueColor,
        Mode::Dark,
        GlyphTier::Unicode,
        Motion::Reduced,
    );
    assert_eq!(live(1, reduced), live(5, reduced));
    assert!(
        live(1, reduced).contains("… Preparing"),
        "{}",
        live(1, reduced)
    );
    assert_ne!(live(1, Theme::default()), live(5, Theme::default()));
}

#[test]
fn the_backdrop_is_not_a_painted_fill_in_any_tier() {
    for tier in [
        ColorTier::TrueColor,
        ColorTier::Ansi256,
        ColorTier::Ansi16,
        ColorTier::Mono,
    ] {
        scoped(tier_theme(tier, Mode::Dark, GlyphTier::Unicode), || {
            let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
            terminal
                .draw(|f| crate::surface::render_backdrop(f, f.area()))
                .unwrap();
            assert!(terminal
                .backend()
                .buffer()
                .content
                .iter()
                .all(|c| c.bg == Color::Reset));
        });
    }
}

/// The colour of the permission block's bottom-left corner: the border the pulse drives.
fn composer_corner_fg(app: &App) -> Color {
    let rows = screen_rows(app, 80, 24);
    let y = rows
        .iter()
        .position(|r| r.starts_with(" ╰"))
        .expect("decision block bottom border") as u16;
    render_buffer(app, 80, 24)[(1, y)].fg
}

#[test]
fn the_border_pulses_for_five_seconds_while_waiting_for_you_then_holds_warn() {
    let mut app = located();
    app.open_permission("write_file", "Write");
    app.tick = 0;
    let warn = composer_corner_fg(&app);
    assert_eq!(warn, color::warn(), "the first second is warn");
    app.tick = 17; // about one second on
    assert_eq!(
        composer_corner_fg(&app),
        color::fg_muted(),
        "second two dims"
    );
    app.tick = 34; // about two seconds on
    assert_eq!(composer_corner_fg(&app), warn);
    app.tick = 120; // well past five seconds
    assert_eq!(
        composer_corner_fg(&app),
        warn,
        "it holds warn after the pulse window"
    );
    // Reduced motion never pulses.
    let reduced = Theme::new(
        ColorTier::TrueColor,
        Mode::Dark,
        GlyphTier::Unicode,
        Motion::Reduced,
    );
    scoped(reduced, || {
        let mut app = located();
        app.open_permission("write_file", "Write");
        app.tick = 0;
        composer_corner_fg(&app);
        app.tick = 17;
        assert_eq!(composer_corner_fg(&app), color::warn());
    });
}

#[test]
fn the_chrome_never_panics_at_tiny_sizes_in_any_state() {
    let mut states = Vec::new();
    for fullscreen in [true, false] {
        let mut idle = routed(located());
        idle.fullscreen = fullscreen;
        let mut busy = idle.clone();
        busy.on_turn_start();
        busy.busy = true;
        busy.turn_elapsed_secs = 3725;
        busy.turn_out = 1_234_567;
        busy.temper = "Read-only".into();
        busy.queued = vec!["a".into(), "b".into()];
        busy.goal_chip = Some("2/5".into());
        busy.loop_chip = Some("5m".into());
        let mut stopped = idle.clone();
        stopped.done = true;
        stopped.last_stop_reason = Some(forge_types::StopReason::TasksUnfinished);
        let mut waiting = idle.clone();
        waiting.open_permission("write_file", "Write");
        waiting.remote_active = true;
        let mut status = busy.clone();
        status.status_open = true;
        states.extend([idle, busy, stopped, waiting, status]);
    }
    for app in &states {
        for w in [1, 2, 3, 4, 5, 8, 13, 21, 34, 40u16] {
            for h in [1, 2, 3, 4, 6, 10u16] {
                let _ = screen_rows(app, w, h);
            }
        }
    }
    for tier in [ColorTier::Mono, ColorTier::Ansi16] {
        scoped(tier_theme(tier, Mode::Light, GlyphTier::Ascii), || {
            for app in &states {
                for (w, h) in [(7, 3), (30, 8), (80, 24)] {
                    let _ = screen_rows(app, w, h);
                }
            }
        });
    }
}

#[test]
fn goal_and_loop_chips_show_progress_on_the_composer_border() {
    let mut app = located();
    app.set_autonomy_chips(Some(2), Some(3));
    let top = screen_rows(&app, 100, 24)
        .into_iter()
        .find(|r| r.starts_with('╭'))
        .unwrap();
    assert!(top.contains("↻ iteration 3 · ◎ iteration 2"), "{top}");
    app.tasks = vec![
        todo("a", forge_types::TodoStatus::Done),
        todo("b", forge_types::TodoStatus::Pending),
    ];
    app.set_autonomy_chips(Some(2), None);
    let top = screen_rows(&app, 100, 24)
        .into_iter()
        .find(|r| r.starts_with('╭'))
        .unwrap();
    assert!(top.contains("◎ 1/2"), "{top}");
    assert!(!top.contains('↻'), "{top}");
    app.set_autonomy_chips(None, None);
    assert!(app.goal_chip.is_none() && app.loop_chip.is_none());
}

#[test]
fn status_fits_the_inline_region_and_wraps_long_rows_instead_of_clipping() {
    let mut app = routed(located());
    app.fullscreen = false;
    app.temper = "Auto-edit".into();
    app.git_workspace =
        Some("/home/someone/a/very/long/path/to/a/project/checkout/worktree".into());
    app.remote_active = true;
    app.status_open = true;
    let rows = screen_rows(&app, 80, LIVE_H);
    assert_eq!(rows.len(), 14);
    let all = rows.join("\n");
    for needle in [
        "MODEL",
        "MODE",
        "WHERE",
        "CONTEXT",
        "TURN",
        "SESSION",
        "QUOTAS",
        "MCP",
        "VERSION",
        "Esc close",
    ] {
        assert!(
            all.contains(needle),
            "{needle} cut off in the inline region:\n{all}"
        );
    }
    assert!(
        all.contains("remote control on"),
        "a long WHERE row wraps rather than clips:\n{all}"
    );
    assert!(
        rows.last().unwrap().contains("ctx"),
        "the status line stays under the panel:\n{all}"
    );
}
