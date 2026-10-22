//! One-shot CLI execution and command/skill prompt expansion.

use super::print_run::RunSetup;
use super::stream_input::{parse_input_line, InputLine};
use super::*;

/// Everything one `forge run` invocation asked for.
pub(crate) struct RunRequest {
    pub(crate) prompt: String,
    pub(crate) mock: bool,
    pub(crate) mode: Option<Mode>,
    pub(crate) tui: bool,
    pub(crate) resume: Option<String>,
    pub(crate) pin: Option<String>,
    /// Standing system messages: `--system`/`--append-system-prompt[-file]` and the `--add-dir`
    /// note.
    pub(crate) system: Vec<String>,
    pub(crate) output_format: OutputFormat,
    pub(crate) publish_to_fleet: bool,
    /// Keep the run in this process: `--no-publish-to-fleet`, or a flag a daemon cannot honour.
    pub(crate) local_only: bool,
    pub(crate) usage_json: Option<std::path::PathBuf>,
    /// `-p`: stdout carries only the final answer.
    pub(crate) print: bool,
    pub(crate) verbose: bool,
    pub(crate) partial_messages: bool,
    /// `--input-format stream-json`: further prompts arrive as JSON lines on stdin.
    pub(crate) stream_input: bool,
    /// `--permission-mode dontAsk`: deny whatever is not pre-approved, even at a terminal.
    pub(crate) never_ask: bool,
    pub(crate) setup: RunSetup,
}

