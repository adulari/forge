//! Ctrl+Z: hand the terminal back to the shell and stop, then take it again on `fg`.

use super::*;

impl Tui {
    /// Restore the terminal, stop the process with SIGTSTP and, once the shell continues it,
    /// rebuild the screen exactly as `run_fullscreen` does after a takeover.
    #[cfg(unix)]
    pub fn suspend(&mut self) {
        use std::io::Write;
        if self.mouse_capture {
            disable_mouse();
        }
        let _ = crossterm::execute!(io::stdout(), DisableBracketedPaste, DisableFocusChange);
        if self.fullscreen {
            let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
            IN_ALT_SCREEN.store(false, std::sync::atomic::Ordering::Relaxed);
        } else {
            let _ = writeln!(io::stdout());
        }
        let _ = crossterm::execute!(io::stdout(), crossterm::cursor::Show);
        let _ = disable_raw_mode();
        let _ = io::stdout().flush();

        // Blocks here until the shell sends SIGCONT.
        // SAFETY: raising a signal on ourselves has no memory-safety preconditions.
        unsafe {
            libc::raise(libc::SIGTSTP);
        }

        let _ = enable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), EnableBracketedPaste, EnableFocusChange);
        if self.fullscreen {
            let _ = crossterm::execute!(io::stdout(), EnterAlternateScreen);
            IN_ALT_SCREEN.store(true, std::sync::atomic::Ordering::Relaxed);
            if self.mouse_capture {
                let _ = enable_mouse();
            }
        }
        if let Ok(terminal) = Terminal::with_options(
            stdout_backend(),
            TerminalOptions {
                viewport: viewport(self.fullscreen),
            },
        ) {
            self.terminal = terminal;
        }
        self.modal_surface_open = false;
        let _ = self.terminal.clear();
    }

    /// There is no job control to hand the terminal to.
    #[cfg(not(unix))]
    pub fn suspend(&mut self) {}
}
