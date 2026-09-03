//! One-shot CLI execution and command/skill prompt expansion.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    prompt: String,
    mock: bool,
    mode: Option<Mode>,
    tui: bool,
    resume: Option<String>,
    pin: Option<String>,
    system: Vec<String>,
    output_format: OutputFormat,
    publish_to_fleet: bool,
    no_publish_to_fleet: bool,
) -> Result<()> {
    if prompt.trim().is_empty() {
        anyhow::bail!("empty prompt — usage: forge run \"<your task>\"");
    }
    // A first-time user's `forge run "hi"` would otherwise dead-end with no provider; offer the
    // guided wizard (no-ops on non-tty / once configured), same as `chat()`.
    maybe_first_run_setup(mock)?;

    // One-shot slash support: `forge run "/rust <task>"` expands a catalog command/skill exactly
    // like the interactive dispatcher — without this the literal `/rust` reaches the model as
    // prose and it guesses at the intent. Unknown tokens (absolute paths, TUI-only builtins)
    // pass through verbatim; `//…` escapes a literal leading slash, mirroring chat.
    let (prompt, command_guidance, tier) = expand_one_shot_slash(&prompt)?;
    let mut guidance = system;
    guidance.extend(command_guidance);

    if let Some(session_id) = maybe_publish_run_to_fleet(
        &prompt,
        pin.as_deref(),
        publish_to_fleet,
        no_publish_to_fleet,
    )
    .await
    {
        println!("run executing in daemon session {session_id}");
        println!("follow it with `forge attach {session_id}` or from the Anywhere apps");
        return Ok(());
    }

    // stream-json: emit NDJSON events on stdout via the StreamJsonPresenter (no TUI, no heartbeat —
    // stdout stays a clean machine-readable event stream). Ctrl-C still returns partial output.
    if output_format == OutputFormat::StreamJson {
        let presenter: Box<dyn Presenter> = Box::new(forge_tui::StreamJsonPresenter::new());
        let mut session = build_session_with(presenter, mock, mode, resume, pin, true).await?;
        let turn = session.run_turn_with(&prompt, &guidance, tier);
        tokio::pin!(turn);
        let result = tokio::select! {
            r = &mut turn => r.context("running agent turn").and_then(fail_if_incomplete),
            _ = tokio::signal::ctrl_c() => Ok(()),
        };
        result?;
        return Ok(());
    }

    let mut session = build_session(mock, mode, tui, resume, pin).await?;

    // TUI mode handles its own Ctrl-C (crossterm) + spinner; keep it unchanged.
    if tui {
        session
            .run_turn_with(&prompt, &guidance, tier)
            .await
            .context("running agent turn")?;
        // Hold the final frame until the user quits (Esc / Ctrl-C).
        let _ = session.read_line();
        return Ok(());
    }

    // Headless heartbeat: a long model call streams nothing until the first token, so tick
    // "working… Ns" to stderr to show the turn is alive. Skipped for `--mock` (instant).
    let heartbeat = (!mock).then(|| {
        tokio::spawn(async {
            let start = std::time::Instant::now();
            let mut iv = tokio::time::interval(std::time::Duration::from_secs(2));
            iv.tick().await; // immediate first tick — skip it
            loop {
                iv.tick().await;
                eprint!("\r\x1b[2m⧖ working… {}s\x1b[0m", start.elapsed().as_secs());
                let _ = std::io::Write::flush(&mut std::io::stderr());
            }
        })
    });

    // Race the turn against Ctrl-C so a hard kill doesn't discard partial output: on interrupt we
    // drop the turn future (it stops at its next await) and return what already streamed.
    let result = {
        let turn = session.run_turn_with(&prompt, &guidance, tier);
        tokio::pin!(turn);
        tokio::select! {
            r = &mut turn => r.context("running agent turn").and_then(fail_if_incomplete),
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\r\x1b[K\x1b[2m⧖ interrupted — stopping turn (partial output kept)\x1b[0m");
                Ok(())
            }
        }
    };
    if let Some(h) = heartbeat {
        h.abort();
        eprint!("\r\x1b[K"); // clear the heartbeat line
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }
    result?;
    Ok(())
}

/// A headless one-shot run is unattended: a turn that stopped with tracked tasks still open has
/// NOT done the job, and exiting 0 makes an orchestrator (or a human reading `$?`) treat it as if
/// it had. Turn it into a process failure carrying the harness's ERROR line.
fn fail_if_incomplete(outcome: forge_types::LoopOutcome) -> Result<()> {
    if outcome.stop_reason == forge_types::StopReason::TasksUnfinished {
        anyhow::bail!(outcome.text);
    }
    Ok(())
}

/// Run this one-shot prompt IN the local daemon so the fleet entry IS the run. Returns the
/// new daemon session id on success. Fail-soft: any failure (publishing off, daemon down,
/// non-2xx, malformed body) logs once at debug level and returns None, and the caller runs the
/// turn locally exactly as if publishing were off.
async fn maybe_publish_run_to_fleet(
    prompt: &str,
    model: Option<&str>,
    publish_to_fleet: bool,
    no_publish_to_fleet: bool,
) -> Option<String> {
    let configured = forge_config::load()
        .map(|c| c.remote.publish_local_runs)
        .unwrap_or(true);
    if !crate::cli::dispatch::resolve_publish_to_fleet(
        publish_to_fleet,
        no_publish_to_fleet,
        configured,
    ) {
        return None;
    }
    let base = crate::attach::resolve_base_url(None);
    let Ok(token) = crate::attach::resolve_token(None) else {
        return None;
    };
    match publish_run_to_fleet(prompt, model, &base, &token).await {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::debug!("fleet publish of `forge run` failed: {e:#}");
            None
        }
    }
}

async fn publish_run_to_fleet(
    prompt: &str,
    model: Option<&str>,
    base: &str,
    token: &str,
) -> anyhow::Result<String> {
    let title: String = prompt
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(60)
        .collect();
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{base}/{token}/api/sessions"))
        .json(&serde_json::json!({
            "cwd": cwd,
            "title": title,
            "model": model,
        }))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("daemon returned {}", resp.status());
    }
    let created: serde_json::Value = resp.json().await?;
    let id = created
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("daemon create-session response had no id"))?
        .to_string();
    let resp = http
        .post(format!("{base}/{token}/api/sessions/{id}/message"))
        .json(&serde_json::json!({ "text": prompt }))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("daemon returned {}", resp.status());
    }
    resp.json::<serde_json::Value>().await?;
    Ok(id)
}