pub(crate) async fn run(req: RunRequest) -> Result<()> {
    let RunRequest {
        prompt,
        mock,
        mode,
        tui,
        resume,
        pin,
        system,
        output_format,
        publish_to_fleet,
        local_only,
        usage_json,
        print,
        verbose,
        partial_messages,
        stream_input,
        never_ask,
        mut setup,
    } = req;
    // With stream-json input, stdin is the message stream, not text to append to the prompt.
    let prompt = if stream_input {
        prompt
    } else {
        with_piped_stdin(prompt)?
    };
    if prompt.trim().is_empty() && !stream_input {
        anyhow::bail!("empty prompt — usage: forge run \"<your task>\"");
    }
    // A first-time user's `forge run "hi"` would otherwise dead-end with no provider; offer the
    // guided wizard (no-ops on non-tty / once configured), same as `chat()`.
    maybe_first_run_setup(mock)?;

    // One-shot slash support: `forge run "/rust <task>"` expands a catalog command/skill exactly
    // like the interactive dispatcher — without this the literal `/rust` reaches the model as
    // prose and it guesses at the intent. Unknown tokens (absolute paths, TUI-only builtins)
    // pass through verbatim; `//…` escapes a literal leading slash, mirroring chat.
    // `forge run "/goal …"` is a loop, not a prompt: it is published to a running daemon as-is
    // and otherwise driven locally by `run_goal_headless` (docs/features/goal-evaluator.md).
    let goal_spec = parse_goal_prompt(&prompt);
    match prompt.split_whitespace().next() {
        Some("/goal") if goal_spec.is_none() => anyhow::bail!(
            "could not parse the /goal command: --max-tokens and --max-minutes need whole numbers"
        ),
        Some("/loop") => anyhow::bail!(
            "/loop is interactive only; use `forge run \"/goal ...\"` for an unattended loop or `forge schedule` for a recurring run"
        ),
        _ => {}
    }
    let (prompt, command_guidance, tier) = if goal_spec.is_some() || prompt.trim().is_empty() {
        (prompt.trim().to_string(), Vec::new(), None)
    } else {
        expand_one_shot_slash(&prompt)?
    };
    let mut guidance = system;
    guidance.extend(command_guidance);
    let at_images = if goal_spec.is_none() {
        super::atfiles::at_image_attachments(&prompt)
    } else {
        Vec::new()
    };
    if goal_spec.is_none() && !prompt.is_empty() {
        let (file_blocks, included, skipped) = expand_at_files(&prompt);
        if !included.is_empty() && !output_format.is_machine() && !print {
            eprintln!("included {}", included.join(", "));
        }
        for s in &skipped {
            eprintln!("skipped {s}");
        }
        guidance.extend(file_blocks);
    }

    // `--output-format stream-json` is a CONTRACT: the caller is a program that will parse stdout.
    // Publishing to the fleet answers with two lines of prose and exit 0 instead, so every such
    // caller sees a successful run and no events — silently, and only on machines that happen to
    // have a daemon reachable. An explicit machine-readable format therefore opts out of the
    // fleet; `forge attach` is still how a human follows a run they started that way.
    // `--usage-json` reports THIS process's session, so a run handed to the daemon would leave
    // the caller with nothing to read. The same holds for `-p` and every flag the daemon cannot
    // honour (`local_only`), and for `--resume`/`--continue`, whose session lives here.
    if let Some(session_id) = maybe_publish_run_to_fleet(
        &prompt,
        pin.as_deref(),
        publish_to_fleet,
        local_only || usage_json.is_some() || resume.is_some(),
        output_format,
    )
    .await
    {
        println!("run executing in daemon session {session_id}");
        println!("follow it with `forge attach {session_id}` or from the Anywhere apps");
        return Ok(());
    }

    let started = std::time::Instant::now();
    let write_usage = |session: &Session, before: &forge_core::SessionUsageDetail| -> Result<()> {
        match &usage_json {
            Some(path) => {
                usage_json::write_usage_json(session, before, path, started.elapsed().as_secs_f64())
            }
            None => Ok(()),
        }
    };
    let first = (!prompt.is_empty()).then(|| OneTurn {
        prompt: prompt.clone(),
        guidance: guidance.clone(),
        tier,
        goal: goal_spec,
    });

    // Machine formats: a JSON result (`json`) or NDJSON events (`stream-json`) on stdout — no
    // TUI, no heartbeat, stdout stays machine-readable. Ctrl-C still returns partial output.
    if output_format.is_machine() {
        let (presenter, init): (Box<dyn Presenter>, Option<forge_tui::StreamInitSlot>) =
            if output_format == OutputFormat::Json {
                (Box::new(forge_tui::JsonResultPresenter::new()), None)
            } else {
                let (presenter, slot) = forge_tui::StreamJsonPresenter::new()
                    .with_partial_messages(partial_messages)
                    .with_deferred_init();
                (Box::new(presenter), Some(slot))
            };
        let mut session = match build_session_with(presenter, mock, mode, resume, pin, true).await {
            Ok(session) => session,
            Err(e) => {
                println!("{}", forge_tui::error_result_line(&format!("{e:#}")));
                return Err(e);
            }
        };
        if let Some(slot) = init {
            slot.fill(super::print_run::stream_init(&session));
        }
        if let Err(e) = setup.apply(&mut session) {
            println!("{}", forge_tui::error_result_line(&format!("{e:#}")));
            setup.finish(&session);
            return Err(e);
        }
        session.attach_images(at_images);
        session.set_one_shot(true);
        let before = session.session_usage_detail();
        // A resumed session's restored totals arrive before routing; the presenter keeps them as
        // a baseline so `usage` reports this invocation, as `claude -p` does.
        session.emit_restored_totals();
        let result = drive(&mut session, &setup, first, stream_input, &guidance, true).await;
        return finish_run(&session, &setup, result, write_usage(&session, &before));
    }

    if tui && first.as_ref().is_some_and(|t| t.goal.is_some()) {
        anyhow::bail!("`forge run --tui` does not run /goal as a loop — use `forge chat`");
    }
    let mut session = if print {
        let presenter = Box::new(forge_tui::PrintPresenter::new(verbose, !never_ask));
        build_session_with(presenter, mock, mode, resume, pin, true).await?
    } else if never_ask && !tui {
        let presenter = Box::new(forge_tui::HeadlessPresenter::new(false));
        build_session_with(presenter, mock, mode, resume, pin, false).await?
    } else {
        build_session(mock, mode, tui, resume, pin).await?
    };
    if let Err(e) = setup.apply(&mut session) {
        setup.finish(&session);
        return Err(e);
    }
    session.attach_images(at_images);
    session.set_one_shot(!tui);
    let before = session.session_usage_detail();

    // TUI mode handles its own Ctrl-C (crossterm) + spinner; keep it unchanged.
    if tui {
        setup.arm_budget(&mut session);
        let result = session
            .run_turn_with(&prompt, &guidance, tier)
            .await
            .context("running agent turn");
        let written = write_usage(&session, &before);
        setup.finish(&session);
        result?;
        written?;
        // Hold the final frame until the user quits (Esc / Ctrl-C).
        let _ = session.read_line();
        return Ok(());
    }

    // Headless heartbeat: a long model call streams nothing until the first token, so tick
    // "working… Ns" to stderr to show the turn is alive. Skipped for `--mock` (instant).
    let heartbeat = (!mock && std::io::IsTerminal::is_terminal(&std::io::stderr())).then(|| {
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
    let result = drive(&mut session, &setup, first, stream_input, &guidance, false).await;
    if let Some(h) = heartbeat {
        h.abort();
        eprint!("\r\x1b[K"); // clear the heartbeat line
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }
    finish_run(&session, &setup, result, write_usage(&session, &before))
}

/// One prompt to run as a turn (or, with `goal`, as a `/goal` loop).
struct OneTurn {
    prompt: String,
    guidance: Vec<String>,
    tier: Option<forge_types::TaskTier>,
    goal: Option<GoalRunSpec>,
}

/// Run the first prompt, then (stream-json input) every user message on stdin. `Ok(true)` means
/// the run was interrupted. A failed turn in a message stream does not end the stream; the run
/// still fails at the end.
async fn drive(
    session: &mut Session,
    setup: &RunSetup,
    first: Option<OneTurn>,
    stream_input: bool,
    guidance: &[String],
    machine: bool,
) -> Result<bool> {
    let mut failure = None;
    if let Some(turn) = first {
        match drive_turn(session, setup, turn, machine).await {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(e) if stream_input => failure = Some(e),
            Err(e) => return Err(e),
        }
    }
    if !stream_input {
        return Ok(false);
    }
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        let line = tokio::select! {
            line = lines.next_line() => line.context("reading stream-json input")?,
            _ = interrupt_signal() => return Ok(true),
        };
        let Some(line) = line else { break };
        let text = match parse_input_line(&line) {
            Ok(InputLine::Prompt(text)) => text,
            Ok(InputLine::Control(reply)) => {
                println!("{reply}");
                continue;
            }
            Ok(InputLine::Ignore) => continue,
            Err(msg) => {
                eprintln!("warning: --input-format stream-json: {msg}");
                continue;
            }
        };
        let turn = match expand_one_shot_slash(&text) {
            Ok((prompt, extra, tier)) => OneTurn {
                prompt,
                guidance: guidance.iter().cloned().chain(extra).collect(),
                tier,
                goal: None,
            },
            Err(e) => {
                failure = Some(e);
                continue;
            }
        };
        match drive_turn(session, setup, turn, machine).await {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(e) => failure = Some(e),
        }
    }
    failure.map_or(Ok(false), Err)
}

/// One turn raced against Ctrl-C, so a hard kill doesn't discard partial output: on interrupt the
/// turn future is dropped (it stops at its next await) and what already streamed is kept.
async fn drive_turn(
    session: &mut Session,
    setup: &RunSetup,
    turn: OneTurn,
    machine: bool,
) -> Result<bool> {
    setup.arm_budget(session);
    if let Some(spec) = turn.goal {
        let cwd = std::env::current_dir().unwrap_or_default();
        let end = run_goal_headless(session, spec, &turn.guidance, &cwd, interrupt_signal()).await;
        return end.and_then(GoalRunEnd::into_result).map(|()| false);
    }
    let run = session.run_turn_with(&turn.prompt, &turn.guidance, turn.tier);
    tokio::pin!(run);
    tokio::select! {
        r = &mut run => match r.context("running agent turn") {
            Ok(outcome) => fail_if_incomplete(outcome).map(|()| false),
            Err(e) => {
                if machine {
                    println!("{}", forge_tui::error_result_line(&format!("{e:#}")));
                }
                Err(e)
            }
        },
        _ = interrupt_signal() => {
            if machine {
                println!("{}", forge_tui::error_result_line("interrupted"));
            } else {
                eprintln!("\r\x1b[K\x1b[2m⧖ interrupted — turn stopped (what streamed above is not saved to the session)\x1b[0m");
            }
            Ok(true)
        }
    }
}

/// The run is over: forget an unsaved session, then report the turn's outcome. An interrupted run
/// exits 128 + SIGINT; a script must not read an aborted turn as success.
fn finish_run(
    session: &Session,
    setup: &RunSetup,
    result: Result<bool>,
    written: Result<()>,
) -> Result<()> {
    setup.finish(session);
    let interrupted = result?;
    written?;
    if interrupted {
        std::process::exit(130);
    }
    Ok(())
}

/// Resolves on Ctrl-C or (unix) SIGTERM, so a `kill` or a supervisor's stop is handled like Ctrl-C.
async fn interrupt_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

/// `cat file | forge run "summarize"` (like `claude -p`): piped stdin is appended to the prompt, or
/// is the prompt when none was given. A terminal stdin is never read.
fn with_piped_stdin(prompt: String) -> Result<String> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return Ok(prompt);
    }
    let mut piped = String::new();
    std::io::stdin()
        .read_to_string(&mut piped)
        .context("reading piped stdin")?;
    Ok(combine_prompt_and_stdin(prompt, &piped))
}

