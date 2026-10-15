//! Harness-injected user-role messages that are not job exits: an agent-scheduled wakeup firing
//! and a user `/heartbeat` tick (see `forge_types::harness_notice`). The model gets them as user
//! turns; the person did not type them, so they render as dim harness rows instead of "you".
//! Long prompts are cut to a couple of lines here; the model transcript keeps the full text.
use ratatui::style::Style;
use ratatui::text::{Line as TextLine, Span};

use forge_types::HarnessNotice;

use super::{truncate, width_cap, App, LineOrigin};
use crate::surface::DIM;

/// Prompt lines shown under a wakeup/heartbeat headline, beyond the headline itself.
const EXTRA_PROMPT_LINES: usize = 1;

impl App {
    /// Render `text` as a harness notice when it is one; `false` leaves it to the caller's
    /// ordinary user-row rendering. Peer messages keep their own renderer in `submit_user`.
    pub(super) fn echo_harness_notice(&mut self, text: &str) -> bool {
        if self.echo_job_notices(text) {
            return true;
        }
        let Some(notice) = forge_types::parse_harness_notice(text) else {
            return false;
        };
        let (prompt, reason) = match &notice {
            HarnessNotice::Wakeup { prompt, reason } => (prompt, reason.as_deref()),
            HarnessNotice::Heartbeat { prompt, .. } => (prompt, None),
            _ => return false,
        };
        self.seal_flush_origins(&LineOrigin::default());
        let width = self.last_width.get();
        let cap = width_cap(width, 6, 30);
        let dim = Style::default().fg(DIM);
        let mut rows: Vec<String> = Vec::new();
        rows.push(format!(
            "  {}",
            notice.headline(cap.saturating_sub(4)).unwrap_or_default()
        ));
        let rest: Vec<&str> = prompt
            .lines()
            .filter(|l| !l.trim().is_empty())
            .skip(1)
            .collect();
        for l in rest.iter().take(EXTRA_PROMPT_LINES) {
            rows.push(format!("      {}", truncate(l.trim(), cap)));
        }
        if rest.len() > EXTRA_PROMPT_LINES {
            rows.push("      …".to_string());
        }
        if let Some(reason) = reason {
            let first = reason.lines().next().unwrap_or("").trim();
            rows.push(format!("      because: {}", truncate(first, cap)));
        }
        for row in rows {
            self.flush.push(TextLine::from(Span::styled(row, dim)));
        }
        self.flush.push(TextLine::default());
        self.seal_flush_origins(&LineOrigin::harness(notice.subtype()));
        true
    }
}
