use super::*;

#[test]
fn subagent_batch_animates_live_then_folds_into_a_scrollback_box() {
    let mut app = App::default();
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "reviewer".into(),
        task: "review the diff".into(),
        model: Some("anthropic::opus".into()),
        phase: None,
    });
    app.apply(PresenterEvent::SubagentStart {
        id: "b".into(),
        agent: "general".into(),
        task: "find call sites".into(),
        model: Some("groq::llama".into()),
        phase: None,
    });
    app.activity_focused = true;

    // Both children appear in the unified activity list while running.
    let live = screen(&app);
    assert!(
        live.contains("reviewer"),
        "running child shown in activity list: {live}"
    );
    // The activity panel shows each child's model (stripped to the short name).
    assert!(live.contains("· opus"), "child model shown: {live}");

    app.apply(PresenterEvent::SubagentProgress {
        id: "a".into(),
        snippet: "inspecting auth".into(),
    });

    app.apply(PresenterEvent::SubagentResult {
        id: "a".into(),
        agent: "reviewer".into(),
        ok: true,
        summary: "2 issues".into(),
        cost_usd: 0.001,
    });
    // Box stays open (b still running) → b animates, a has flowed to scrollback.
    assert!(
        screen(&app).contains("general"),
        "remaining child still live"
    );

    app.apply(PresenterEvent::SubagentResult {
        id: "b".into(),
        agent: "general".into(),
        ok: true,
        summary: "5 sites".into(),
        cost_usd: 0.002,
    });

    // Once all done: the live panel switches to "done" state (still shows agent names) and the
    // group box also lands in scrollback. on_turn_start collapses the panel for the next turn.
    let done_screen = screen(&app);
    assert!(
        done_screen.contains("Agents"),
        "activity panel visible: {done_screen}"
    );
    assert!(
        done_screen.contains("reviewer"),
        "done panel shows agent names: {done_screen}"
    );
    let sb = flush_text(&mut app);
    assert!(sb.contains("◆ Agents"), "group header: {sb}");
    assert!(
        sb.contains("reviewer") && sb.contains("2 issues"),
        "branch: {sb}"
    );
    assert!(sb.contains("2 agents"), "footer with count: {sb}");
    // Panel collapses at next turn start, not on completion.
    app.on_turn_start();
    assert!(
        !screen(&app).contains("reviewer"),
        "panel gone after on_turn_start"
    );
}

#[test]
fn inline_welcome_is_a_compact_identity_row_and_keys_row() {
    let rows: Vec<String> = banner_lines(160)
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    let text = rows.join("\n");
    assert!(text.contains("✦ forge"), "identity row: {text}");
    assert!(text.contains("Your coding partner"), "tagline: {text}");
    assert!(
        text.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))),
        "version: {text}"
    );
    assert!(
        text.contains("Keys") && text.contains("Ctrl+K"),
        "keys: {text}"
    );
    assert!(
        !text.contains('█'),
        "no block art in the inline welcome: {text}"
    );
    assert!(rows.len() <= 5, "3-4 compact lines plus padding: {rows:?}");
}

#[test]
fn narrow_inline_welcome_clips_the_tagline_and_drops_keys() {
    let rows: Vec<String> = banner_lines(40)
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    for row in &rows {
        assert!(
            unicode_width::UnicodeWidthStr::width(row.as_str()) <= 40,
            "row fits 40 cells: {row:?}"
        );
    }
}

#[test]
fn welcome_shows_only_on_an_empty_transcript_and_vanishes_after_the_first_message() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    let rows = screen_rows(&app, 160, 48).join("\n");
    assert!(rows.contains("Your coding partner"), "{rows}");
    assert!(
        rows.contains("█▀▀"),
        "tall terminals draw the wordmark: {rows}"
    );
    assert!(rows.contains("Try"), "{rows}");
    assert!(rows.contains("Keys"), "{rows}");

    app.submit_user("hello");
    app.drain_flush();
    let rows = screen_rows(&app, 160, 48).join("\n");
    assert!(!rows.contains("Your coding partner"), "{rows}");
    assert!(rows.contains("hello"), "{rows}");
}

#[test]
fn welcome_stays_within_thirteen_rows_at_80x24() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.set_project_initialized(false);
    app.set_recent_sessions(vec![("hello".into(), "3m ago".into())]);
    let lines = crate::app::render::welcome::welcome_lines(&app, 80, 24);
    assert!(lines.len() <= 13, "{} rows", lines.len());
    let text = screen_rows(&app, 80, 24).join("\n");
    assert!(
        text.contains("✦ forge"),
        "one-row mark on short terminals: {text}"
    );
    assert!(!text.contains("█▀▀"), "no wordmark at 24 rows: {text}");
    assert!(text.contains("/init"), "onboarding nudge: {text}");
    assert!(text.contains("Recent"), "recent sessions: {text}");
}

