//! The board's "while you were away" overlay: when it opens by itself, what it says, and the
//! small state file that remembers when the board was last looked at.

use forge_tui::board::{BoardEvent, BriefCard, BriefLine, BriefTone, ToastLevel};
use tokio::sync::mpsc;

use super::Ev;
use crate::brief_client::{fetch_brief, headline, session_line, span, title_line};
use crate::serve_brief::{Brief, Outcome};

/// Shorter gaps are an ordinary glance, not a return (the phone card uses the same threshold).
pub(crate) const AWAY_AFTER_SECS: i64 = 30 * 60;
/// How far back `B` looks when the board has never been opened before.
pub(crate) const FIRST_WINDOW_SECS: i64 = 8 * 3600;

fn state_file() -> Option<std::path::PathBuf> {
    forge_config::data_dir().map(|d| d.join("board-last-visit"))
}

pub(crate) fn read_last_visit() -> Option<i64> {
    let raw = std::fs::read_to_string(state_file()?).ok()?;
    raw.trim().parse().ok().filter(|v: &i64| *v > 0)
}

pub(crate) fn stamp_visit(now: i64) {
    if let Some(path) = state_file() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, now.to_string());
    }
}

/// The window to open with: the previous visit when it is old enough to count as a return.
pub(crate) fn away_window(last_visit: Option<i64>, now: i64) -> Option<i64> {
    last_visit.filter(|last| now - last >= AWAY_AFTER_SECS)
}

pub(crate) fn card_of(brief: &Brief) -> BriefCard {
    BriefCard {
        title: title_line(brief),
        headline: match &brief.queue {
            Some(q) => format!("{} · {}", headline(brief), q.line),
            None => headline(brief),
        },
        lines: brief
            .sessions
            .iter()
            .map(|s| BriefLine {
                session_id: s.id.clone(),
                text: session_line(s, brief.now),
                detail: s.summary.clone(),
                tone: match s.outcome {
                    Outcome::Finished => BriefTone::Good,
                    Outcome::Failed | Outcome::Waiting => BriefTone::Bad,
                    Outcome::Interrupted => BriefTone::Warn,
                    Outcome::Working => BriefTone::Quiet,
                },
            })
            .collect(),
    }
}

/// Fetch the digest and hand it to the board. `manual` (the `B` key) answers even when nothing
/// happened; the automatic open stays silent unless there is something to say.
pub(crate) async fn show_brief(
    base: String,
    token: String,
    since: i64,
    manual: bool,
    ev: mpsc::UnboundedSender<Ev>,
) {
    let event = match fetch_brief(&base, &token, since).await {
        Ok(brief) if !brief.sessions.is_empty() || brief.queue.is_some() => {
            BoardEvent::Brief(card_of(&brief))
        }
        Ok(brief) if manual => BoardEvent::Toast(
            ToastLevel::Info,
            format!(
                "nothing happened in the last {}",
                span(brief.now - brief.since)
            ),
        ),
        Ok(_) => return,
        Err(_) if !manual => return,
        Err(e) => BoardEvent::Toast(ToastLevel::Error, format!("away summary: {e}")),
    };
    let _ = ev.send(Ev::Board(event));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_long_gap_opens_the_overlay() {
        assert_eq!(away_window(None, 10_000), None);
        assert_eq!(
            away_window(Some(10_000 - AWAY_AFTER_SECS + 1), 10_000),
            None
        );
        assert_eq!(
            away_window(Some(10_000 - AWAY_AFTER_SECS), 10_000),
            Some(10_000 - AWAY_AFTER_SECS)
        );
    }
}
