use super::*;

#[test]
fn remote_snapshot_suppresses_permission_prompt_while_question_active() {
    // A plain permission prompt projects as `permission_prompt`.
    let mut app = App {
        prompt: Some("allow write_file (FileWrite) [y/n/a=always]".into()),
        ..Default::default()
    };
    let snap = app.remote_snapshot();
    assert_eq!(
        snap.permission_prompt.as_deref(),
        Some("allow write_file (FileWrite) [y/n/a=always]")
    );

    // A question arms `prompt` with its input hint — the snapshot must NOT project that as a
    // permission prompt (the page would render dead Allow/Deny buttons over the options).
    let options = vec![QChoice {
        label: "Postgres".into(),
        description: "relational".into(),
    }];
    app.set_question("which database?", &options, true);
    let snap = app.remote_snapshot();
    assert!(
        snap.permission_prompt.is_none(),
        "question suppresses the permission_prompt projection: {:?}",
        snap.permission_prompt
    );
    assert_eq!(snap.question.as_deref(), Some("which database?"));
    assert_eq!(snap.question_options.len(), 1);
    assert!(snap.question_allow_other);

    // Resolving the question clears both; nothing lingers on the wire.
    assert!(app.resolve_question("1").is_some());
    let snap = app.remote_snapshot();
    assert!(snap.permission_prompt.is_none());
    assert!(snap.question.is_none());
}

#[test]
fn remote_diff_card_tracks_preview_permission_and_landing() {
    let mut app = App::default();
    assert!(app.remote_snapshot().diff.is_none(), "no changes, no card");

    // The write preview arrives BEFORE the permission gate — with the prompt armed, the
    // card is the ONE pending change ("what will this touch" before Allow).
    app.apply(crate::PresenterEvent::Diff(sample_diff(
        "src/a.rs",
        "old line\n",
        "new line\n",
    )));
    app.prompt = Some("allow write_file (Write) [y/n/a=always]".into());
    let diff = app.remote_snapshot().diff.expect("pending card");
    assert!(diff.pending);
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].path, "src/a.rs");
    assert_eq!(diff.files[0].kind, "modified");
    assert_eq!((diff.files[0].adds, diff.files[0].dels), (1, 1));
    let hunk = &diff.files[0].hunks[0];
    assert!(
        hunk.header.starts_with("@@ -"),
        "line spans: {}",
        hunk.header
    );
    assert!(
        hunk.lines.iter().any(|l| l == "+new line"),
        "{:?}",
        hunk.lines
    );
    assert!(
        hunk.lines.iter().any(|l| l == "-old line"),
        "{:?}",
        hunk.lines
    );

    // Allowed + ran ok → the change LANDED: the turn card carries it from now on.
    app.prompt = None;
    write_result(&mut app, true);
    let diff = app.remote_snapshot().diff.expect("landed card");
    assert!(!diff.pending);
    assert_eq!(diff.files.len(), 1);

    // A denied/failed write never touched the file: its preview is dropped.
    app.apply(crate::PresenterEvent::Diff(sample_diff(
        "src/b.rs", "x\n", "y\n",
    )));
    write_result(&mut app, false);
    let diff = app.remote_snapshot().diff.expect("landed card unchanged");
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].path, "src/a.rs");

    // A new user prompt starts a new turn: the card resets.
    app.submit_user("next thing");
    assert!(app.remote_snapshot().diff.is_none());
}