#[test]
fn onboarding_line_is_state_and_clears_once_the_project_is_initialised() {
    let dir = std::env::temp_dir().join(format!("forge-welcome-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = App {
        fullscreen: true,
        git_workspace: Some(dir.clone()),
        ..Default::default()
    };
    app.set_project_initialized(false);
    assert!(
        app.drain_flush().is_empty(),
        "fullscreen pushes no scrollback warning"
    );
    let text = screen_rows(&app, 100, 40).join("\n");
    assert!(text.contains("isn't set up for Forge"), "{text}");

    app.panels_ui = Default::default();
    app.set_project_initialized(true);
    let text = screen_rows(&app, 100, 40).join("\n");
    assert!(!text.contains("set up for Forge"), "{text}");
}

#[test]
fn inline_uninitialised_project_still_gets_the_scrollback_warning() {
    let mut app = App::default();
    app.set_project_initialized(false);
    let sb = flush_text(&mut app);
    assert!(
        sb.contains("/init") && sb.contains("set up for Forge"),
        "{sb}"
    );
}

#[test]
fn tasks_panel_visible_while_active_and_absent_when_empty() {
    use forge_types::TodoStatus;
    let mut app = App::default();
    // No tasks → no sticky panel in the idle preview region.
    assert!(!screen(&app).contains("Tasks"), "panel hidden when empty");

    app.apply(PresenterEvent::Tasks(vec![
        todo("scan repo", TodoStatus::Done),
        todo("write tests", TodoStatus::InProgress),
        todo("ship it", TodoStatus::Pending),
    ]));
    assert!(
        screen(&app).contains("Tasks 1/3") && screen(&app).contains("write tests"),
        "collapsed band names progress and the task in progress"
    );
    app.activity_focused = true;
    let s = screen(&app);
    assert!(s.contains("Tasks 1/3"), "panel header + breakdown: {s}");
    // The in-progress item is shown with its glyph (prioritized into the small region).
    assert!(s.contains('◉'), "in-progress glyph shown: {s}");

    // Emptying the list collapses the panel.
    app.apply(PresenterEvent::Tasks(vec![]));
    assert!(
        !screen(&app).contains("Tasks"),
        "panel collapses when the list empties"
    );
}

#[test]
fn tasks_panel_names_the_assignee_of_a_delegated_task() {
    use forge_types::TodoStatus;
    let mut app = App {
        activity_focused: true,
        ..Default::default()
    };
    let delegated = forge_types::TodoItem {
        title: "port the store layer".into(),
        status: TodoStatus::InProgress,
        assignee: Some("builder-a3".into()),
    };
    app.apply(PresenterEvent::Tasks(vec![
        delegated,
        todo("review the diff", TodoStatus::Pending),
    ]));
    let s = screen(&app);
    assert!(
        s.contains("port the store layer @builder-a3"),
        "delegated task names its owner: {s}"
    );
    // A task the model kept gets no owner suffix invented for it.
    assert!(
        !s.contains("review the diff @"),
        "unassigned task stays bare: {s}"
    );
}

#[test]
fn plan_card_renders_title_steps_and_frame() {
    let plan = forge_types::PlanProposal {
        title: "Refactor main.rs".into(),
        steps: vec![
            forge_types::PlanStep {
                title: "Extract clap defs".into(),
                detail: "into cli/args.rs".into(),
            },
            forge_types::PlanStep {
                title: "Split dispatch".into(),
                detail: String::new(),
            },
        ],
        notes: Some("keep the CLI surface identical".into()),
    };
    let text = plan_card_lines(&plan)
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("PLAN"), "has the plan label: {text}");
    assert!(text.contains("2 steps"), "step count: {text}");
    assert!(text.contains("Refactor main.rs"), "has the title");
    assert!(text.contains("Extract clap defs") && text.contains("Split dispatch"));
    assert!(text.contains("into cli/args.rs"), "shows step detail");
    assert!(
        text.contains("keep the CLI surface identical"),
        "shows notes"
    );
    assert!(
        !text.contains('╭'),
        "the plan card is a block on the rail, not a box: {text}"
    );
    assert!(
        text.contains("1  Extract clap defs"),
        "numbered steps: {text}"
    );
    assert!(text.contains("Risks:"), "risks labelled: {text}");
    assert!(text.contains("approve to build"), "footer hint present");
}

#[test]
fn plan_card_wraps_long_text_within_the_frame() {
    let long = "Move every clap struct and enum out of main.rs into cli/args.rs without \
                changing any field, variant, attribute, or doc comment so the parsed CLI \
                surface stays byte-for-byte identical across the whole refactor";
    let plan = forge_types::PlanProposal {
        title: "Refactor".into(),
        steps: vec![forge_types::PlanStep {
            title: long.into(),
            detail: long.into(),
        }],
        notes: Some(long.into()),
    };
    let lines: Vec<String> = crate::transcript::wrap_lines(&plan_card_lines(&plan), 80)
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    // The long step title wraps, and its continuation rows hang under the title, not the number.
    let step_start = lines
        .iter()
        .position(|l| l.starts_with("  1  Move every"))
        .expect("step one row");
    let continuation = &lines[step_start + 1];
    assert!(
        continuation.starts_with("     ") && !continuation.trim().is_empty(),
        "wrapped text hangs at the step title: {continuation:?}"
    );
    for l in &lines {
        assert!(
            unicode_width::UnicodeWidthStr::width(l.as_str()) <= 80,
            "nothing runs past the measure: {l:?}"
        );
    }
    assert!(
        lines
            .iter()
            .filter(|l| l.starts_with("  1 ") || l.starts_with("     "))
            .count()
            > 3
    );
}

#[test]
fn tasks_panel_stays_visible_while_streaming() {
    use forge_types::TodoStatus;
    let mut app = App::default();
    app.apply(PresenterEvent::Tasks(vec![todo("a", TodoStatus::Pending)]));
    app.activity_focused = true;
    // The task panel has its OWN region now, so a live reply does NOT mask it — both show.
    app.apply(PresenterEvent::AssistantDelta("streaming now".into()));
    let s = screen(&app);
    assert!(s.contains("streaming now"), "stream shown: {s}");
    assert!(
        s.contains("Tasks 0/1"),
        "tasks panel stays visible while streaming: {s}"
    );
}

#[test]
fn subagent_panel_stays_visible_while_streaming() {
    let mut app = App::default();
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "reviewer".into(),
        task: "review the diff".into(),
        model: Some("anthropic::opus".into()),
        phase: None,
    });
    app.apply(PresenterEvent::AssistantDelta("thinking out loud".into()));
    app.activity_focused = true;
    let s = screen(&app);
    assert!(s.contains("thinking out loud"), "stream shown: {s}");
    assert!(
        s.contains("Agents"),
        "activity panel stays visible while streaming: {s}"
    );
}

