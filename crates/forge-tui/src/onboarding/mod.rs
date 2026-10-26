//! The terminal front end of Forge onboarding.
//!
//! [`Screen`] is pure state plus key handling; [`render`] draws it. Neither touches the machine:
//! everything that does (jobs, browser, terminal hand-off, re-detection) is an [`Effect`] the
//! host carries out. The host also owns the terminal; [`terminal`] has the pieces it needs.

mod batch;
mod models;
mod pages;
mod plumbing;
mod qr;
mod screen;
mod text;
mod view;
mod view_flows;
mod view_models;
mod view_pages;

#[cfg(test)]
mod tests;

pub use batch::{Batch, BatchItem, BatchStatus};
pub use models::CardStatus;
pub use screen::{
    Effect, Entry, Exit, Notice, NoticeKind, Purpose, Screen, SecretTarget, SignIn, Tracked,
};
pub use view::{render, RAIL_MIN_WIDTH};

/// Terminal lifecycle for a full-screen onboarding run.
pub mod terminal {
    use std::io;

    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };

    /// Restore the terminal if the process panics while the screen is up.
    pub fn install_panic_restore() {
        crate::driver::install_panic_restore();
    }

    /// Raw mode plus the alternate screen.
    pub fn enter() -> io::Result<()> {
        enable_raw_mode()?;
        crossterm::execute!(io::stdout(), EnterAlternateScreen)
    }

    /// Back to the normal screen, for the end of the run or to hand the terminal to a child.
    pub fn leave() {
        let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}
