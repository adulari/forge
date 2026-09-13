//! Shared drawing helpers for the board: the span-row builder that records click rects, the
//! width-budgeted line builder, unicode-aware clipping and wrapping, the small meters and glyphs
//! every surface reuses, the contextual keybar, and the three overlays (composer, confirm, help).
//!
//! Everything here is layout-only: it reads the board's state and writes into the frame plus a
//! local hit list. Nothing mutates [`BoardApp`].

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::SPINNER;
use crate::surface::{
    self, SurfaceTone, ACCENT, DIM, ERRRED, OKGREEN, ORANGE, SELECT_BG, SEPARATOR, TEXT, TOOLCYAN,
    VERY_DIM, WARNYEL,
};

use super::keys::HELP;
use super::model::{fmt_cost, model_short, Card, Health};
use super::state::{BoardApp, Button, ComposerMode, Focus, Hit, ToastLevel};
use super::wire::{GitFile, Subagent};

// ─────────────────────────────────────────────────────────────────── geometry

/// The overlap of two rectangles, or `None` when they do not touch.
pub(super) fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = a.x.saturating_add(a.width).min(b.x.saturating_add(b.width));
    let y2 =
        a.y.saturating_add(a.height)
            .min(b.y.saturating_add(b.height));
    (x2 > x1 && y2 > y1).then(|| Rect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    })
}

/// Record a clickable region, clipped to what is actually on screen.
pub(super) fn push_hit(hits: &mut Vec<(Rect, Hit)>, rect: Rect, clip: Rect, hit: Hit) {
    if let Some(r) = intersect(rect, clip) {
        hits.push((r, hit));
    }
}

// ───────────────────────────────────────────────────────────── text utilities

