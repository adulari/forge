//! A ratatui backend wrapper that remaps every cell colour into the active colour tier.
//!
//! Tokens already hold tier-correct values, so for them the remap is a no-op. It exists for the
//! strays: syntect highlighting, external statusline SGR and any legacy literal. Modifiers are
//! kept in every tier (that is what carries hierarchy in Mono).

use std::io::{self, Stdout};

use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};

use super::{degrade, t, ColorTier};

/// The backend every interactive surface draws through.
pub type TermBackend = RemapBackend<CrosstermBackend<Stdout>>;

pub fn stdout_backend() -> TermBackend {
    RemapBackend::new(CrosstermBackend::new(io::stdout()))
}

pub struct RemapBackend<B> {
    inner: B,
    tier: ColorTier,
}

impl<B> RemapBackend<B> {
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            tier: t().tier,
        }
    }

    pub fn with_tier(inner: B, tier: ColorTier) -> Self {
        Self { inner, tier }
    }

    pub fn inner(&self) -> &B {
        &self.inner
    }
}

fn remap(cell: &Cell, tier: ColorTier) -> Cell {
    let mut out = cell.clone();
    out.fg = degrade::map_color(cell.fg, tier);
    out.bg = degrade::map_color(cell.bg, tier);
    out
}

impl<B: Backend> Backend for RemapBackend<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        if self.tier == ColorTier::TrueColor {
            return self.inner.draw(content);
        }
        let tier = self.tier;
        let mapped: Vec<(u16, u16, Cell)> = content
            .map(|(x, y, cell)| (x, y, remap(cell, tier)))
            .collect();
        self.inner
            .draw(mapped.iter().map(|(x, y, cell)| (*x, *y, cell)))
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }

    fn scroll_region_up(
        &mut self,
        region: std::ops::Range<u16>,
        amount: u16,
    ) -> Result<(), Self::Error> {
        self.inner.scroll_region_up(region, amount)
    }

    fn scroll_region_down(
        &mut self,
        region: std::ops::Range<u16>,
        amount: u16,
    ) -> Result<(), Self::Error> {
        self.inner.scroll_region_down(region, amount)
    }
}
