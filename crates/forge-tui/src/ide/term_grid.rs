//! A small terminal emulator: raw PTY bytes in, a cell grid out for a ratatui view to draw.

use std::collections::VecDeque;

use unicode_width::UnicodeWidthChar;
use vtparse::{CsiParam, VTActor, VTParser};

const SCROLLBACK_CAP: usize = 1000;
const TAB_STOP: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct CellStyle {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

/// A wide character occupies its cell plus a following continuation cell whose `ch` is `'\0'`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub style: CellStyle,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            style: CellStyle::default(),
        }
    }
}

static BLANK: Cell = Cell {
    ch: ' ',
    style: CellStyle {
        fg: Color::Default,
        bg: Color::Default,
        bold: false,
        dim: false,
        italic: false,
        underline: false,
        reverse: false,
    },
};

#[derive(Clone)]
struct Screen {
    lines: Vec<Vec<Cell>>,
    cols: usize,
    rows: usize,
}

impl Screen {
    fn new(cols: usize, rows: usize) -> Self {
        Self {
            lines: vec![vec![Cell::default(); cols]; rows],
            cols,
            rows,
        }
    }

    fn row_mut(&mut self, row: usize) -> &mut [Cell] {
        &mut self.lines[row]
    }

    fn row(&self, row: usize) -> &[Cell] {
        &self.lines[row]
    }

    fn resized(&self, cols: usize, rows: usize) -> Self {
        let mut next = Screen::new(cols, rows);
        for r in 0..rows.min(self.rows) {
            let n = cols.min(self.cols);
            next.row_mut(r)[..n].copy_from_slice(&self.row(r)[..n]);
        }
        next
    }
}

#[derive(Clone, Copy, Default)]
struct CursorState {
    row: usize,
    col: usize,
    pending_wrap: bool,
}

#[derive(Clone, Copy, Default)]
struct SavedCursor {
    cursor: CursorState,
    pen: CellStyle,
}

pub struct TermGrid {
    parser: Option<VTParser>,
    main: Screen,
    alt: Screen,
    in_alt: bool,
    cursor: CursorState,
    cursor_visible: bool,
    autowrap: bool,
    pen: CellStyle,
    saved: Option<SavedCursor>,
    region_top: usize,
    region_bottom: usize,
    scrollback: VecDeque<Vec<Cell>>,
    title: Option<String>,
}