#[test]
fn subagent_panel_present_while_running_and_after_done_collapses_on_next_turn() {
    let mut app = App::default();
    assert_eq!(app.running_subagents(), 0);
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "reviewer".into(),
        task: "review the diff".into(),
        model: Some("anthropic::opus".into()),
        phase: None,
    });
    assert_eq!(app.running_subagents(), 1);
    app.activity_focused = true;
    let s = screen(&app);
    assert!(s.contains("Agents"), "panel header while running: {s}");
    assert!(s.contains("reviewer"), "agent label shown: {s}");

    // After SubagentResult the panel stays visible (shows "done") so it's never invisible
    // when Start+Result arrive in the same render drain (the common bridge case).
    app.apply(PresenterEvent::SubagentResult {
        id: "a".into(),
        agent: "reviewer".into(),
        ok: true,
        summary: "ok".into(),
        cost_usd: 0.001,
    });
    assert_eq!(
        app.running_subagents(),
        0,
        "no running children after result"
    );
    let s = screen(&app);
    assert!(
        s.contains("Agents"),
        "panel stays visible showing done state: {s}"
    );

    // The panel collapses at the START of the next user turn, not immediately on result.
    app.on_turn_start();
    assert!(
        !screen(&app).contains("activity ("),
        "panel collapses after on_turn_start: {s}"
    );
}

#[test]
fn follow_up_result_finishes_the_active_reused_id_row() {
    let mut app = App::default();
    app.apply(PresenterEvent::SubagentStart {
        id: "persisted-child".into(),
        agent: "reviewer".into(),
        task: "review the diff".into(),
        model: Some("anthropic::opus".into()),
        phase: None,
    });
    app.apply(PresenterEvent::SubagentResult {
        id: "persisted-child".into(),
        agent: "reviewer".into(),
        ok: true,
        summary: "initial review complete".into(),
        cost_usd: 0.001,
    });

    app.apply(PresenterEvent::SubagentStart {
        id: "persisted-child".into(),
        agent: "reviewer".into(),
        task: "check the updated diff".into(),
        model: Some("anthropic::opus".into()),
        phase: None,
    });

    assert_eq!(app.subagents.len(), 2, "both historical rows are retained");
    assert!(app.subagents[0].done, "the initial task remains complete");
    assert!(!app.subagents[1].done, "the follow-up task is running");
    assert_eq!(app.running_subagents(), 1);

    app.apply(PresenterEvent::SubagentResult {
        id: "persisted-child".into(),
        agent: "reviewer".into(),
        ok: true,
        summary: "follow-up review complete".into(),
        cost_usd: 0.001,
    });

    assert!(app.subagents.iter().all(|row| row.done));
    assert_eq!(app.running_subagents(), 0);
}

#[test]
fn a_second_batch_in_the_same_turn_does_not_wipe_the_first() {
    let mut app = App::default();
    app.apply(PresenterEvent::SubagentStart {
        id: "phase1-a".into(),
        agent: "researcher".into(),
        task: "survey the codebase".into(),
        model: Some("anthropic::opus".into()),
        phase: None,
    });
    app.apply(PresenterEvent::SubagentResult {
        id: "phase1-a".into(),
        agent: "researcher".into(),
        ok: true,
        summary: "found 3 relevant files".into(),
        cost_usd: 0.001,
    });
    assert_eq!(app.subagents.len(), 1, "phase 1's row exists");
    assert!(app.subagents[0].done);

    // A second batch starts IN THE SAME TURN (no `on_turn_start` in between) — e.g. a
    // workflow script's second phase. Before the fix, `SubagentStart` itself cleared every
    // prior finished row here, silently deleting phase 1's row (and its Ctrl+O-viewable
    // transcript) the instant phase 2 began.
    app.apply(PresenterEvent::SubagentStart {
        id: "phase2-a".into(),
        agent: "implementer".into(),
        task: "apply the fix".into(),
        model: Some("anthropic::sonnet".into()),
        phase: None,
    });
    assert_eq!(
        app.subagents.len(),
        2,
        "phase 1's row must survive phase 2 starting"
    );
    assert!(
        app.subagents.iter().any(|r| r.id == "phase1-a"),
        "phase 1's row specifically must still be present"
    );
    app.activity_focused = true;
    let s = screen(&app);
    assert!(
        s.contains("researcher") && s.contains("implementer"),
        "both phases' agents visible together: {s}"
    );

    // Only a genuinely new user turn clears the retained history.
    app.apply(PresenterEvent::SubagentResult {
        id: "phase2-a".into(),
        agent: "implementer".into(),
        ok: true,
        summary: "applied".into(),
        cost_usd: 0.002,
    });
    app.on_turn_start();
    assert!(
        app.subagents.is_empty(),
        "on_turn_start still clears at the real turn boundary"
    );
}

#[test]
fn a_second_batchs_scrollback_footer_counts_only_its_own_rows() {
    // Regression: retaining rows across batches (the fix above) must not make the SECOND
    // batch's "N agents · $total" scrollback footer sum every retained row from earlier
    // phases too — found via live end-to-end testing of a real two-phase workflow script,
    // where a 2-agent second phase's footer read "4 agents" (2 retained from phase 1 + 2 of
    // its own), not "2 agents".
    let mut app = App::default();
    app.apply(PresenterEvent::SubagentStart {
        id: "p1-a".into(),
        agent: "researcher".into(),
        task: "survey".into(),
        model: Some("anthropic::opus".into()),
        phase: Some("research".into()),
    });
    app.apply(PresenterEvent::SubagentResult {
        id: "p1-a".into(),
        agent: "researcher".into(),
        ok: true,
        summary: "done".into(),
        cost_usd: 0.001,
    });
    let phase1_flush = flush_text(&mut app);
    assert!(
        phase1_flush.contains("1 agent "),
        "phase 1's own footer: {phase1_flush}"
    );

    app.apply(PresenterEvent::SubagentStart {
        id: "p2-a".into(),
        agent: "implementer".into(),
        task: "fix".into(),
        model: Some("anthropic::sonnet".into()),
        phase: Some("fix".into()),
    });
    app.apply(PresenterEvent::SubagentStart {
        id: "p2-b".into(),
        agent: "implementer".into(),
        task: "fix more".into(),
        model: Some("anthropic::sonnet".into()),
        phase: Some("fix".into()),
    });
    app.apply(PresenterEvent::SubagentResult {
        id: "p2-a".into(),
        agent: "implementer".into(),
        ok: true,
        summary: "done".into(),
        cost_usd: 0.002,
    });
    app.apply(PresenterEvent::SubagentResult {
        id: "p2-b".into(),
        agent: "implementer".into(),
        ok: true,
        summary: "done".into(),
        cost_usd: 0.003,
    });
    let phase2_flush = flush_text(&mut app);
    assert!(
        phase2_flush.contains("2 agents"),
        "phase 2 has 2 of its own agents, not phase 1's total too: {phase2_flush}"
    );
    assert!(
        !phase2_flush.contains("3 agents") && !phase2_flush.contains("4 agents"),
        "must not cumulatively count phase 1's retained row: {phase2_flush}"
    );
}

