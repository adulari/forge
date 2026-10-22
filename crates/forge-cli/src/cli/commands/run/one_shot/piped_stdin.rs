//! `cat file | forge run "summarize"`: piped stdin joins the prompt, but only if it shows up.
//!
//! A supervisor that spawns `forge run "task"` with an open pipe on stdin and never writes or
//! closes it would otherwise hang the run forever in `read_to_string`. Like `claude -p`, the read
//! therefore waits a bounded time for the first byte (or EOF); once data has started to flow the
//! producer is alive and the read runs to EOF.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// How long a run that already has a prompt waits for a piped stdin to produce anything.
const WAIT_WITH_PROMPT: Duration = Duration::from_secs(3);
/// With no prompt argument stdin is the only source, so a slow producer gets longer; it is still
/// bounded so an idle pipe ends in an error instead of a hang.
const WAIT_WITHOUT_PROMPT: Duration = Duration::from_secs(30);

pub(super) fn with_piped_stdin(prompt: String) -> Result<String> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return Ok(prompt);
    }
    let has_prompt = !prompt.trim().is_empty();
    let wait = if has_prompt {
        WAIT_WITH_PROMPT
    } else {
        WAIT_WITHOUT_PROMPT
    };
    if !stdin_ready_within(wait) {
        if !has_prompt {
            anyhow::bail!(
                "empty prompt — nothing arrived on stdin within {}s; usage: forge run \"<your task>\"",
                wait.as_secs()
            );
        }
        eprintln!(
            "warning: no stdin data within {}s, proceeding without it. For a slow command, save its \
             output to a file and redirect it (`< file`), or pass `< /dev/null` to skip the wait.",
            wait.as_secs()
        );
        return Ok(prompt);
    }
    let mut piped = String::new();
    std::io::stdin()
        .read_to_string(&mut piped)
        .context("reading piped stdin")?;
    Ok(combine_prompt_and_stdin(prompt, &piped))
}

pub(super) fn combine_prompt_and_stdin(prompt: String, piped: &str) -> String {
    let piped = piped.trim_end();
    match (prompt.trim().is_empty(), piped.is_empty()) {
        (_, true) => prompt,
        (true, false) => piped.to_string(),
        (false, false) => format!("{prompt}\n\n{piped}"),
    }
}

#[cfg(unix)]
fn stdin_ready_within(wait: Duration) -> bool {
    fd_ready_within(libc::STDIN_FILENO, wait)
}

/// Whether a read on `fd` would return now: data, EOF (hang-up) or an error all count, because
/// the read that follows reports each of them. Only a silent, still-open pipe is "not ready".
#[cfg(unix)]
fn fd_ready_within(fd: std::os::fd::RawFd, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let millis = left.as_nanos().div_ceil(1_000_000).min(i32::MAX as u128) as libc::c_int;
        let mut probe = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `probe` is one valid, initialised pollfd and nfds is 1.
        let ready = unsafe { libc::poll(&mut probe, 1, millis) };
        match ready {
            0 if Instant::now() >= deadline => return false,
            0 => {}
            n if n > 0 => return true,
            _ if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted => {}
            _ => return true,
        }
    }
}

/// No portable readiness probe: keep the plain blocking read off Unix.
#[cfg(not(unix))]
fn stdin_ready_within(_wait: Duration) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdin_becomes_or_extends_the_prompt() {
        let combine = combine_prompt_and_stdin;
        assert_eq!(combine(String::new(), "data\n"), "data");
        assert_eq!(combine("sum".into(), "data\n"), "sum\n\ndata");
        assert_eq!(combine("sum".into(), ""), "sum");
    }

    #[cfg(unix)]
    mod readiness {
        use super::super::fd_ready_within;
        use std::io::Write;
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;
        use std::time::{Duration, Instant};

        #[test]
        fn a_silent_open_pipe_times_out_instead_of_blocking() {
            let (reader, _writer) = UnixStream::pair().unwrap();
            let started = Instant::now();
            assert!(!fd_ready_within(
                reader.as_raw_fd(),
                Duration::from_millis(80)
            ));
            let took = started.elapsed();
            assert!(
                took >= Duration::from_millis(80),
                "returned early: {took:?}"
            );
            assert!(took < Duration::from_secs(2), "overslept: {took:?}");
        }

        #[test]
        fn data_that_arrives_during_the_wait_is_ready() {
            let (reader, mut writer) = UnixStream::pair().unwrap();
            let feeder = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(40));
                writer.write_all(b"x").unwrap();
                writer
            });
            assert!(fd_ready_within(reader.as_raw_fd(), Duration::from_secs(5)));
            drop(feeder.join().unwrap());
        }

        #[test]
        fn a_closed_writer_is_ready_so_the_read_sees_eof_at_once() {
            let (reader, writer) = UnixStream::pair().unwrap();
            drop(writer);
            let started = Instant::now();
            assert!(fd_ready_within(reader.as_raw_fd(), Duration::from_secs(5)));
            assert!(started.elapsed() < Duration::from_secs(1));
        }

        #[test]
        fn a_closed_descriptor_is_ready_so_the_read_reports_it() {
            let (reader, _writer) = UnixStream::pair().unwrap();
            let fd = reader.as_raw_fd();
            drop(reader);
            assert!(fd_ready_within(fd, Duration::from_secs(5)));
        }
    }
}
