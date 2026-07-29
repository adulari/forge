//! Remote-control projections for the TUI state.

use super::*;
use crate::app::{mesh_pace_suffix, SubagentSnapshot};

impl App {
    /// Build a [`remote::Snapshot`]-shaped view of the live state, for the remote-control WS to
    /// broadcast. Plain fields only (no ratatui types), so `forge-tui` needn't depend on the
    /// remote module — the caller maps this into the snapshot type.
    pub fn remote_snapshot(&self) -> RemoteSnapshot {
        let (question_options, question_allow_other) = match &self.question {
            Some((opts, allow_other)) => (opts.clone(), *allow_other),
            None => (Vec::new(), false),
        };
        RemoteSnapshot {
            busy: self.busy,
            done: self.done,
            temper: self.temper.clone(),
            effort: self.effort,
            tier: self.routing.as_ref().map(|r| r.tier.clone()),
            model: self
                .routing
                .as_ref()
                .map(|r| r.model.clone())
                .unwrap_or_else(|| "—".to_string()),
            cost_usd: self.cost_usd,
            context_tokens: self.context_tokens,
            context_limit: self.context_limit,
            streaming: self.streaming.clone(),
            transcript: self
                .recent_transcript
                .iter()
                .map(|r| r.text.clone())
                .collect(),
            transcript_rows: self.recent_transcript.iter().cloned().collect(),
            tasks: self.tasks.clone(),
            // `log` is left empty — the remote wire type (`remote::SnapSubagent`) never reads
            // it, so cloning the real (unbounded-growing) log buffer here on every dirty/busy
            // frame would be pure waste. `ViewSnapshot` (the OTHER consumer of this same
            // `SubagentSnapshot` type, for session-resume persistence) still gets the real
            // values via its own construction path below. `id` IS cloned (v8.1): clients key
            // drill-ins on it.
            subagents: self
                .subagents
                .iter()
                .map(|r| SubagentSnapshot {
                    id: r.id.clone(),
                    agent: r.agent.clone(),
                    task: r.task.clone(),
                    model: r.model.clone(),
                    phase: r.phase.clone(),
                    last: r.last.clone(),
                    log: Vec::new(),
                    done: r.done,
                    ok: r.ok,
                    cost: r.cost,
                })
                // Workflow rows live in the dedicated view, not `subagents` — the phone still
                // wants to see them, so they ride along in the same wire shape, phase attached.
                .chain(self.workflow.rows.iter().map(|r| SubagentSnapshot {
                    id: r.id.clone(),
                    agent: r.agent.clone(),
                    task: r.task.clone(),
                    model: r.model.clone(),
                    phase: r.phase_idx.map(|i| self.workflow.phases[i].title.clone()),
                    last: r.last.clone(),
                    log: Vec::new(),
                    done: r.done,
                    ok: r.ok,
                    cost: r.cost,
                }))
                .collect(),
            workflow: self.workflow.exists().then(|| WorkflowRemote {
                active: self.workflow.active,
                name: self.workflow.name.clone(),
                phases: self
                    .workflow
                    .phases
                    .iter()
                    .map(|p| p.title.clone())
                    .collect(),
                logs: self
                    .workflow
                    .logs
                    .iter()
                    .rev()
                    .take(WORKFLOW_REMOTE_LOG_TAIL)
                    .rev()
                    .cloned()
                    .collect(),
                finished_ok: self.workflow.finished.as_ref().map(|(ok, _)| *ok),
                summary: self.workflow.finished.as_ref().map(|(_, s)| s.clone()),
            }),
            queued: self.queued.clone(),
            // A pending AskUserQuestion also arms `self.prompt` (the input-line hint text, e.g.
            // "type a number, or your own answer"), but on the wire `permission_prompt` means
            // "y/n permission gate" and the control page gives it precedence over `question`.
            // Projecting both wedged the remote: the page rendered Allow/Deny buttons for a
            // question that has no pending permission reply, so taps were silent no-ops and the
            // real options were unreachable. While a question is active, the prompt is its input
            // hint — never a permission gate — so suppress it here.
            permission_prompt: if self.question.is_some() {
                None
            } else {
                self.prompt.clone()
            },
            question: self.question_prompt.clone(),
            question_options,
            question_allow_other,
            diff: self.remote_diff(),
            plan: self.plan.clone(),
            suggested_prompt: self.suggested_prompt.clone(),
        }
    }