#[test]
fn remote_diff_card_caps_hunk_lines_and_files() {
    let mut app = App::default();
    // 200 changed lines → carried hunk lines cap at the per-file budget, the remainder is
    // counted (the page shows "+N more"), and the add total stays exact.
    let big_new: String = (0..200).map(|i| format!("line {i}\n")).collect();
    app.apply(crate::PresenterEvent::Diff(sample_diff(
        "big.rs", "", &big_new,
    )));
    write_result(&mut app, true);
    let d = app.remote_snapshot().diff.unwrap();
    assert_eq!(d.files[0].adds, 200, "adds count the WHOLE change");
    let carried: usize = d.files[0].hunks.iter().map(|h| h.lines.len()).sum();
    assert_eq!(carried, crate::render::REMOTE_DIFF_MAX_HUNK_LINES);
    assert_eq!(d.files[0].skipped_lines, 200 - carried);

    // 12 more files land → the list holds the newest REMOTE_DIFF_MAX_FILES, evictions
    // (big.rs + the two oldest) fold into skipped_files.
    for i in 0..12 {
        app.apply(crate::PresenterEvent::Diff(sample_diff(
            &format!("f{i}.rs"),
            "a\n",
            "b\n",
        )));
        write_result(&mut app, true);
    }
    let d = app.remote_snapshot().diff.unwrap();
    assert_eq!(d.files.len(), REMOTE_DIFF_MAX_FILES);
    assert_eq!(d.skipped_files, 3);
    assert_eq!(d.files.last().unwrap().path, "f11.rs");

    // Re-editing a carried path replaces it in place (latest wins) — no new slot burned.
    app.apply(crate::PresenterEvent::Diff(sample_diff(
        "f11.rs", "b\n", "c\n",
    )));
    write_result(&mut app, true);
    let d = app.remote_snapshot().diff.unwrap();
    assert_eq!(d.files.len(), REMOTE_DIFF_MAX_FILES);
    assert_eq!(d.skipped_files, 3, "a same-path re-edit evicts nothing");
}

#[test]
fn remote_diff_card_handles_binary_files() {
    let mut app = App::default();
    app.apply(crate::PresenterEvent::Diff(forge_types::FileDiff {
        path: "logo.png".into(),
        kind: forge_types::DiffKind::Created,
        old: None,
        new: Some("\u{fffd}bytes".into()),
        lang: None,
        binary: true,
    }));
    write_result(&mut app, true);
    let d = app.remote_snapshot().diff.unwrap();
    assert!(d.files[0].binary, "binary flag rides the wire");
    assert!(d.files[0].hunks.is_empty(), "no textual hunks for binary");
    assert_eq!(d.files[0].kind, "created");
}

#[test]
fn remote_plan_card_projects_and_clears_on_next_prompt() {
    let mut app = App::default();
    assert!(app.remote_snapshot().plan.is_none());
    app.apply(crate::PresenterEvent::PlanProposed(
        forge_types::PlanProposal {
            title: "Ship it".into(),
            steps: vec![forge_types::PlanStep {
                title: "step 1".into(),
                detail: "do the thing".into(),
            }],
            notes: Some("risky".into()),
        },
    ));
    let p = app
        .remote_snapshot()
        .plan
        .expect("plan retained as live state");
    assert_eq!(p.title, "Ship it");
    assert_eq!(p.steps.len(), 1);
    assert_eq!(p.notes.as_deref(), Some("risky"));

    // The approval question arrives at turn end; the page's Approve button answers its
    // "Build it" option by number — the identical path a local choice takes.
    let options = vec![
        QChoice {
            label: "Build it".into(),
            description: "Switch to Auto-edit and implement the plan now".into(),
        },
        QChoice {
            label: "Cancel".into(),
            description: "Discard the plan; stay in planning mode".into(),
        },
    ];
    app.set_question("Build this plan?", &options, true);
    assert!(
        app.remote_snapshot().plan.is_some(),
        "card survives the question"
    );
    assert_eq!(app.resolve_question("1").as_deref(), Some("Build it"));
    // The card stays visible through the build (no user submit happens on approval)…
    assert!(app.remote_snapshot().plan.is_some());
    // …and clears when the NEXT user prompt starts a new turn.
    app.submit_user("another task");
    assert!(app.remote_snapshot().plan.is_none());
}

