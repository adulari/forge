use super::*;
use crate::theme::glyphs::g;
use crate::theme::styles;

/// The `/voice` recording overlay: a focused single-purpose card with a live REC indicator,
/// live waveform, and (on first use) a whisper-model download progress bar. Its keys sit in the
/// frame's footer border. See voice.md.
pub fn render_voice_overlay(f: &mut Frame, app: &App) {
    use ratatui::text::Line;

    let Some(v) = &app.voice else { return };
    let glyphs = g();
    let area = f.area();
    surface::dim_backdrop(f, area);
    let w = area.width.saturating_sub(4).clamp(20, 60);
    let h = 9u16.min(area.height);
    if area.width < 4 || h == 0 {
        return;
    }
    let card = surface::modal_area(area, w, h);

    let title = match &v.phase {
        VoicePhase::Downloading { .. } => "Voice · fetching model".to_string(),
        VoicePhase::Recording { .. } => "Voice".to_string(),
        VoicePhase::Transcribing => "Voice · transcribing".to_string(),
        VoicePhase::Error(_) => format!("{} Voice", glyphs.warn),
    };
    let tone = match &v.phase {
        VoicePhase::Recording { .. } => surface::SurfaceTone::Brand,
        VoicePhase::Error(_) => surface::SurfaceTone::Danger,
        VoicePhase::Downloading { .. } | VoicePhase::Transcribing => surface::SurfaceTone::Accent,
    };
    let footer_text = match &v.phase {
        VoicePhase::Downloading { .. } => "Esc cancel".to_string(),
        VoicePhase::Recording { ptt_active: true } => {
            "hold to keep recording · release to transcribe · Enter/Esc/r also work".to_string()
        }
        VoicePhase::Recording { ptt_active: false } => {
            "Enter transcribe · Esc cancel · r restart".to_string()
        }
        VoicePhase::Transcribing => "please wait".to_string(),
        VoicePhase::Error(_) => "any key dismiss".to_string(),
    };
    let inner = crate::overlays::padded_panel(
        f,
        card,
        surface::title(title, tone),
        Some(Span::styled(" /voice ", styles::faint())),
        Some(Span::styled(format!(" {footer_text} "), styles::faint())),
        tone,
    );
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    match &v.phase {
        VoicePhase::Downloading {
            model,
            done_mb,
            total_mb,
        } => {
            let pct = total_mb
                .filter(|t| *t > 0.0)
                .map(|t| (done_mb / t * 100.0).min(100.0));
            let label = match (total_mb, pct) {
                (Some(t), Some(p)) => {
                    format!("Fetching whisper-{model} · {done_mb:.0}/{t:.0} MB ({p:.0}%)")
                }
                _ => format!("Fetching whisper-{model} · {done_mb:.0} MB"),
            };
            let bar = glyphs.bar(pct.map_or(0.0, |p| p / 100.0), inner.width as usize);
            let lines = vec![
                Line::from(Span::styled(label, styles::fg())),
                Line::from(Span::styled(bar, styles::user())),
            ];
            f.render_widget(Paragraph::new(lines), inner);
        }
        VoicePhase::Recording { .. } => {
            let blink_on = !crate::theme::t().animates() || (v.anim_tick / 6).is_multiple_of(2);
            let rec_span = if blink_on {
                Span::styled(format!("{} REC", glyphs.live_dot), styles::err().bold())
            } else {
                Span::styled(format!("{} REC", glyphs.task_open), styles::muted())
            };
            let header = Line::from(vec![
                rec_span,
                Span::raw("   "),
                Span::styled(v.mmss(), styles::fg()),
            ]);
            let waveform = render_voice_waveform(&v.waveform, false);
            f.render_widget(
                Paragraph::new(vec![header, Line::from(""), waveform]),
                inner,
            );
        }
        VoicePhase::Transcribing => {
            let header = Line::from(vec![
                Span::styled(
                    format!(
                        "{} transcribing{}",
                        glyphs.spin(v.anim_tick as usize),
                        glyphs.ellipsis
                    ),
                    styles::user(),
                ),
                Span::raw("   "),
                Span::styled(v.mmss(), styles::fg()),
            ]);
            let waveform = render_voice_waveform(&v.waveform, true);
            f.render_widget(
                Paragraph::new(vec![header, Line::from(""), waveform]),
                inner,
            );
        }
        VoicePhase::Error(msg) => {
            f.render_widget(
                Paragraph::new(vec![Line::from(Span::styled(msg.clone(), styles::err()))])
                    .wrap(ratatui::widgets::Wrap { trim: false }),
                inner,
            );
        }
    }
}

/// Render the waveform ring buffer as a line of `▁▂▃▄▅▆▇█` bars, one per sample, oldest first
/// (so newer samples appear to scroll in from the right as the ring buffer fills and drops old
/// ones off the front). `dim` renders it frozen/greyed (the transcribing state).
fn render_voice_waveform(
    waveform: &std::collections::VecDeque<f32>,
    dim: bool,
) -> ratatui::text::Line<'static> {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    const ASCII_BARS: [char; 8] = ['_', '.', ':', '-', '=', '+', '*', '#'];
    let bars = if g().unicode { BARS } else { ASCII_BARS };
    let style = if dim { styles::muted() } else { styles::user() };
    let spans: Vec<Span<'static>> = waveform
        .iter()
        .map(|&lvl| {
            let idx = ((lvl.clamp(0.0, 1.0) * (bars.len() - 1) as f32).round() as usize)
                .min(bars.len() - 1);
            Span::styled(bars[idx].to_string(), style)
        })
        .collect();
    ratatui::text::Line::from(spans)
}
