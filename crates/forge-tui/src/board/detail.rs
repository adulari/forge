//! The detail pane: everything about one session that the card could not say, plus the buttons
//! that resolve it. Five sections — Overview, Live, Tasks, Changes, Tools — over an attention
//! block that is drawn whenever the session is blocked on a human.
//!
//! Rendering is read-only over [`BoardApp`] apart from one write-back: while the live tail is
//! following, the pane reports the offset it settled on so a later scroll starts from there.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::surface::{
    self, ACCENT, DIM, ERRRED, OKGREEN, SELECT_BG, TEXT, TOOLCYAN, USER, VERY_DIM, WARNYEL,
};

use super::model::{fmt_age, fmt_cost, model_short, stop_reason_words, Card, SignalLevel};
use super::state::{BoardApp, Button, DetailTab, Hit};
use super::widgets as w;
use super::wire::LiveSnapshot;

pub fn draw_detail(app: &mut BoardApp, frame: &mut Frame, area: Rect) {
    let mut hits: Vec<(Rect, Hit)> = Vec::new();
    let scroll = render_pane(app, frame, area, &mut hits);
    app.hits.extend(hits);
    if let Some(s) = scroll {
        app.detail_scroll = s;
    }
}

fn render_pane(
    app: &BoardApp,
    frame: &mut Frame,
    area: Rect,
    hits: &mut Vec<(Rect, Hit)>,
) -> Option<usize> {
    if area.width < 12 || area.height < 5 {
        return None;
    }
    let card = app.selected_card()?;
    let snap = app.selected_snapshot();
    let tone = w::health_tone(card.health);
    let (glyph, _) = w::health_glyph(card, app.tick);
    let head = format!(
        "{glyph} {}  {}",
        w::clip_cells(
            &card.display_title(),
            area.width.saturating_sub(24) as usize
        ),
        card.short_id()
    );
    let inner = surface::render_panel(frame, area, surface::title(head, tone), None, tone);

    // The wheel handler locates the pane by this hit, so it is recorded whenever the pane draws.
    let close = Rect {
        x: area.x + area.width.saturating_sub(4),
        y: area.y,
        width: 3,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " ✕ ",
            Style::default().fg(DIM).bold(),
        ))),
        close,
    );
    w::push_hit(hits, close, area, Hit::CloseDetail);

    if inner.width < 8 || inner.height < 2 {
        return None;
    }
    draw_tabs(app, frame, inner, card, snap, hits);
    if inner.height < 4 {
        return None;
    }
    w::rule(
        frame,
        Rect {
            y: inner.y + 1,
            height: 1,
            ..inner
        },
    );

    let width = inner.width as usize;
    let mut top = inner.y + 2;
    let mut room = inner.height - 2;

    let plan = snap
        .filter(|s| s.waiting())
        .map(|s| attention_plan(s, width));
    if let Some(plan) = &plan {
        let h = ((plan.len() + 2) as u16).min(room.saturating_sub(1)).max(3);
        draw_attention(
            frame,
            Rect {
                y: top,
                height: h,
                ..inner
            },
            plan,
            hits,
        );
        top += h;
        room = room.saturating_sub(h);
    }

    let chips = w::action_chips(card);
    let act_h = w::chip_rows(&chips, inner.width);
    let body_h = room.saturating_sub(act_h);
    let mut scroll_out = None;
    if body_h > 0 {
        let body = Rect {
            y: top,
            height: body_h,
            ..inner
        };
        scroll_out = draw_body(app, frame, body, card, snap, hits);
    }
    if act_h > 0 {
        w::draw_actions(
            frame,
            Rect {
                y: top + body_h,
                height: act_h,
                ..inner
            },
            &chips,
            hits,
        );
    }
    scroll_out
}

// ───────────────────────────────────────────────────────────────────── tabs

