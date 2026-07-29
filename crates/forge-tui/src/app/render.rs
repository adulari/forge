//! Shared rendering helpers and composition modules.

use super::*;

mod input;
mod live;
mod overlays;
mod status_line;
mod status_widgets;
mod transcript;
mod voice;
pub use input::{input_box_height, input_cursor_up};
pub(crate) use input::{input_spans, input_text_rows, render_input};
pub use live::render_live;
pub(crate) use live::split_panel_budget;
pub(crate) use overlays::{cost_cell, mesh_pace_suffix};
pub use overlays::{render_mesh_overlay, render_usage_overlay};
pub(crate) use status_line::{render_statusline, statusline_hint};
pub(crate) use status_widgets::{
    compact_band_height, context_gauge_spans, effort_status, fmt_dur, format_tok, human,
    render_compact_band, render_effort_slider, rl_age_note, statusline_height,
    statusline_wants_row2,
};
pub(crate) use transcript::{
    model_short, needs_phase_header, prompt_height, render_activity_panel, render_permission,
    render_preview, render_transcript_area, tasks_panel_lines,
};
pub use voice::render_voice_overlay;
