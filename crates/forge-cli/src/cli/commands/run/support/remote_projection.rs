use super::*;

/// Who/where a snapshot frame describes — the per-session identity fields of
/// [`remote::Snapshot`] that the driving loop (TUI render loop or a headless `forge serve`
/// session driver) knows and `App` doesn't.
pub(crate) struct SnapshotIdentity<'a> {
    pub session_id: &'a str,
    /// Session display title (v6). Empty when unnamed.
    pub title: &'a str,
    pub cwd: &'a str,
    /// The isolated worktree the session runs in (v6), if any.
    pub worktree: Option<&'a str>,
    /// Canonical project initialization state for this session's working directory.
    pub project_initialized: bool,
    /// Guidance shown when the project has not been initialized for Forge.
    pub project_init_hint: Option<String>,
    /// "loopback" | "LAN" | "public (provider)" — see [`remote::exposure_label`].
    pub exposure: String,
}

/// Build one wire [`remote::Snapshot`] frame from the App's remote projection. The ONE snapshot
/// producer shared by `run_chat_tui`'s broadcast block and the headless `forge serve` session
/// driver, so both paths serialize the identical shape. `revision` should carry the LAST
/// broadcast revision — the caller compares for change and bumps it just before an actual send.
pub(crate) fn build_snapshot_frame(
    app: &forge_tui::App,
    ident: SnapshotIdentity<'_>,
    copy_text: Option<String>,
    prompt_seq: u64,
    notes: Vec<String>,
    revision: u64,
) -> remote::Snapshot {
    let mut view = app.remote_snapshot();
    // Build the plan card first, while `view` is still whole: each step's status is read off the
    // live task list (see `plan_step_status`), which the literal below also consumes.
    let plan = view.plan.take().map(|p| remote::SnapPlan {
        title: p.title,
        steps: p
            .steps
            .into_iter()
            .map(|s| remote::SnapPlanStep {
                status: plan_step_status(&view.tasks, &s.title),
                title: s.title,
                detail: s.detail,
            })
            .collect(),
        notes: p.notes,
    });
    remote::Snapshot {
        protocol: remote::PROTOCOL_VERSION,
        session_id: ident.session_id.to_string(),
        title: ident.title.to_string(),
        cwd: ident.cwd.to_string(),
        worktree: ident.worktree.map(str::to_string),
        project_initialized: ident.project_initialized,
        project_init_hint: ident.project_init_hint,
        exposure: ident.exposure,
        busy: view.busy,
        done: view.done,
        temper: view.temper,
        effort: view
            .effort
            .map(forge_types::EffortLevel::as_str)
            .unwrap_or("medium")
            .to_string(),
        tier: view.tier,
        model: view.model,
        cost_usd: view.cost_usd,
        context_tokens: view.context_tokens,
        context_limit: view.context_limit,
        streaming: view.streaming,
        transcript: view.transcript,
        transcript_rows: view
            .transcript_rows
            .into_iter()
            .map(|r| remote::SnapTranscriptRow {
                kind: r.kind.as_str().to_string(),
                text: r.text,
                tool: r.tool,
                meta: r.meta,
            })
            .collect(),
        tasks: view
            .tasks
            .iter()
            .map(|t| remote::SnapTask {
                title: t.title.clone(),
                status: match t.status {
                    forge_types::TodoStatus::Pending => "pending",
                    forge_types::TodoStatus::InProgress => "in_progress",
                    forge_types::TodoStatus::Done => "done",
                }
                .to_string(),
                // Whoever the model named in `update_tasks` — `None` for work it kept itself.
                assignee: t.assignee.clone(),
            })
            .collect(),
        subagents: view
            .subagents
            .iter()
            .map(|s| remote::SnapSubagent {
                id: s.id.clone(),
                agent: s.agent.clone(),
                task: s.task.clone(),
                model: s.model.clone(),
                phase: s.phase.clone(),
                last: s.last.clone(),
                done: s.done,
                ok: s.ok,
                cost: s.cost,
                // Reserved: children run headless (an `Ask` resolves as Deny inside a subagent),
                // so no child is ever parked on its own prompt — every prompt is session-level
                // and rides in `permission_prompt` below.
                permission_prompt: None,
            })
            .collect(),
        queued: view.queued,
        permission_prompt: view.permission_prompt,
        question: view.question,
        question_options: view
            .question_options
            .iter()
            .map(|o| remote::SnapOption {
                label: o.label.clone(),
                description: o.description.clone(),
            })
            .collect(),
        question_allow_other: view.question_allow_other,
        // The generic overlay projection: whatever modal surface owns the keyboard
        // (palette / any picker / config / usage / mesh / workflow).
        overlay: app.remote_overlay().map(map_overlay_snapshot),
        diff: view.diff.map(map_diff_snapshot),
        plan,
        workflow: view.workflow.map(|w| remote::SnapWorkflow {
            active: w.active,
            name: w.name,
            phases: w.phases,
            logs: w.logs,
            finished_ok: w.finished_ok,
            summary: w.summary,
        }),
        suggested_prompt: view.suggested_prompt,
        copy_text,
        prompt_seq,
        notes,
        revision,
        resync: false,
        closed: false,
    }
}