fn draw_tabs(
    app: &BoardApp,
    frame: &mut Frame,
    inner: Rect,
    card: &Card,
    snap: Option<&LiveSnapshot>,
    hits: &mut Vec<(Rect, Hit)>,
) {
    let done = snap.map_or(0, |s| s.tasks.iter().filter(|t| t.status == "done").count());
    let total = snap.map_or(0, |s| s.tasks.len());
    let changes = snap
        .and_then(|s| s.diff.as_ref())
        .map_or(0, |d| d.files.len())
        + app.git.get(&card.id).map_or(0, |g| g.dirty_count());
    let tools = tool_calls(app, card).len();

    let mut row = w::Row::at(inner.x, inner.y);
    for (i, tab) in DetailTab::ALL.iter().enumerate() {
        if i > 0 {
            row.sep();
        }
        let count = match tab {
            DetailTab::Tasks if total > 0 => format!(" {done}/{total}"),
            DetailTab::Changes if changes > 0 => format!(" {changes}"),
            DetailTab::Tools if tools > 0 => format!(" {tools}"),
            _ => String::new(),
        };
        let style = if *tab == app.detail_tab {
            Style::default().fg(ACCENT).bg(SELECT_BG).bold()
        } else {
            Style::default().fg(DIM)
        };
        let r = row.add(format!(" {}{count} ", tab.label()), style);
        w::push_hit(hits, r, inner, Hit::DetailTab(*tab));
    }
    row.render(frame, inner);

    if app.detail_tab == DetailTab::Tail {
        let text = if app.show_tools {
            " t tools on "
        } else {
            " t tools off "
        };
        let mut right = w::Row::at(
            inner.x + inner.width.saturating_sub(w::cells(text) as u16),
            inner.y,
        );
        let r = right.add(
            text,
            Style::default().fg(if app.show_tools { TOOLCYAN } else { VERY_DIM }),
        );
        w::push_hit(hits, r, inner, Hit::Button(Button::ToggleTools));
        right.render(frame, inner);
    }
}

// ──────────────────────────────────────────────────────────────── attention

/// One row of the "this session is blocked on you" box.
enum Att {
    Text(String, Color),
    Buttons(Vec<(String, Button, Color)>),
}

fn attention_plan(snap: &LiveSnapshot, width: usize) -> Vec<Att> {
    let w_in = width.saturating_sub(4).max(8);
    let mut out = Vec::new();
    if let Some(prompt) = &snap.permission_prompt {
        for l in w::wrap(prompt, w_in).into_iter().take(8) {
            out.push(Att::Text(l, TEXT));
        }
        out.push(Att::Buttons(vec![
            ("[ y  allow ]".into(), Button::Allow, OKGREEN),
            ("[ n  deny ]".into(), Button::Deny, ERRRED),
        ]));
        return out;
    }
    if let Some(q) = &snap.question {
        for l in w::wrap(q, w_in).into_iter().take(4) {
            out.push(Att::Text(l, TEXT));
        }
        for (i, opt) in snap.question_options.iter().enumerate().take(6) {
            let desc = if opt.description.is_empty() {
                String::new()
            } else {
                format!(" — {}", opt.description)
            };
            out.push(Att::Buttons(vec![(
                w::clip_cells(&format!("[ {} ] {}{desc}", i + 1, opt.label), w_in),
                Button::Answer(i + 1),
                ACCENT,
            )]));
        }
        out.push(Att::Text("[ e ] type an answer".into(), DIM));
    }
    out
}

fn draw_attention(frame: &mut Frame, area: Rect, plan: &[Att], hits: &mut Vec<(Rect, Hit)>) {
    let inner = w::framed(
        frame,
        area,
        Some(Line::from(Span::styled(
            " needs you ",
            Style::default().fg(ERRRED).bold(),
        ))),
        ERRRED,
        true,
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    for (i, item) in plan.iter().take(inner.height as usize).enumerate() {
        let y = inner.y + i as u16;
        match item {
            Att::Text(t, color) => {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        w::clip_cells(t, inner.width as usize),
                        Style::default().fg(*color),
                    ))),
                    Rect {
                        y,
                        height: 1,
                        ..inner
                    },
                );
            }
            Att::Buttons(buttons) => {
                let mut row = w::Row::at(inner.x, y);
                for (label, button, color) in buttons {
                    let r = row.add(
                        format!("{label}  "),
                        Style::default().fg(*color).bg(SELECT_BG).bold(),
                    );
                    w::push_hit(hits, r, inner, Hit::Button(*button));
                }
                row.render(frame, inner);
            }
        }
    }
}

// ───────────────────────────────────────────────────────────────── body tabs

