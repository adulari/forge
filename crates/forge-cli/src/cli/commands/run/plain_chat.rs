//! Interactive chat entry selection and plain line-mode lifecycle.

use super::*;

pub(crate) async fn chat(
    mock: bool,
    mode: Option<Mode>,
    resume_mode: ResumeMode,
    plain: bool,
    fullscreen: bool,
    pin: Option<String>,
    mut first_prompt: Option<String>,
) -> Result<()> {
    crate::startup_trace::mark("chat entered");
    let _ = crate::onboarding::maybe_open(mock)?;
    crate::startup_trace::mark("first-run check");
    maybe_autostart_local();
    crate::startup_trace::mark("local autostart check");
    // Default to the interactive (animated) TUI on a real terminal.
    let dumb_term = std::env::var("TERM").is_ok_and(|t| t == "dumb");
    if !plain && !dumb_term && std::io::stdout().is_terminal() {
        // Update check happens in background inside run_chat_tui (via the UiMsg channel) so it
        // never delays TUI startup. The check has a 3s network timeout — blocking here would
        // freeze the terminal for up to 3s once per day.
        let ran = run_chat_tui(mock, mode, resume_mode, fullscreen, pin, first_prompt).await;
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
    session.fire_session_start(None).await;
    // `forge chat "prompt"`: the first message comes from the command line, then stdin takes over.
    while let Some(line) = first_prompt.take().or_else(|| session.read_line()) {
        match chat_action(&line) {
            ChatAction::Quit => break,
            ChatAction::Skip => continue,
            ChatAction::Run(task) => {
                session.name_from_prompt(&task);
                let task = match session.run_prompt_submit_hooks(&task).await {
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
    session.fire_session_end("prompt_input_exit").await;
    if std::io::stdout().is_terminal() {
        if let Some(line) = super::exit_hint::exit_hint(&session) {
            println!("{line}");
        }
    }
    Ok(())
}
