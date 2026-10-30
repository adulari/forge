//! `/name` lines that are not builtins: file commands and skills from the catalog
//! (docs/features/command-skill-system.md). Lives outside `dispatch.rs`, which sits at its size
//! ratchet.

use forge_skills::Resolved;

use super::one_shot_hooks::expansion_gate;
use super::skill_invoke;
use super::*;

/// Resolve a `/line` that isn't a builtin against the file catalog: expand a command, load a
/// skill's methodology, or report a missing-arg / unknown error. A project-scope definition is
/// gated on first use (re-run confirms) unless `trust_project`. The definition's frontmatter
/// grant (`allowed-tools`, `effort`, `context: fork`) is queued on the session for the turn this
/// starts, and its `` !`cmd` `` injections run before the model sees the text.
pub(crate) async fn dispatch_catalog(
    line: &str,
    catalog: &forge_skills::Catalog,
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    armed: &mut std::collections::HashSet<String>,
    trust_project: bool,
    busy: bool,
) -> Result<DispatchOutcome> {
    // The running turn holds the session lock for its whole length, so awaiting it here would stop
    // the render loop (no redraw, no keys, not even Esc), and deadlock for good once that turn
    // asks the UI a question or for a permission: the turn waits for the UI, the UI for the lock.
    // A command or skill is refused while busy anyway, so say so without the session.
    if busy {
        note_busy_line(line, catalog, app);
        return Ok(DispatchOutcome::Handled);
    }
    let (vars, workspace) = {
        let s = session.lock().await;
        (s.skill_template_vars(), s.workspace_root().to_path_buf())
    };
    match catalog.resolve_with(line, &vars) {
        Resolved::Command {
            cmd,
            prompt,
            guidance,
        } => {
            if !project_trust_ok(&cmd.name, cmd.scope, trust_project, armed, app) {
                return Ok(DispatchOutcome::Handled);
            }
            let Some(context) = expansion_gate(session, app, line, cmd.scope).await else {
                return Ok(DispatchOutcome::Handled);
            };
            app.note(&format!(
                "⚒ command · /{} ({})",
                cmd.name,
                cmd.scope.label()
            ));
            let grant = skill_invoke::grant_for(
                &format!("/{}", cmd.name),
                &cmd.invocation,
                cmd.scope,
                cmd.model.as_deref(),
                line,
                &workspace,
                &vars,
            );
            let Some((prompt, guidance)) =
                prepare_turn(session, app, &grant, prompt, guidance).await
            else {
                return Ok(DispatchOutcome::Handled);
            };
            queue_grant(session, app, grant).await;
            Ok(DispatchOutcome::RunTurn {
                prompt,
                guidance: [guidance, context].concat(),
                tier: cmd.tier,
            })
        }
        Resolved::Skill { meta, prompt } => {
            if !project_trust_ok(&meta.name, meta.scope, trust_project, armed, app) {
                return Ok(DispatchOutcome::Handled);
            }
            let Some(context) = expansion_gate(session, app, line, meta.scope).await else {
                return Ok(DispatchOutcome::Handled);
            };
            forge_config::skill_usage::record_skill_use(&meta.name);
            let skill = forge_skills::Skill::load(&meta);
            for w in &skill.warnings {
                app.note(&format!("⚠ {w}"));
            }
            app.note(&format!("⚒ skill · {} ({})", meta.name, meta.scope.label()));
            if !skill.resources.is_empty() {
                app.note(&format!(
                    "↳ loaded methodology + {} resource(s)",
                    skill.resources.len()
                ));
            }
            let skill_vars = vars.for_skill_dir(&meta.dir);
            let guidance = vec![skill.render(Some(&prompt), &vars)];
            let grant = skill_invoke::grant_for(
                &format!("skill {}", meta.name),
                &meta.invocation,
                meta.scope,
                meta.model.as_deref(),
                line,
                &workspace,
                &skill_vars,
            );
            let Some((prompt, guidance)) =
                prepare_turn(session, app, &grant, prompt, guidance).await
            else {
                return Ok(DispatchOutcome::Handled);
            };
            let guidance = [guidance, context].concat();
            let forked = grant.fork.is_some();
            queue_grant(session, app, grant).await;
            if prompt.trim().is_empty() && !forked {
                // No task given: prime the methodology into the transcript (no model call) so it
                // shapes the next turn the user types; the skill's grant waits for that turn.
                {
                    let mut s = session.lock().await;
                    s.prime_guidance(&guidance)
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                }
                app.note("↳ methodology primed — type your task");
                Ok(DispatchOutcome::Handled)
            } else {
                Ok(DispatchOutcome::RunTurn {
                    prompt,
                    guidance,
                    tier: meta.tier,
                })
            }
        }
        other => {
            note_unresolved(&other, line, app);
            Ok(DispatchOutcome::Handled)
        }
    }
}