#[test]
fn cancelling_the_plan_question_clears_the_remote_plan_card() {
    let mut app = App::default();
    app.apply(crate::PresenterEvent::PlanProposed(
        forge_types::PlanProposal {
            title: "t".into(),
            steps: vec![forge_types::PlanStep {
                title: "one".into(),
                detail: String::new(),
            }],
            notes: None,
        },
    ));
    let options = vec![
        QChoice {
            label: "Build it".into(),
            description: String::new(),
        },
        QChoice {
            label: "Cancel".into(),
            description: String::new(),
        },
    ];
    app.set_question("Build this plan?", &options, true);
    assert_eq!(app.resolve_question("2").as_deref(), Some("Cancel"));
    assert!(app.remote_snapshot().plan.is_none());
}

#[test]
fn remote_overlay_is_none_when_nothing_modal_is_open() {
    assert_eq!(App::default().remote_overlay(), None);
}

#[test]
fn remote_overlay_projects_every_picker_kind() {
    use crate::commands::PickerKind as P;
    // One projection arm covers EVERY picker kind — each maps to a stable wire tag, so any
    // future variant fails to compile in `picker_kind_wire` until it gets a tag too.
    for (kind, wire) in [
        (P::Sessions, "picker:sessions"),
        (P::Checkpoints, "picker:checkpoints"),
        (P::Tempers, "picker:tempers"),
        (P::AssayChoice, "picker:assay"),
        (P::Models, "picker:models"),
        (P::ResumeMode, "picker:resume_mode"),
        (P::CopyBlocks, "picker:copy_blocks"),
        (P::ModelPin, "picker:model_pin"),
        (P::Duel, "picker:duel"),
    ] {
        let mut app = App::default();
        app.picker
            .open_with(kind, "pick one", overlay_picker_rows());
        app.picker.move_down();
        let o = app.remote_overlay().expect("open picker projects");
        assert_eq!(o.kind, wire, "{kind:?} wire tag");
        assert_eq!(o.title, "pick one");
        assert_eq!(o.rows.len(), 2);
        assert_eq!(o.rows[0].id, "row-a");
        assert_eq!(o.rows[0].label, "Alpha");
        assert_eq!(o.rows[0].detail, "first");
        assert!(!o.rows[0].selected);
        assert!(o.rows[1].selected, "cursor row is flagged");
        assert_eq!(o.selected, 1);
        assert_eq!(o.filter.as_deref(), Some(""), "pickers are filterable");
        assert!(!o.free_text);
        assert!(o.body.is_none());
    }
}

#[test]
fn remote_overlay_picker_filter_narrows_rows() {
    let mut app = App::default();
    app.picker.open_with(
        crate::commands::PickerKind::Sessions,
        "resume a session",
        overlay_picker_rows(),
    );
    app.picker.query = "beta".into();
    app.picker.clamp();
    let o = app.remote_overlay().unwrap();
    assert_eq!(o.filter.as_deref(), Some("beta"));
    assert_eq!(o.rows.len(), 1, "rows are the FILTERED matches");
    assert_eq!(o.rows[0].id, "row-b");
}

#[test]
fn remote_overlay_carries_a_window_of_a_huge_picker_not_every_row() {
    let rows: Vec<crate::commands::PickerRow> = (0..5_000)
        .map(|i| crate::commands::PickerRow {
            id: i.to_string(),
            title: format!("prompt {i}"),
            subtitle: format!("#{i}"),
        })
        .collect();
    let mut app = App::default();
    app.picker
        .open_with(crate::commands::PickerKind::Checkpoints, "rewind", rows);
    let o = app.remote_overlay().unwrap();
    assert_eq!(
        o.rows.len(),
        200,
        "a snapshot never carries all 5,000 checkpoints"
    );
    assert_eq!(o.rows[0].id, "0");
    assert!(
        o.body.as_deref().unwrap().contains("of 5000"),
        "{:?}",
        o.body
    );

    app.picker.end();
    let o = app.remote_overlay().unwrap();
    assert_eq!(o.rows.len(), 200);
    assert_eq!(o.rows.last().unwrap().id, "4999");
    assert!(o.rows[o.selected].selected, "`selected` indexes the window");
    assert_eq!(o.rows[o.selected].id, "4999");
}

