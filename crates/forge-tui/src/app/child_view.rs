//! The subagent session view: a child agent's persisted conversation shown in place of the chat,
//! through the same transcript renderer the main session uses. The parent's own state is never
//! touched while it is open, so Esc returns to exactly the scroll position, details and queue the
//! user left.
//!
//! The view owns a private transcript-only [`App`]. The host feeds it [`ReplayItem`]s read from
//! the store (the same mapping a resumed session replays) and re-feeds them while the child runs.
//! The composer and status line are the parent's own: what is typed goes to the child, and the
//! host runs it as a follow-up turn of that child's session.

use super::*;
use crate::theme::glyphs::g;
use crate::theme::styles;

const FALLBACK_WIDTH: u16 = 80;
const FALLBACK_HEIGHT: u16 = 20;

#[derive(Debug, Clone)]
pub(crate) struct ChildView {
    pub(crate) id: String,
    transcript: Box<App>,
    items: Vec<ReplayItem>,
    built_width: u16,
}

/// What the host must do after a key reached the agents panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelKey {
    /// Not a panel key; the chat handles it.
    Unhandled,
    /// The panel took it (focus, selection).
    Handled,
    /// Open this subagent's session view (the host loads its conversation).
    OpenChild(String),
    /// Open the pager on activity entry `idx`: a critic in full-screen, anything in inline mode.
    OpenPager(usize),
}