fn combine_prompt_and_stdin(prompt: String, piped: &str) -> String {
    let piped = piped.trim_end();
    match (prompt.trim().is_empty(), piped.is_empty()) {
        (_, true) => prompt,
        (true, false) => piped.to_string(),
        (false, false) => format!("{prompt}\n\n{piped}"),
    }
}

#[cfg(test)]
mod stdin_tests {
    use super::combine_prompt_and_stdin as combine;

    #[test]
    fn stdin_becomes_or_extends_the_prompt() {
        assert_eq!(combine(String::new(), "data\n"), "data");
        assert_eq!(combine("sum".into(), "data\n"), "sum\n\ndata");
        assert_eq!(combine("sum".into(), ""), "sum");
    }
}

/// A headless one-shot run is unattended: a turn that stopped with tracked tasks still open has
/// NOT done the job, and exiting 0 makes an orchestrator (or a human reading `$?`) treat it as if
/// it had. Turn it into a process failure carrying the harness's ERROR line.
fn fail_if_incomplete(outcome: forge_types::LoopOutcome) -> Result<()> {
    use forge_types::StopReason;
    if matches!(
        outcome.stop_reason,
        StopReason::TasksUnfinished
            | StopReason::MaxSteps
            | StopReason::BudgetExhausted
            | StopReason::NoOutput
            | StopReason::DaemonShutdown
    ) {
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
    output_format: OutputFormat,
) -> Option<String> {
    if output_format.is_machine() {
        return None;
    }
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

    /// `--output-format stream-json` promises a machine-readable stdout. Publishing to the fleet
    /// answers with prose and exit 0 instead, so a parsing caller sees a successful run and no
    /// events — and only on machines where a daemon happens to be reachable, which is why this
    /// went unnoticed: CI has no daemon, so the e2e test passed everywhere except a developer's
    /// own box. The opt-out must come BEFORE any config or network work.
    #[tokio::test]
    async fn stream_json_never_hands_the_run_to_the_fleet() {
        assert!(
            maybe_publish_run_to_fleet("hi", None, true, false, OutputFormat::StreamJson)
                .await
                .is_none(),
            "an explicit machine-readable format must not be answered with prose"
        );
    }
}