fn draw_body(
    app: &BoardApp,
    frame: &mut Frame,
    area: Rect,
    card: &Card,
    snap: Option<&LiveSnapshot>,
    hits: &mut Vec<(Rect, Hit)>,
) -> Option<usize> {
    let width = area.width as usize;
    let lines = match app.detail_tab {
        DetailTab::Overview => overview(app, card, snap, width),
        DetailTab::Tail => tail(app, snap, width),
        DetailTab::Tasks => tasks(snap, width),
        DetailTab::Changes => changes(app, card, snap, width),
        DetailTab::Tools => tools(app, card, width),
    };
    let h = area.height as usize;
    let max = lines.len().saturating_sub(h);
    let following = app.detail_tab == DetailTab::Tail && app.tail_follow;
    let off = if following {
        max
    } else {
        app.detail_scroll.min(max)
    };
    let view: Vec<Line> = lines.into_iter().skip(off).take(h).collect();
    frame.render_widget(Paragraph::new(view), area);

    if app.detail_tab == DetailTab::Tail && !app.tail_follow && max > 0 {
        let chip = " ↓ following paused — F ";
        let mut row = w::Row::at(
            area.x + area.width.saturating_sub(w::cells(chip) as u16),
            area.y + area.height - 1,
        );
        row.add(chip, Style::default().fg(WARNYEL).bg(SELECT_BG));
        row.render(frame, area);
    }
    let _ = hits;
    Some(off)
}

fn overview(
    app: &BoardApp,
    card: &Card,
    snap: Option<&LiveSnapshot>,
    width: usize,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line> = Vec::new();
    let tier = card
        .tier
        .as_deref()
        .map(|t| format!("  ({t})"))
        .unwrap_or_default();
    let effort = snap
        .map(|s| s.effort.clone())
        .filter(|e| !e.is_empty())
        .map(|e| format!("  effort {e}"))
        .unwrap_or_default();
    out.push(w::kv(
        "model",
        format!("{}{tier}{effort}", model_short(&card.model)),
        Style::default().fg(TEXT),
        width,
    ));
    if let Some(s) = snap {
        let temper = if s.temper.is_empty() {
            String::new()
        } else {
            format!("  ·  {}", s.temper)
        };
        let mode = if s.permission_mode.is_empty() {
            "default".to_string()
        } else {
            s.permission_mode.clone()
        };
        out.push(w::kv(
            "mode",
            format!("{mode}{temper}"),
            Style::default().fg(TEXT),
            width,
        ));
    }
    out.push(w::kv(
        "cwd",
        card.cwd.clone(),
        Style::default().fg(DIM),
        width,
    ));
    if let Some(wt) = &card.worktree {
        out.push(w::kv(
            "worktree",
            format!("⎇ {wt}"),
            Style::default().fg(ACCENT),
            width,
        ));
    }
    if let Some(git) = app.git.get(&card.id) {
        let base = git
            .base_branch
            .as_deref()
            .map(|b| format!("  from {b}"))
            .unwrap_or_default();
        out.push(w::kv(
            "branch",
            format!("{}{base}", git.branch),
            Style::default().fg(TEXT),
            width,
        ));
    }
    out.push(w::kv(
        "created",
        format!("{} ago", fmt_age(app.now.saturating_sub(card.created_at))),
        Style::default().fg(DIM),
        width,
    ));
    out.push(w::kv(
        "last activity",
        format!(
            "{} ago",
            fmt_age(app.now.saturating_sub(card.last_activity))
        ),
        Style::default().fg(DIM),
        width,
    ));
    out.push(w::kv(
        "cost",
        fmt_cost(card.cost_usd),
        Style::default().fg(TEXT),
        width,
    ));
    if let Some(pct) = card.context_pct {
        let tokens = snap.map_or(0, |s| s.context_tokens);
        let limit = snap.and_then(|s| s.context_limit).unwrap_or(0);
        let mut fit = w::Fit::new(width);
        fit.add("      context  ", Style::default().fg(VERY_DIM));
        fit.extend(w::gauge(f64::from(pct) / 100.0, 12, w::context_color(pct)));
        fit.add(
            format!("  {pct}%  ({tokens} / {limit})"),
            Style::default().fg(w::context_color(pct)),
        );
        out.push(fit.line());
    }
    if let Some(s) = snap {
        if s.last_turn_outcome.is_some() || s.last_stop_reason.is_some() {
            let ok = s.last_turn_outcome.as_deref() != Some("failed");
            out.push(w::kv(
                "last turn",
                stop_reason_words(s.last_stop_reason.as_deref()),
                Style::default().fg(if ok { OKGREEN } else { WARNYEL }),
                width,
            ));
        }
        if !s.queued.is_empty() {
            out.push(Line::from(""));
            out.push(w::heading(&format!("queued · {}", s.queued.len()), width));
            for q in s.queued.iter().take(5) {
                out.push(w::bullet("⇥", q, WARNYEL, width));
            }
        }
    }
    if !card.signals.is_empty() {
        out.push(Line::from(""));
        out.push(w::heading("signals", width));
        for sig in &card.signals {
            let color = match sig.level {
                SignalLevel::Danger => ERRRED,
                SignalLevel::Warn => WARNYEL,
                SignalLevel::Info => DIM,
            };
            out.push(w::bullet("•", &sig.text, color, width));
        }
    }
    if !card.subagents.is_empty() {
        out.push(Line::from(""));
        out.push(w::heading("subagents", width));
        out.extend(w::subagent_lines(&card.subagents, width));
    }
    let in_progress: Vec<&super::wire::Task> = snap
        .map(|s| {
            s.tasks
                .iter()
                .filter(|t| t.status == "in_progress")
                .collect()
        })
        .unwrap_or_default();
    if !in_progress.is_empty() {
        out.push(Line::from(""));
        out.push(w::heading("in progress", width));
        for t in in_progress {
            out.push(w::bullet("▸", &t.title, ACCENT, width));
        }
    }
    if let Some(s) = snap {
        let recent: Vec<String> = s
            .rows()
            .iter()
            .rev()
            .filter(|r| r.kind != "system" && !r.text.trim().is_empty())
            .take(3)
            .map(|r| r.text.clone())
            .collect();
        if !recent.is_empty() {
            out.push(Line::from(""));
            out.push(w::heading("latest", width));
            for t in recent.into_iter().rev() {
                out.push(w::bullet("·", &t, DIM, width));
            }
        }
    }
    out
}