#[test]
fn remote_overlay_projects_the_palette() {
    let mut app = App::default();
    app.palette.open_with("mod");
    let o = app.remote_overlay().expect("open palette projects");
    assert_eq!(o.kind, "palette");
    assert_eq!(o.filter.as_deref(), Some("mod"));
    assert!(
        o.rows.iter().any(|r| r.id == "model"),
        "filtered commands present: {:?}",
        o.rows.iter().map(|r| &r.id).collect::<Vec<_>>()
    );
    let model = o.rows.iter().find(|r| r.id == "model").unwrap();
    assert_eq!(model.label, "/model");
    assert!(!model.detail.is_empty(), "desc/usage hint rides along");
}

#[test]
fn remote_overlay_projects_the_at_path_picker() {
    let mut app = App::default();
    app.at_picker
        .open_with("ma", vec!["src/main.rs".into(), "Cargo.toml".into()]);
    let o = app.remote_overlay().expect("open @path picker projects");
    assert_eq!(o.kind, "picker:at_path");
    assert_eq!(o.filter.as_deref(), Some("ma"));
    assert_eq!(o.rows.len(), 1, "matches() filtering applies");
    assert_eq!(o.rows[0].id, "src/main.rs");
    assert_eq!(o.rows[0].label, "@src/main.rs");
}

#[test]
fn remote_overlay_projects_the_config_editor() {
    let mut app = App::default();
    app.config_editor.open_with(vec![crate::SettingRow {
        path: "tui.fullscreen".into(),
        group: "tui".into(),
        label: "Full-screen mode".into(),
        value: "true".into(),
        source: "user".into(),
        ..Default::default()
    }]);
    let o = app.remote_overlay().expect("open config editor projects");
    assert_eq!(o.kind, "config");
    assert_eq!(o.title, "settings — user scope");
    assert_eq!(o.rows[0].id, "tui.fullscreen");
    assert_eq!(o.rows[0].group.as_deref(), Some("tui"));
    assert!(o.rows[0].detail.contains("true"));
    assert!(!o.free_text, "not editing yet");

    // While a value is being typed, free_text turns on and the buffer shows in the body.
    app.config_editor.editing = Some("fal".into());
    let o = app.remote_overlay().unwrap();
    assert!(o.free_text, "editing → free-text box");
    assert!(
        o.body.as_deref().unwrap_or("").contains("fal"),
        "buffer echoed: {:?}",
        o.body
    );
}

#[test]
fn remote_overlay_projects_usage_as_a_text_body() {
    let mut app = App::default();
    app.usage_overlay.open = true;
    app.usage_overlay.session_usd = 1.25;
    app.usage_overlay.by_model = vec![("groq::llama-3.3-70b".into(), 0.5, 1000, 2000)];
    app.usage_overlay.claude_5h_pct = Some(42.0);
    let o = app.remote_overlay().expect("open usage overlay projects");
    assert_eq!(o.kind, "overlay:usage");
    assert!(o.rows.is_empty(), "informational — no selectable rows");
    let body = o.body.expect("pre-rendered text body");
    assert!(body.contains("$1.2500"), "session spend: {body}");
    assert!(body.contains("groq::llama-3.3-70b"));
    assert!(body.contains("claude 5h: 42% used"));
}

#[test]
fn remote_overlay_projects_mesh_candidates_and_verdict() {
    let mut app = App::default();
    app.mesh_overlay.open = true;
    app.mesh_overlay.classified = "complex".into();
    app.mesh_overlay.routed = "complex".into();
    app.mesh_overlay.pick = "claude-cli::opus".into();
    app.mesh_overlay.conserve_line = "off".into();
    app.mesh_overlay.candidates = vec![
        MeshCandRow {
            effort: None,
            rank: 1,
            model: "claude-cli::opus".into(),
            score: 0.9,
            cost_tag: "subscription".into(),
            frontier: true,
            usable: true,
            selected: true,
            penalty: 0.0,
            reorder_reason: None,
        },
        MeshCandRow {
            effort: None,
            rank: 2,
            model: "groq::llama".into(),
            score: 0.7,
            cost_tag: "free".into(),
            usable: true,
            ..Default::default()
        },
    ];
    app.mesh_overlay.cursor = 1;
    let o = app.remote_overlay().expect("open mesh overlay projects");
    assert_eq!(o.kind, "overlay:mesh");
    assert_eq!(o.rows.len(), 2);
    assert_eq!(o.rows[0].id, "claude-cli::opus");
    assert!(o.rows[0].detail.contains("routed pick"));
    assert!(o.rows[1].selected, "browsing cursor row flagged");
    assert_eq!(o.selected, 1);
    let body = o.body.expect("routing verdict body");
    assert!(body.contains("classified: complex"));
    assert!(body.contains("pick: claude-cli::opus"));
}