    /// The remote diff card (v7). While a permission prompt is armed and a write preview is
    /// pending, the card is that ONE proposed change (`pending: true` — review before Allow);
    /// otherwise it's everything that landed this turn. `None` when there's nothing to show.
    fn remote_diff(&self) -> Option<DiffSnapshot> {
        if self.prompt.is_some() {
            if let Some(d) = &self.pending_diff {
                return Some(DiffSnapshot {
                    pending: true,
                    files: vec![d.clone()],
                    skipped_files: 0,
                });
            }
        }
        if self.turn_diffs.is_empty() {
            return None;
        }
        Some(DiffSnapshot {
            pending: false,
            files: self.turn_diffs.clone(),
            skipped_files: self.turn_diffs_skipped,
        })
    }

    /// Record a landed file change for the remote diff card: latest edit per path wins, and the
    /// file list stays bounded ([`REMOTE_DIFF_MAX_FILES`]) — evictions fold into the "+N more
    /// files" count instead of disappearing silently.
    pub(crate) fn push_turn_diff(&mut self, d: DiffFileSnapshot) {
        self.turn_diffs.retain(|e| e.path != d.path);
        while self.turn_diffs.len() >= REMOTE_DIFF_MAX_FILES {
            self.turn_diffs.remove(0);
            self.turn_diffs_skipped += 1;
        }
        self.turn_diffs.push(d);
    }
}

/// The stable wire tag for a [`PickerKind`](crate::commands::PickerKind), used as
/// [`OverlaySnapshot::kind`]. Exhaustive on purpose: adding a picker variant without a wire tag
/// is a compile error, so every future picker is remote-drivable by adding exactly one arm here.
pub fn picker_kind_wire(kind: crate::commands::PickerKind) -> &'static str {
    use crate::commands::PickerKind as P;
    match kind {
        P::Sessions => "picker:sessions",
        P::Checkpoints => "picker:checkpoints",
        P::Tempers => "picker:tempers",
        P::AssayChoice => "picker:assay",
        P::Models => "picker:models",
        P::ResumeMode => "picker:resume_mode",
        P::CopyBlocks => "picker:copy_blocks",
        P::ModelPin => "picker:model_pin",
        P::Duel => "picker:duel",
    }
}

/// One row of a remotely-projected overlay ([`OverlaySnapshot::rows`]): an opaque `id` the remote
/// client echoes back (`OverlaySelect`), two display strings, the cursor flag, and an optional
/// group header (e.g. a workflow phase title).
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayRowSnapshot {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub selected: bool,
    pub group: Option<String>,
}

/// A plain-data projection of whichever modal overlay currently owns the keyboard — the command
/// palette, any [`Picker`](crate::commands::Picker) kind, the `@path` picker, the `/config`
/// wizard, or an informational overlay (`/usage`, `/mesh`, the workflow view). Produced by
/// [`App::remote_overlay`] and mapped into the remote-control wire type by the render loop, so a
/// browser can render + drive the exact surface the TUI shows. `None` when nothing modal is open.
///
/// `filter` is `Some` when the overlay has a type-to-filter query the client may replace
/// (`OverlayFilter`); `free_text` is true while the overlay is collecting a free-form value
/// (e.g. a `/config` field being edited); `body` carries pre-rendered text for overlays that are
/// informational rather than selectable.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlaySnapshot {
    /// A stable discriminator: `"command_center"`, `"palette"`, `"picker:<kind>"`,
    /// `"picker:at_path"`, `"config"`, `"overlay:usage"`, `"overlay:mesh"`,
    /// `"overlay:workflow"`.
    pub kind: String,
    pub title: String,
    pub rows: Vec<OverlayRowSnapshot>,
    /// Cursor index into `rows` (server-authoritative).
    pub selected: usize,
    pub filter: Option<String>,
    pub free_text: bool,
    pub body: Option<String>,
}