/// Flatten and clip to `max` terminal cells (not chars), adding an ellipsis when it had to cut.
pub(super) fn clip_cells(s: &str, max: usize) -> String {
    let s = s.replace(['\n', '\r', '\t'], " ");
    if max == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s.as_str()) <= max {
        return s;
    }
    if max == 1 {
        return "…".into();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > max - 1 {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Cell width of a string, as the terminal will lay it out.
pub(super) fn cells(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Greedy word wrap to `width` cells, preserving blank lines.
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let flattened = text.replace('\t', "    ");
    let mut out: Vec<String> = Vec::new();
    for raw in flattened.split('\n') {
        let mut cur = String::new();
        let mut cur_w = 0usize;
        for word in raw.split_whitespace() {
            for piece in split_long(word, width) {
                let pw = cells(&piece);
                if cur_w == 0 {
                    cur = piece;
                    cur_w = pw;
                } else if cur_w + 1 + pw <= width {
                    cur.push(' ');
                    cur.push_str(&piece);
                    cur_w += 1 + pw;
                } else {
                    out.push(std::mem::take(&mut cur));
                    cur = piece;
                    cur_w = pw;
                }
            }
        }
        out.push(cur);
    }
    out
}

/// A word longer than the line is broken on cell boundaries rather than overflowing.
fn split_long(word: &str, width: usize) -> Vec<String> {
    if cells(word) <= width {
        return vec![word.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut w = 0usize;
    for ch in word.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            w = 0;
        }
        cur.push(ch);
        w += cw;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ───────────────────────────────────────────────────────────── span builders

/// A single screen row built span by span, which knows where each span landed so the caller can
/// register a click target for it.
pub(super) struct Row {
    x0: u16,
    y: u16,
    w: u16,
    spans: Vec<Span<'static>>,
}

impl Row {
    pub(super) fn at(x: u16, y: u16) -> Self {
        Self {
            x0: x,
            y,
            w: 0,
            spans: Vec::new(),
        }
    }

    /// Append a span and return the rectangle it occupies.
    pub(super) fn add(&mut self, text: impl Into<String>, style: Style) -> Rect {
        let text: String = text.into();
        let width = cells(&text) as u16;
        let rect = Rect {
            x: self.x0.saturating_add(self.w),
            y: self.y,
            width,
            height: 1,
        };
        self.w = self.w.saturating_add(width);
        self.spans.push(Span::styled(text, style));
        rect
    }

    pub(super) fn sep(&mut self) {
        self.add(" · ", Style::default().fg(VERY_DIM));
    }

    pub(super) fn width(&self) -> u16 {
        self.w
    }

    /// Draw the row, clipped to `clip`.
    pub(super) fn render(self, frame: &mut Frame, clip: Rect) {
        let rect = Rect {
            x: self.x0,
            y: self.y,
            width: self.w,
            height: 1,
        };
        if let Some(r) = intersect(rect, clip) {
            frame.render_widget(Paragraph::new(Line::from(self.spans)), r);
        }
    }
}

/// Lay a row out flush to `right_edge` instead of a known left edge.
pub(super) fn right_row(items: Vec<(String, Style)>, right_edge: u16, y: u16) -> Row {
    let w: u16 = items.iter().map(|(t, _)| cells(t) as u16).sum();
    let mut row = Row::at(right_edge.saturating_sub(w), y);
    for (t, s) in items {
        row.add(t, s);
    }
    row
}

/// A line assembled under a hard cell budget: every append is clipped to what is left, so a card
/// or pane row can never overflow its column.
pub(super) struct Fit {
    spans: Vec<Span<'static>>,
    left: usize,
    total: usize,
}

impl Fit {
    pub(super) fn new(width: usize) -> Self {
        Self {
            spans: Vec::new(),
            left: width,
            total: width,
        }
    }

    pub(super) fn add(&mut self, text: impl Into<String>, style: Style) {
        if self.left == 0 {
            return;
        }
        let text = clip_cells(&text.into(), self.left);
        if text.is_empty() {
            return;
        }
        self.left = self.left.saturating_sub(cells(&text));
        self.spans.push(Span::styled(text, style));
    }

    pub(super) fn extend(&mut self, spans: Vec<Span<'static>>) {
        for s in spans {
            let style = s.style;
            self.add(s.content.into_owned(), style);
        }
    }

    pub(super) fn left(&self) -> usize {
        self.left
    }

    /// Finish, pushing `right` flush to the right edge when there is room for it.
    pub(super) fn right(mut self, text: impl Into<String>, style: Style) -> Line<'static> {
        let text: String = text.into();
        let w = cells(&text);
        if w > 0 && w < self.left {
            let pad = self.left - w;
            self.spans.push(Span::raw(" ".repeat(pad)));
            self.spans.push(Span::styled(text, style));
            self.left = 0;
        }
        self.line()
    }

    pub(super) fn line(self) -> Line<'static> {
        Line::from(self.spans)
    }

    pub(super) fn used(&self) -> usize {
        self.total - self.left
    }
}

// ──────────────────────────────────────────────────────────────────── glyphs

pub(super) fn spinner(tick: u64) -> &'static str {
    SPINNER[(tick as usize) % SPINNER.len()]
}

/// The slow on/off of an attention dot: five ticks lit, five dark.
pub(super) fn pulse(tick: u64) -> bool {
    (tick / 5).is_multiple_of(2)
}

pub(super) fn health_glyph(card: &Card, tick: u64) -> (String, Color) {
    match card.health {
        Health::Waiting => (if pulse(tick) { "●" } else { "○" }.to_string(), ERRRED),
        Health::Busy => (spinner(tick).to_string(), TOOLCYAN),
        Health::Stalled => ("▲".into(), WARNYEL),
        Health::Failed => ("✗".into(), ERRRED),
        Health::Idle => ("●".into(), OKGREEN),
        Health::Finished => ("✓".into(), DIM),
    }
}

pub(super) fn health_tone(health: Health) -> SurfaceTone {
    match health {
        Health::Waiting => SurfaceTone::Danger,
        Health::Stalled | Health::Failed => SurfaceTone::Warning,
        Health::Busy => SurfaceTone::Tool,
        Health::Idle => SurfaceTone::Success,
        Health::Finished => SurfaceTone::Brand,
    }
}

/// `▰▰▰▱▱` — a compact meter that reads at a glance even on a 30-cell column.
pub(super) fn gauge(frac: f64, width: usize, color: Color) -> Vec<Span<'static>> {
    let frac = if frac.is_finite() {
        frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((frac * width as f64).round() as usize).min(width);
    vec![
        Span::styled("▰".repeat(filled), Style::default().fg(color)),
        Span::styled("▱".repeat(width - filled), Style::default().fg(VERY_DIM)),
    ]
}

pub(super) fn context_color(pct: u8) -> Color {
    if pct >= 95 {
        ERRRED
    } else if pct >= 80 {
        WARNYEL
    } else {
        TOOLCYAN
    }
}

pub(super) fn italic(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::ITALIC)
}