#[test]
fn remote_overlay_projects_the_workflow_view_with_phase_groups() {
    let mut app = App::default();
    app.workflow.begin(Some("release".into()));
    app.workflow.phases = vec![crate::WfPhase {
        title: "build".into(),
    }];
    app.workflow.rows = vec![crate::WfRow {
        id: "wf-1".into(),
        agent: "builder".into(),
        task: "compile it".into(),
        model: None,
        phase_idx: Some(0),
        last: "running tests".into(),
        log: Vec::new(),
        done: false,
        ok: true,
        cost: 0.0,
    }];
    app.workflow.logs = vec!["phase build started".into()];
    let o = app.remote_overlay().expect("open workflow view projects");
    assert_eq!(o.kind, "overlay:workflow");
    assert!(o.title.contains("release"), "title: {}", o.title);
    assert_eq!(o.rows[0].id, "wf-1");
    assert_eq!(o.rows[0].group.as_deref(), Some("build"));
    assert!(o.rows[0].detail.contains("compile it"));
    assert!(o.body.unwrap().contains("phase build started"));
}

#[test]
fn remote_overlay_precedence_mirrors_key_routing() {
    // Both a picker and the palette open → the palette (which owns the keys) projects.
    let mut app = App::default();
    app.picker.open_with(
        crate::commands::PickerKind::Sessions,
        "resume",
        overlay_picker_rows(),
    );
    app.palette.open_with("");
    assert_eq!(app.remote_overlay().unwrap().kind, "palette");
    // The workflow view outranks everything (it's checked first by the key loop).
    app.workflow.open = true;
    assert_eq!(app.remote_overlay().unwrap().kind, "overlay:workflow");
}

#[test]
fn job_notice_rows_reach_the_remote_ring_as_job_notice_kind() {
    let mut app = App::default();
    app.submit_user("[background job 9 `make` exited 2]\nlast lines:\nerror: boom");
    app.drain_flush_remote();
    let rows: Vec<_> = app
        .recent_transcript
        .iter()
        .map(|r| (r.kind.as_str(), r.meta.as_deref()))
        .collect();
    assert_eq!(
        rows,
        vec![("job_notice", Some("failed")), ("job_notice", Some("tail"))]
    );
    app.submit_user("[background job 9 `make` exited 2] but really, retry it");
    assert!(drain_text_lines(&mut app).contains("you"));
}

#[test]
fn heartbeat_rows_reach_the_remote_ring_with_a_subtype() {
    let mut app = App::default();
    app.submit_user(&forge_types::format_heartbeat_delivery(None, "check in"));
    app.drain_flush_remote();
    let rows: Vec<_> = app
        .recent_transcript
        .iter()
        .map(|r| (r.kind.as_str(), r.meta.as_deref()))
        .collect();
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|r| *r == ("harness_notice", Some("heartbeat"))),
        "{rows:?}"
    );
}

#[test]
fn peer_message_rows_carry_the_harness_notice_kind() {
    let mut app = App::default();
    app.submit_user("[message from cli] hello there");
    app.drain_flush_remote();
    assert!(app
        .recent_transcript
        .iter()
        .all(|r| (r.kind.as_str(), r.meta.as_deref()) == ("harness_notice", Some("peer"))));
}