impl App {
    /// Project the top-most open modal overlay for the remote-control wire (see
    /// [`OverlaySnapshot`]). Precedence mirrors the render loop's key routing, so the projection
    /// is always the surface a keystroke would actually reach: workflow view → `/config` editor →
    /// command center → palette → `/usage` → `/mesh` → `@path` picker → picker. The `/keys` configurator is
    /// exempt by design: it runs a blocking fullscreen loop on the host terminal (never App
    /// state), so there is nothing here to project — the remote drain notes it as host-only.
    pub fn remote_overlay(&self) -> Option<OverlaySnapshot> {
        if self.workflow.open {
            return Some(self.workflow_overlay());
        }
        if self.config_editor.open {
            return Some(self.config_overlay());
        }
        if self.command_center.open {
            return Some(self.command_center_overlay());
        }
        if self.palette.open {
            return Some(self.palette_overlay());
        }
        if self.usage_overlay.open {
            return Some(self.usage_overlay_snapshot());
        }
        if self.mesh_overlay.open {
            return Some(self.mesh_overlay_snapshot());
        }
        if self.at_picker.open {
            return Some(self.at_picker_overlay());
        }
        if self.picker.open {
            return Some(self.picker_overlay());
        }
        None
    }

    /// Whether a framed modal currently covers part of the active viewport. The terminal driver
    /// uses this edge-triggered signal to invalidate Ratatui's diff buffer on open/close: native
    /// libraries can bypass Ratatui and write directly to the terminal, so a normal incremental
    /// draw would otherwise leave stale text visible beneath an overlay.
    pub(crate) fn modal_surface_open(&self) -> bool {
        self.command_center.open
            || self.palette.open
            || self.picker.open
            || self.at_picker.open
            || self.config_editor.open
            || self.usage_overlay.open
            || self.mesh_overlay.open
            || self.voice.is_some()
    }

    fn command_center_overlay(&self) -> OverlaySnapshot {
        let rows = self
            .command_center
            .matches(&self.palette.extra)
            .into_iter()
            .enumerate()
            .map(|(i, entry)| OverlayRowSnapshot {
                id: entry.name.clone(),
                label: format!("/{}", entry.name),
                detail: entry.desc,
                selected: i == self.command_center.selected,
                group: Some(entry.category.to_string()),
            })
            .collect();
        OverlaySnapshot {
            kind: "command_center".to_string(),
            title: "command center".to_string(),
            rows,
            selected: self.command_center.selected,
            filter: Some(self.command_center.query.clone()),
            free_text: false,
            body: None,
        }
    }

    fn picker_overlay(&self) -> OverlaySnapshot {
        let kind = self
            .picker
            .kind
            .map(picker_kind_wire)
            .unwrap_or("picker:unknown");
        let rows = self
            .picker
            .matches()
            .into_iter()
            .enumerate()
            .map(|(i, r)| OverlayRowSnapshot {
                id: r.id.clone(),
                label: r.title.clone(),
                detail: r.subtitle.clone(),
                selected: i == self.picker.selected,
                group: None,
            })
            .collect();
        OverlaySnapshot {
            kind: kind.to_string(),
            title: self.picker.heading.clone(),
            rows,
            selected: self.picker.selected,
            filter: Some(self.picker.query.clone()),
            free_text: false,
            body: None,
        }
    }

    fn palette_overlay(&self) -> OverlaySnapshot {
        let rows = self
            .palette
            .matches()
            .into_iter()
            .enumerate()
            .map(|(i, e)| OverlayRowSnapshot {
                id: e.name.clone(),
                label: format!("/{}", e.name),
                detail: if e.usage.is_empty() {
                    e.desc.clone()
                } else {
                    format!("{} · {}", e.desc, e.usage)
                },
                selected: i == self.palette.selected,
                group: None,
            })
            .collect();
        OverlaySnapshot {
            kind: "palette".to_string(),
            title: "commands".to_string(),
            rows,
            selected: self.palette.selected,
            filter: Some(self.palette.query.clone()),
            free_text: false,
            body: None,
        }
    }