/// What the host must do after a key reached the open view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildKey {
    /// No view is open; the key belongs to the chat.
    Inactive,
    /// The view took the key (scroll, edit, close).
    Handled,
    /// Show this other subagent's view (the host loads its conversation).
    Switch(String),
    /// The user submitted this message to the open child.
    Submit(String),
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

    /// A key for the agents panel: Ctrl+O shows it, ↑↓ select, Enter opens the selected subagent as
    /// its own session view (full-screen), Esc hides it.
    pub fn activity_key(&mut self, key: KeyKind) -> PanelKey {
        if matches!(key, KeyKind::ToggleSubagentDetail) {
            if self.workflow.exists() {
                self.workflow.open = true;
            } else if self.has_activity() || !self.tasks().is_empty() {
                self.activity_focused = !self.activity_focused;
                if self.activity_focused {
                    self.activity_idx =
                        self.activity_idx.min(self.activity_len().saturating_sub(1));
                }
            }
            return PanelKey::Handled;
        }
        if !self.activity_focused {
            return PanelKey::Unhandled;
        }
        let n = self.activity_len();
        if n == 0 {
            // A bare task list has nothing to select; only Esc is the panel's.
            if matches!(key, KeyKind::Esc) {
                self.activity_focused = false;
                return PanelKey::Handled;
            }
            return PanelKey::Unhandled;
        }
        match key {
            KeyKind::Up => self.activity_idx = (self.activity_idx + n - 1) % n,
            KeyKind::Down => self.activity_idx = (self.activity_idx + 1) % n,
            KeyKind::Esc => self.activity_focused = false,
            KeyKind::Enter => {
                let idx = self.activity_idx;
                if self.fullscreen && idx == 0 {
                    self.activity_focused = false;
                } else if let Some(id) = self.subagent_session_at(idx).filter(|_| self.fullscreen) {
                    return PanelKey::OpenChild(id);
                } else {
                    return PanelKey::OpenPager(idx);
                }
            }
            _ => {}
        }
        PanelKey::Handled
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
        self.remember_child_scroll();
        let width = self.last_width.get();
        let mut transcript = self.blank_transcript();
        transcript.last_width.set(width);
        transcript.transcript_follow = true;
        fill_transcript(&mut transcript, items);
        if let Some(&(scroll, follow)) = self.child_scrolls.get(id) {
            transcript.transcript_scroll = scroll;
            transcript.transcript_follow = follow;
        }
        self.child_view = Some(ChildView {
            id: id.to_string(),
            transcript: Box::new(transcript),
            items: items.to_vec(),
            built_width: width,
        });
        self.child_notice = None;
        self.activity_focused = false;
        true
    }

    /// Replace the open view's conversation with `items`, keeping the scroll position (or the
    /// tail, while following).
    pub fn refresh_child_view(&mut self, items: &[ReplayItem]) {
        let Some(view) = self.child_view.as_ref() else {
            return;
        };
        let width = view.transcript.last_width.get();
        let mut next = self.blank_transcript();
        let Some(view) = self.child_view.as_mut() else {
            return;
        };
        let old = &view.transcript;
        next.transcript_scroll = old.transcript_scroll;
        next.transcript_follow = old.transcript_follow;
        next.tick = old.tick;
        next.last_width.set(width);
        fill_transcript(&mut next, items);
        *view.transcript = next;
        view.items = items.to_vec();
        view.built_width = width;
    }

    pub fn close_child_view(&mut self) {
        self.remember_child_scroll();
        self.child_view = None;
        self.child_notice = None;
    }

    fn remember_child_scroll(&mut self) {
        if let Some(v) = &self.child_view {
            self.child_scrolls.insert(
                v.id.clone(),
                (
                    v.transcript.transcript_scroll,
                    v.transcript.transcript_follow,
                ),
            );
        }
    }

    /// Mark a finished child as working again: the host is running a follow-up turn on it.
    pub fn reopen_subagent(&mut self, id: &str) {
        self.settled_subagents.remove(id);
        if let Some(r) = self.subagents.iter_mut().find(|r| r.id == id) {
            r.done = false;
            r.ok = true;
        }
    }

    /// Say, in the open view's header, why the last message was not sent.
    pub fn child_view_notice(&mut self, text: &str) {
        self.child_notice = Some(text.to_string());
    }

    /// The agent name behind the open view, for the host's messages.
    pub fn child_view_agent(&self) -> Option<String> {
        let id = self.child_view_id()?;
        self.child_meta(id).map(|m| m.agent)
    }

    /// Ids a left/right press cycles through: the main chat (not an id) is position 0, then every
    /// subagent of the list the open one belongs to.
    fn child_ring(&self) -> Vec<String> {
        let id = self.child_view_id().unwrap_or_default();
        if self.workflow.rows.iter().any(|r| r.id == id) {
            self.workflow.rows.iter().map(|r| r.id.clone()).collect()
        } else {
            self.subagents.iter().map(|r| r.id.clone()).collect()
        }
    }

    /// Whether the child behind the open view is still working (the host keeps re-reading its
    /// conversation only then).
    pub fn child_view_running(&self) -> bool {
        self.child_view_id()
            .and_then(|id| self.child_meta(id))
            .is_some_and(|m| m.status == ActivityStatus::Running)
    }

    /// Advance the open view's spinner clock to the parent's, and re-wrap its conversation when the
    /// terminal width it was built for has changed.
    pub fn sync_child_view_tick(&mut self) {
        let tick = self.tick;
        let Some(v) = self.child_view.as_mut() else {
            return;
        };
        v.transcript.tick = tick;
        let width = v.transcript.last_width.get();
        if width != 0 && width != v.built_width {
            let items = v.items.clone();
            self.refresh_child_view(&items);
        }
    }

    /// Feed a key to the open view: scrolling and closing are the view's; everything else edits
    /// the composer, whose Enter sends the line to the child.
    pub fn child_view_key(&mut self, key: KeyKind) -> ChildKey {
        if self.child_view.is_none() || super::render::prompt_owns_screen(self) {
            return ChildKey::Inactive;
        }
        self.child_notice = None;
        match key {
            KeyKind::Esc | KeyKind::Interrupt => {
                self.close_child_view();
                return ChildKey::Handled;
            }
            KeyKind::Left | KeyKind::Right if self.input.is_empty() => {
                let ring = self.child_ring();
                let here = self
                    .child_view_id()
                    .and_then(|id| ring.iter().position(|r| r == id))
                    .map_or(0, |i| i + 1);
                let slots = ring.len() + 1;
                let next = if key == KeyKind::Left {
                    (here + slots - 1) % slots
                } else {
                    (here + 1) % slots
                };
                return match next.checked_sub(1).and_then(|i| ring.get(i)) {
                    Some(id) if next != here => ChildKey::Switch(id.clone()),
                    _ => {
                        self.close_child_view();
                        ChildKey::Handled
                    }
                };
            }
            _ => {}
        }
        let Some(view) = self.child_view.as_mut() else {
            return ChildKey::Inactive;
        };
        let t = &mut *view.transcript;
        let (width, height) = t
            .transcript_geom
            .get()
            .map_or((FALLBACK_WIDTH, FALLBACK_HEIGHT), |g| (g.width, g.height));
        let (_, max_scroll) = t.transcript_metrics(width, height);
        let page = usize::from(height.saturating_sub(1)).max(1);
        match key {
            KeyKind::Up => t.transcript_scroll_up(1),
            KeyKind::Down => t.transcript_scroll_down(1, max_scroll),
            KeyKind::PageUp => t.transcript_scroll_up(page),
            KeyKind::PageDown => t.transcript_scroll_down(page, max_scroll),
            KeyKind::JumpPrevUser => t.transcript_to_top(),
            KeyKind::JumpBottom => t.transcript_to_bottom(),
            KeyKind::ToggleToolCard => {
                t.toggle_last_tool_card();
            }
            KeyKind::PrevToolGroup | KeyKind::NextToolGroup | KeyKind::ToggleToolGroups => {
                t.tool_group_key(key);
            }
            _ => {
                if let InputOutcome::Submit(raw) = self.edit_key(key) {
                    let line = raw.trim().to_string();
                    if !line.is_empty() {
                        return ChildKey::Submit(line);
                    }
                }
            }
        }
        ChildKey::Handled
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

/// Draw the view over the whole frame: breadcrumb, task line, the child's transcript, then the
/// composer and status line the chat has.
pub(crate) fn render_child_view(frame: &mut Frame, view: &ChildView, parent: &App) {
    let area = frame.area();
    if area.height < 8 {
        return;
    }
    let meta = parent.child_meta(&view.id);
    let composer_h = super::render::input_box_height(&parent.input, area.width)
        .min(area.height.saturating_sub(5));
    let status_h = super::render::statusline_height(parent).min(1);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(composer_h),
        Constraint::Length(status_h),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(header_lines(
            parent,
            &view.id,
            meta.as_ref(),
            rows[0].width as usize,
        )),
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
        t.last_width.set(rows[1].width);
    } else {
        super::render::render_transcript_area(frame, rows[1], t);
    }
    super::render::render_input(frame, rows[2], parent);
    if status_h > 0 {
        super::render::render_statusline(frame, rows[3], parent);
    }
}