// ──────────────────────────────────────────────────────────────────── keybar

/// One keybar entry: the key, what it does, and whether it is the urgent one right now.
type Bind = (&'static str, &'static str, bool);

/// The contextual keybar, derived from the same semantics `keys.rs` implements.
pub(super) fn keybar(app: &BoardApp) -> Vec<Bind> {
    match app.focus {
        Focus::Help => vec![("any key", "closes", false)],
        Focus::Confirm => vec![("Enter", "yes", true), ("Esc", "no", false)],
        Focus::Composer => {
            let mut v = vec![("Enter", "send", true), ("Esc", "cancel", false)];
            if matches!(
                app.composer.as_ref().map(|c| &c.mode),
                Some(ComposerMode::NewSession { .. })
            ) {
                v.push(("Tab", "worktree on/off", false));
            }
            v
        }
        Focus::Filter => vec![
            ("type", "to filter", false),
            ("Enter", "keep", false),
            ("Esc", "clear", false),
        ],
        Focus::Detail => vec![
            ("[ ]", "section", false),
            ("↑↓", "scroll", false),
            ("F", "follow", false),
            ("Esc", "back", false),
            ("a", "attach", false),
            ("p", "prompt", false),
            ("i", "interrupt", false),
        ],
        Focus::Board => board_keybar(app),
    }
}

fn board_keybar(app: &BoardApp) -> Vec<Bind> {
    let mut v: Vec<Bind> = Vec::new();
    let snap = app.selected_snapshot();
    if snap.is_some_and(|s| s.permission_prompt.is_some()) {
        v.push(("y", "allow", true));
        v.push(("n", "deny", true));
    } else if snap.is_some_and(|s| s.question.is_some()) {
        v.push(("1-n", "pick", true));
        v.push(("e", "type", true));
    }
    if app.selected_card().is_some_and(|c| c.past) {
        v.push(("r", "resume", false));
    }
    v.extend_from_slice(&[
        ("↑↓", "move", false),
        ("←→", "column", false),
        ("Enter", "open", false),
        ("a", "attach", false),
        ("p", "prompt", false),
        ("i", "interrupt", false),
        ("m", "model", false),
        ("x", "archive", false),
        ("N", "new", false),
        ("f", "project", false),
        ("/", "filter", false),
        ("?", "help", false),
        ("q", "quit", false),
    ]);
    v
}

pub(super) fn keybar_line(binds: &[Bind], width: usize) -> Line<'static> {
    let mut fit = Fit::new(width);
    for (i, (key, desc, urgent)) in binds.iter().enumerate() {
        if i > 0 {
            fit.add(" · ", Style::default().fg(VERY_DIM));
        }
        let key_style = if *urgent {
            Style::default().fg(ERRRED).bold()
        } else {
            Style::default().fg(ACCENT).bold()
        };
        fit.add(*key, key_style);
        fit.add(" ", Style::default());
        fit.add(*desc, Style::default().fg(DIM));
        if fit.left() == 0 {
            break;
        }
    }
    fit.line()
}