    fn at_picker_overlay(&self) -> OverlaySnapshot {
        let rows = self
            .at_picker
            .matches()
            .into_iter()
            .enumerate()
            .map(|(i, p)| OverlayRowSnapshot {
                id: p.clone(),
                label: format!("@{p}"),
                detail: String::new(),
                selected: i == self.at_picker.selected,
                group: None,
            })
            .collect();
        OverlaySnapshot {
            kind: "picker:at_path".to_string(),
            title: "mention a file".to_string(),
            rows,
            selected: self.at_picker.selected,
            filter: Some(self.at_picker.query.clone()),
            free_text: false,
            body: None,
        }
    }

    fn config_overlay(&self) -> OverlaySnapshot {
        let editing = self.config_editor.editing.as_ref();
        let matches = self.config_editor.matches();
        let rows = matches
            .iter()
            .enumerate()
            .map(|(i, &ri)| {
                let r = &self.config_editor.rows[ri];
                OverlayRowSnapshot {
                    id: r.path.clone(),
                    label: r.label.clone(),
                    detail: format!("{} = {} ({})", r.path, r.value, r.source),
                    selected: i == self.config_editor.selected,
                    group: Some(r.group.clone()),
                }
            })
            .collect();
        let scope = if self.config_editor.project_scope {
            "project"
        } else {
            "user"
        };
        let mut body = self.config_editor.status.clone();
        if let Some(buf) = editing {
            let path = self
                .config_editor
                .selected_row()
                .map(|r| r.path.clone())
                .unwrap_or_default();
            body = Some(format!("editing {path}: {buf}"));
        }
        OverlaySnapshot {
            kind: "config".to_string(),
            title: format!("settings — {scope} scope"),
            rows,
            selected: self.config_editor.selected,
            filter: Some(self.config_editor.filter.clone()),
            free_text: editing.is_some(),
            body,
        }
    }

    fn usage_overlay_snapshot(&self) -> OverlaySnapshot {
        let u = &self.usage_overlay;
        let mut body = String::new();
        if u.loading {
            body.push_str("loading subscription stats…\n\n");
        }
        body.push_str(&format!(
            "session: ${:.4} · {} in / {} out tokens\nmonth:   ${:.2}\n",
            u.session_usd, u.session_in, u.session_out, u.month_usd
        ));
        for (label, rows) in [
            ("last 5 hours", &u.by_model_5h),
            ("today", &u.by_model),
            ("this week", &u.by_model_week),
        ] {
            if rows.is_empty() {
                continue;
            }
            body.push_str(&format!("\n{label}:\n"));
            for (model, usd, tin, tout) in rows {
                body.push_str(&format!("  {model}  ${usd:.4}  {tin} in / {tout} out\n"));
            }
        }
        let caps: Vec<String> = [
            ("daily", u.daily_cap),
            ("weekly", u.weekly_cap),
            ("monthly", u.monthly_cap),
        ]
        .iter()
        .filter_map(|(l, c)| c.map(|v| format!("{l} ${v:.2}")))
        .collect();
        if !caps.is_empty() {
            body.push_str(&format!("\ncaps: {}\n", caps.join(" · ")));
        }
        for (label, pct) in [
            ("claude 5h", u.claude_5h_pct),
            ("claude weekly", u.claude_weekly_pct),
            ("codex 5h", u.codex_5h_pct),
            ("codex weekly", u.codex_weekly_pct),
        ] {
            if let Some(p) = pct {
                body.push_str(&format!("{label}: {p:.0}% used\n"));
            }
        }
        OverlaySnapshot {
            kind: "overlay:usage".to_string(),
            title: "usage".to_string(),
            rows: Vec::new(),
            selected: 0,
            filter: None,
            free_text: false,
            body: Some(body),
        }
    }