#[test]
fn a_failed_subagent_shows_as_failed_not_a_false_success_checkmark() {
    // Regression: `ActivitySummary::status` only ever produced `Running`/`Done` for a
    // subagent row — a failed `agent()` call (found via live end-to-end testing of a real
    // workflow script whose subagents all errored) still rendered a green ✓ once `done`,
    // indistinguishable from success, even though the scrollback box right above it
    // correctly showed a red ✗ for the exact same agent.
    let mut app = App::default();
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "researcher".into(),
        task: "look something up".into(),
        model: Some("anthropic::opus".into()),
        phase: None,
    });
    app.apply(PresenterEvent::SubagentResult {
        id: "a".into(),
        agent: "researcher".into(),
        ok: false,
        summary: "error: hit the step limit".into(),
        cost_usd: 0.0,
    });
    let views = app.activity_summaries();
    let row = views
        .iter()
        .find(|v| v.kind == ActivityKind::Subagent)
        .expect("subagent row present");
    assert_eq!(
        row.status,
        ActivityStatus::Failed,
        "a failed agent call must not report Done"
    );
}

#[test]
fn activity_panel_groups_workflow_phases_with_header_lines() {
    let mut app = App::default();
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "researcher".into(),
        task: "survey the codebase".into(),
        model: Some("anthropic::opus".into()),
        phase: Some("research".into()),
    });
    app.apply(PresenterEvent::SubagentStart {
        id: "b".into(),
        agent: "researcher2".into(),
        task: "survey more".into(),
        model: Some("anthropic::opus".into()),
        phase: Some("research".into()),
    });
    app.apply(PresenterEvent::SubagentStart {
        id: "c".into(),
        agent: "implementer".into(),
        task: "apply the fix".into(),
        model: Some("anthropic::sonnet".into()),
        phase: Some("implement".into()),
    });
    app.activity_focused = true;
    // The default test screen's activity panel is only a few rows tall — too short to fit 3
    // rows + 2 phase headers alongside the input box/statusline. Use a taller one.
    let s = screen_wh(&app, 100, 30);
    assert!(s.contains("◆ research"), "research phase header shown: {s}");
    assert!(
        s.contains("◆ implement"),
        "implement phase header shown: {s}"
    );
    // Exactly one header per phase — the second `research` row must NOT repeat the header.
    assert_eq!(
        s.matches("◆ research").count(),
        1,
        "header shown once per phase, not once per row: {s}"
    );
}

#[test]
fn activity_panel_shows_no_phase_headers_for_a_plain_spawn_agents_batch() {
    let mut app = App::default();
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "reviewer".into(),
        task: "review the diff".into(),
        model: Some("anthropic::opus".into()),
        phase: None,
    });
    let s = screen(&app);
    assert!(
        !s.contains('▶'),
        "no phase header glyph for a plain batch: {s}"
    );
}

#[test]
fn mesh_overlay_renders_without_panic() {
    let mesh_overlay = MeshOverlay {
        pick_effort: None,
        open: true,
        loading: false,
        prompt: "design a lock-free queue".into(),
        classified: "complex".into(),
        classifier: "heuristic".into(),
        routed: "complex".into(),
        code_heavy: false,
        reasons: "reasoning term".into(),
        conserve_fired: true,
        conserve_line: "FIRED (roll 0.05 < P 0.53) → spread to free frontier".into(),
        quota: vec![MeshQuotaRow {
            provider: "claude-cli".into(),
            fraction: Some(0.78),
            plan: "max-20x".into(),
            status: "Ok".into(),
            spread_complex: 0.5,
            projected_fraction_at_reset: Some(0.93),
            exhaustion_warning: true,
            pace_note: "weekly 78% used · 21% allowed · OVER PACE → opus held, sonnet".into(),
            over_pace: true,
        }],
        candidates: vec![
            MeshCandRow {
                effort: None,
                rank: 1,
                model: "groq::llama-3.3-70b-versatile".into(),
                score: 6.65,
                cost_tag: "free".into(),
                frontier: true,
                usable: true,
                selected: true,
                penalty: 0.0,
                reorder_reason: None,
            },
            MeshCandRow {
                effort: None,
                rank: 2,
                model: "codex-cli::gpt-5.5".into(),
                score: 3.05,
                cost_tag: "subscription".into(),
                frontier: true,
                usable: true,
                selected: false,
                penalty: 4.0,
                reorder_reason: None,
            },
        ],
        pick: "groq::llama-3.3-70b-versatile".into(),
        fallbacks: vec!["codex-cli::gpt-5.5".into()],
        rationale: "auto-selected best".into(),
        anim_tick: 50, // fully revealed
        cursor: 0,
    };
    let app = App {
        mesh_overlay,
        ..Default::default()
    };
    let s = screen_wh(&app, 100, 30);
    assert!(s.contains("Model routing"), "title rendered");
    assert!(s.contains("groq::llama-3.3-70b-versatile"), "pick shown");
    assert!(
        s.contains("fallbacks"),
        "fallback chain remains discoverable"
    );
    // A tiny terminal must not panic on the layout math.
    let _ = screen_wh(&app, 30, 6);
}