fn tail(app: &BoardApp, snap: Option<&LiveSnapshot>, width: usize) -> Vec<Line<'static>> {
    let Some(s) = snap else {
        return vec![w::empty(
            "no live transcript — this session is not streaming",
            width,
        )];
    };
    let mut out: Vec<Line> = Vec::new();
    for r in s.rows() {
        let (prefix, style) = match r.kind.as_str() {
            "user" => ("› ".to_string(), Style::default().fg(USER)),
            "tool" => {
                if !app.show_tools {
                    continue;
                }
                let name = r.tool.clone().unwrap_or_else(|| "tool".into());
                (format!("⚙ {name}  "), Style::default().fg(TOOLCYAN))
            }
            "system" => {
                let t = r.text.to_ascii_lowercase();
                let warn = r.text.contains('⚠')
                    || t.contains("warning")
                    || t.contains("benched")
                    || t.contains("stall");
                (
                    "  ".to_string(),
                    w::italic(if warn { WARNYEL } else { DIM }),
                )
            }
            _ => ("  ".to_string(), Style::default().fg(TEXT)),
        };
        let body = if r.kind == "tool" {
            match r.meta.as_deref() {
                Some("ok") => format!("{}  ok", r.text),
                Some("failed") => format!("{}  failed", r.text),
                _ => r.text.clone(),
            }
        } else {
            r.text.clone()
        };
        let meta_color = match r.meta.as_deref() {
            Some("failed") => Some(ERRRED),
            Some("ok") => Some(OKGREEN),
            _ => None,
        };
        for (i, l) in w::wrap(&body, width.saturating_sub(w::cells(&prefix)))
            .into_iter()
            .enumerate()
        {
            let mut fit = w::Fit::new(width);
            if i == 0 {
                fit.add(prefix.clone(), style);
            } else {
                fit.add(" ".repeat(w::cells(&prefix)), Style::default());
            }
            fit.add(l, meta_color.map_or(style, |c| Style::default().fg(c)));
            out.push(fit.line());
        }
    }
    if !s.streaming.trim().is_empty() {
        for (i, l) in w::wrap(&s.streaming, width.saturating_sub(2))
            .into_iter()
            .enumerate()
        {
            let mut fit = w::Fit::new(width);
            fit.add(
                if i == 0 {
                    format!("{} ", w::spinner(app.tick))
                } else {
                    "  ".into()
                },
                Style::default().fg(TOOLCYAN),
            );
            fit.add(l, w::italic(TEXT));
            out.push(fit.line());
        }
    }
    if !s.notes.is_empty() {
        out.push(Line::from(""));
        for n in &s.notes {
            out.push(w::bullet("·", n, VERY_DIM, width));
        }
    }
    if out.is_empty() {
        out.push(w::empty("nothing has streamed yet", width));
    }
    out
}

