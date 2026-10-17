//! Interactive chat entry selection and plain line-mode lifecycle.

use super::*;

pub(crate) async fn chat(
    mock: bool,
    mode: Option<Mode>,
    resume_mode: ResumeMode,
    plain: bool,
    fullscreen: bool,
    pin: Option<String>,
) -> Result<()> {
    maybe_first_run_setup(mock)?;
    maybe_autostart_local();
    // Default to the interactive (animated) TUI on a real terminal.
    let dumb_term = std::env::var("TERM").is_ok_and(|t| t == "dumb");
    if !plain && !dumb_term && std::io::stdout().is_terminal() {
        // Update check happens in background inside run_chat_tui (via the UiMsg channel) so it
        // never delays TUI startup. The check has a 3s network timeout — blocking here would
        // freeze the terminal for up to 3s once per day.
        let ran = run_chat_tui(mock, mode, resume_mode, fullscreen, pin).await;
        return match super::super::switch::take_requested() {
            Some(id) if ran.is_ok() => {
                super::super::switch::switch_forge_session_to_claude(&id).await
            }
            _ => ran,
        };
    }
    // Plain path: blocking update check is fine (no TUI to corrupt).
    update_check::maybe_notify(&forge_config::load().unwrap_or_default()).await;

    // Plain line mode: read prompts from stdin.
    // Picker is already ruled out by resolve_resume_mode for headless/plain.
    let resume_id = match resume_mode {
        ResumeMode::Id(id) => Some(id),
        ResumeMode::Fresh | ResumeMode::Picker => None,
    };
    let mut session = build_session_with(
        Box::new(HeadlessPresenter::default()),
        mock,
        mode,
        resume_id,
        pin,
        false,
    )
    .await?;
    if std::io::stdin().is_terminal() {
        println!(
            "{}",
            forge_tui::theme::glyphs::normalise(
                "forge chat — type a task and press enter; /quit to exit"
            )
        );
    }
    {
        let sid = session.session_id().to_string();
        let hooks = session.hooks().to_vec();
        let workspace = session.workspace_root().to_path_buf();
        let context = forge_core::hooks::run_session_hooks_in(
            &hooks,
            forge_config::HookEvent::SessionStart,
            &sid,
            Some(&workspace),
        )
        .await;
        for c in &context {
            session.inject_hook_context(c);
        }
    }
    while let Some(line) = session.read_line() {
        match chat_action(&line) {
            ChatAction::Quit => break,
            ChatAction::Skip => continue,
            ChatAction::Run(task) => {
                session.name_from_prompt(&task);
                let hooks = session.hooks().to_vec();
                let hook_workspace = session.workspace_root().to_path_buf();
                let hook_runner = session.llm_hook_runner().await;
                let task = match forge_core::hooks_llm::with_runner(
                    hook_runner,
                    forge_core::hooks::run_prompt_hooks_for(
                        &hooks,
                        &task,
                        Some(&hook_workspace),
                        session.id(),
                    ),
                )
                .await
                {
                    Ok(t) => t,
                    Err(reason) => {
                        eprintln!(
                            "{}",
                            forge_tui::theme::glyphs::normalise(&format!(
                                "⎇ prompt blocked by hook: {reason}"
                            ))
                        );
                        continue;
                    }
                };
                session
                    .run_turn(&task)
                    .await
                    .context("running agent turn")?;
            }
        }
    }
    {
        let sid = session.session_id().to_string();
        let hooks = session.hooks().to_vec();
        let workspace = session.workspace_root().to_path_buf();
        forge_core::hooks::run_session_hooks_in(
            &hooks,
            forge_config::HookEvent::SessionEnd,
            &sid,
            Some(&workspace),
        )
        .await;
    }
    Ok(())
}