#[test]
fn subagent_views_capture_full_transcript_and_result() {
    let mut app = App::default();
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "general".into(),
        task: "find call sites".into(),
        model: Some("groq::llama".into()),
        phase: None,
    });
    // More progress than the old 200-snippet cap — the full transcript must be kept. Each
    // snippet ends in a newline so the line-assembler keeps them as distinct lines.
    for i in 0..250 {
        app.apply(PresenterEvent::SubagentProgress {
            id: "a".into(),
            snippet: format!("step {i}\n"),
        });
    }
    app.apply(PresenterEvent::SubagentResult {
        id: "a".into(),
        agent: "general".into(),
        ok: true,
        summary: "found 3 call sites".into(),
        cost_usd: 0.01,
    });
    // activity_views = main chat (index 0) + the subagent (index 1), retained after the batch
    // finishes so the full-screen viewer can still open them.
    let views = app.activity_views();
    assert_eq!(views.len(), 2);
    assert_eq!(views[0].kind, ActivityKind::MainChat);
    let v = &views[1];
    assert_eq!(v.kind, ActivityKind::Subagent);
    assert_eq!(v.status, ActivityStatus::Done);
    assert!(v.line_count > 200, "full log kept: {}", v.line_count);
    let body: String = v
        .lines
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(body.contains("step 0"), "oldest line kept: {body}");
    assert!(body.contains("step 249"), "newest line kept");
    assert!(
        body.contains("found 3 call sites"),
        "result appended to transcript"
    );

    // A new batch starting in the same turn RETAINS the previous (finished) batch's rows —
    // both stay viewable, e.g. for a multi-phase workflow's earlier phases.
    app.apply(PresenterEvent::SubagentStart {
        id: "b".into(),
        agent: "general".into(),
        task: "next".into(),
        model: None,
        phase: None,
    });
    let views = app.activity_views();
    assert_eq!(views.len(), 3, "main chat + both batches' subagents");
    assert_eq!(
        views[1].subtitle, "find call sites",
        "batch a's row survives"
    );
    assert_eq!(views[2].subtitle, "next");
}

#[test]
fn panel_budget_gives_both_full_height_when_they_fit() {
    // Full-screen mode: a generous budget → each panel keeps its full desired height.
    assert_eq!(split_panel_budget(9, 7, 40), (9, 7));
}

#[test]
fn panel_budget_splits_fairly_when_contended() {
    // Both want more than half of a tight inline budget → fair split, neither starved.
    let (a, t) = split_panel_budget(9, 7, 10);
    assert_eq!(a + t, 10, "uses the whole budget");
    assert!(a >= 4 && t >= 4, "neither panel is starved: {a},{t}");
    // A small panel keeps its full size; the big one takes the slack.
    assert_eq!(split_panel_budget(2, 9, 10), (2, 8));
}

#[test]
fn a_live_workflow_owns_its_agent_rows_and_never_touches_the_activity_panel() {
    let mut app = App::default();
    app.apply(PresenterEvent::WorkflowStarted { name: None });
    assert!(
        app.workflow.open && app.workflow.active,
        "the view auto-opens when a workflow starts"
    );
    app.apply(PresenterEvent::WorkflowPhase {
        title: "research".into(),
    });
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "general".into(),
        task: "dig".into(),
        model: Some("m".into()),
        phase: Some("research".into()),
    });
    app.apply(PresenterEvent::SubagentProgress {
        id: "a".into(),
        snippet: "reading\n".into(),
    });
    app.apply(PresenterEvent::SubagentResult {
        id: "a".into(),
        agent: "general".into(),
        ok: true,
        summary: "found it".into(),
        cost_usd: 0.01,
    });
    // The whole point of the dedicated view: nothing lands in the activity panel.
    assert!(
        !app.has_activity(),
        "workflow rows must not populate the subagent activity panel"
    );
    assert_eq!(app.workflow.rows.len(), 1);
    assert!(app.workflow.rows[0].done && app.workflow.rows[0].ok);
    assert_eq!(app.workflow.phases[0].title, "research");
    app.apply(PresenterEvent::WorkflowFinished {
        ok: true,
        summary: "report".into(),
    });
    assert!(!app.workflow.active);
    assert!(app.workflow.exists(), "rows stay browsable after the run");
    // A new user turn clears the finished run.
    app.on_turn_start();
    assert!(!app.workflow.exists());
}

#[test]
fn a_plain_spawn_agents_batch_still_uses_the_activity_panel() {
    let mut app = App::default();
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "general".into(),
        task: "x".into(),
        model: Some("m".into()),
        phase: None,
    });
    assert!(
        app.has_activity(),
        "no workflow → the panel works as before"
    );
    assert!(app.workflow.rows.is_empty());
}