/// The newest toasts (up to two, newest last), fading over their last ten ticks.
pub(super) fn toast_items(app: &BoardApp) -> Vec<(String, Style)> {
    let recent: Vec<_> = app.toasts.iter().rev().take(2).collect();
    recent
        .into_iter()
        .rev()
        .map(|t| {
            let life: u64 = if t.level == ToastLevel::Error { 80 } else { 40 };
            let age = app.tick.saturating_sub(t.born);
            let fading = life.saturating_sub(age) <= 10;
            let color = if fading {
                DIM
            } else {
                match t.level {
                    ToastLevel::Ok => OKGREEN,
                    ToastLevel::Info => ACCENT,
                    ToastLevel::Error => ERRRED,
                }
            };
            (
                format!("  {}", clip_cells(&t.text, 48)),
                Style::default().fg(color),
            )
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────── overlays

/// The one-line composer: label in the frame, the text with a real cursor, the send/cancel hint.
pub(super) fn draw_composer(app: &BoardApp, frame: &mut Frame, area: Rect) {
    let Some(c) = app.composer.as_ref() else {
        return;
    };
    if area.width < 12 || area.height < 5 {
        return;
    }
    let rect = surface::modal_area(area, area.width.saturating_sub(8).clamp(24, 96), 5);
    let hint = match &c.mode {
        ComposerMode::NewSession { .. } => "Enter send · Esc cancel · Tab: worktree on/off",
        _ => "Enter send · Esc cancel",
    };
    let inner = surface::render_panel(
        frame,
        rect,
        surface::title(clip_cells(&c.label(), 60), SurfaceTone::Accent),
        Some(surface::hint(hint)),
        SurfaceTone::Accent,
    );
    if inner.width < 2 || inner.height == 0 {
        return;
    }
    let width = inner.width as usize;
    let chars: Vec<char> = c.text.chars().collect();
    let cursor = c.cursor.min(chars.len());
    let start = (cursor + 1).saturating_sub(width);
    let before: String = chars[start..cursor].iter().collect();
    let (at, after) = match chars.get(cursor) {
        Some(ch) => (
            ch.to_string(),
            chars[(cursor + 1).min(chars.len())..].iter().collect(),
        ),
        None => (" ".to_string(), String::new()),
    };
    let mut fit = Fit::new(width);
    fit.add(before, Style::default().fg(TEXT));
    fit.add(at, Style::default().fg(TEXT).bg(SELECT_BG));
    fit.add(after, Style::default().fg(TEXT));
    frame.render_widget(Paragraph::new(fit.line()), inner);
}

/// The archive-style confirmation: what it will do, then yes/no.
pub(super) fn draw_confirm(app: &BoardApp, frame: &mut Frame, area: Rect) {
    let Some(c) = app.confirm.as_ref() else {
        return;
    };
    if area.width < 16 || area.height < 6 {
        return;
    }
    let width = area.width.saturating_sub(10).clamp(24, 68);
    let body = wrap(&c.body, width.saturating_sub(2) as usize);
    let height = (body.len() as u16 + 3)
        .min(area.height.saturating_sub(2))
        .max(4);
    let rect = surface::modal_area(area, width, height);
    let inner = surface::render_panel(
        frame,
        rect,
        surface::title(clip_cells(&c.title, 60), SurfaceTone::Warning),
        Some(surface::hint("Enter yes · Esc no")),
        SurfaceTone::Warning,
    );
    let lines: Vec<Line> = body
        .into_iter()
        .map(|l| Line::from(Span::styled(l, Style::default().fg(TEXT))))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Every key the board answers to, in two columns, straight out of [`HELP`].
pub(super) fn draw_help(frame: &mut Frame, area: Rect) {
    if area.width < 20 || area.height < 6 {
        return;
    }
    // Two columns only when each gets room for the longest description; otherwise one column,
    // clipped to the height rather than clipping the words.
    let width = area.width.saturating_sub(6).clamp(24, 140);
    let two_columns = width >= 122;
    let rows = if two_columns {
        HELP.len().div_ceil(2)
    } else {
        HELP.len()
    };
    let height = (rows as u16 + 2).min(area.height.saturating_sub(2)).max(4);
    let rect = surface::modal_area(area, width, height);
    let inner = surface::render_panel(
        frame,
        rect,
        surface::title("forge board — keys", SurfaceTone::Brand),
        Some(surface::hint("any key closes")),
        SurfaceTone::Brand,
    );
    let half = inner.width as usize / 2;
    let mut lines: Vec<Line> = Vec::new();
    for i in 0..rows {
        let mut fit = Fit::new(inner.width as usize);
        if two_columns {
            help_cell(&mut fit, HELP.get(i), half);
            help_cell(&mut fit, HELP.get(i + rows), inner.width as usize - half);
        } else {
            help_cell(&mut fit, HELP.get(i), inner.width as usize);
        }
        lines.push(fit.line());
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn help_cell(fit: &mut Fit, entry: Option<&(&'static str, &'static str)>, width: usize) {
    let Some((key, desc)) = entry else {
        return;
    };
    let before = fit.used();
    fit.add(format!("{key:>11}  "), Style::default().fg(ACCENT).bold());
    fit.add(*desc, Style::default().fg(DIM));
    let used = fit.used() - before;
    if used < width {
        fit.add(" ".repeat(width - used), Style::default());
    }
}

/// A thin rule the width of `area`, used under column headers and the board header.
pub(super) fn rule(frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(area.width as usize),
            Style::default().fg(SEPARATOR),
        ))),
        Rect { height: 1, ..area },
    );
}

/// A rounded, tone-coloured frame with an optional title, returning the content rectangle.
pub(super) fn framed(
    frame: &mut Frame,
    area: Rect,
    title: Option<Line<'static>>,
    color: Color,
    bold: bool,
) -> Rect {
    let style = if bold {
        Style::default().fg(color).bold()
    } else {
        Style::default().fg(color)
    };
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style);
    if let Some(t) = title {
        block = block.title_top(t);
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

// ───────────────────────────────────────────────────── pane rows and buttons

/// One `label   value` row of the pane's key/value grid.
pub(super) fn kv(label: &str, value: String, style: Style, width: usize) -> Line<'static> {
    let mut fit = Fit::new(width);
    fit.add(format!("{label:>13}  "), Style::default().fg(VERY_DIM));
    fit.add(value, style);
    fit.line()
}

pub(super) fn heading(text: &str, width: usize) -> Line<'static> {
    let mut fit = Fit::new(width);
    fit.add(format!("  {text}"), Style::default().fg(ORANGE).bold());
    fit.line()
}

pub(super) fn bullet(glyph: &str, text: &str, color: Color, width: usize) -> Line<'static> {
    let mut fit = Fit::new(width);
    fit.add(format!("    {glyph} "), Style::default().fg(color));
    fit.add(
        clip_cells(text, width.saturating_sub(6)),
        Style::default().fg(color),
    );
    fit.line()
}