/// The composer hints while the view is open.
pub(crate) fn hints(app: &App) -> Option<Vec<super::render::chrome::Hint>> {
    use super::render::chrome::hint;
    app.child_view.as_ref()?;
    let mut hints = vec![hint("Enter", "send"), hint("Esc", "back to chat")];
    if app.input.is_empty() && app.child_ring().len() > 1 {
        hints.push(hint("←→", "switch"));
    }
    hints.push(hint("↑↓", "scroll"));
    hints.push(hint("Ctrl+T", "tool"));
    Some(hints)
}

/// What the empty composer says while the view is open: who a message goes to, or why it cannot yet.
pub(crate) fn composer_ghost(app: &App) -> Option<String> {
    let id = app.child_view_id()?;
    let meta = app.child_meta(id)?;
    let ellipsis = g().ellipsis;
    Some(if meta.status == ActivityStatus::Running {
        format!("{} is working{ellipsis}", meta.agent)
    } else if app.busy {
        format!("a turn is running — message {} once it ends", meta.agent)
    } else {
        format!("Message {}{ellipsis}", meta.agent)
    })
}

fn header_lines(
    parent: &App,
    id: &str,
    meta: Option<&ChildMeta>,
    width: usize,
) -> Vec<TextLine<'static>> {
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
    let ring = parent.child_ring();
    if ring.len() > 1 {
        if let Some(i) = ring.iter().position(|r| r == id) {
            spans.push(Span::styled(
                format!(" {}/{}", i + 1, ring.len()),
                styles::faint(),
            ));
        }
    }
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
    let second = match &parent.child_notice {
        Some(notice) => Span::styled(format!("  {notice}"), styles::warn()),
        None => Span::styled(
            format!(
                "  {}",
                forge_types::truncate_ellipsis(&task, width.saturating_sub(4).max(10))
            ),
            styles::faint(),
        ),
    };
    vec![TextLine::from(spans), TextLine::from(second)]
}