#[test]
fn the_workflow_view_renders_name_phases_rows_totals_and_narration() {
    let mut app = App::default();
    app.apply(PresenterEvent::WorkflowStarted {
        name: Some("audit".into()),
    });
    app.apply(PresenterEvent::WorkflowPhase {
        title: "research".into(),
    });
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "general".into(),
        task: "scan the repo".into(),
        model: Some("prov::mini".into()),
        phase: Some("research".into()),
    });
    app.apply(PresenterEvent::SubagentProgress {
        id: "a".into(),
        snippet: "reading files".into(),
    });
    app.apply(PresenterEvent::SubagentStart {
        id: "b".into(),
        agent: "general".into(),
        task: "check docs".into(),
        model: Some("prov::mini".into()),
        phase: Some("research".into()),
    });
    app.apply(PresenterEvent::SubagentResult {
        id: "b".into(),
        agent: "general".into(),
        ok: true,
        summary: "ok".into(),
        cost_usd: 0.02,
    });
    app.apply(PresenterEvent::WorkflowLog("halfway there".into()));
    app.workflow.anim_tick = 1_000; // reveal settled — everything visible
    let out = screen_wh(&app, 100, 24);
    assert!(
        out.contains("workflow") && out.contains("audit"),
        "header: {out:?}"
    );
    assert!(out.contains("1  research"), "phase checklist: {out:?}");
    assert!(out.contains("1/2 agents"), "totals: {out:?}");
    assert!(out.contains("1 running"), "running count: {out:?}");
    assert!(out.contains("halfway there"), "narration feed: {out:?}");
    assert!(out.contains("reading files"), "live activity edge: {out:?}");
    // The scrollback record still tells the story after the view closes.
    let flushed = flush_text(&mut app);
    assert!(flushed.contains("workflow audit started"));
    assert!(flushed.contains("phase research"));
}

#[test]
fn a_workflow_log_flood_lands_in_the_feed_not_scrollback() {
    let mut app = App::default();
    app.apply(PresenterEvent::WorkflowStarted { name: None });
    // The exact failure seen live: a script `log()`s its entire multi-KB report, which used
    // to land in scrollback verbatim as one giant warning line.
    let report = "REPORT LINE\n".repeat(500);
    app.apply(PresenterEvent::WorkflowLog(report));
    let flushed = flush_text(&mut app);
    assert_eq!(
        flushed.matches("REPORT LINE").count(),
        1,
        "scrollback gets one trimmed note, the feed keeps the rest: {flushed:?}"
    );
}

#[test]
fn plan_card_selection_drops_frame_and_padding() {
    let plan = forge_types::PlanProposal {
        title: "Ship it".into(),
        steps: vec![forge_types::PlanStep {
            title: "do the thing".into(),
            detail: String::new(),
        }],
        notes: None,
    };
    let rows = crate::transcript::wrap_lines(&plan_card_lines(&plan), 100);
    let text = crate::copy_mark::selection_text(rows.iter().map(|r| (r, 0, usize::MAX)));
    assert!(text.contains("do the thing"), "{text:?}");
    assert!(!text.contains(['╭', '╮', '╰', '╯', '│', '─']), "{text:?}");
    assert!(text.lines().all(|l| l == l.trim_end()), "{text:?}");
}

fn tasks_and_agent_app() -> App {
    use forge_types::TodoStatus;
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.submit_user("go");
    app.drain_flush();
    app.apply(PresenterEvent::Tasks(vec![
        todo("Read the uploader", TodoStatus::Done),
        todo("Wire retry into the uploader", TodoStatus::InProgress),
        todo("Add a test for the give-up path", TodoStatus::Pending),
    ]));
    app.apply(PresenterEvent::SubagentStart {
        id: "a".into(),
        agent: "reviewer".into(),
        task: "review".into(),
        model: Some("anthropic::opus".into()),
        phase: None,
    });
    app
}

#[test]
fn collapsed_band_shows_progress_task_and_running_agents() {
    let app = tasks_and_agent_app();
    let rows = screen_rows(&app, 100, 24);
    let band = rows
        .iter()
        .find(|r| r.contains("Tasks 1/3"))
        .unwrap_or_else(|| panic!("band row missing: {rows:#?}"));
    assert!(band.contains("◉ Wire retry into the uploader"), "{band}");
    assert!(band.contains("1 agent running"), "{band}");
    assert!(band.contains("Ctrl+O details"), "{band}");
}

#[test]
fn expanded_band_lists_tasks_and_agents_below_150_columns() {
    let mut app = tasks_and_agent_app();
    app.activity_focused = true;
    let text = screen_rows(&app, 100, 30).join("\n");
    assert!(text.contains("▾ Tasks 1/3"), "{text}");
    assert!(text.contains("✓ Read the uploader"), "{text}");
    assert!(text.contains("○ Add a test for the give-up path"), "{text}");
    assert!(text.contains("▾ Agents 1"), "{text}");
    assert!(text.contains("reviewer · opus"), "{text}");
    assert!(
        !text.contains("CONTEXT"),
        "no rail below 150 columns: {text}"
    );
}

#[test]
fn details_become_a_38_column_rail_from_150_columns() {
    use crate::app::render::panels::rail_width;
    let mut app = tasks_and_agent_app();
    assert_eq!(
        rail_width(&app, Rect::new(0, 0, 160, 40)),
        0,
        "collapsed: no rail"
    );
    app.activity_focused = true;
    assert_eq!(rail_width(&app, Rect::new(0, 0, 160, 40)), 38);
    assert_eq!(
        rail_width(&app, Rect::new(0, 0, 149, 40)),
        0,
        "narrower than 150"
    );

    let rows = screen_rows(&app, 160, 48);
    let text = rows.join("\n");
    assert!(
        text.contains("TASKS 1/3") && text.contains("AGENTS"),
        "{text}"
    );
    assert!(text.contains("CONTEXT"), "{text}");
    assert!(
        !text.contains("▾ Tasks"),
        "the band folds into the rail: {text}"
    );
    let rail_col = 160 - 38;
    assert!(
        rows[3].chars().nth(rail_col) == Some('│'),
        "rule on the rail's left edge: {:?}",
        rows[3]
    );
    // The transcript shrank with it: its hit-test geometry stops at the rail.
    let geom = app.transcript_geom.get().expect("transcript drawn");
    assert_eq!(
        geom.col0 + geom.width,
        160 - 38,
        "selection stays left of the rail"
    );
}

#[test]
fn rail_never_opens_inline() {
    use crate::app::render::panels::rail_width;
    let mut app = tasks_and_agent_app();
    app.fullscreen = false;
    app.activity_focused = true;
    assert_eq!(rail_width(&app, Rect::new(0, 0, 200, 14)), 0);
}

