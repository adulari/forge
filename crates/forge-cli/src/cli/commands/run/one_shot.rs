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

    // stream-json: emit NDJSON events on stdout via the StreamJsonPresenter (no TUI, no heartbeat —
    // stdout stays a clean machine-readable event stream). Ctrl-C still returns partial output.
    if output_format == OutputFormat::StreamJson {
        let presenter: Box<dyn Presenter> = Box::new(forge_tui::StreamJsonPresenter::new());
        let mut session = build_session_with(presenter, mock, mode, resume, pin, true).await?;
        let turn = session.run_turn_with(&prompt, &guidance, tier);
        tokio::pin!(turn);
        let result = tokio::select! {
            r = &mut turn => r.map(|_| ()).context("running agent turn"),
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
            r = &mut turn => r.map(|_| ()).context("running agent turn"),
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
