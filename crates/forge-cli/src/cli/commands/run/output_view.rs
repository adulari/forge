//! The chat shell's side of the full-output viewer: opening it, and what its keys ask for.

use forge_tui::{App, OutputAction, Tui};

/// `/output`: the latest tool call's complete output in the full-screen viewer. Inline mode has no
/// room for the viewer, so the latest kept file goes straight to `$PAGER`.
pub(super) fn open(app: &mut App, tui: Option<&mut Tui>) -> std::io::Result<()> {
    if app.fullscreen {
        if !app.open_latest_tool_output() {
            app.note("no tool output to show yet");
        }
        return Ok(());
    }
    match (app.latest_kept_output().map(str::to_string), tui) {
        (Some(path), Some(tui)) => tui.run_fullscreen(|| forge_tui::page_file(&path))?,
        (Some(path), None) => app.note(&format!("full output: {path}")),
        (None, _) => app.note("nothing kept yet: only output too long for the scrollback is kept"),
    }
    Ok(())
}

pub(super) fn perform(
    action: OutputAction,
    clipboard: &mut Option<arboard::Clipboard>,
    tui: &mut Tui,
) -> std::io::Result<()> {
    match action {
        OutputAction::Copy(text) => super::copy::copy_selection(clipboard, &text),
        // `run_fullscreen` puts the chat's raw mode, alternate screen and mouse capture back once
        // the pager exits.
        OutputAction::OpenPager(path) => tui.run_fullscreen(|| forge_tui::page_file(&path))?,
        OutputAction::Redraw | OutputAction::Close => {}
    }
    Ok(())
}