impl TermGrid {
    pub fn new(cols: u16, rows: u16) -> Self {
        let cols = usize::from(cols.max(1));
        let rows = usize::from(rows.max(1));
        Self {
            parser: Some(VTParser::new()),
            main: Screen::new(cols, rows),
            alt: Screen::new(cols, rows),
            in_alt: false,
            cursor: CursorState::default(),
            cursor_visible: true,
            autowrap: true,
            pen: CellStyle::default(),
            saved: None,
            region_top: 0,
            region_bottom: rows - 1,
            scrollback: VecDeque::new(),
            title: None,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        if let Some(mut parser) = self.parser.take() {
            parser.parse(bytes, self);
            self.parser = Some(parser);
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = usize::from(cols.max(1));
        let rows = usize::from(rows.max(1));
        if cols == self.main.cols && rows == self.main.rows {
            return;
        }
        self.main = self.main.resized(cols, rows);
        self.alt = self.alt.resized(cols, rows);
        self.region_top = 0;
        self.region_bottom = rows - 1;
        self.clamp_cursor();
        if let Some(saved) = self.saved.as_mut() {
            saved.cursor.row = saved.cursor.row.min(rows - 1);
            saved.cursor.col = saved.cursor.col.min(cols - 1);
        }
    }

    pub fn size(&self) -> (u16, u16) {
        (self.screen().cols as u16, self.screen().rows as u16)
    }

    pub fn rows(&self) -> u16 {
        self.screen().rows as u16
    }

    pub fn cursor(&self) -> (u16, u16, bool) {
        (
            self.cursor.row as u16,
            self.cursor.col as u16,
            self.cursor_visible,
        )
    }

    pub fn cell(&self, row: u16, col: u16) -> &Cell {
        let screen = self.screen();
        let (row, col) = (usize::from(row), usize::from(col));
        if row >= screen.rows || col >= screen.cols {
            return &BLANK;
        }
        &screen.lines[row][col]
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    pub fn scrollback_line(&self, index: usize) -> Option<&[Cell]> {
        self.scrollback.get(index).map(Vec::as_slice)
    }

    pub fn row_text(&self, row: u16) -> String {
        let row = usize::from(row);
        if row >= self.screen().rows {
            return String::new();
        }
        cells_text(self.screen().row(row))
    }

    pub fn visible_text(&self) -> String {
        let screen = self.screen();
        let mut out = String::new();
        for r in 0..screen.rows {
            if r > 0 {
                out.push('\n');
            }
            out.push_str(&cells_text(screen.row(r)));
        }
        out.truncate(out.trim_end_matches('\n').len());
        out
    }

    fn screen(&self) -> &Screen {
        if self.in_alt {
            &self.alt
        } else {
            &self.main
        }
    }

    fn screen_mut(&mut self) -> &mut Screen {
        if self.in_alt {
            &mut self.alt
        } else {
            &mut self.main
        }
    }

    fn cols(&self) -> usize {
        self.main.cols
    }

    fn rows_n(&self) -> usize {
        self.main.rows
    }

    fn clamp_cursor(&mut self) {
        self.cursor.row = self.cursor.row.min(self.rows_n() - 1);
        self.cursor.col = self.cursor.col.min(self.cols() - 1);
    }

    fn blank(&self) -> Cell {
        Cell {
            ch: ' ',
            style: CellStyle {
                bg: self.pen.bg,
                ..CellStyle::default()
            },
        }
    }

    fn put_char(&mut self, c: char) {
        let width = match c.width() {
            Some(w) if w > 0 => w,
            _ => return,
        };
        let cols = self.cols();
        if width > cols {
            return;
        }
        if self.cursor.pending_wrap && self.autowrap {
            self.cursor.col = 0;
            self.index();
        }
        self.cursor.pending_wrap = false;
        if self.cursor.col + width > cols {
            if self.autowrap {
                self.cursor.col = 0;
                self.index();
            } else {
                self.cursor.col = cols - width;
            }
        }
        let (row, col) = (self.cursor.row, self.cursor.col);
        let style = self.pen;
        let blank = self.blank();
        let line = self.screen_mut().row_mut(row);
        clear_wide_overlap(line, col, blank);
        if width == 2 {
            clear_wide_overlap(line, col + 1, blank);
        }
        line[col] = Cell { ch: c, style };
        if width == 2 {
            line[col + 1] = Cell { ch: '\0', style };
        }
        let next = col + width;
        if next >= cols {
            self.cursor.col = cols - 1;
            self.cursor.pending_wrap = true;
        } else {
            self.cursor.col = next;
        }
    }

    fn control(&mut self, byte: u8) {
        match byte {
            b'\r' => {
                self.cursor.col = 0;
                self.cursor.pending_wrap = false;
            }
            b'\n' | 0x0b | 0x0c => {
                self.cursor.pending_wrap = false;
                self.index();
            }
            0x08 => {
                self.cursor.col = self.cursor.col.saturating_sub(1);
                self.cursor.pending_wrap = false;
            }
            b'\t' => {
                let next = (self.cursor.col / TAB_STOP + 1) * TAB_STOP;
                self.cursor.col = next.min(self.cols() - 1);
                self.cursor.pending_wrap = false;
            }
            _ => {}
        }
    }

    fn index(&mut self) {
        if self.cursor.row == self.region_bottom {
            self.scroll_up(1);
        } else if self.cursor.row + 1 < self.rows_n() {
            self.cursor.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.cursor.row == self.region_top {
            self.scroll_down(1);
        } else {
            self.cursor.row = self.cursor.row.saturating_sub(1);
        }
    }

    fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = (self.region_top, self.region_bottom);
        let n = n.min(bottom - top + 1);
        if n == 0 {
            return;
        }
        let blank = self.blank();
        let keep_history = !self.in_alt && top == 0 && bottom == self.rows_n() - 1;
        let cols = self.cols();
        let screen = if self.in_alt {
            &mut self.alt
        } else {
            &mut self.main
        };
        let removed: Vec<Vec<Cell>> = screen.lines.drain(top..top + n).collect();
        screen.lines.splice(
            bottom + 1 - n..bottom + 1 - n,
            (0..n).map(|_| vec![blank; cols]),
        );
        if keep_history {
            self.scrollback.extend(removed);
            while self.scrollback.len() > SCROLLBACK_CAP {
                self.scrollback.pop_front();
            }
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = (self.region_top, self.region_bottom);
        let n = n.min(bottom - top + 1);
        if n == 0 {
            return;
        }
        let blank = self.blank();
        let cols = self.cols();
        let lines = &mut self.screen_mut().lines;
        lines.drain(bottom + 1 - n..bottom + 1);
        lines.splice(top..top, (0..n).map(|_| vec![blank; cols]));
    }

    fn move_to(&mut self, row: usize, col: usize) {
        self.cursor.row = row.min(self.rows_n() - 1);
        self.cursor.col = col.min(self.cols() - 1);
        self.cursor.pending_wrap = false;
    }

    fn csi(&mut self, params: &[CsiParam], byte: u8) {
        let (leader, groups) = split_params(params);
        let arg = |i: usize| groups.get(i).and_then(|g| g[0]).filter(|&v| v != 0);
        let count = |i: usize| arg(i).unwrap_or(1).min(i64::from(u16::MAX)) as usize;
        let (row, col) = (self.cursor.row, self.cursor.col);
        match (leader, byte) {
            (None, b'A') => {
                let floor = if row >= self.region_top {
                    self.region_top
                } else {
                    0
                };
                self.move_to(row.saturating_sub(count(0)).max(floor), col);
            }
            (None, b'B' | b'e') => {
                let ceil = if row <= self.region_bottom {
                    self.region_bottom
                } else {
                    self.rows_n() - 1
                };
                self.move_to(row.saturating_add(count(0)).min(ceil), col);
            }
            (None, b'C' | b'a') => self.move_to(row, col.saturating_add(count(0))),
            (None, b'D') => self.move_to(row, col.saturating_sub(count(0))),
            (None, b'E') => self.move_to(row.saturating_add(count(0)), 0),
            (None, b'F') => self.move_to(row.saturating_sub(count(0)), 0),
            (None, b'G' | b'`') => self.move_to(row, count(0) - 1),
            (None, b'd') => self.move_to(count(0) - 1, col),
            (None, b'H' | b'f') => self.move_to(count(0) - 1, count(1) - 1),
            (None, b'J') => self.erase_display(arg(0).unwrap_or(0)),
            (None, b'K') => self.erase_line(arg(0).unwrap_or(0)),
            (None, b'X') => {
                let end = col.saturating_add(count(0)).min(self.cols());
                let blank = self.blank();
                self.screen_mut().row_mut(row)[col..end].fill(blank);
            }
            (None, b'P') => {
                let (cols, blank) = (self.cols(), self.blank());
                let n = count(0).min(cols - col);
                let line = self.screen_mut().row_mut(row);
                line[col..].rotate_left(n);
                line[cols - n..].fill(blank);
            }
            (None, b'@') => {
                let blank = self.blank();
                let n = count(0).min(self.cols() - col);
                let line = self.screen_mut().row_mut(row);
                line[col..].rotate_right(n);
                line[col..col + n].fill(blank);
            }
            (None, b'L' | b'M') => {
                if (self.region_top..=self.region_bottom).contains(&row) {
                    let saved_top = std::mem::replace(&mut self.region_top, row);
                    if byte == b'L' {
                        self.scroll_down(count(0));
                    } else {
                        self.scroll_up(count(0));
                    }
                    self.region_top = saved_top;
                    self.cursor.col = 0;
                    self.cursor.pending_wrap = false;
                }
            }
            (None, b'S') => self.scroll_up(count(0)),
            (None, b'T') => self.scroll_down(count(0)),
            (None, b'm') => self.sgr(&groups),
            (None, b'r') => {
                let rows = self.rows_n();
                let top = (count(0) - 1).min(rows - 1);
                let bottom = arg(1).map_or(rows, |v| v as usize).clamp(1, rows) - 1;
                if top < bottom {
                    self.region_top = top;
                    self.region_bottom = bottom;
                } else {
                    self.region_top = 0;
                    self.region_bottom = rows - 1;
                }
                self.move_to(0, 0);
            }
            (None, b's') if groups.is_empty() => self.save_cursor(),
            (None, b'u') => self.restore_cursor(),
            (Some(b'?'), b'h' | b'l') => {
                for g in &groups {
                    if let Some(code) = g[0] {
                        self.dec_mode(code, byte == b'h');
                    }
                }
            }
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: i64) {
        let (row, col, cols) = (self.cursor.row, self.cursor.col, self.cols());
        let blank = self.blank();
        let range = match mode {
            0 => col..cols,
            1 => 0..col + 1,
            2 => 0..cols,
            _ => return,
        };
        self.screen_mut().row_mut(row)[range].fill(blank);
        self.cursor.pending_wrap = false;
    }

    fn erase_display(&mut self, mode: i64) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        let rows = self.rows_n();
        let blank = self.blank();
        let screen = self.screen_mut();
        match mode {
            0 => {
                screen.row_mut(row)[col..].fill(blank);
                for r in row + 1..rows {
                    screen.row_mut(r).fill(blank);
                }
            }
            1 => {
                for r in 0..row {
                    screen.row_mut(r).fill(blank);
                }
                screen.row_mut(row)[..col + 1].fill(blank);
            }
            2 => {
                for r in 0..rows {
                    screen.row_mut(r).fill(blank);
                }
            }
            3 => {
                self.scrollback.clear();
                return;
            }
            _ => return,
        }
        self.cursor.pending_wrap = false;
    }

    fn sgr(&mut self, groups: &[Vec<Option<i64>>]) {
        if groups.is_empty() {
            self.pen = CellStyle::default();
            return;
        }
        let mut i = 0;
        while i < groups.len() {
            let group = &groups[i];
            i += 1;
            let code = group[0].unwrap_or(0);
            match code {
                0 => self.pen = CellStyle::default(),
                1 => {
                    self.pen.bold = true;
                    self.pen.dim = false;
                }
                2 => {
                    self.pen.dim = true;
                    self.pen.bold = false;
                }
                3 => self.pen.italic = true,
                4 => self.pen.underline = group.get(1).copied().flatten() != Some(0),
                7 => self.pen.reverse = true,
                21 => self.pen.underline = true,
                22 => {
                    self.pen.bold = false;
                    self.pen.dim = false;
                }
                23 => self.pen.italic = false,
                24 => self.pen.underline = false,
                27 => self.pen.reverse = false,
                30..=37 => self.pen.fg = Color::Indexed((code - 30) as u8),
                39 => self.pen.fg = Color::Default,
                40..=47 => self.pen.bg = Color::Indexed((code - 40) as u8),
                49 => self.pen.bg = Color::Default,
                90..=97 => self.pen.fg = Color::Indexed((code - 90 + 8) as u8),
                100..=107 => self.pen.bg = Color::Indexed((code - 100 + 8) as u8),
                38 | 48 => {
                    let (color, used) = extended_color(group, &groups[i..]);
                    i += used;
                    if let Some(color) = color {
                        if code == 38 {
                            self.pen.fg = color;
                        } else {
                            self.pen.bg = color;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn dec_mode(&mut self, code: i64, on: bool) {
        match code {
            25 => self.cursor_visible = on,
            7 => self.autowrap = on,
            1049 => {
                if on {
                    if !self.in_alt {
                        self.save_cursor();
                    }
                    self.enter_alt();
                } else {
                    self.leave_alt();
                    self.restore_cursor();
                }
            }
            1047 | 47 => {
                if on {
                    self.enter_alt();
                } else {
                    self.leave_alt();
                }
            }
            _ => {}
        }
    }

    fn enter_alt(&mut self) {
        if self.in_alt {
            return;
        }
        self.in_alt = true;
        self.alt = Screen::new(self.cols(), self.rows_n());
        self.region_top = 0;
        self.region_bottom = self.rows_n() - 1;
        self.cursor.pending_wrap = false;
    }

    fn leave_alt(&mut self) {
        if !self.in_alt {
            return;
        }
        self.in_alt = false;
        self.region_top = 0;
        self.region_bottom = self.rows_n() - 1;
        self.cursor.pending_wrap = false;
    }

    fn save_cursor(&mut self) {
        self.saved = Some(SavedCursor {
            cursor: self.cursor,
            pen: self.pen,
        });
    }

    fn restore_cursor(&mut self) {
        if let Some(saved) = self.saved {
            self.cursor = saved.cursor;
            self.pen = saved.pen;
            self.clamp_cursor();
        }
    }

    fn esc(&mut self, byte: u8) {
        match byte {
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            b'D' => {
                self.cursor.pending_wrap = false;
                self.index();
            }
            b'E' => {
                self.cursor.col = 0;
                self.cursor.pending_wrap = false;
                self.index();
            }
            b'M' => {
                self.cursor.pending_wrap = false;
                self.reverse_index();
            }
            b'c' => {
                let (cols, rows) = (self.cols() as u16, self.rows_n() as u16);
                let parser = self.parser.take();
                *self = TermGrid::new(cols, rows);
                self.parser = parser;
            }
            _ => {}
        }
    }
}

impl VTActor for TermGrid {
    fn print(&mut self, c: char) {
        self.put_char(c);
    }

    fn execute_c0_or_c1(&mut self, control: u8) {
        self.control(control);
    }

    fn dcs_hook(&mut self, _: u8, _: &[i64], _: &[u8], _: bool) {}

    fn dcs_put(&mut self, _: u8) {}

    fn dcs_unhook(&mut self) {}

    fn esc_dispatch(&mut self, _: &[i64], intermediates: &[u8], _: bool, byte: u8) {
        if intermediates.is_empty() {
            self.esc(byte);
        }
    }

    fn csi_dispatch(&mut self, params: &[CsiParam], _: bool, byte: u8) {
        self.csi(params, byte);
    }

    fn osc_dispatch(&mut self, params: &[&[u8]]) {
        if let [b"0" | b"2", rest @ ..] = params {
            let title = rest
                .iter()
                .map(|p| String::from_utf8_lossy(p))
                .collect::<Vec<_>>()
                .join(";");
            self.title = if title.is_empty() { None } else { Some(title) };
        }
    }

    fn apc_dispatch(&mut self, _: Vec<u8>) {}
}

/// Splits CSI params into `;`-separated groups of `:`-separated values; a sequence carrying an
/// intermediate byte (e.g. `CSI SP q`) comes back as a leader of `0xff` so no handler matches it.
fn split_params(params: &[CsiParam]) -> (Option<u8>, Vec<Vec<Option<i64>>>) {
    let mut leader = None;
    let mut groups = Vec::new();
    let mut group = Vec::new();
    let mut value = None;
    let mut seen = false;
    for (i, param) in params.iter().enumerate() {
        match param {
            CsiParam::Integer(v) => {
                value = Some(*v);
                seen = true;
            }
            CsiParam::P(b';') => {
                group.push(value.take());
                groups.push(std::mem::take(&mut group));
                seen = true;
            }
            CsiParam::P(b':') => {
                group.push(value.take());
                seen = true;
            }
            CsiParam::P(c @ (b'?' | b'<' | b'=' | b'>')) if i == 0 => leader = Some(*c),
            CsiParam::P(_) => leader = Some(0xff),
        }
    }
    if seen {
        group.push(value);
        groups.push(group);
    }
    (leader, groups)
}

/// Decodes the colour after SGR 38/48 in either `38;5;n` / `38;2;r;g;b` or `38:5:n` /
/// `38:2:[cs:]r:g:b` form, returning how many following groups were consumed.
fn extended_color(group: &[Option<i64>], rest: &[Vec<Option<i64>>]) -> (Option<Color>, usize) {
    let byte = |v: Option<i64>| v.unwrap_or(0).clamp(0, 255) as u8;
    if group.len() > 1 {
        let color = match group[1] {
            Some(5) => group.get(2).map(|&v| Color::Indexed(byte(v))),
            Some(2) if group.len() >= 5 => {
                let n = group.len();
                Some(Color::Rgb(
                    byte(group[n - 3]),
                    byte(group[n - 2]),
                    byte(group[n - 1]),
                ))
            }
            _ => None,
        };
        return (color, 0);
    }
    let first = |i: usize| rest.get(i).map(|g| g[0]);
    match first(0) {
        Some(Some(5)) => match first(1) {
            Some(v) => (Some(Color::Indexed(byte(v))), 2),
            None => (None, rest.len()),
        },
        Some(Some(2)) => match (first(1), first(2), first(3)) {
            (Some(r), Some(g), Some(b)) => (Some(Color::Rgb(byte(r), byte(g), byte(b))), 4),
            _ => (None, rest.len()),
        },
        Some(_) => (None, 1),
        None => (None, 0),
    }
}

fn clear_wide_overlap(line: &mut [Cell], col: usize, blank: Cell) {
    if line[col].ch == '\0' && col > 0 {
        line[col - 1] = blank;
    }
    if col + 1 < line.len() && line[col + 1].ch == '\0' && line[col].ch != '\0' {
        line[col + 1] = blank;
    }
}

fn cells_text(cells: &[Cell]) -> String {
    let text: String = cells
        .iter()
        .filter(|c| c.ch != '\0')
        .map(|c| c.ch)
        .collect();
    text.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(cols: u16, rows: u16, input: &str) -> TermGrid {
        let mut g = TermGrid::new(cols, rows);
        g.feed(input.as_bytes());
        g
    }

    #[test]
    fn plain_text_and_crlf() {
        let g = grid(10, 3, "hello\r\nworld");
        assert_eq!(g.row_text(0), "hello");
        assert_eq!(g.row_text(1), "world");
        assert_eq!(g.cursor(), (1, 5, true));
        assert_eq!(g.visible_text(), "hello\nworld");
    }

    #[test]
    fn bare_lf_keeps_column() {
        let g = grid(10, 3, "ab\ncd");
        assert_eq!(g.row_text(0), "ab");
        assert_eq!(g.row_text(1), "  cd");
    }

    #[test]
    fn cursor_addressing() {
        let g = grid(10, 5, "\x1b[3;4HX\x1b[1;1HY\x1b[2BZ\x1b[2CW\x1b[5D!");
        assert_eq!(g.cell(2, 3).ch, 'X');
        assert_eq!(g.cell(0, 0).ch, 'Y');
        assert_eq!(g.cell(2, 1).ch, 'Z');
        assert_eq!(g.cell(2, 4).ch, 'W');
        assert_eq!(g.cell(2, 0).ch, '!');
        let g = grid(10, 5, "\x1b[4;6H\x1b[2A\x1b[3G^\x1b[5d_");
        assert_eq!(g.cell(1, 2).ch, '^');
        assert_eq!(g.cell(4, 3).ch, '_');
        let g = grid(10, 5, "\x1b[2;5H\x1b[E1\x1b[2F2");
        assert_eq!(g.cell(2, 0).ch, '1');
        assert_eq!(g.cell(0, 0).ch, '2');
    }

    #[test]
    fn cursor_clamps() {
        let g = grid(10, 5, "\x1b[99;99H");
        assert_eq!(g.cursor(), (4, 9, true));
        let g = grid(10, 5, "\x1b[99A\x1b[99D");
        assert_eq!(g.cursor(), (0, 0, true));
    }

    #[test]
    fn sgr_colors_and_attrs() {
        let g = grid(
            20,
            2,
            "\x1b[1;31mR\x1b[0m\x1b[38;5;200mI\x1b[48;2;1;2;3mT\x1b[0;3;4;7;2mA\x1b[22;23;24;27mB\x1b[mC",
        );
        let r = g.cell(0, 0).style;
        assert_eq!(r.fg, Color::Indexed(1));
        assert!(r.bold);
        assert_eq!(g.cell(0, 1).style.fg, Color::Indexed(200));
        assert!(!g.cell(0, 1).style.bold);
        assert_eq!(g.cell(0, 2).style.bg, Color::Rgb(1, 2, 3));
        assert_eq!(g.cell(0, 2).style.fg, Color::Indexed(200));
        let a = g.cell(0, 3).style;
        assert!(a.italic && a.underline && a.reverse && a.dim);
        assert_eq!(g.cell(0, 4).style, CellStyle::default());
        assert_eq!(g.cell(0, 5).style, CellStyle::default());
    }

    #[test]
    fn bright_and_truecolor_fg() {
        let g = grid(10, 1, "\x1b[91mA\x1b[38;2;255;128;0mB\x1b[39mC");
        assert_eq!(g.cell(0, 0).style.fg, Color::Indexed(9));
        assert_eq!(g.cell(0, 1).style.fg, Color::Rgb(255, 128, 0));
        assert_eq!(g.cell(0, 2).style.fg, Color::Default);
    }

    #[test]
    fn colon_and_semicolon_color_forms() {
        let g = grid(
            10,
            1,
            "\x1b[38:2::10:20:30mA\x1b[48:5:99mB\x1b[38:2:10:20:30mC\x1b[4:0mD",
        );
        assert_eq!(g.cell(0, 0).style.fg, Color::Rgb(10, 20, 30));
        assert_eq!(g.cell(0, 1).style.bg, Color::Indexed(99));
        assert_eq!(g.cell(0, 2).style.fg, Color::Rgb(10, 20, 30));
        assert!(!g.cell(0, 3).style.underline);
        let g = grid(10, 1, "\x1b[1;38;5;196;48;2;1;2;3;4mA");
        let s = g.cell(0, 0).style;
        assert_eq!((s.fg, s.bg), (Color::Indexed(196), Color::Rgb(1, 2, 3)));
        assert!(s.bold && s.underline);
    }

    #[test]
    fn default_params_and_private_markers() {
        let g = grid(10, 3, "\x1b[;3HX\x1b[0;0HY\x1b[?25;7l");
        assert_eq!(g.cell(0, 2).ch, 'X');
        assert_eq!(g.cell(0, 0).ch, 'Y');
        assert!(!g.cursor().2);
        let g = grid(10, 2, "ab\x1b[>1;2cc\x1b[ qd\x1b[?Hx");
        assert_eq!(g.row_text(0), "abcdx");
    }

    #[test]
    fn osc_title_with_semicolons() {
        let g = grid(10, 1, "\x1b]0;a;b\x07");
        assert_eq!(g.title(), Some("a;b"));
    }

    #[test]
    fn erase_in_line_modes() {
        let g = grid(10, 1, "abcdefghij\x1b[5G\x1b[K");
        assert_eq!(g.row_text(0), "abcd");
        let g = grid(10, 1, "abcdefghij\x1b[5G\x1b[1K");
        assert_eq!(g.row_text(0), "     fghij");
        let g = grid(10, 1, "abcdefghij\x1b[5G\x1b[2K");
        assert_eq!(g.row_text(0), "");
    }

    #[test]
    fn erase_in_display_modes() {
        let fill = "aaaa\r\nbbbb\r\ncccc";
        let g = grid(4, 3, &format!("{fill}\x1b[2;3H\x1b[J"));
        assert_eq!(g.visible_text(), "aaaa\nbb");
        let g = grid(4, 3, &format!("{fill}\x1b[2;3H\x1b[1J"));
        assert_eq!(g.visible_text(), "\n   b\ncccc");
        let g = grid(4, 3, &format!("{fill}\x1b[2J"));
        assert_eq!(g.visible_text(), "");
    }

    #[test]
    fn insert_delete_erase_chars() {
        let g = grid(8, 1, "abcdefgh\x1b[3G\x1b[2P");
        assert_eq!(g.row_text(0), "abefgh");
        let g = grid(8, 1, "abcdefgh\x1b[3G\x1b[2@");
        assert_eq!(g.row_text(0), "ab  cdef");
        let g = grid(8, 1, "abcdefgh\x1b[3G\x1b[3X");
        assert_eq!(g.row_text(0), "ab   fgh");
    }

    #[test]
    fn insert_delete_lines_and_scroll() {
        let g = grid(3, 4, "a\r\nb\r\nc\r\nd\x1b[2;1H\x1b[L");
        assert_eq!(g.visible_text(), "a\n\nb\nc");
        let g = grid(3, 4, "a\r\nb\r\nc\r\nd\x1b[2;1H\x1b[M");
        assert_eq!(g.visible_text(), "a\nc\nd");
        let g = grid(3, 3, "a\r\nb\r\nc\x1b[S");
        assert_eq!(g.visible_text(), "b\nc");
        let g = grid(3, 3, "a\r\nb\r\nc\x1b[T");
        assert_eq!(g.visible_text(), "\na\nb");
    }

    #[test]
    fn escape_split_across_feeds() {
        let mut g = TermGrid::new(10, 2);
        g.feed(b"\x1b[3");
        g.feed(b"1mX\x1b");
        g.feed(b"[0mY");
        assert_eq!(g.cell(0, 0).style.fg, Color::Indexed(1));
        assert_eq!(g.cell(0, 1).style, CellStyle::default());
        assert_eq!(g.row_text(0), "XY");
    }

    #[test]
    fn utf8_split_across_feeds() {
        let bytes = "é世x".as_bytes();
        let mut g = TermGrid::new(10, 1);
        for b in bytes {
            g.feed(std::slice::from_ref(b));
        }
        assert_eq!(g.row_text(0), "é世x");
    }

    #[test]
    fn wide_chars_take_two_cells() {
        let g = grid(6, 2, "世界a");
        assert_eq!(g.cell(0, 0).ch, '世');
        assert_eq!(g.cell(0, 1).ch, '\0');
        assert_eq!(g.cell(0, 2).ch, '界');
        assert_eq!(g.cell(0, 4).ch, 'a');
        assert_eq!(g.cursor().1, 5);
        let g = grid(5, 2, "abcd世");
        assert_eq!(g.row_text(0), "abcd");
        assert_eq!(g.cell(1, 0).ch, '世');
        let g = grid(6, 1, "世界\x1b[2G!");
        assert_eq!(g.cell(0, 0).ch, ' ');
        assert_eq!(g.cell(0, 1).ch, '!');
    }

    #[test]
    fn deferred_wrap_at_right_edge() {
        let g = grid(4, 2, "abcd");
        assert_eq!(g.cursor(), (0, 3, true));
        let g = grid(4, 2, "abcde");
        assert_eq!(g.row_text(0), "abcd");
        assert_eq!(g.row_text(1), "e");
        let g = grid(4, 2, "abcd\r\nx");
        assert_eq!(g.visible_text(), "abcd\nx");
    }

    #[test]
    fn autowrap_off_overwrites_last_column() {
        let g = grid(4, 2, "\x1b[?7labcdef");
        assert_eq!(g.row_text(0), "abcf");
        assert_eq!(g.row_text(1), "");
        let g = grid(4, 2, "\x1b[?7l\x1b[?7habcde");
        assert_eq!(g.row_text(1), "e");
    }

    #[test]
    fn scrolling_pushes_scrollback() {
        let mut g = TermGrid::new(5, 2);
        g.feed(b"one\r\ntwo\r\nthree\r\nfour");
        assert_eq!(g.scrollback_len(), 2);
        assert_eq!(g.visible_text(), "three\nfour");
        assert_eq!(cells_text(g.scrollback_line(0).unwrap()), "one");
        assert_eq!(cells_text(g.scrollback_line(1).unwrap()), "two");
    }

    #[test]
    fn wrap_at_bottom_scrolls() {
        let g = grid(3, 2, "abcdefgh");
        assert_eq!(g.visible_text(), "def\ngh");
        assert_eq!(g.scrollback_len(), 1);
    }

    #[test]
    fn scrollback_is_capped() {
        let mut g = TermGrid::new(5, 2);
        for i in 0..1500 {
            g.feed(format!("{i}\r\n").as_bytes());
        }
        assert_eq!(g.scrollback_len(), SCROLLBACK_CAP);
        assert_eq!(
            cells_text(g.scrollback_line(SCROLLBACK_CAP - 1).unwrap()),
            "1498"
        );
    }

    #[test]
    fn alt_screen_restores_main() {
        let mut g = TermGrid::new(6, 3);
        g.feed(b"main\x1b[2;3H");
        g.feed(b"\x1b[?1049h");
        assert_eq!(g.visible_text(), "");
        g.feed(b"alt\r\nalt2\r\nalt3\r\nalt4");
        assert_eq!(g.scrollback_len(), 0);
        assert_eq!(g.row_text(0), "alt2");
        g.feed(b"\x1b[?1049l");
        assert_eq!(g.visible_text(), "main");
        assert_eq!(g.cursor(), (1, 2, true));
    }

    #[test]
    fn alt_screen_variants() {
        let mut g = TermGrid::new(6, 2);
        g.feed(b"keep\x1b[?47hxx\x1b[?47l");
        assert_eq!(g.visible_text(), "keep");
        g.feed(b"\x1b[?1047hyy\x1b[?1047l");
        assert_eq!(g.visible_text(), "keep");
    }

    #[test]
    fn scroll_region_confines_lf() {
        let mut g = TermGrid::new(3, 5);
        g.feed(b"a\r\nb\r\nc\r\nd\r\ne");
        g.feed(b"\x1b[2;4r");
        assert_eq!(g.cursor(), (0, 0, true));
        g.feed(b"\x1b[4;1H\n");
        assert_eq!(g.visible_text(), "a\nc\nd\n\ne");
        assert_eq!(g.scrollback_len(), 0);
        g.feed(b"\x1b[2;1H\x1bM");
        assert_eq!(g.visible_text(), "a\n\nc\nd\ne");
        g.feed(b"\x1b[r\x1b[5;1H\n");
        assert_eq!(g.scrollback_len(), 1);
    }

    #[test]
    fn save_restore_cursor() {
        let g = grid(10, 4, "\x1b[2;3H\x1b7\x1b[4;9Hz\x1b8Q");
        assert_eq!(g.cell(1, 2).ch, 'Q');
        let g = grid(10, 4, "\x1b[2;3H\x1b[s\x1b[4;9H\x1b[uQ");
        assert_eq!(g.cell(1, 2).ch, 'Q');
    }

    #[test]
    fn cursor_visibility_and_title() {
        let mut g = TermGrid::new(10, 2);
        g.feed(b"\x1b[?25l");
        assert!(!g.cursor().2);
        g.feed(b"\x1b[?25h");
        assert!(g.cursor().2);
        assert_eq!(g.title(), None);
        g.feed(b"\x1b]0;my title\x07");
        assert_eq!(g.title(), Some("my title"));
        g.feed(b"\x1b]2;other\x1b\\");
        assert_eq!(g.title(), Some("other"));
    }

    #[test]
    fn tabs_and_backspace() {
        let g = grid(20, 1, "a\tb\x08c");
        assert_eq!(g.cell(0, 8).ch, 'c');
        assert_eq!(g.cell(0, 0).ch, 'a');
        let g = grid(5, 1, "\t\t\tx");
        assert_eq!(g.cell(0, 4).ch, 'x');
    }

    #[test]
    fn bel_is_ignored() {
        let g = grid(5, 1, "a\x07b");
        assert_eq!(g.row_text(0), "ab");
    }

    #[test]
    fn resize_keeps_content_and_clamps_cursor() {
        let mut g = TermGrid::new(10, 4);
        g.feed(b"hello\r\nworld\x1b[4;10H");
        g.resize(6, 2);
        assert_eq!(g.size(), (6, 2));
        assert_eq!(g.rows(), 2);
        assert_eq!(g.row_text(0), "hello");
        assert_eq!(g.row_text(1), "world");
        assert_eq!(g.cursor(), (1, 5, true));
        g.resize(12, 5);
        assert_eq!(g.row_text(1), "world");
        assert_eq!(g.size(), (12, 5));
        g.feed(b"\x1b[5;1HX");
        assert_eq!(g.cell(4, 0).ch, 'X');
    }

    #[test]
    fn resize_while_in_alt_screen() {
        let mut g = TermGrid::new(10, 4);
        g.feed(b"main\x1b[?1049halt");
        g.resize(5, 3);
        g.feed(b"\x1b[?1049l");
        assert_eq!(g.visible_text(), "main");
        assert_eq!(g.size(), (5, 3));
    }

    #[test]
    fn out_of_range_cell_is_blank() {
        let g = TermGrid::new(3, 3);
        assert_eq!(*g.cell(50, 50), Cell::default());
        assert_eq!(g.row_text(50), "");
    }

    #[test]
    fn realistic_ls_capture_and_clear() {
        let mut g = TermGrid::new(40, 6);
        g.feed(b"$ ls --color\r\n");
        g.feed(b"\x1b[0m\x1b[01;34msrc\x1b[0m  \x1b[01;32mrun.sh\x1b[0m  Cargo.toml  \x1b[01;31marchive.tar\x1b[0m\r\n");
        g.feed(b"\x1b[0m\x1b[01;36mlink\x1b[0m -> src\r\n$ ");
        assert_eq!(g.row_text(0), "$ ls --color");
        assert_eq!(g.row_text(1), "src  run.sh  Cargo.toml  archive.tar");
        assert_eq!(g.row_text(2), "link -> src");
        assert_eq!(g.cell(1, 0).style.fg, Color::Indexed(4));
        assert!(g.cell(1, 0).style.bold);
        assert_eq!(g.cell(1, 3).style, CellStyle::default());
        assert_eq!(g.cell(1, 3).ch, ' ');
        assert_eq!(g.cell(1, 7).style.fg, Color::Indexed(2));
        assert_eq!(g.cell(1, 25).style.fg, Color::Indexed(1));
        assert_eq!(g.cursor(), (3, 2, true));
        g.feed(b"\x1b[2J\x1b[H");
        assert_eq!(g.visible_text(), "");
        assert_eq!(g.cursor(), (0, 0, true));
    }

    #[test]
    fn full_reset_clears() {
        let mut g = TermGrid::new(5, 2);
        g.feed(b"abc\x1b[31m\x1bcX");
        assert_eq!(g.visible_text(), "X");
        assert_eq!(g.cell(0, 0).style, CellStyle::default());
    }

    #[test]
    fn unknown_sequences_are_ignored() {
        let g = grid(
            10,
            2,
            "\x1b[?2004h\x1b[>c\x1b[6n\x1b=\x1bPq#0;2;0;0;0\x1b\\\x1b_Gx\x1b\\ok",
        );
        assert_eq!(g.row_text(0), "ok");
    }

    fn xorshift(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    #[test]
    fn random_bytes_do_not_panic() {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut g = TermGrid::new(40, 12);
        for round in 0..4 {
            let bytes: Vec<u8> = (0..25_000).map(|_| xorshift(&mut state) as u8).collect();
            for chunk in bytes.chunks(97) {
                g.feed(chunk);
            }
            g.resize(10 + round * 7, 4 + round * 3);
        }
        let _ = g.visible_text();
    }

    #[test]
    fn escape_heavy_random_bytes_do_not_panic() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let alphabet = b"\x1b[];?0123456789HJKmrABCDhlLMPX@STdG \r\n\t\x08(7)8>=\\";
        let bytes: Vec<u8> = (0..100_000)
            .map(|_| alphabet[(xorshift(&mut state) % alphabet.len() as u64) as usize])
            .collect();
        let mut g = TermGrid::new(30, 10);
        for (i, chunk) in bytes.chunks(61).enumerate() {
            g.feed(chunk);
            if i % 300 == 0 {
                g.resize(5 + (i % 40) as u16, 2 + (i % 15) as u16);
            }
        }
        let (rows, cols, _) = g.cursor();
        assert!(rows < g.rows() && cols < g.size().0);
    }

    #[test]
    fn feeds_one_megabyte_of_colored_text_quickly() {
        let line =
            "\x1b[1;32mhello world\x1b[0m the quick brown fox \x1b[38;5;123mjumps\x1b[0m over\r\n";
        let mut data = Vec::with_capacity(1 << 20);
        while data.len() < 1 << 20 {
            data.extend_from_slice(line.as_bytes());
        }
        let mut g = TermGrid::new(120, 40);
        let start = std::time::Instant::now();
        for chunk in data.chunks(4096) {
            g.feed(chunk);
        }
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 500, "took {elapsed:?}");
        assert_eq!(g.scrollback_len(), SCROLLBACK_CAP);
    }
}
