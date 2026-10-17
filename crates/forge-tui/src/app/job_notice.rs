//! Background-job completion notices in the transcript.
//!
//! The harness tells the model a `shell{background:true}` job exited by injecting a user-role
//! message (`forge_core::job_wake`). The model needs it as a user turn; the person does not need
//! to see it as something they typed. Wherever a user row would be echoed, a message that is
//! entirely job notices renders here instead: one header line per job, its log tail dim and
//! indented underneath.
use crate::theme::styles::{self, NoticeKind};
use ratatui::text::{Line as TextLine, Span};

use forge_types::{JobExitNotice, JobNotice};

use super::lines::notice;
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
                    let rows = notice::notice_with(
                        NoticeKind::Info,
                        false,
                        vec![Span::styled(
                            format!("{n} earlier job notice(s) dropped · `shell_job` lists them"),
                            styles::muted(),
                        )],
                        &[],
                    );
                    self.flush.extend(rows);
                    self.seal_flush_origins(&LineOrigin::job(None));
                }
                JobNotice::Exit(exit) => {
                    let skipped = exit.tail.len().saturating_sub(TAIL_SHOWN);
                    let tail: Vec<String> = exit
                        .tail
                        .iter()
                        .skip(skipped)
                        .map(|l| truncate(l, width_cap(width, 8, 40)))
                        .collect();
                    let mut rows = notice::notice_with(
                        if exit.ok() {
                            NoticeKind::Ok
                        } else {
                            NoticeKind::Err
                        },
                        false,
                        job_header(exit, width),
                        &tail,
                    )
                    .into_iter();
                    if let Some(head) = rows.next() {
                        self.flush.push(head);
                    }
                    self.seal_flush_origins(&LineOrigin::job(Some(if exit.ok() {
                        "ok"
                    } else {
                        "failed"
                    })));
                    self.flush.extend(rows);
                    self.seal_flush_origins(&LineOrigin::job(Some("tail")));
                }
            }
        }
        self.flush.push(TextLine::default());
        self.seal_flush_origins(&LineOrigin::job(Some("tail")));
        true
    }
}

/// `job 3429249 exited 0 · /tmp/batch.sh …`: the head of a job notice, after its glyph.
fn job_header(exit: &JobExitNotice, width: u16) -> Vec<Span<'static>> {
    let head = format!("job {} ", exit.pid);
    let status = format!("exited {}", exit.code);
    let reserve = 6 + head.chars().count() + status.chars().count() + 3;
    let status_style = if exit.ok() {
        styles::ok()
    } else {
        styles::err()
    };
    vec![
        Span::styled(head, styles::fg()),
        Span::styled(status, status_style),
        Span::styled(
            format!(
                " · {}",
                truncate(&exit.command, width_cap(width, reserve, 30))
            ),
            styles::muted(),
        ),
    ]
}
