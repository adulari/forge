//! `/context`: the context window, by what fills it.

use forge_core::context_report::ContextReport;

const BAR_CELLS: usize = 30;

fn compact(tokens: u64) -> String {
    if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn share(tokens: usize, window: Option<u32>) -> String {
    match window.filter(|w| *w > 0) {
        Some(w) => format!(
            "{:>3}%",
            (tokens as f64 * 100.0 / f64::from(w)).round() as u64
        ),
        None => String::new(),
    }
}

/// The lines `/context` prints. `window` is the active model's context size when known;
/// `last_request` is what the provider reported for the previous request (0 before the first).
pub(super) fn report_lines(
    report: &ContextReport,
    window: Option<u32>,
    last_request: u64,
) -> Vec<String> {
    let total = report.total();
    let window = window.filter(|w| *w > 0);
    let mut lines = Vec::new();
    let headline = match window {
        Some(w) => {
            let filled = (total * BAR_CELLS / w as usize).min(BAR_CELLS);
            format!(
                "⛁ context  {}{}  ~{} of {} tokens ({}) for the next request",
                "█".repeat(filled),
                "░".repeat(BAR_CELLS - filled),
                compact(total as u64),
                compact(u64::from(w)),
                share(total, Some(w)).trim()
            )
        }
        None => format!(
            "⛁ context  ~{} tokens for the next request (this model's window is not known yet)",
            compact(total as u64)
        ),
    };
    lines.push(headline);
    for category in report.categories.iter().filter(|c| c.tokens > 0) {
        let count = if category.items > 0 {
            format!("  ({})", category.items)
        } else {
            String::new()
        };
        lines.push(format!(
            "  {:<24}{:>8} {}{count}",
            category.label,
            compact(category.tokens as u64),
            share(category.tokens, window)
        ));
    }
    if let Some(w) = window {
        let free = (w as usize).saturating_sub(total);
        lines.push(format!(
            "  {:<24}{:>8} {}",
            "Free",
            compact(free as u64),
            share(free, window)
        ));
    }
    if last_request > 0 {
        lines.push(format!(
            "  last request: {} tokens as the provider counted them",
            compact(last_request)
        ));
    }
    lines.push(
        "  estimated with the o200k tokenizer before window fitting · /compact (Alt+L) summarises older turns, /uncompact restores them".to_string(),
    );
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::context_report::ContextCategory;

    fn report() -> ContextReport {
        ContextReport {
            categories: vec![
                ContextCategory {
                    label: "System prompt",
                    tokens: 3_000,
                    items: 0,
                },
                ContextCategory {
                    label: "Tool definitions",
                    tokens: 12_000,
                    items: 30,
                },
                ContextCategory {
                    label: "Harness notes",
                    tokens: 0,
                    items: 0,
                },
                ContextCategory {
                    label: "Your messages",
                    tokens: 5_000,
                    items: 4,
                },
            ],
        }
    }

    #[test]
    fn a_known_window_gets_a_bar_shares_and_free_space() {
        let lines = report_lines(&report(), Some(200_000), 18_500);
        assert!(
            lines[0].contains("~20.0k of 200.0k tokens (10%)"),
            "{}",
            lines[0]
        );
        assert!(lines[0].contains('█') && lines[0].contains('░'));
        assert!(lines.iter().any(|l| l.contains("Tool definitions")
            && l.contains("12.0k")
            && l.contains("6%")
            && l.contains("(30)")));
        assert!(lines
            .iter()
            .any(|l| l.contains("Free") && l.contains("180.0k") && l.contains("90%")));
        assert!(
            !lines.iter().any(|l| l.contains("Harness notes")),
            "empty rows are left out"
        );
        assert!(lines.iter().any(|l| l.contains("last request: 18.5k")));
    }

    #[test]
    fn an_unknown_window_still_breaks_the_request_down() {
        let lines = report_lines(&report(), None, 0);
        assert!(lines[0].contains("~20.0k tokens"));
        assert!(lines[0].contains("not known"));
        assert!(!lines.iter().any(|l| l.contains("Free")));
        assert!(!lines.iter().any(|l| l.contains("last request")));
    }
}
