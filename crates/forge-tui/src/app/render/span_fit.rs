use super::*;
use unicode_width::UnicodeWidthStr;

/// Clip `spans` to `max` terminal cells, ending with an ellipsis instead of a hard mid-word cut,
/// so the left cluster never runs flush into the right-aligned version text.
pub(super) fn fit_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(|s| s.content.width()).sum();
    if total <= max {
        return spans;
    }
    let budget = max.saturating_sub(1);
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for span in spans {
        let mut text = String::new();
        for ch in span.content.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + cw > budget {
                if !text.is_empty() {
                    out.push(Span::styled(text, span.style));
                }
                out.push(Span::styled("…", span.style));
                return out;
            }
            used += cw;
            text.push(ch);
        }
        out.push(Span::styled(text, span.style));
    }
    out
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn long_left_clusters_end_in_an_ellipsis_within_budget() {
        let spans = vec![Span::raw("[trivial] llama3.2"), Span::raw(" │ ◈ forge")];
        let fitted = fit_spans(spans, 12);
        let out = text(&fitted);
        assert!(out.ends_with('…'));
        assert!(out.width() <= 12, "{out}");
        assert_eq!(text(&fit_spans(vec![Span::raw("short")], 12)), "short");
    }
}