#[test]
fn task_elapsed_counts_for_running_work_and_freezes_when_done() {
    use forge_types::TodoStatus;
    let mut app = tasks_and_agent_app();
    app.activity_focused = true;
    let _ = screen_rows(&app, 100, 30);
    assert!(app
        .panel_elapsed("task:Wire retry into the uploader", true, false)
        .is_some());
    app.apply(PresenterEvent::Tasks(vec![todo(
        "Wire retry into the uploader",
        TodoStatus::Done,
    )]));
    let a = app.panel_elapsed("task:Wire retry into the uploader", false, true);
    let b = app.panel_elapsed("task:Wire retry into the uploader", false, true);
    assert_eq!(a, b, "finished tasks stop counting");
    assert!(app.panel_elapsed("task:never seen", false, true).is_none());
}

#[test]
fn panel_cost_formatting_table() {
    use crate::app::render::panels::fmt_cost;
    for (usd, want) in [
        (0.0, "$0"),
        (0.004, "<$0.01"),
        (0.01, "$0.01"),
        (0.126, "$0.13"),
        (12.0, "$12.00"),
    ] {
        assert_eq!(fmt_cost(usd), want, "{usd}");
    }
}

#[test]
fn btw_card_shows_question_markdown_answer_and_provenance_in_the_footer() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.submit_user("hello");
    app.drain_flush();
    app.open_btw("what is this");
    let waiting = screen_rows(&app, 100, 30).join("\n");
    assert!(waiting.contains("what is this"), "{waiting}");
    assert!(
        waiting.contains("asking"),
        "spinner row while it runs: {waiting}"
    );
    assert!(waiting.contains("Esc close"), "{waiting}");

    app.apply(PresenterEvent::BtwAnswer {
        question: "what is this".into(),
        answer: "It **renders** the chat.\n\n- pickers\n- the board".into(),
        model: "ollama::llama3.2".into(),
        cost_usd: Some(0.0),
    });
    let rows = screen_rows(&app, 160, 30);
    let text = rows.join("\n");
    assert!(text.contains("╭─ btw"), "titled frame: {text}");
    assert!(text.contains("side question · not saved"), "{text}");
    assert!(
        text.contains("renders") && !text.contains("**"),
        "markdown rendered: {text}"
    );
    assert!(text.contains("pickers"), "{text}");
    let footer = rows
        .iter()
        .find(|r| r.contains("scroll"))
        .expect("footer row");
    assert!(
        footer.contains("ollama::llama3.2") && footer.contains("Esc close"),
        "{footer}"
    );
    let top = rows.iter().find(|r| r.contains("╭─ btw")).unwrap();
    let frame_w = top.chars().filter(|c| *c != ' ').count();
    assert!(
        frame_w <= 84 + 30,
        "card stays narrow at 160 columns: {top:?}"
    );
    let card_cols: Vec<usize> = top
        .char_indices()
        .filter(|(_, c)| *c == '╭' || *c == '╮')
        .map(|(i, _)| i)
        .collect();
    assert!(card_cols[1] - card_cols[0] <= 3 * 84, "{top:?}");
}

#[test]
fn btw_scroll_stops_at_the_end_of_the_answer() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.open_btw("q");
    app.apply(PresenterEvent::BtwAnswer {
        question: "q".into(),
        answer: (0..60).map(|i| format!("line {i}\n\n")).collect(),
        model: "m".into(),
        cost_usd: None,
    });
    let _ = screen_rows(&app, 100, 30);
    for _ in 0..500 {
        app.btw_key(&KeyKind::Down);
    }
    let scroll = app.btw.as_ref().unwrap().scroll;
    assert!(
        scroll > 0 && scroll < 200,
        "clamped to the content: {scroll}"
    );
    let text = screen_rows(&app, 100, 30).join("\n");
    assert!(
        text.contains("line 59"),
        "the last line is reachable: {text}"
    );
}

#[test]
fn workflow_view_is_a_header_phase_checklist_nested_agents_log_and_hint_row() {
    let mut app = App::default();
    app.apply(PresenterEvent::WorkflowStarted {
        name: Some("review-and-fix".into()),
    });
    app.apply(PresenterEvent::WorkflowPhase {
        title: "Survey".into(),
    });
    app.apply(PresenterEvent::SubagentStart {
        id: "s1".into(),
        agent: "scout".into(),
        task: "survey".into(),
        model: Some("anthropic::opus".into()),
        phase: Some("Survey".into()),
    });
    app.apply(PresenterEvent::SubagentResult {
        id: "s1".into(),
        agent: "scout".into(),
        ok: true,
        summary: "ok".into(),
        cost_usd: 0.06,
    });
    app.apply(PresenterEvent::WorkflowPhase {
        title: "Fix".into(),
    });
    app.apply(PresenterEvent::SubagentStart {
        id: "f1".into(),
        agent: "builder-a".into(),
        task: "fix it".into(),
        model: Some("moonshot::kimi-k3".into()),
        phase: Some("Fix".into()),
    });
    app.apply(PresenterEvent::SubagentProgress {
        id: "f1".into(),
        snippet: "editing src/upload.rs".into(),
    });
    app.apply(PresenterEvent::WorkflowLog(
        "builder-a: running cargo test".into(),
    ));
    app.workflow.anim_tick = 1_000;
    app.workflow.selected = 1;
    let rows = screen_rows(&app, 100, 24);
    let text = rows.join("\n");
    assert!(
        rows[0].contains("✦ workflow") && rows[0].contains("review-and-fix"),
        "{text}"
    );
    assert!(
        rows[0].contains("phase 2") && rows[0].contains("Esc background"),
        "{text}"
    );
    assert!(
        text.contains("✓ 1  Survey") && text.contains("1 agent"),
        "{text}"
    );
    assert!(
        text.contains("◉ 2  Fix") && text.contains("running"),
        "{text}"
    );
    let selected = rows.iter().find(|r| r.contains("builder-a")).unwrap();
    assert!(
        selected.contains("❯")
            && selected.contains("kimi-k3")
            && selected.contains("editing src/upload.rs"),
        "{selected}"
    );
    let nested = rows.iter().find(|r| r.contains("scout")).unwrap();
    assert!(
        !nested.contains('❯') && nested.contains("$0.06"),
        "{nested}"
    );
    assert!(
        text.contains("LOG") && text.contains("running cargo test"),
        "{text}"
    );
    let last = rows.last().unwrap();
    assert!(
        last.contains("select agent")
            && last.contains("Enter zoom")
            && last.contains("Ctrl+C stop"),
        "{last}"
    );
}