fn tasks(snap: Option<&LiveSnapshot>, width: usize) -> Vec<Line<'static>> {
    let Some(s) = snap else {
        return vec![w::empty("no task list for this session", width)];
    };
    let mut out: Vec<Line> = Vec::new();
    if s.tasks.is_empty() {
        out.push(w::empty("no tasks yet", width));
    }
    for t in &s.tasks {
        let (glyph, color, bold) = match t.status.as_str() {
            "done" => ("✓", OKGREEN, false),
            "in_progress" => ("▸", ACCENT, true),
            _ => ("○", DIM, false),
        };
        let mut fit = w::Fit::new(width);
        fit.add(format!("    {glyph} "), Style::default().fg(color));
        let style = if bold {
            Style::default().fg(TEXT).bold()
        } else {
            Style::default().fg(if t.status == "done" { DIM } else { TEXT })
        };
        fit.add(t.title.clone(), style);
        if let Some(a) = &t.assignee {
            fit.add(format!("  [{a}]"), Style::default().fg(VERY_DIM));
        }
        out.push(fit.line());
    }
    if !s.subagents.is_empty() {
        out.push(Line::from(""));
        out.push(w::heading("subagents", width));
        out.extend(w::subagent_lines(&s.subagents, width));
    }
    if let Some(plan) = &s.plan {
        out.push(Line::from(""));
        out.push(w::heading(&format!("plan · {}", plan.title), width));
        for step in &plan.steps {
            let color = match step.status.as_str() {
                "done" => OKGREEN,
                "in_progress" => ACCENT,
                _ => DIM,
            };
            out.push(w::bullet(
                "·",
                &format!("{} — {}", step.title, step.status),
                color,
                width,
            ));
        }
    }
    if let Some(wf) = &s.workflow {
        out.push(Line::from(""));
        let name = wf.name.clone().unwrap_or_else(|| "workflow".into());
        out.push(w::heading(
            &format!("{name}{}", if wf.active { " · running" } else { "" }),
            width,
        ));
        for p in wf.phases.iter().take(8) {
            out.push(w::bullet("▸", p, ACCENT, width));
        }
        for l in wf.logs.iter().rev().take(5).rev() {
            out.push(w::bullet("·", l, DIM, width));
        }
        if let Some(sum) = &wf.summary {
            let color = if wf.finished_ok == Some(false) {
                ERRRED
            } else {
                OKGREEN
            };
            out.push(w::bullet("✓", sum, color, width));
        }
    }
    out
}

fn changes(
    app: &BoardApp,
    card: &Card,
    snap: Option<&LiveSnapshot>,
    width: usize,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line> = Vec::new();
    if let Some(diff) = snap.and_then(|s| s.diff.as_ref()) {
        out.push(w::heading("proposed edits", width));
        if diff.pending {
            out.push(w::bullet(
                "!",
                "proposed — awaiting permission",
                ERRRED,
                width,
            ));
        }
        for f in diff.files.iter().take(30) {
            out.push(w::diff_line(
                &f.path, &f.kind, f.adds, f.dels, f.binary, width,
            ));
        }
        if diff.skipped_files > 0 {
            out.push(w::bullet(
                "·",
                &format!("+{} more files", diff.skipped_files),
                DIM,
                width,
            ));
        }
        out.push(Line::from(""));
    }
    match app.git.get(&card.id) {
        None => out.push(w::empty("no git info yet", width)),
        Some(git) => {
            let base = git
                .base_branch
                .as_deref()
                .map(|b| format!("  from {b}"))
                .unwrap_or_default();
            out.push(w::heading(&format!("⎇ {}{base}", git.branch), width));
            out.push(w::kv(
                "working tree",
                format!(
                    "staged {} · unstaged {} · untracked {}",
                    git.staged.len(),
                    git.unstaged.len(),
                    git.untracked.len()
                ),
                Style::default().fg(DIM),
                width,
            ));
            if git.dirty_count() == 0 {
                out.push(w::bullet("✓", "working tree clean", OKGREEN, width));
            }
            for (label, files) in [
                ("staged", &git.staged),
                ("unstaged", &git.unstaged),
                ("untracked", &git.untracked),
            ] {
                for f in files.iter().take(20) {
                    out.push(w::git_line(label, f, width));
                }
            }
            if git.truncated > 0 {
                out.push(w::bullet(
                    "·",
                    &format!("+{} more files", git.truncated),
                    DIM,
                    width,
                ));
            }
        }
    }
    out
}