/// What a `/line` that expands to nothing says: missing args, a model-only definition, an
/// unknown name.
fn note_unresolved(resolved: &Resolved, line: &str, app: &mut forge_tui::App) {
    match resolved {
        Resolved::MissingArgs { name, missing } => {
            let need = missing
                .iter()
                .map(|m| format!("<{m}>"))
                .collect::<Vec<_>>()
                .join(" ");
            app.note(&format!("/{name} requires {need}"));
        }
        Resolved::NotUserInvocable(name) => app.note(&format!(
            "/{name} is for the model only (user-invocable: false) — it loads it when relevant"
        )),
        Resolved::Unknown(x) => app.note(&super::dispatch_checks::unknown_command_note(x, line)),
        // A `/`-line never resolves to Plain, but stay safe rather than silently submit it.
        Resolved::Plain(_) => app.note("unknown command — try /help"),
        Resolved::Command { .. } | Resolved::Skill { .. } => {}
    }
}

/// A `/line` typed while a turn runs: a command or skill waits for the idle session, anything
/// else gets its usual note. Reads only the catalog, never the session.
fn note_busy_line(line: &str, catalog: &forge_skills::Catalog, app: &mut forge_tui::App) {
    match catalog.resolve(line) {
        Resolved::Command { .. } | Resolved::Skill { .. } => {
            app.note("⚠ finish or Esc the current turn first");
        }
        other => note_unresolved(&other, line, app),
    }
}

/// Run the invocation's `` !`cmd` `` injections, checked against the session's rules plus the
/// grant's own. `None` (after a note saying why) when a command was refused or failed, which
/// aborts the invocation as in Claude Code.
async fn prepare_turn(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    grant: &forge_core::TurnGrant,
    prompt: String,
    guidance: Vec<String>,
) -> Option<(String, Vec<String>)> {
    if !skill_invoke::needs_shell(&prompt, &guidance) {
        return Some((prompt, guidance));
    }
    let policy = session.lock().await.skill_shell_policy(&grant.rules);
    if !policy.trusted {
        app.note("⚠ skill shell commands skipped: this project is not trusted (`forge trust`)");
    }
    let rendered = tokio::task::spawn_blocking(move || {
        skill_invoke::run_injections(&prompt, &guidance, &policy)
    })
    .await;
    match rendered {
        Ok(Ok(ready)) => Some(ready),
        Ok(Err(why)) => {
            app.note(&format!("✗ {why}"));
            None
        }
        Err(e) => {
            app.note(&format!("✗ skill shell commands did not finish: {e}"));
            None
        }
    }
}

/// Queue `grant` for the turn about to start, and say what it changes.
async fn queue_grant(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    grant: forge_core::TurnGrant,
) {
    let summary = grant.describe();
    if !summary.is_empty() {
        app.note(&format!("↳ {summary}"));
    }
    if let Some(fork) = &grant.fork {
        app.note(&format!("↳ runs in a `{}` subagent", fork.agent));
    }
    session.lock().await.grant_next_turn(grant);
}