#[test]
fn welcome_try_focus_picks_an_example_into_the_composer() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    screen_rows(&app, 80, 24);
    assert!(
        !app.welcome_key(&KeyKind::Enter),
        "Enter is untouched unfocused"
    );
    app.input = "x".into();
    assert!(
        !app.welcome_key(&KeyKind::Down),
        "a draft keeps history keys"
    );
    app.input.clear();
    assert!(app.welcome_key(&KeyKind::Down));
    assert!(app.welcome_key(&KeyKind::Down));
    let text = screen_rows(&app, 80, 24).join("\n");
    assert!(text.contains("❯ explain how"), "{text}");
    assert!(app.welcome_key(&KeyKind::Up));
    assert!(app.welcome_key(&KeyKind::Enter));
    assert_eq!(app.input, "fix the failing test in this project");
    assert_eq!(app.input_cursor, app.input.len());
    assert!(app.panels_ui.try_focus.is_none());
    assert!(app.welcome_key(&KeyKind::Down) || !app.input.is_empty());
}

#[test]
fn welcome_try_focus_yields_to_typing_and_esc() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    screen_rows(&app, 80, 24);
    assert!(app.welcome_key(&KeyKind::Down));
    assert!(!app.welcome_key(&KeyKind::Char('a')));
    assert!(app.panels_ui.try_focus.is_none());
    assert!(app.welcome_key(&KeyKind::Down));
    assert!(app.welcome_key(&KeyKind::Esc));
    assert!(app.panels_ui.try_focus.is_none());
}

#[test]
fn the_welcome_keys_row_does_not_repeat_the_composer_hint() {
    let app = App {
        fullscreen: true,
        ..Default::default()
    };
    let rows = screen_rows(&app, 120, 40);
    let keys = rows
        .iter()
        .find(|r| r.contains("Keys"))
        .expect("a keys row");
    let border = rows
        .iter()
        .find(|r| r.contains("? keys"))
        .expect("the hint");
    for chord in ["Ctrl+K", "Shift+Tab", "commands", "files"] {
        assert!(border.contains(chord), "{border}");
        assert!(!keys.contains(chord), "{chord} printed twice: {keys}");
    }
    assert!(
        keys.contains("Ctrl+J") && keys.contains("newline"),
        "{keys}"
    );
}

/// A detached (background) agent admitted in one turn keeps its row after that turn ends: the
/// band and the rail list it while it runs, its later result settles it, and the repeat its
/// delivery at the next turn boundary sends is dropped instead of printed twice.
#[test]
fn a_detached_agent_is_listed_until_it_settles_and_its_delivery_is_not_repeated() {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.submit_user("look into it in the background");
    app.drain_flush();
    app.apply(PresenterEvent::SubagentStart {
        id: "6ddf5b5c-plan".into(),
        agent: "plan".into(),
        task: "draft the plan".into(),
        model: Some("inferhub::cb/deepseek-v4.1-flash".into()),
        phase: None,
    });
    let _ = flush_text(&mut app);

    // The admitting turn has ended; the child is still running in the background.
    let band = screen_rows(&app, 100, 24).join("\n");
    assert!(
        band.contains("Agents"),
        "band lists the running agent: {band}"
    );
    app.activity_focused = true;
    let rail = screen_rows(&app, 160, 40).join("\n");
    assert!(
        rail.contains("AGENTS") && rail.contains("plan"),
        "rail lists the detached agent: {rail}"
    );
    assert_eq!(app.remote_snapshot().subagents.len(), 1);
    assert!(!app.remote_snapshot().subagents[0].done);

    app.apply(PresenterEvent::SubagentProgress {
        id: "6ddf5b5c-plan".into(),
        snippet: "reading the router".into(),
    });
    app.apply(PresenterEvent::SubagentResult {
        id: "6ddf5b5c-plan".into(),
        agent: "plan".into(),
        ok: true,
        summary: "three steps".into(),
        cost_usd: 0.02,
    });
    let first = flush_text(&mut app);
    assert!(first.contains("three steps"), "{first}");
    let snap = app.remote_snapshot();
    assert!(snap.subagents[0].done && snap.subagents[0].ok);
    assert!((snap.subagents[0].cost - 0.02).abs() < 1e-9);

    // Next turn: the settled row folds away, then delivery repeats the result.
    app.on_turn_start();
    app.apply(PresenterEvent::SubagentResult {
        id: "6ddf5b5c-plan".into(),
        agent: "plan".into(),
        ok: true,
        summary: "three steps".into(),
        cost_usd: 0.02,
    });
    let repeat = flush_text(&mut app);
    assert!(
        !repeat.contains("three steps"),
        "a settled agent's repeated result is not printed again: {repeat}"
    );

    // `send_to_agent` reruns a child under its old id; that run's result counts.
    app.apply(PresenterEvent::SubagentStart {
        id: "6ddf5b5c-plan".into(),
        agent: "plan".into(),
        task: "refine".into(),
        model: None,
        phase: None,
    });
    app.apply(PresenterEvent::SubagentResult {
        id: "6ddf5b5c-plan".into(),
        agent: "plan".into(),
        ok: true,
        summary: "refined".into(),
        cost_usd: 0.01,
    });
    assert!(flush_text(&mut app).contains("refined"));
    assert!(app.subagents.iter().all(|r| r.done));
}
