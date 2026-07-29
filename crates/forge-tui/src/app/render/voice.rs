use super::*;

/// The `/voice` recording overlay: a focused single-purpose card with a pulsing REC indicator,
/// live waveform, and (on first use) a whisper-model download progress bar. See voice.md.
pub fn render_voice_overlay(f: &mut Frame, app: &App) {
    use ratatui::text::Line;

    let Some(v) = &app.voice else { return };
    let area = f.area();
    let w = area.width.saturating_sub(4).clamp(20, 56);
    let h = 9u16.min(area.height);
    if area.width < 4 || h == 0 {
        return;
    }
    let card = surface::modal_area(area, w, h);

    let title = match &v.phase {
        VoicePhase::Downloading { .. } => "⌬ voice — fetching model".to_string(),
        VoicePhase::Recording { .. } => "● voice".to_string(),
        VoicePhase::Transcribing => "⌬ voice — transcribing".to_string(),
        VoicePhase::Error(_) => "⚠ voice".to_string(),
    };
    let tone = match &v.phase {
        VoicePhase::Recording { .. } | VoicePhase::Error(_) => surface::SurfaceTone::Danger,
        VoicePhase::Downloading { .. } | VoicePhase::Transcribing => surface::SurfaceTone::Accent,
    };
    let inner = surface::render_panel(f, card, surface::title(title, tone), None, tone);
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
            let bar_w = inner.width.saturating_sub(2) as usize;
            let filled = pct
                .map(|p| ((p / 100.0) * bar_w as f64).round() as usize)
                .unwrap_or(0)
                .min(bar_w);
            let bar = format!(
                "[{}{}]",
                "█".repeat(filled),
                "░".repeat(bar_w.saturating_sub(filled))
            );
            let lines = vec![
                Line::from(Span::styled(label, Style::default().fg(TEXT))),
                Line::from(Span::styled(bar, Style::default().fg(ACCENT))),
                Line::from(""),
                Line::from(Span::styled("  Esc to cancel", Style::default().fg(DIM))),
            ];
            f.render_widget(Paragraph::new(lines), inner);
        }
        VoicePhase::Recording { ptt_active } => {
            let blink_on = (v.anim_tick / 6).is_multiple_of(2);
            let rec_span = if blink_on {
                Span::styled("● REC", Style::default().fg(ERRRED).bold())
            } else {
                Span::styled("○ REC", Style::default().fg(DIM))
            };
            let header = Line::from(vec![
                rec_span,
                Span::raw("   "),
                Span::styled(v.mmss(), Style::default().fg(TEXT)),
            ]);
            let waveform = render_voice_waveform(&v.waveform, false);
            let footer_text = if *ptt_active {
                "  hold to keep recording · release to transcribe · Enter/Esc/r also work"
            } else {
                "  Enter transcribe · Esc cancel · r restart"
            };
            let footer = Line::from(Span::styled(footer_text, Style::default().fg(DIM)));
            let lines = vec![header, Line::from(""), waveform, Line::from(""), footer];
            f.render_widget(Paragraph::new(lines), inner);
        }
        VoicePhase::Transcribing => {
            let spinner = SPINNER[(v.anim_tick as usize) % SPINNER.len()];
            let header = Line::from(vec![
                Span::styled(
                    format!("{spinner} transcribing…"),
                    Style::default().fg(ACCENT),
                ),
                Span::raw("   "),
                Span::styled(v.mmss(), Style::default().fg(TEXT)),
            ]);
            let waveform = render_voice_waveform(&v.waveform, true);
            let footer = Line::from(Span::styled("  please wait…", Style::default().fg(DIM)));
            let lines = vec![header, Line::from(""), waveform, Line::from(""), footer];
            f.render_widget(Paragraph::new(lines), inner);
        }
        VoicePhase::Error(msg) => {
            let lines = vec![
                Line::from(Span::styled(msg.clone(), Style::default().fg(ERRRED))),
                Line::from(""),
                Line::from(Span::styled(
                    "  any key to dismiss",
                    Style::default().fg(DIM),
                )),
            ];
            f.render_widget(Paragraph::new(lines), inner);
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
    let color = if dim { DIM } else { ACCENT };
    let spans: Vec<Span<'static>> = waveform
        .iter()
        .map(|&lvl| {
            let idx = ((lvl.clamp(0.0, 1.0) * (BARS.len() - 1) as f32).round() as usize)
                .min(BARS.len() - 1);
            Span::styled(BARS[idx].to_string(), Style::default().fg(color))
        })
        .collect();
    ratatui::text::Line::from(spans)
}
