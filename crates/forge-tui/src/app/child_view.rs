//! The subagent session view: a child agent's persisted conversation shown in place of the chat,
//! through the same transcript renderer the main session uses. The parent's own state is never
//! touched while it is open, so Esc returns to exactly the scroll position, details and queue the
//! user left.
//!
//! The view owns a private transcript-only [`App`]. The host feeds it [`ReplayItem`]s read from
//! the store (the same mapping a resumed session replays) and re-feeds them while the child runs.

use super::*;
use crate::theme::glyphs::g;
use crate::theme::styles;

const FALLBACK_WIDTH: u16 = 80;
const FALLBACK_HEIGHT: u16 = 20;

#[derive(Debug, Clone)]
pub(crate) struct ChildView {
    pub(crate) id: String,
    transcript: Box<App>,
}

/// What the header shows about the child; read live from the parent's rows each frame.
struct ChildMeta {
    agent: String,
    task: String,
    model: Option<String>,
    status: ActivityStatus,
    cost: f64,
}

impl App {
    /// The session id of the subagent at activity-list index `idx` (0 is the main chat).
    pub fn subagent_session_at(&self, idx: usize) -> Option<String> {
        let row = self.subagents.get(idx.checked_sub(1)?)?;
        Some(row.id.clone())
    }

    /// The session id of the workflow agent the workflow view has selected.
    pub fn selected_workflow_session(&self) -> Option<String> {
        self.workflow
            .rows
            .get(self.workflow.selected)
            .map(|r| r.id.clone())
    }

    /// The session id of the child being viewed, if the view is open.
    pub fn child_view_id(&self) -> Option<&str> {
        self.child_view.as_ref().map(|v| v.id.as_str())
    }

    /// Open the view on child session `id` with its conversation so far. Returns false when `id`
    /// is not a subagent this session knows.
    pub fn open_child_view(&mut self, id: &str, items: &[ReplayItem]) -> bool {
        if self.child_meta(id).is_none() {
            return false;
        }
        let mut transcript = self.blank_transcript();
        transcript.transcript_follow = true;
        fill_transcript(&mut transcript, items);
        self.child_view = Some(ChildView {
            id: id.to_string(),
            transcript: Box::new(transcript),
        });
        self.activity_focused = false;
        true
    }

    /// Replace the open view's conversation with `items`, keeping the scroll position (or the
    /// tail, while following).
    pub fn refresh_child_view(&mut self, items: &[ReplayItem]) {
        let mut next = self.blank_transcript();
        let Some(view) = self.child_view.as_mut() else {
            return;
        };
        let old = &view.transcript;
        next.transcript_scroll = old.transcript_scroll;
        next.transcript_follow = old.transcript_follow;
        next.tick = old.tick;
        fill_transcript(&mut next, items);
        *view.transcript = next;
    }

    pub fn close_child_view(&mut self) {
        self.child_view = None;
    }

    /// Whether the child behind the open view is still working (the host keeps re-reading its
    /// conversation only then).
    pub fn child_view_running(&self) -> bool {
        self.child_view_id()
            .and_then(|id| self.child_meta(id))
            .is_some_and(|m| m.status == ActivityStatus::Running)
    }

    /// Advance the open view's spinner clock to the parent's.
    pub fn sync_child_view_tick(&mut self) {
        let tick = self.tick;
        if let Some(v) = self.child_view.as_mut() {
            v.transcript.tick = tick;
        }
    }

    /// Feed a key to the open view. Returns true when the view is open (it owns every key).
    pub fn child_view_key(&mut self, key: KeyKind) -> bool {
        let Some(view) = self.child_view.as_mut() else {
            return false;
        };
        let t = &mut *view.transcript;
        let (width, height) = t
            .transcript_geom
            .get()
            .map_or((FALLBACK_WIDTH, FALLBACK_HEIGHT), |g| (g.width, g.height));
        let (_, max_scroll) = t.transcript_metrics(width, height);
        let page = usize::from(height.saturating_sub(1)).max(1);
        match key {
            KeyKind::Esc | KeyKind::Interrupt | KeyKind::Char('q') => self.child_view = None,
            KeyKind::Up | KeyKind::Char('k') => t.transcript_scroll_up(1),
            KeyKind::Down | KeyKind::Char('j') => t.transcript_scroll_down(1, max_scroll),
            KeyKind::PageUp => t.transcript_scroll_up(page),
            KeyKind::PageDown | KeyKind::Char(' ') => t.transcript_scroll_down(page, max_scroll),
            KeyKind::Home | KeyKind::Char('g') => t.transcript_to_top(),
            KeyKind::End | KeyKind::JumpBottom | KeyKind::Char('G') => t.transcript_to_bottom(),
            KeyKind::ToggleToolCard => {
                t.toggle_last_tool_card();
            }
            KeyKind::PrevToolGroup | KeyKind::NextToolGroup | KeyKind::ToggleToolGroups => {
                t.tool_group_key(key);
            }
            _ => {}
        }
        true
    }