pub(super) fn diff_line(
    path: &str,
    kind: &str,
    adds: usize,
    dels: usize,
    binary: bool,
    width: usize,
) -> Line<'static> {
    let mut fit = Fit::new(width);
    fit.add("    ", Style::default());
    fit.add(format!("+{adds} "), Style::default().fg(OKGREEN));
    fit.add(format!("−{dels}  "), Style::default().fg(ERRRED));
    fit.add(path.to_string(), Style::default().fg(TEXT));
    fit.add(format!("  {kind}"), Style::default().fg(VERY_DIM));
    if binary {
        fit.add("  binary", Style::default().fg(WARNYEL));
    }
    fit.line()
}

pub(super) fn git_line(label: &str, f: &GitFile, width: usize) -> Line<'static> {
    let mark = f
        .status
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    let mut fit = Fit::new(width);
    fit.add(
        format!("    {mark} "),
        Style::default().fg(match label {
            "staged" => OKGREEN,
            "unstaged" => WARNYEL,
            _ => DIM,
        }),
    );
    fit.add(f.path.clone(), Style::default().fg(TEXT));
    if f.adds > 0 || f.dels > 0 {
        fit.add(format!("  +{}", f.adds), Style::default().fg(OKGREEN));
        fit.add(format!(" −{}", f.dels), Style::default().fg(ERRRED));
    }
    fit.line()
}