#[test]
fn the_remote_transcript_sees_each_call_once_with_its_outcome() {
    // The phone reads the flushed-line ring, not `main_log`, so a card redrawn in place has
    // to be handed over explicitly — but exactly once: the same-frame case rewrites the
    // queued row, and pushing there too showed the call twice on the phone.
    let mut app = card_app();
    // Same-frame: start and result before any drain.
    app.apply(PresenterEvent::ToolStart {
        name: "shell".into(),
        args: r#"{"command":"fast"}"#.into(),
    });
    app.apply(PresenterEvent::ToolResult {
        name: "shell".into(),
        ok: true,
        summary: "exit 0".into(),
        detail: None,
    });
    let _ = app.drain_flush_remote();
    let rows: Vec<String> = app
        .recent_transcript
        .iter()
        .map(|r| r.text.clone())
        .filter(|t| t.contains("fast"))
        .collect();
    assert_eq!(rows.len(), 1, "one row for one call: {rows:?}");
    assert!(rows[0].contains("exit 0"), "carrying the outcome: {rows:?}");

    // Across frames: the start row is flushed first, the finished row follows it.
    app.apply(PresenterEvent::ToolStart {
        name: "shell".into(),
        args: r#"{"command":"slow"}"#.into(),
    });
    let _ = app.drain_flush_remote();
    app.apply(PresenterEvent::ToolResult {
        name: "shell".into(),
        ok: true,
        summary: "exit 0".into(),
        detail: None,
    });
    let _ = app.drain_flush_remote();
    let rows: Vec<String> = app
        .recent_transcript
        .iter()
        .map(|r| r.text.clone())
        .filter(|t| t.contains("slow"))
        .collect();
    assert_eq!(
        rows.len(),
        2,
        "the running row, then the finished one: {rows:?}"
    );
    assert!(rows[1].contains("exit 0"), "{rows:?}");
}

#[test]
fn the_remote_tail_counts_every_row_it_ever_held() {
    let mut app = App::default();
    for n in 0..30 {
        app.submit_user(&format!("prompt {n}"));
        let _ = app.drain_flush_remote();
    }
    let view = app.remote_snapshot();
    assert_eq!(view.transcript.len(), REMOTE_TRANSCRIPT_MAX);
    assert!(
        view.transcript_total > view.transcript.len() as u64,
        "the total keeps counting rows the ring dropped"
    );
    let first_abs = view.transcript_total - view.transcript.len() as u64;
    assert!(first_abs >= 30, "the first retained row's absolute index");
}

#[test]
fn remote_transcript_rows_carry_the_provenance_of_each_line() {
    let mut app = App::default();
    app.submit_user("read Cargo.toml");
    app.apply(PresenterEvent::ToolStart {
        name: "read_file".into(),
        args: "{\"path\":\"Cargo.toml\"}".into(),
    });
    app.apply(PresenterEvent::ToolResult {
        name: "read_file".into(),
        ok: false,
        summary: "no such file".into(),
        detail: None,
    });
    // An untagged emitter between tool activity and the reply must NOT inherit either.
    app.apply(PresenterEvent::Warning("retrying".into()));
    app.apply(PresenterEvent::AssistantText("it isn't there".into()));
    let _ = app.drain_flush_remote();

    let view = app.remote_snapshot();
    assert_eq!(
        view.transcript.len(),
        view.transcript_rows.len(),
        "the tagged twin must stay index-aligned with the flat transcript"
    );
    for (text, row) in view.transcript.iter().zip(view.transcript_rows.iter()) {
        assert_eq!(text, &row.text);
    }
    let kind_of = |needle: &str| {
        view.transcript_rows
            .iter()
            .find(|r| r.text.contains(needle))
            .unwrap_or_else(|| panic!("no transcript row for {needle:?}"))
    };
    assert_eq!(kind_of("read Cargo.toml").kind, TranscriptKind::User);
    assert_eq!(kind_of("it isn't there").kind, TranscriptKind::Assistant);
    assert_eq!(kind_of("retrying").kind, TranscriptKind::System);

    // Inline prints one finished card per call, so the single Tool row carries the outcome.
    let result = kind_of("Read Cargo.toml");
    assert_eq!(result.kind, TranscriptKind::Tool);
    assert_eq!(result.tool.as_deref(), Some("read_file"));
    assert_eq!(result.meta.as_deref(), Some("failed"));
    assert!(result.text.contains("no such file"), "{}", result.text);
}

