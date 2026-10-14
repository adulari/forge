//! Running `git` and `gh` without a TTY: no credential prompt, no pager, a hard timeout, and the
//! failure text a model can act on.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt as _;

const TIMEOUT: Duration = Duration::from_secs(180);

pub(super) struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run `program args…` in `cwd`. `Err` only when it could not run at all (missing binary,
/// timeout); a non-zero exit comes back as `ok: false` with its stderr.
pub(super) async fn run_raw(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    env: &[(&str, &str)],
    stdin: Option<&str>,
) -> Result<Output, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    for (key, value) in env {
        cmd.env(key, value);
    }
    let shown = program.display();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not run {shown}: {e}"))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes())
            .await
            .map_err(|e| format!("{shown}: writing stdin: {e}"))?;
    }
    let output = tokio::time::timeout(TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("{shown} {} timed out", args.join(" ")))?
        .map_err(|e| format!("{shown}: {e}"))?;
    Ok(Output {
        ok: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

/// [`run_raw`] that treats a non-zero exit as an error and returns the trimmed stdout.
pub(super) async fn run(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    env: &[(&str, &str)],
) -> Result<String, String> {
    run_with_stdin(program, args, cwd, env, None).await
}

pub(super) async fn run_with_stdin(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    env: &[(&str, &str)],
    stdin: Option<&str>,
) -> Result<String, String> {
    let out = run_raw(program, args, cwd, env, stdin).await?;
    if out.ok {
        return Ok(out.stdout);
    }
    let detail = if out.stderr.is_empty() {
        out.stdout
    } else {
        out.stderr
    };
    Err(format!(
        "`{} {}` failed: {detail}",
        program.file_name().map_or_else(
            || program.display().to_string(),
            |n| n.to_string_lossy().into_owned()
        ),
        args.join(" ")
    ))
}