pub(super) fn empty(text: &str, width: usize) -> Line<'static> {
    Line::from(Span::styled(
        clip_cells(text, width),
        Style::default().fg(VERY_DIM),
    ))
    .centered()
}

/// The fan-out under one session: running agents first, each with its task, model and spend.
pub(super) fn subagent_lines(agents: &[Subagent], width: usize) -> Vec<Line<'static>> {
    let mut agents: Vec<&Subagent> = agents.iter().collect();
    agents.sort_by_key(|s| s.done);
    agents
        .iter()
        .take(12)
        .map(|s| {
            let (glyph, color) = if !s.done {
                ("⑂", TOOLCYAN)
            } else if s.ok {
                ("✓", OKGREEN)
            } else {
                ("✗", ERRRED)
            };
            let mut fit = Fit::new(width);
            fit.add(format!("    {glyph} "), Style::default().fg(color));
            fit.add(s.agent.clone(), Style::default().fg(TEXT));
            fit.add(format!(" · {}", s.task), Style::default().fg(DIM));
            if let Some(m) = &s.model {
                fit.add(
                    format!(" · {}", model_short(m)),
                    Style::default().fg(VERY_DIM),
                );
            }
            if !s.last.is_empty() {
                fit.add(format!(" · {}", s.last), italic(VERY_DIM));
            }
            fit.right(fmt_cost(s.cost), Style::default().fg(DIM))
        })
        .collect()
}

/// One button of the pane's action bar: key, label, what it does, whether it applies now.
pub(super) type Chip = (&'static str, &'static str, Button, bool);

/// The buttons that can actually resolve this card. The ones that cannot are still drawn, dimmed,
/// so the bar never moves under the cursor.
pub(super) fn action_chips(card: &Card) -> Vec<Chip> {
    if card.past {
        return vec![
            ("r", "resume", Button::Resume, true),
            ("c", "copy", Button::Copy, true),
        ];
    }
    let live = !card.read_only;
    vec![
        ("a", "attach", Button::Attach, live),
        ("p", "prompt", Button::Prompt, live),
        ("s", "steer", Button::Steer, live && card.busy),
        ("i", "interrupt", Button::Interrupt, card.busy),
        ("m", "model", Button::Model, live),
        ("M", "mode", Button::Mode, live && !card.terminal),
        ("x", "archive", Button::Archive, !card.terminal),
        ("c", "copy", Button::Copy, true),
    ]
}

fn chip_width(c: &Chip) -> u16 {
    (cells(c.0) + cells(c.1) + 4) as u16
}

pub(super) fn chip_rows(chips: &[Chip], width: u16) -> u16 {
    let total: u16 = chips.iter().map(chip_width).sum();
    if total <= width {
        1
    } else {
        2
    }
}

pub(super) fn draw_actions(
    frame: &mut Frame,
    area: Rect,
    chips: &[Chip],
    hits: &mut Vec<(Rect, Hit)>,
) {
    let mut y = area.y;
    let mut row = Row::at(area.x, y);
    let mut pending: Vec<(Rect, Hit)> = Vec::new();
    for chip in chips {
        if row.width() + chip_width(chip) > area.width && row.width() > 0 {
            row.render(frame, area);
            hits.append(&mut pending);
            y += 1;
            if y >= area.y + area.height {
                return;
            }
            row = Row::at(area.x, y);
        }
        let style = if chip.3 {
            Style::default().fg(ACCENT).bold()
        } else {
            Style::default().fg(VERY_DIM)
        };
        let start = row.add(format!(" {} ", chip.0), style);
        let end = row.add(
            format!("{}  ", chip.1),
            Style::default().fg(if chip.3 { TEXT } else { VERY_DIM }),
        );
        pending.push((
            Rect {
                x: start.x,
                y: start.y,
                width: start.width + end.width,
                height: 1,
            },
            Hit::Button(chip.2),
        ));
    }
    row.render(frame, area);
    for (r, h) in pending {
        push_hit(hits, r, area, h);
    }
}
