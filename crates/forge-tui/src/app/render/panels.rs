//! The panels surface: the tasks and agents band, the wide-terminal details rail, and the
//! composer hints while details own the keyboard.

use super::live::split_panel_budget;
use super::*;

mod band;
mod common;
mod rail;

pub(crate) use common::{fmt_cost, Clock};

/// Whether rendering `v` (given the previous shown row's phase was `prev`) needs a phase-header
/// line first — a workflow-script `phase()` transition (docs/rfcs/forge-workflow.md). Never true
/// for `None` phases, so a plain `spawn_agents` batch (every row's phase is `None`) never groups.
pub(crate) fn needs_phase_header(prev: Option<&str>, v: &ActivitySummary) -> bool {
    matches!(v.phase.as_deref(), Some(p) if Some(p) != prev)
}

/// Height of the band between the transcript and the decision block: the one-line summary, or
/// the expanded tasks and agents sections while details are open. On wide terminals open details
/// move into the rail and the band disappears. `width` is the full frame width and `avail` the
/// room the layout can spare.
pub(crate) fn band_height(app: &App, width: u16, avail: u16) -> u16 {
    if app.activity_focused {
        if rail::wanted(app, Rect::new(0, 0, width, 1)) {
            return 0;
        }
        let (activity_h, task_h) =
            split_panel_budget(app.activity_panel_height(), app.tasks_panel_height(), avail);
        activity_h + task_h
    } else {
        u16::from(app.has_activity() || !app.tasks.is_empty() || app.running_assay_critics() > 0)
            .min(avail)
    }
}

/// Draw the band into `area` (whose height is what [`band_height`] returned).
pub(crate) fn render_band(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    if !app.activity_focused {
        band::render_summary(frame, area, app);
        return;
    }
    let (activity_h, task_h) = split_panel_budget(
        app.activity_panel_height(),
        app.tasks_panel_height(),
        area.height,
    );
    // Tasks lead (they are the plan), agents follow.
    let rows =
        Layout::vertical([Constraint::Length(task_h), Constraint::Length(activity_h)]).split(area);
    if task_h > 0 {
        frame.render_widget(
            Paragraph::new(band::task_section_lines(app, rows[0].height, rows[0].width)),
            rows[0],
        );
    }
    if activity_h > 0 {
        frame.render_widget(
            Paragraph::new(band::activity_section_lines(
                app,
                rows[1].height,
                rows[1].width,
            )),
            rows[1],
        );
    }
}

/// Width of the details rail: 38 cells on terminals 150 or more wide while details are open.
pub(crate) fn rail_width(app: &App, area: Rect) -> u16 {
    if rail::wanted(app, area) {
        rail::RAIL_WIDTH
    } else {
        0
    }
}

/// Draw the details rail into `area` (never called while [`rail_width`] is zero).
pub(crate) fn render_rail(frame: &mut Frame, area: Rect, app: &App) {
    rail::render_rail(frame, area, app);
}

/// Composer hints while the details own the keyboard (`Ctrl+O`). `None` leaves the composer's own.
pub(crate) fn hints(app: &App) -> Option<Vec<super::chrome::Hint>> {
    use super::chrome::hint;
    if !app.activity_focused {
        return None;
    }
    let mut hints = Vec::new();
    if app.has_activity() {
        hints.push(hint("↑↓", "select"));
        hints.push(hint("Enter", "open"));
    }
    hints.push(hint("Ctrl+O", "hide"));
    hints.push(hint("Esc", "close"));
    Some(hints)
}