/// Tool calls oldest-first (the wire serves history newest-first), each with its result.
fn tool_calls<'a>(app: &'a BoardApp, card: &Card) -> Vec<(&'a str, &'a str, Option<&'a str>)> {
    let Some(rows) = app.history.get(&card.id) else {
        return Vec::new();
    };
    let ordered: Vec<&super::wire::HistoryRow> = rows.iter().rev().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < ordered.len() {
        let r = ordered[i];
        if r.kind != "tool" || r.tool_phase.as_deref() == Some("result") {
            i += 1;
            continue;
        }
        let name = r.tool.as_deref().unwrap_or("tool");
        let result = ordered
            .get(i + 1)
            .filter(|n| n.kind == "tool" && n.tool_phase.as_deref() == Some("result"))
            .map(|n| n.content.as_str());
        out.push((name, r.content.as_str(), result));
        i += if result.is_some() { 2 } else { 1 };
    }
    out
}

fn tools(app: &BoardApp, card: &Card, width: usize) -> Vec<Line<'static>> {
    let calls = tool_calls(app, card);
    if calls.is_empty() {
        return vec![w::empty("no tool calls yet", width)];
    }
    let mut out: Vec<Line> = Vec::new();
    for (name, call, result) in calls.iter().rev().take(60).rev() {
        let mut fit = w::Fit::new(width);
        fit.add(
            format!("  ⚙ {name}  "),
            Style::default().fg(TOOLCYAN).bold(),
        );
        fit.add(call.replace('\n', " "), Style::default().fg(TEXT));
        out.push(fit.line());
        if let Some(res) = result {
            let first = res.lines().next().unwrap_or("").trim().to_string();
            let low = first.to_ascii_lowercase();
            let bad = low.starts_with("error") || low.starts_with("failed");
            let mut fit = w::Fit::new(width);
            fit.add(
                "      ↳ ",
                Style::default().fg(if bad { ERRRED } else { VERY_DIM }),
            );
            fit.add(first, Style::default().fg(if bad { ERRRED } else { DIM }));
            out.push(fit.line());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use crate::board::state::{ComposerMode, ConfirmKind, Focus};
    use crate::board::wire::{DiffCard, DiffFile, GitInfo, HistoryRow, QOption};
    use crate::board::{BoardEvent, Confirm};

    const NOW: i64 = 1_700_000_000;

    fn screen(app: &mut BoardApp, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal.draw(|f| app.draw(f)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn opened(snap: LiveSnapshot) -> BoardApp {
        let mut app = BoardApp::new(None, NOW);
        app.apply(BoardEvent::Fleet(vec![super::super::wire::FleetRow {
            id: "aaaa1111".into(),
            title: "port the mesh router".into(),
            cwd: "/home/dev/forge".into(),
            model: "anthropic::claude-opus".into(),
            last_activity: NOW - 60,
            created_at: NOW - 600,
            ..Default::default()
        }]));
        app.apply(BoardEvent::Snapshot("aaaa1111".into(), snap));
        app.select("aaaa1111");
        app.detail_open = true;
        app.focus = Focus::Detail;
        app
    }

    #[test]
    fn the_pane_shows_its_tab_strip_and_a_close_button() {
        let mut app = opened(LiveSnapshot {
            model: "anthropic::claude-opus".into(),
            tier: Some("complex".into()),
            ..Default::default()
        });
        let s = screen(&mut app, 160, 45);
        assert!(s.contains("Overview"), "{s}");
        assert!(s.contains("Live"), "{s}");
        assert!(s.contains("Changes"), "{s}");
        assert!(s.contains("Tools"), "{s}");
        assert!(s.contains('✕'), "{s}");
        assert!(app.hits.iter().any(|(_, h)| *h == Hit::CloseDetail));
        assert!(app
            .hits
            .iter()
            .any(|(_, h)| *h == Hit::DetailTab(DetailTab::Tasks)));
    }

    #[test]
    fn the_overview_names_the_model_and_the_cost() {
        let mut app = opened(LiveSnapshot {
            model: "anthropic::claude-opus".into(),
            tier: Some("complex".into()),
            cost_usd: 2.5,
            context_tokens: 90_000,
            context_limit: Some(100_000),
            ..Default::default()
        });
        let s = screen(&mut app, 160, 45);
        assert!(s.contains("claude-opus"), "{s}");
        assert!(s.contains("complex"), "{s}");
        assert!(s.contains("$2.50"), "{s}");
        assert!(s.contains("90%"), "{s}");
    }

    #[test]
    fn a_permission_prompt_gets_allow_and_deny_buttons() {
        let mut app = opened(LiveSnapshot {
            permission_prompt: Some("run: rm -rf target/debug".into()),
            prompt_seq: 3,
            ..Default::default()
        });
        let s = screen(&mut app, 160, 45);
        assert!(s.contains("needs you"), "{s}");
        assert!(s.contains("rm -rf target/debug"), "{s}");
        assert!(s.contains("allow"), "{s}");
        assert!(s.contains("deny"), "{s}");
        assert!(app
            .hits
            .iter()
            .any(|(_, h)| *h == Hit::Button(Button::Allow)));
        assert!(app
            .hits
            .iter()
            .any(|(_, h)| *h == Hit::Button(Button::Deny)));
    }

    #[test]
    fn a_question_gets_one_button_per_option() {
        let mut app = opened(LiveSnapshot {
            question: Some("which base branch?".into()),
            question_options: vec![
                QOption {
                    label: "main".into(),
                    description: "the default".into(),
                },
                QOption {
                    label: "develop".into(),
                    description: "the integration branch".into(),
                },
            ],
            ..Default::default()
        });
        let s = screen(&mut app, 160, 45);
        assert!(s.contains("which base branch?"), "{s}");
        assert!(s.contains("[ 1 ] main"), "{s}");
        assert!(s.contains("type an answer"), "{s}");
        assert!(app
            .hits
            .iter()
            .any(|(_, h)| *h == Hit::Button(Button::Answer(2))));
    }

    #[test]
    fn the_live_tab_colours_rows_and_can_hide_tools() {
        let mut app = opened(LiveSnapshot {
            transcript_rows: vec![
                super::super::wire::TranscriptRow {
                    kind: "user".into(),
                    text: "fix the router".into(),
                    ..Default::default()
                },
                super::super::wire::TranscriptRow {
                    kind: "tool".into(),
                    text: "crates/forge-mesh".into(),
                    tool: Some("read_file".into()),
                    meta: Some("ok".into()),
                },
            ],
            streaming: "reading the ranking pass".into(),
            ..Default::default()
        });
        app.detail_tab = DetailTab::Tail;
        let s = screen(&mut app, 160, 45);
        assert!(s.contains("fix the router"), "{s}");
        assert!(s.contains("read_file"), "{s}");
        assert!(s.contains("reading the ranking pass"), "{s}");
        app.show_tools = false;
        let s = screen(&mut app, 160, 45);
        assert!(!s.contains("read_file"), "{s}");
        assert!(app
            .hits
            .iter()
            .any(|(_, h)| *h == Hit::Button(Button::ToggleTools)));
    }

    #[test]
    fn the_changes_tab_shows_the_diff_and_the_branch() {
        let mut app = opened(LiveSnapshot {
            diff: Some(DiffCard {
                pending: true,
                files: vec![DiffFile {
                    path: "crates/forge-tui/src/board/render.rs".into(),
                    kind: "modified".into(),
                    adds: 12,
                    dels: 3,
                    binary: false,
                }],
                skipped_files: 2,
            }),
            ..Default::default()
        });
        app.apply(BoardEvent::Git(
            "aaaa1111".into(),
            GitInfo {
                branch: "feat/project-board".into(),
                base_branch: Some("main".into()),
                ..Default::default()
            },
        ));
        app.detail_tab = DetailTab::Changes;
        let s = screen(&mut app, 160, 45);
        assert!(s.contains("awaiting permission"), "{s}");
        assert!(s.contains("board/render.rs"), "{s}");
        assert!(s.contains("+2 more files"), "{s}");
        assert!(s.contains("feat/project-board"), "{s}");
        assert!(s.contains("working tree clean"), "{s}");
    }

    #[test]
    fn the_tools_tab_pairs_calls_with_their_results() {
        let mut app = opened(LiveSnapshot::default());
        app.apply(BoardEvent::History(
            "aaaa1111".into(),
            vec![
                HistoryRow {
                    kind: "tool".into(),
                    tool: Some("shell".into()),
                    tool_phase: Some("result".into()),
                    content: "error: no such file".into(),
                    ..Default::default()
                },
                HistoryRow {
                    kind: "tool".into(),
                    tool: Some("shell".into()),
                    tool_phase: Some("call".into()),
                    content: "cargo test -p forge-agent-tui".into(),
                    ..Default::default()
                },
            ],
        ));
        app.detail_tab = DetailTab::Tools;
        let s = screen(&mut app, 160, 45);
        assert!(s.contains("cargo test -p forge-agent-tui"), "{s}");
        assert!(s.contains("error: no such file"), "{s}");
        assert!(s.contains("Tools 1"), "{s}");
    }

    #[test]
    fn the_tasks_tab_marks_progress() {
        let mut app = opened(LiveSnapshot {
            tasks: vec![
                super::super::wire::Task {
                    title: "read the router".into(),
                    status: "done".into(),
                    assignee: Some("builder".into()),
                },
                super::super::wire::Task {
                    title: "rank the catalog".into(),
                    status: "in_progress".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        app.detail_tab = DetailTab::Tasks;
        let s = screen(&mut app, 160, 45);
        assert!(s.contains("read the router"), "{s}");
        assert!(s.contains("[builder]"), "{s}");
        assert!(s.contains("Tasks 1/2"), "{s}");
    }

    #[test]
    fn the_action_bar_offers_the_resolving_buttons() {
        let mut app = opened(LiveSnapshot::default());
        let s = screen(&mut app, 160, 45);
        assert!(s.contains("attach"), "{s}");
        assert!(s.contains("prompt"), "{s}");
        assert!(s.contains("archive"), "{s}");
        for b in [
            Button::Attach,
            Button::Prompt,
            Button::Archive,
            Button::Copy,
        ] {
            assert!(
                app.hits.iter().any(|(_, h)| *h == Hit::Button(b)),
                "missing {b:?}"
            );
        }
    }

    #[test]
    fn the_composer_overlay_names_what_it_is_collecting() {
        let mut app = opened(LiveSnapshot::default());
        app.open_composer(ComposerMode::Prompt, "aaaa1111", "ship it");
        let s = screen(&mut app, 120, 30);
        assert!(s.contains("prompt"), "{s}");
        assert!(s.contains("ship it"), "{s}");
        assert!(s.contains("Enter send"), "{s}");
    }

    #[test]
    fn the_confirm_overlay_asks_before_archiving() {
        let mut app = opened(LiveSnapshot::default());
        app.confirm = Some(Confirm {
            kind: ConfirmKind::Archive("aaaa1111".into()),
            title: "Archive this session?".into(),
            body: "It stops and leaves the board.".into(),
        });
        app.focus = Focus::Confirm;
        let s = screen(&mut app, 120, 30);
        assert!(s.contains("Archive this session?"), "{s}");
        assert!(s.contains("Enter yes"), "{s}");
    }

    #[test]
    fn the_help_overlay_lists_the_keys() {
        let mut app = opened(LiveSnapshot::default());
        app.focus = Focus::Help;
        let s = screen(&mut app, 140, 34);
        assert!(s.contains("forge board — keys"), "{s}");
        assert!(s.contains("attach: drop into the session"), "{s}");
        assert!(s.contains("any key closes"), "{s}");
    }

    #[test]
    fn a_tiny_pane_does_not_panic() {
        let mut app = opened(LiveSnapshot {
            permission_prompt: Some("run: rm -rf /".into()),
            ..Default::default()
        });
        screen(&mut app, 20, 5);
        screen(&mut app, 40, 8);
        screen(&mut app, 12, 6);
    }

    #[test]
    fn following_the_tail_writes_the_offset_back() {
        let mut app = opened(LiveSnapshot {
            transcript_rows: (0..80)
                .map(|i| super::super::wire::TranscriptRow {
                    kind: "assistant".into(),
                    text: format!("line {i}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        app.detail_tab = DetailTab::Tail;
        app.tail_follow = true;
        let s = screen(&mut app, 160, 45);
        assert!(s.contains("line 79"), "{s}");
        assert!(app.detail_scroll > 0);
        app.tail_follow = false;
        app.detail_scroll = 0;
        let s = screen(&mut app, 160, 45);
        assert!(s.contains("line 0"), "{s}");
        assert!(s.contains("following paused"), "{s}");
    }
}
