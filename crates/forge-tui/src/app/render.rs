//! Shared rendering helpers and composition modules.

use super::*;

pub(crate) mod chrome;
pub(crate) mod header;
pub(crate) mod input;
pub(crate) mod list_view;
pub(crate) mod live;
mod overlays;
pub(crate) mod panels;
pub(crate) mod permission;
pub(crate) mod popups;
mod question;
mod queue_band;
mod span_fit;
pub(crate) mod status_line;
pub(crate) mod status_panel;
mod status_widgets;
mod transcript;
mod update_band;
mod voice;
pub(crate) mod welcome;
pub(crate) use input::render_input;
pub use input::{input_box_height, input_cursor_up};
pub use live::render_live;
pub(crate) use overlays::{cost_cell, mesh_pace_suffix};
pub use overlays::{render_btw_overlay, render_mesh_overlay, render_usage_overlay};
pub(crate) use panels::needs_phase_header;
pub(crate) use status_line::render_statusline;
pub(crate) use status_widgets::{
    compact_band_height, fmt_dur, format_tok, human, mesh_effort_chip, model_effort_marker,
    render_compact_band, render_effort_slider, rl_age_note, statusline_height,
};
pub(crate) use transcript::{model_short, render_preview, render_transcript_area};
pub use voice::render_voice_overlay;