/// Map one persisted transcript row into the wire type. Shared by the TUI's `/remote` provider
/// and the daemon's `/api/history` route so both serve the identical shape, including the v9
/// replay fields: `kind` in the same vocabulary as the live transcript, and `elapsed_ms`, the
/// offset from `epoch` (the session's first visible row).
///
/// `kind == "tool"` only ever appears on a page the caller asked for with `include_tools` —
/// otherwise `Store::load_history_page` selects only user/assistant turns plus `ui` notes and no
/// tool row reaches here at all. `epoch` MUST come from `Store::history_epoch_with` for the same
/// flag, or the offsets would be measured against rows the page doesn't contain.
pub(crate) fn map_history_row(
    row: forge_store::HistoryRow,
    epoch: Option<i64>,
) -> remote::HistoryRow {
    let visibility = row.visibility.as_str().to_string();
    let role = row.role.as_str().to_string();
    // `ui` rows are Forge talking to the user (notices, command feedback), not a turn of the
    // conversation — they carry role='assistant' in the store but read as system lines.
    let kind = if visibility == "ui" {
        "system"
    } else {
        match role.as_str() {
            "user" => "user",
            "assistant" => "assistant",
            "tool" => "tool",
            _ => "system",
        }
    };
    remote::HistoryRow {
        seq: row.seq,
        role,
        content: row.content,
        model: row.model,
        created_at: row.created_at,
        visibility,
        kind: kind.to_string(),
        // Milliseconds on the wire (what a scrubber wants) but SECOND resolution in fact:
        // `message.created_at` is stored as unix seconds, so this is exact to the second, not
        // finer. Clamped at 0 because a row can share the epoch's second.
        elapsed_ms: epoch.map(|e| (row.created_at - e).max(0) * 1_000),
        // Only the store can say which tool a result row came from; it stays `None` rather than
        // being inferred from the result prose when the carrier is unrecoverable.
        tool: row.tool_name,
        // Call-vs-result only means anything on a tool row: a `ui` note written with role='tool'
        // reads as a system line here, and tagging it would say something false about it.
        tool_phase: (kind == "tool")
            .then(|| row.tool_phase.map(|phase| phase.as_str().to_string()))
            .flatten(),
    }
}

/// A plan step's execution state (v9), correlated with the live task list rather than invented:
/// a `PlanStep` has no state of its own, but approving a plan seeds one task per step with that
/// step's trimmed title (`Session::activate_plan_tasks`), and the model then drives those through
/// `update_tasks`. So a step is exactly as far along as the task carrying its title — and
/// "queued" whenever no such task exists (an unapproved proposal, or a list the model has since
/// rewritten past recognition), never a guess at progress.
pub(crate) fn plan_step_status(tasks: &[forge_types::TodoItem], step_title: &str) -> String {
    let step_title = step_title.trim();
    tasks
        .iter()
        .find(|t| t.title.trim() == step_title)
        .map(|t| match t.status {
            forge_types::TodoStatus::InProgress => "in_progress",
            forge_types::TodoStatus::Done => "done",
            forge_types::TodoStatus::Pending => "queued",
        })
        .unwrap_or("queued")
        .to_string()
}

/// Map the TUI-side diff projection into the remote wire type (same split as the overlay).
pub(crate) fn map_diff_snapshot(d: forge_tui::DiffSnapshot) -> remote::SnapDiff {
    remote::SnapDiff {
        pending: d.pending,
        files: d
            .files
            .into_iter()
            .map(|f| remote::SnapDiffFile {
                path: f.path,
                kind: f.kind,
                binary: f.binary,
                adds: f.adds,
                dels: f.dels,
                hunks: f
                    .hunks
                    .into_iter()
                    .map(|h| remote::SnapDiffHunk {
                        header: h.header,
                        lines: h.lines,
                    })
                    .collect(),
                skipped_lines: f.skipped_lines,
            })
            .collect(),
        skipped_files: d.skipped_files,
    }
}

/// Map the TUI-side overlay projection into the remote wire type (kept apart so `forge-tui`
/// never depends on the server module — same split as `RemoteSnapshot` → `Snapshot`).
pub(crate) fn map_overlay_snapshot(o: forge_tui::OverlaySnapshot) -> remote::SnapOverlay {
    remote::SnapOverlay {
        kind: o.kind,
        title: o.title,
        rows: o
            .rows
            .into_iter()
            .map(|r| remote::SnapRow {
                id: r.id,
                label: r.label,
                detail: r.detail,
                selected: r.selected,
                group: r.group,
            })
            .collect(),
        selected: o.selected,
        filter: o.filter,
        free_text: o.free_text,
        body: o.body,
    }
}
