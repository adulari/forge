//! Shared helpers for the panel surfaces: aligned rows, durations, cost text and the observed
//! start times that give tasks and agents an elapsed counter.

use std::time::{Duration, Instant};

use ratatui::text::{Line as TextLine, Span};
use unicode_width::UnicodeWidthStr;

use super::super::span_fit::fit_spans;
use crate::app::render::fmt_dur;
use crate::app::App;

pub(crate) fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// `left` clipped with an ellipsis, then `right` flush against the last column (`margin` cells of
/// air after it). The right cluster wins the room it needs; the left shrinks first.
pub(crate) fn aligned(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
    margin: usize,
) -> TextLine<'static> {
    let right_w = spans_width(&right);
    let room = width.saturating_sub(right_w + margin + if right_w > 0 { 1 } else { 0 });
    let mut spans = fit_spans(left, room.max(1));
    let used = spans_width(&spans);
    let gap = width.saturating_sub(used + right_w + margin);
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right);
    TextLine::from(spans)
}

/// A duration for a counter: `41s`, `2m 10s`, `1h 05m`.
pub(crate) fn fmt_elapsed(d: Duration) -> String {
    let secs = d.as_secs();
    if (60..3600).contains(&secs) {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    } else {
        fmt_dur(secs)
    }
}

pub(crate) use crate::overlays::fmt_cost;

/// Start and finish times observed for tasks and agents, keyed by a stable label. The protocol
/// carries no timestamps for either, so the clock starts when the panel first sees the item
/// running and stops when it sees it finish.
#[derive(Debug, Clone, Default)]
pub(crate) struct Clock {
    seen: std::collections::HashMap<String, (Instant, Option<Instant>)>,
}

impl Clock {
    /// Elapsed time for `key`. `running` items start (or keep) their clock; finished items freeze
    /// it; items never seen running have none.
    pub(crate) fn elapsed(&mut self, key: &str, running: bool, finished: bool) -> Option<Duration> {
        let now = Instant::now();
        if running {
            let entry = self.seen.entry(key.to_string()).or_insert((now, None));
            entry.1 = None;
            return Some(now.duration_since(entry.0));
        }
        if finished {
            let entry = self.seen.get_mut(key)?;
            let end = *entry.1.get_or_insert(now);
            return Some(end.duration_since(entry.0));
        }
        None
    }
}

impl App {
    /// Elapsed counter for a task or agent; see [`Clock`].
    pub(crate) fn panel_elapsed(
        &self,
        key: &str,
        running: bool,
        finished: bool,
    ) -> Option<Duration> {
        self.panels_ui
            .clock
            .borrow_mut()
            .elapsed(key, running, finished)
    }
}
