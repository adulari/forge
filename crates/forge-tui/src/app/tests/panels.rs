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
    assert!(live.contains("[opus]"), "child model shown: {live}");

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
        done_screen.contains("activity ("),
        "activity panel visible: {done_screen}"
    );
    assert!(
        done_screen.contains("reviewer"),
        "done panel shows agent names: {done_screen}"
    );
    let sb = flush_text(&mut app);
    assert!(sb.contains("subagents"), "group header: {sb}");
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
fn welcome_banner_builds_ascii_wordmark() {
    let text: String = banner_lines(80)
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect();
    assert!(text.contains('█'), "ASCII wordmark in banner");
    assert!(text.contains("Your coding partner"), "tagline in banner");
}

#[test]
fn narrow_terminal_banner_falls_back_to_compact_wordmark() {
    let text: String = banner_lines(30)
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        text.contains("FORGE"),
        "compact wordmark on narrow terminal"
    );
    assert!(!text.contains('█'), "no block art when too narrow");
}

#[test]
fn tasks_panel_visible_while_active_and_absent_when_empty() {
    use forge_types::TodoStatus;
    let mut app = App::default();
    // No tasks → no sticky panel in the idle preview region.
    assert!(!screen(&app).contains("tasks ("), "panel hidden when empty");

    app.apply(PresenterEvent::Tasks(vec![
        todo("scan repo", TodoStatus::Done),
        todo("write tests", TodoStatus::InProgress),
        todo("ship it", TodoStatus::Pending),
    ]));
    assert!(
        screen(&app).contains("Work summary"),
        "basic chat summarizes work before it is expanded"
    );
    app.activity_focused = true;
    let s = screen(&app);
    assert!(
        s.contains("3 tasks (1 done, 1 in progress, 1 open)"),
        "panel header + breakdown: {s}"
    );
    // The in-progress item is shown with its glyph (prioritized into the small region).
    assert!(s.contains('◼'), "in-progress glyph shown: {s}");

    // Emptying the list collapses the panel.
    app.apply(PresenterEvent::Tasks(vec![]));
    assert!(
        !screen(&app).contains("tasks ("),
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
    assert!(text.contains("⬡ PLAN"), "has the plan tag: {text}");
    assert!(text.contains("Refactor main.rs"), "has the title");
    assert!(text.contains("Extract clap defs") && text.contains("Split dispatch"));
    assert!(text.contains("into cli/args.rs"), "shows step detail");
    assert!(
        text.contains("keep the CLI surface identical"),
        "shows notes"
    );
    assert!(text.contains('╭') && text.contains('╰'), "has a frame");
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
    let lines: Vec<String> = plan_card_lines(&plan)
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    // Every framed row (starts with the left border) must end at the right border and all
    // share the same visible width — i.e. nothing overflowed past the box.
    let framed: Vec<&String> = lines.iter().filter(|l| l.starts_with("  │")).collect();
    assert!(framed.len() > 3, "long text wrapped to multiple rows");
    let w = framed[0].chars().count();
    for l in &framed {
        assert!(l.ends_with('│'), "row stays inside the right border: {l:?}");
        assert_eq!(l.chars().count(), w, "all framed rows equal width: {l:?}");
    }
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
        s.contains("tasks ("),
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
        s.contains("activity ("),
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
    assert!(s.contains("activity ("), "panel header while running: {s}");
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
        s.contains("activity ("),
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
        phase1_flush.contains("1 agents"),
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
    assert!(s.contains("▶ research"), "research phase header shown: {s}");
    assert!(
        s.contains("▶ implement"),
        "implement phase header shown: {s}"
    );
    // Exactly one header per phase — the second `research` row must NOT repeat the header.
    assert_eq!(
        s.matches("▶ research").count(),
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
    assert!(out.contains("workflow 'audit'"), "header: {out:?}");
    assert!(out.contains("research"), "phase tree: {out:?}");
    assert!(out.contains("1/2 agents"), "totals: {out:?}");
    assert!(out.contains("1 running"), "running count: {out:?}");
    assert!(out.contains("halfway there"), "narration feed: {out:?}");
    assert!(out.contains("reading files"), "live activity edge: {out:?}");
    // The scrollback record still tells the story after the view closes.
    let flushed = flush_text(&mut app);
    assert!(flushed.contains("workflow 'audit' started"));
    assert!(flushed.contains("phase: research"));
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