    /// Mouse-wheel scroll for the open view. Returns true when the view took it.
    pub fn child_view_scroll(&mut self, up: bool, rows: usize) -> bool {
        let Some(view) = self.child_view.as_mut() else {
            return false;
        };
        let t = &mut *view.transcript;
        if up {
            t.transcript_scroll_up(rows);
        } else {
            let (width, height) = t
                .transcript_geom
                .get()
                .map_or((FALLBACK_WIDTH, FALLBACK_HEIGHT), |g| (g.width, g.height));
            let (_, max_scroll) = t.transcript_metrics(width, height);
            t.transcript_scroll_down(rows, max_scroll);
        }
        true
    }

    fn blank_transcript(&self) -> App {
        App {
            fullscreen: true,
            fold_tool_groups: self.fold_tool_groups,
            ..App::default()
        }
    }

    fn child_meta(&self, id: &str) -> Option<ChildMeta> {
        if let Some(r) = self.subagents.iter().find(|r| r.id == id) {
            return Some(ChildMeta {
                agent: r.agent.clone(),
                task: r.task.clone(),
                model: r.model.clone(),
                status: row_status(r.done, r.ok),
                cost: r.cost,
            });
        }
        let r = self.workflow.rows.iter().find(|r| r.id == id)?;
        Some(ChildMeta {
            agent: r.agent.clone(),
            task: r.task.clone(),
            model: r.model.clone(),
            status: row_status(r.done, r.ok),
            cost: r.cost,
        })
    }
}

fn row_status(done: bool, ok: bool) -> ActivityStatus {
    match (done, ok) {
        (false, _) => ActivityStatus::Running,
        (true, true) => ActivityStatus::Done,
        (true, false) => ActivityStatus::Failed,
    }
}

fn fill_transcript(t: &mut App, items: &[ReplayItem]) {
    t.replay_history(items);
    t.drain_flush();
}

/// Draw the view over the whole frame: breadcrumb, task line, the child's transcript, key hints.
pub(crate) fn render_child_view(frame: &mut Frame, view: &ChildView, parent: &App) {
    let area = frame.area();
    if area.height < 4 {
        return;
    }
    let meta = parent.child_meta(&view.id);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(header_lines(parent, meta.as_ref(), rows[0].width as usize)),
        rows[0],
    );
    let t = &*view.transcript;
    if super::render::welcome::transcript_is_empty(t, rows[1].width) {
        frame.render_widget(
            Paragraph::new(TextLine::from(Span::styled(
                "  waiting for the agent's first output…",
                styles::muted(),
            ))),
            rows[1],
        );
        t.transcript_geom.set(None);
    } else {
        super::render::render_transcript_area(frame, rows[1], t);
    }
    frame.render_widget(Paragraph::new(footer_line(t)), rows[2]);
}

fn header_lines(parent: &App, meta: Option<&ChildMeta>, width: usize) -> Vec<TextLine<'static>> {
    let glyphs = g();
    let sep = format!(" {} ", glyphs.sep);
    let crumb = format!(" {} ", if glyphs.unicode { "›" } else { ">" });
    let agent = meta.map_or("agent", |m| m.agent.as_str());
    let mut spans = vec![
        Span::styled("  ", styles::faint()),
        Span::styled("main chat", styles::muted()),
        Span::styled(crumb, styles::faint()),
        Span::styled(agent.to_string(), styles::brand().bold()),
    ];
    if let Some(m) = meta {
        if let Some(model) = &m.model {
            spans.push(Span::styled(
                format!("{sep}{}", model_short(Some(model))),
                styles::muted(),
            ));
        }
        let (mark, label, style) = match m.status {
            ActivityStatus::Running => (
                glyphs.spin(parent.tick).to_string(),
                "running",
                styles::spinner(parent.tick),
            ),
            ActivityStatus::Failed => (glyphs.fail.to_string(), "failed", styles::err()),
            _ => (glyphs.ok.to_string(), "done", styles::ok()),
        };
        spans.push(Span::styled(format!("{sep}{mark} {label}"), style));
        if m.cost > 0.0 {
            spans.push(Span::styled(
                format!("{sep}{}", super::render::panels::fmt_cost(m.cost)),
                styles::faint(),
            ));
        }
    }
    let task = meta.map_or(String::new(), |m| {
        m.task.split_whitespace().collect::<Vec<_>>().join(" ")
    });
    vec![
        TextLine::from(spans),
        TextLine::from(Span::styled(
            format!(
                "  {}",
                forge_types::truncate_ellipsis(&task, width.saturating_sub(4).max(10))
            ),
            styles::faint(),
        )),
    ]
}

fn footer_line(t: &App) -> TextLine<'static> {
    let glyphs = g();
    let sep = format!(" {} ", glyphs.sep);
    let mut spans = Vec::new();
    for (i, (key, label)) in [
        ("Esc", "back to chat"),
        ("↑↓ PgUp/PgDn", "scroll"),
        ("Home/End", "ends"),
        ("Ctrl+T", "tool"),
    ]
    .into_iter()
    .enumerate()
    {
        if i > 0 {
            spans.push(Span::styled(sep.clone(), styles::faint()));
        } else {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(key.to_string(), styles::key_chip()));
        spans.push(Span::styled(format!(" {label}"), styles::faint()));
    }
    if !t.transcript_follow {
        spans.push(Span::styled(format!("{sep}scrolled up"), styles::muted()));
    }
    TextLine::from(spans)
}
