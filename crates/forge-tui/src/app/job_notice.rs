//! Background-job completion notices in the transcript.
//!
//! The harness tells the model a `shell{background:true}` job exited by injecting a user-role
//! message (`forge_core::job_wake`). The model needs it as a user turn; the person does not need
//! to see it as something they typed. Wherever a user row would be echoed, a message that is
//! entirely job notices renders here instead: one header line per job, its log tail dim and
//! indented underneath.
use crate::theme::{color, styles};
use ratatui::style::Style;
use ratatui::text::{Line as TextLine, Span};

use forge_types::{JobExitNotice, JobNotice};

use super::{truncate, width_cap, App, LineOrigin};

/// Log lines shown under a job header; the notice already carries only the tail of the log.
const TAIL_SHOWN: usize = 4;

impl App {
    /// Render `text` as job notices when it is nothing but notices; `false` leaves it to the
    /// caller's ordinary user-row rendering.
    pub(super) fn echo_job_notices(&mut self, text: &str) -> bool {
        let Some(items) = forge_types::parse_job_notices(text) else {
            return false;
        };
        self.seal_flush_origins(&LineOrigin::default());
        let width = self.last_width.get();
        for item in &items {
            match item {
                JobNotice::Dropped(n) => {
                    self.flush.push(TextLine::from(Span::styled(
                        format!("  ⚙ {n} earlier job notice(s) dropped · `shell_job` lists them"),
                        styles::muted(),
                    )));
                    self.seal_flush_origins(&LineOrigin::job(None));
                }
                JobNotice::Exit(exit) => {
                    self.flush.push(job_header(exit, width));
                    self.seal_flush_origins(&LineOrigin::job(Some(if exit.ok() {
                        "ok"
                    } else {
                        "failed"
                    })));
                    let skipped = exit.tail.len().saturating_sub(TAIL_SHOWN);
                    for l in exit.tail.iter().skip(skipped) {
                        self.flush.push(TextLine::from(Span::styled(
                            format!("      {}", truncate(l, width_cap(width, 8, 40))),
                            styles::muted(),
                        )));
                    }
                    self.seal_flush_origins(&LineOrigin::job(Some("tail")));
                }
            }
        }
        self.flush.push(TextLine::default());
        self.seal_flush_origins(&LineOrigin::job(Some("tail")));
        true
    }
}

fn job_header(exit: &JobExitNotice, width: u16) -> TextLine<'static> {
    let (mark, color) = if exit.ok() {
        ("✓", color::ok())
    } else {
        ("✗", color::err())
    };
    let head = format!("job {} ", exit.pid);
    let status = format!("exited {}", exit.code);
    let reserve = 6 + head.chars().count() + status.chars().count() + 3;
    TextLine::from(vec![
        Span::styled("  ⚙ ", styles::muted()),
        Span::styled(head, styles::fg()),
        Span::styled(format!("{mark} {status}"), Style::default().fg(color)),
        Span::styled(
            format!(
                " · {}",
                truncate(&exit.command, width_cap(width, reserve, 30))
            ),
            styles::muted(),
        ),
    ])
}