#[test]
fn remote_active_shows_indicator_in_the_header_and_inline_status_line() {
    // Remote control off → no indicator anywhere.
    let app = App::default();
    assert!(
        !screen_wh(&app, 90, LIVE_H).contains("● remote"),
        "no indicator when remote control is off"
    );
    // Inline has no header, so the green `● remote` segment sits in the status line.
    let app = App {
        remote_active: true,
        ..Default::default()
    };
    assert!(
        screen_wh(&app, 90, LIVE_H).contains("● remote"),
        "indicator shown when remote control is on"
    );
    // Fullscreen carries it in the header at every width.
    let app = App {
        remote_active: true,
        fullscreen: true,
        ..Default::default()
    };
    for width in [48, 90, 160] {
        let header = screen_rows(&app, width, 20).remove(0);
        assert!(header.contains("● remote"), "header at {width}: {header}");
    }
}

#[test]
fn restored_turn_state_seeds_outcome_and_model_but_not_the_turn_count() {
    let mut app = App::default();
    app.seed_restored_turn_state(
        Some(forge_types::StopReason::FinalAnswer),
        Some("ollama::qwen3".into()),
    );
    let snap = app.remote_snapshot();
    assert_eq!(
        snap.last_stop_reason,
        Some(forge_types::StopReason::FinalAnswer)
    );
    assert_eq!(snap.model, "ollama::qwen3");
    assert_eq!(app.turns_finished, 0, "the count is per-process");

    // A value a live turn already set is never overwritten by a late seed.
    let mut live = App {
        last_stop_reason: Some(forge_types::StopReason::MaxSteps),
        routing: Some(RoutingView {
            tier: "standard".into(),
            model: "groq::live".into(),
            rationale: String::new(),
        }),
        ..Default::default()
    };
    live.seed_restored_turn_state(
        Some(forge_types::StopReason::FinalAnswer),
        Some("ollama::old".into()),
    );
    assert_eq!(
        live.last_stop_reason,
        Some(forge_types::StopReason::MaxSteps)
    );
    assert_eq!(live.remote_snapshot().model, "groq::live");
}

#[test]
fn a_replys_transcript_rows_keep_the_shape_the_daemon_supervisor_reads() {
    // `serve_dispatch::supervisor::final_reply` extracts a worker's answer from these rows: a
    // `⚒ forge` assistant header (stable on the wire, whatever the terminal draws), the answer
    // rows, then system rows (footer, notices, recap) it skips.
    let mut app = App::default();
    app.submit_user("do it");
    app.apply(PresenterEvent::Routing {
        tier: "standard".into(),
        model: "claude-cli".into(),
        rationale: String::new(),
        effort: None,
    });
    app.apply(PresenterEvent::AssistantDelta(
        "Done — the file is written.".into(),
    ));
    app.apply(PresenterEvent::AssistantDone);
    app.apply(PresenterEvent::Done {
        stop_reason: forge_types::StopReason::FinalAnswer,
        final_text: String::new(),
    });
    let _ = app.drain_flush_remote();
    let rows: Vec<(String, String)> = app
        .remote_snapshot()
        .transcript_rows
        .iter()
        .map(|r| (r.kind.as_str().to_string(), r.text.clone()))
        .collect();
    let assistant: Vec<&str> = rows
        .iter()
        .filter(|(k, _)| k == "assistant")
        .map(|(_, t)| t.as_str())
        .collect();
    assert_eq!(
        assistant,
        vec!["  ⚒ forge", "  Done — the file is written."],
        "{rows:?}"
    );
    let (kind, footer) = rows.last().unwrap();
    assert_eq!(
        kind, "system",
        "the footer is a system row the supervisor skips: {rows:?}"
    );
    assert!(footer.starts_with("  ") && footer.contains("s"), "{footer}");
}
