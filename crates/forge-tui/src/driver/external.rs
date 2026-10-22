//! Ctrl+G: hand the terminal to the user's editor for the draft, then take it back.

use std::io;
use std::path::Path;
use std::process::Command;

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
};
use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};

use super::{disable_mouse, Tui, IN_ALT_SCREEN};

impl Tui {
    /// Run a program that needs the real terminal (the user's `$EDITOR`): leave the alternate
    /// screen, mouse reporting and raw mode, run `f`, then take the terminal back and repaint
    /// from scratch. Editors reset bracketed paste and focus reporting on exit, so both are
    /// re-enabled here.
    pub fn run_external<T>(&mut self, f: impl FnOnce() -> T) -> io::Result<T> {
        if self.fullscreen {
            if self.mouse_capture {
                disable_mouse();
            }
            let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
            IN_ALT_SCREEN.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        let _ = crossterm::execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableFocusChange,
            crossterm::cursor::Show
        );
        let _ = disable_raw_mode();
        let out = f();
        crossterm::execute!(io::stdout(), EnableBracketedPaste, EnableFocusChange)?;
        if self.fullscreen {
            IN_ALT_SCREEN.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.run_fullscreen(|| Ok(out))
    }
}

/// The editor Ctrl+G opens: `$VISUAL`, then `$EDITOR`, then the platform's plain editor.
pub fn external_editor_command() -> String {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| if cfg!(windows) { "notepad" } else { "vi" }.to_string())
}

/// Edit `text` in `editor` (a shell command line such as `code --wait` or `nvim`) through a temp
/// file, returning what was saved. The trailing newline editors add on save is dropped. An editor
/// that exits non-zero leaves the draft as it was (`Err` explains).
pub fn edit_in_external_editor(editor: &str, text: &str) -> Result<String, String> {
    let path = std::env::temp_dir().join(format!(
        "forge-prompt-{}-{}.md",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::write(&path, text).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    let result = run_editor(editor, &path).and_then(|()| {
        std::fs::read_to_string(&path).map_err(|e| format!("could not read the edited draft: {e}"))
    });
    let _ = std::fs::remove_file(&path);
    result.map(|edited| strip_final_newline(&edited).to_string())
}

fn run_editor(editor: &str, path: &Path) -> Result<(), String> {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(format!("{editor} \"{}\"", path.display()));
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c")
            .arg(format!("{editor} \"$1\""))
            .arg("forge-editor")
            .arg(path);
        c
    };
    match cmd.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!(
            "`{editor}` exited with {status}; the draft is unchanged"
        )),
        Err(e) => Err(format!("could not start `{editor}`: {e}")),
    }
}

fn strip_final_newline(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_saved_text_comes_back_without_the_editor_newline() {
        let saved =
            std::env::temp_dir().join(format!("forge-editor-fixture-{}", std::process::id()));
        std::fs::write(&saved, "my edited prompt\n").unwrap();
        let editor = format!("cp '{}'", saved.display());
        let out = edit_in_external_editor(&editor, "my draft").unwrap();
        let _ = std::fs::remove_file(&saved);
        assert_eq!(out, "my edited prompt");
    }

    #[test]
    fn a_failing_editor_keeps_the_draft() {
        let err = edit_in_external_editor("false", "draft").unwrap_err();
        assert!(err.contains("draft is unchanged"), "{err}");
    }

    #[test]
    fn multiline_drafts_round_trip() {
        let out = edit_in_external_editor("true", "line one\nline two").unwrap();
        assert_eq!(out, "line one\nline two");
    }
}
