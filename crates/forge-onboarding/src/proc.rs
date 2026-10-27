//! Small child-process helpers shared by detection and action jobs.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Run `program args` and return `(success, stdout)`, or `None` when it cannot start or does not
/// finish inside `timeout` (the child is killed). Never inherits stdio.
pub(crate) fn run_capture(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Option<(bool, String)> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let status = wait_with_deadline(&mut child, timeout)?;
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    Some((status, out))
}

fn wait_with_deadline(child: &mut Child, timeout: Duration) -> Option<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Run `program args`, streaming each line of stdout and stderr into `job` as it arrives. A
/// non-zero exit is an error carrying the last lines of output.
pub(crate) fn stream_child(
    job: &crate::job::JobHandle,
    program: &str,
    args: &[String],
) -> Result<(), String> {
    use std::io::{BufRead, BufReader};
    use std::sync::{Arc, Mutex};

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {program}: {e}"))?;
    let tail: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut readers = Vec::new();
    let mut pipe = |reader: Box<dyn Read + Send>| {
        let job = job.clone();
        let tail = tail.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(reader).lines().map_while(Result::ok) {
                let line = line.trim_end().to_string();
                if line.is_empty() {
                    continue;
                }
                job.output(&line);
                let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                tail.push(line);
                if tail.len() > 4 {
                    tail.remove(0);
                }
            }
        }));
    };
    if let Some(out) = child.stdout.take() {
        pipe(Box::new(out));
    }
    if let Some(err) = child.stderr.take() {
        pipe(Box::new(err));
    }
    let status = child
        .wait()
        .map_err(|e| format!("waiting for {program}: {e}"))?;
    for reader in readers {
        let _ = reader.join();
    }
    if status.success() {
        return Ok(());
    }
    let tail = tail.lock().unwrap_or_else(|e| e.into_inner()).join("\n");
    Err(if tail.is_empty() {
        format!("{program} exited with {status}")
    } else {
        tail
    })
}

/// The first line of `<program> --version`, trimmed to the version token when it has one
/// (`2.1.4 (Claude Code)` -> `2.1.4`).
pub(crate) fn version_of(program: &str, timeout: Duration) -> Option<String> {
    let (ok, out) = run_capture(program, &["--version"], timeout)?;
    if !ok {
        return None;
    }
    parse_version(&out)
}

fn parse_version(out: &str) -> Option<String> {
    let line = out.lines().map(str::trim).find(|l| !l.is_empty())?;
    let token = line
        .split_whitespace()
        .map(|t| t.trim_start_matches('v'))
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or(line);
    Some(token.to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_quick_command_returns_its_output() {
        let (ok, out) = run_capture("sh", &["-c", "echo hi"], Duration::from_secs(2)).unwrap();
        assert!(ok);
        assert_eq!(out.trim(), "hi");
    }

    #[test]
    fn a_slow_command_is_killed_at_the_deadline() {
        let start = Instant::now();
        assert!(run_capture("sh", &["-c", "sleep 5"], Duration::from_millis(100)).is_none());
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_missing_program_is_none() {
        assert!(run_capture("forge-no-such-program-xyz", &[], Duration::from_secs(1)).is_none());
    }

    #[test]
    fn the_version_token_is_picked_out_of_the_first_line() {
        assert_eq!(
            parse_version("2.1.4 (Claude Code)\n").as_deref(),
            Some("2.1.4")
        );
        assert_eq!(parse_version("codex-cli 0.46.0").as_deref(), Some("0.46.0"));
        assert_eq!(
            parse_version("ollama version is 0.5.1").as_deref(),
            Some("0.5.1")
        );
        assert_eq!(parse_version("v1.2").as_deref(), Some("1.2"));
        assert_eq!(parse_version("weird").as_deref(), Some("weird"));
        assert_eq!(parse_version("\n"), None);
    }
}