    fn mesh_overlay_snapshot(&self) -> OverlaySnapshot {
        let m = &self.mesh_overlay;
        let rows = m
            .candidates
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let mut badges = vec![c.cost_tag.clone()];
                if c.frontier {
                    badges.push("frontier".to_string());
                }
                if !c.usable {
                    badges.push("unusable".to_string());
                }
                if c.selected {
                    badges.push("← routed pick".to_string());
                }
                OverlayRowSnapshot {
                    id: c.model.clone(),
                    label: format!("#{} {}", c.rank, c.model),
                    detail: format!("score {:.2} · {}", c.score, badges.join(" · ")),
                    selected: i == m.cursor,
                    group: None,
                }
            })
            .collect();
        let mut body = String::new();
        if m.loading {
            body.push_str("computing routing explanation…\n");
        } else {
            if !m.prompt.is_empty() {
                body.push_str(&format!("prompt: {}\n", m.prompt));
            }
            body.push_str(&format!(
                "classified: {} ({}) → routed: {}{}\nreasons: {}\nconserve: {}\npick: {}\n",
                m.classified,
                m.classifier,
                m.routed,
                if m.code_heavy { " · code-heavy" } else { "" },
                m.reasons,
                m.conserve_line,
                m.pick,
            ));
            if !m.fallbacks.is_empty() {
                body.push_str(&format!("fallbacks: {}\n", m.fallbacks.join(" → ")));
            }
            if !m.rationale.is_empty() {
                body.push_str(&format!("rationale: {}\n", m.rationale));
            }
            for q in &m.quota {
                body.push_str(&format!(
                    "quota {}: {:.0}% used ({} · {}){}\n",
                    q.provider,
                    q.fraction * 100.0,
                    q.plan,
                    q.status,
                    mesh_pace_suffix(q.projected_fraction_at_reset, q.exhaustion_warning),
                ));
            }
        }
        OverlaySnapshot {
            kind: "overlay:mesh".to_string(),
            title: "mesh — routing inspector".to_string(),
            rows,
            selected: m.cursor,
            filter: None,
            free_text: false,
            body: Some(body),
        }
    }

    fn workflow_overlay(&self) -> OverlaySnapshot {
        let w = &self.workflow;
        let rows = w
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let status = if !r.done {
                    "◐"
                } else if r.ok {
                    "✓"
                } else {
                    "✗"
                };
                OverlayRowSnapshot {
                    id: r.id.clone(),
                    label: format!("{status} {}", r.agent),
                    detail: format!("{} — {}", r.task, r.last),
                    selected: i == w.selected,
                    group: r.phase_idx.map(|p| w.phases[p].title.clone()),
                }
            })
            .collect();
        let title = match (&w.name, &w.finished) {
            (Some(n), None) => format!("workflow: {n} — running"),
            (Some(n), Some((ok, _))) => {
                format!(
                    "workflow: {n} — {}",
                    if *ok { "finished" } else { "failed" }
                )
            }
            (None, None) => "workflow — running".to_string(),
            (None, Some((ok, _))) => {
                format!("workflow — {}", if *ok { "finished" } else { "failed" })
            }
        };
        let mut body = String::new();
        if let Some((_, summary)) = &w.finished {
            body.push_str(summary);
            body.push('\n');
        }
        if !w.logs.is_empty() {
            body.push_str(&w.logs.join("\n"));
        }
        OverlaySnapshot {
            kind: "overlay:workflow".to_string(),
            title,
            rows,
            selected: w.selected,
            filter: None,
            free_text: false,
            body: (!body.is_empty()).then_some(body),
        }
    }
}

/// How many landed file diffs a turn retains for the remote diff card. Older files are evicted
/// into a "+N more files" count; the full history is always in the TUI scrollback + tool results.
pub const REMOTE_DIFF_MAX_FILES: usize = 10;

/// One `@@` hunk of a remotely-projected file diff: the unified-diff header (old/new line spans)
/// plus its body lines, each prefixed `+` / `-` / ` ` (gutter as the first character).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiffHunkSnapshot {
    pub header: String,
    pub lines: Vec<String>,
}