/// Resolve a one-shot `/command` or skill prompt against the file catalog (the same
/// `Catalog::resolve` the interactive dispatcher uses) so `forge run "/rust <task>"` behaves like
/// typing it in the TUI. Returns `(prompt, guidance, tier)`:
/// - `//foo` escapes to a literal `/foo` prompt (mirrors chat).
/// - A command expands to its prompt (+ guidance/tier); a skill with a task runs the task under
///   the skill's methodology.
/// - A bare skill or missing args is a usage error — one-shot has no "next turn" to prime.
/// - Project-scope definitions require `[commands] trust_project = true`: the interactive
///   run-again-to-confirm gate has no headless equivalent, so default-deny with a pointer.
/// - Anything unresolved (absolute paths, unknown tokens, TUI-only builtins) passes through
///   verbatim.
pub(super) fn expand_one_shot_slash(
    raw: &str,
) -> Result<(String, Vec<String>, Option<forge_types::TaskTier>)> {
    let t = raw.trim();
    if let Some(rest) = t.strip_prefix("//") {
        return Ok((format!("/{rest}"), Vec::new(), None));
    }
    if !t.starts_with('/') {
        return Ok((t.to_string(), Vec::new(), None));
    }
    let catalog = forge_skills::Catalog::load(&forge_config::command_sources());
    let trust_project = forge_config::load()
        .map(|c| c.commands.trust_project)
        .unwrap_or(false);
    use forge_skills::Resolved;
    match catalog.resolve(t) {
        Resolved::Command {
            cmd,
            prompt,
            guidance,
        } => {
            if cmd.scope == forge_skills::Scope::Project && !trust_project {
                anyhow::bail!(
                    "/{} is a project-scope command — set `[commands] trust_project = true` in \
                     your config, or run it interactively via `forge chat`",
                    cmd.name
                );
            }
            eprintln!("⚒ command · /{} ({})", cmd.name, cmd.scope.label());
            Ok((prompt, guidance, cmd.tier))
        }
        Resolved::Skill { meta, prompt } => {
            if meta.scope == forge_skills::Scope::Project && !trust_project {
                anyhow::bail!(
                    "/{} is a project-scope skill — set `[commands] trust_project = true` in \
                     your config, or run it interactively via `forge chat`",
                    meta.name
                );
            }
            if prompt.trim().is_empty() {
                anyhow::bail!(
                    "skill /{name} needs a task in one-shot mode — usage: forge run \
                     \"/{name} <task>\"",
                    name = meta.name
                );
            }
            let skill = forge_skills::Skill::load(&meta);
            for w in &skill.warnings {
                eprintln!("⚠ {w}");
            }
            eprintln!("⚒ skill · {} ({})", meta.name, meta.scope.label());
            Ok((prompt, vec![skill.guidance()], meta.tier))
        }
        Resolved::MissingArgs { name, missing } => {
            let need = missing
                .iter()
                .map(|m| format!("<{m}>"))
                .collect::<Vec<_>>()
                .join(" ");
            anyhow::bail!("/{name} requires {need}")
        }
        Resolved::Unknown(_) => Ok((t.to_string(), Vec::new(), None)),
        // Unreachable here (the early returns above cover non-slash + `//` escapes), but the
        // catalog's own contract for it is "pass straight to run_turn" — honor that.
        Resolved::Plain(p) => Ok((p, Vec::new(), None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_run_to_fleet_errors_when_no_daemon_listens() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let base = format!("http://127.0.0.1:{port}");
        assert!(publish_run_to_fleet("hello", None, &base, "tok")
            .await
            .is_err());
    }
}
