//! Recognises the background-job completion notices Forge injects into a session as user-role
//! messages (`forge_core::job_wake`), so every surface can show them as harness notices instead of
//! as something the person typed. The model-facing text is unchanged; this only reads it back.
//!
//! Shapes (one notice per block, blocks joined by a blank line when an idle prompt coalesces them):
//!
//! ```text
//! [background job <pid> `<command>` exited <code>]
//! last lines:
//! <tail>
//! [<n> earlier background-job notice(s) were dropped; check `shell_job` list]
//! ```

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobNotice {
    Exit(JobExitNotice),
    /// Notices lost to the inbox cap before delivery.
    Dropped(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobExitNotice {
    pub pid: u32,
    pub command: String,
    /// -1 when a signal killed the job.
    pub code: i64,
    pub tail: Vec<String>,
}

impl JobExitNotice {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    /// `job 3429249 ✓ exited 0 · /tmp/batch.sh`
    pub fn summary(&self) -> String {
        let mark = if self.ok() { '✓' } else { '✗' };
        format!(
            "job {} {mark} exited {} · {}",
            self.pid, self.code, self.command
        )
    }
}

/// Every notice in `text`, or `None` when `text` is anything other than job notices (a person's
/// prompt that merely mentions one stays a prompt).
pub fn parse_job_notices(text: &str) -> Option<Vec<JobNotice>> {
    let mut out: Vec<JobNotice> = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(n) = parse_dropped(line) {
            out.push(JobNotice::Dropped(n));
            continue;
        }
        let mut exit = parse_header(line)?;
        if lines.peek() == Some(&"last lines:") {
            lines.next();
            while let Some(next) = lines.peek() {
                if parse_header(next).is_some() || parse_dropped(next).is_some() {
                    break;
                }
                exit.tail.push((*next).to_string());
                lines.next();
            }
            while exit.tail.last().is_some_and(|l| l.trim().is_empty()) {
                exit.tail.pop();
            }
        }
        out.push(JobNotice::Exit(exit));
    }
    (!out.is_empty()).then_some(out)
}

fn parse_header(line: &str) -> Option<JobExitNotice> {
    let rest = line.strip_prefix("[background job ")?.strip_suffix(']')?;
    let (pid, rest) = rest.split_once(' ')?;
    let rest = rest.strip_prefix('`')?;
    let split = rest.rfind("` exited ")?;
    let command = &rest[..split];
    let code = rest[split + "` exited ".len()..].parse().ok()?;
    Some(JobExitNotice {
        pid: pid.parse().ok()?,
        command: command.to_string(),
        code,
        tail: Vec::new(),
    })
}

fn parse_dropped(line: &str) -> Option<usize> {
    line.strip_prefix('[')?
        .strip_suffix(" earlier background-job notice(s) were dropped; check `shell_job` list]")?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DROPPED: &str =
        "[3 earlier background-job notice(s) were dropped; check `shell_job` list]";

    fn exit(pid: u32, command: &str, code: i64, tail: &[&str]) -> JobNotice {
        JobNotice::Exit(JobExitNotice {
            pid,
            command: command.into(),
            code,
            tail: tail.iter().map(|s| s.to_string()).collect(),
        })
    }

    #[test]
    fn plain_notice() {
        let text = "[background job 3429249 `/tmp/batch.sh > /tmp/batch_result.log 2>&1` exited 0]";
        assert_eq!(
            parse_job_notices(text),
            Some(vec![exit(
                3429249,
                "/tmp/batch.sh > /tmp/batch_result.log 2>&1",
                0,
                &[]
            )])
        );
    }

    #[test]
    fn summary_marks_the_outcome() {
        let n = JobExitNotice {
            pid: 4,
            command: "ls".into(),
            code: 1,
            tail: vec![],
        };
        assert_eq!(n.summary(), "job 4 ✗ exited 1 · ls");
    }

    #[test]
    fn notice_with_tail_and_signal_code() {
        let text = "[background job 7 `make` exited -1]\nlast lines:\nline a\n\nline c";
        let got = parse_job_notices(text).unwrap();
        assert_eq!(got, vec![exit(7, "make", -1, &["line a", "", "line c"])]);
        let JobNotice::Exit(e) = &got[0] else {
            panic!()
        };
        assert!(!e.ok());
    }

    #[test]
    fn coalesced_prompt_with_dropped_line_and_backticks_in_command() {
        let text = format!(
            "{DROPPED}\n\n[background job 1 `echo `x`` exited 2]\nlast lines:\nboom\n\n[background job 2 `ls` exited 0]"
        );
        assert_eq!(
            parse_job_notices(&text),
            Some(vec![
                JobNotice::Dropped(3),
                exit(1, "echo `x`", 2, &["boom"]),
                exit(2, "ls", 0, &[]),
            ])
        );
    }

    #[test]
    fn ordinary_prompts_are_not_notices() {
        assert_eq!(parse_job_notices("fix the bug"), None);
        assert_eq!(parse_job_notices(""), None);
        assert_eq!(
            parse_job_notices("[background job 1 `ls` exited 0]\nand also please rename it"),
            None
        );
        assert_eq!(parse_job_notices("[background job x `ls` exited 0]"), None);
    }
}