/// One file's change, projected for the remote wire (see `render::diff_file_snapshot`): full
/// add/del counts, capped hunks, and how many hunk lines were dropped over the cap.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiffFileSnapshot {
    pub path: String,
    /// "created" | "modified" | "deleted".
    pub kind: String,
    /// Non-UTF-8 target: no textual hunks, the page shows a one-line "binary file" summary.
    pub binary: bool,
    pub adds: usize,
    pub dels: usize,
    pub hunks: Vec<DiffHunkSnapshot>,
    pub skipped_lines: usize,
}

/// The remote diff card: either the ONE proposed change a pending write permission would apply
/// (`pending: true` — "what will this touch" before Allow), or every change that landed this
/// turn (`pending: false`, capped to [`REMOTE_DIFF_MAX_FILES`] files).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiffSnapshot {
    pub pending: bool,
    pub files: Vec<DiffFileSnapshot>,
    pub skipped_files: usize,
}

/// A plain-data view of the live state, produced by [`App::remote_snapshot`] and mapped into the
/// `remote::Snapshot` JSON by the render loop. Defined here (in forge-tui) so the pure render
/// crate owns the projection without depending on the server module.
#[derive(Debug, Clone, Default)]
pub struct RemoteSnapshot {
    pub busy: bool,
    pub done: bool,
    pub temper: String,
    pub effort: Option<forge_types::EffortLevel>,
    pub tier: Option<String>,
    pub model: String,
    pub cost_usd: f64,
    pub context_tokens: u64,
    pub context_limit: Option<u32>,
    pub streaming: String,
    pub transcript: Vec<String>,
    /// The same lines as `transcript`, each carrying the provenance its emitter recorded (v9) —
    /// see [`TranscriptRow`]. Same length and order.
    pub transcript_rows: Vec<TranscriptRow>,
    pub tasks: Vec<forge_types::TodoItem>,
    pub subagents: Vec<SubagentSnapshot>,
    pub queued: Vec<String>,
    pub permission_prompt: Option<String>,
    pub question: Option<String>,
    pub question_options: Vec<crate::QChoice>,
    pub question_allow_other: bool,
    /// The structured diff card (v7) — see [`DiffSnapshot`]. `None` when nothing changed.
    pub diff: Option<DiffSnapshot>,
    /// The most recent plan proposal (v7), for the remote plan-approval card.
    pub plan: Option<forge_types::PlanProposal>,
    /// The predicted next prompt (v8), if any — the page shows it as the composer's placeholder.
    pub suggested_prompt: Option<String>,
    /// The live (or just-finished) workflow run this turn (v8.1) — see [`WorkflowRemote`].
    pub workflow: Option<WorkflowRemote>,
}

/// Remote projection of the dedicated workflow view (v8.1): phases + narration + finish state.
/// Agent rows ride in [`RemoteSnapshot::subagents`] with `phase` attached, so this stays a
/// cheap clone per frame (no per-row transcript).
#[derive(Debug, Clone, Default)]
pub struct WorkflowRemote {
    pub active: bool,
    pub name: Option<String>,
    pub phases: Vec<String>,
    /// Tail of the script's `log()` narration (bounded to [`WORKFLOW_REMOTE_LOG_TAIL`]).
    pub logs: Vec<String>,
    pub finished_ok: Option<bool>,
    pub summary: Option<String>,
}

/// How many trailing `log()` lines ride in each remote frame — enough for a live narration
/// feed without recloning the whole bounded buffer every broadcast.
const WORKFLOW_REMOTE_LOG_TAIL: usize = 30;

/// Where a finalized scrollback line came from. Scrollback is a flat list of styled lines by the
/// time it reaches the remote ring, so the emitter records this as it pushes — a remote client
/// can't recover "this was a tool result, not the model talking" from the text afterwards.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TranscriptKind {
    /// Anything that is neither a user echo, model prose, nor tool activity: warnings, errors,
    /// command feedback, banners. The default, so an untagged emitter can never masquerade as
    /// the model or a tool.
    #[default]
    System,
    User,
    Assistant,
    Tool,
}
