//! Light text buffer for the IDE editor: char-indexed cursor, coalescing undo, O(line) edits.

const UNDO_CAP: usize = 500;
const INDENT: &str = "  ";

type Pos = (usize, usize);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Run {
    None,
    Insert,
    Back,
}

struct Edit {
    start: Pos,
    removed: String,
    inserted: String,
    before: Pos,
    after: Pos,
    version_before: u64,
    version_after: u64,
}

pub struct EditBuf {
    lines: Vec<String>,
    row: usize,
    col: usize,
    want_col: usize,
    version: u64,
    top_version: u64,
    saved_version: u64,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    trailing_newline: bool,
    run: Run,
}

fn end_of(start: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        None => (start.0, start.1 + text.chars().count()),
        Some(i) => (
            start.0 + text.matches('\n').count(),
            text[i + 1..].chars().count(),
        ),
    }
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn byte_idx(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map_or(s.len(), |(i, _)| i)
}

impl EditBuf {
    pub fn new(text: &str) -> Self {
        let trailing_newline = text.is_empty() || text.ends_with('\n');
        let body = text.strip_suffix('\n').unwrap_or(text);
        let lines: Vec<String> = body.split('\n').map(str::to_string).collect();
        Self {
            lines,
            row: 0,
            col: 0,
            want_col: 0,
            version: 0,
            top_version: 0,
            saved_version: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            trailing_newline,
            run: Run::None,
        }
    }

    pub fn to_text(&self) -> String {
        if self.lines.len() == 1 && self.lines[0].is_empty() {
            return String::new();
        }
        let mut out = self.lines.join("\n");
        if self.trailing_newline {
            out.push('\n');
        }
        out
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.run = Run::None;
        self.row = row.min(self.lines.len() - 1);
        self.col = col.min(self.line_len(self.row));
        self.want_col = self.col;
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn mark_saved(&mut self) {
        self.saved_version = self.version;
    }

    pub fn is_dirty(&self) -> bool {
        self.version != self.saved_version
    }

    fn line_len(&self, row: usize) -> usize {
        char_len(&self.lines[row])
    }

    fn replace(&mut self, from: Pos, to: Pos, ins: &str) -> String {
        let (r0, c0) = from;
        let (r1, c1) = to;
        let b0 = byte_idx(&self.lines[r0], c0);
        if r0 == r1 && !ins.contains('\n') {
            let b1 = byte_idx(&self.lines[r0], c1);
            let removed = self.lines[r0][b0..b1].to_string();
            self.lines[r0].replace_range(b0..b1, ins);
            return removed;
        }
        let b1 = byte_idx(&self.lines[r1], c1);
        let removed = if r0 == r1 {
            self.lines[r0][b0..b1].to_string()
        } else {
            let mut s = self.lines[r0][b0..].to_string();
            for line in &self.lines[r0 + 1..r1] {
                s.push('\n');
                s.push_str(line);
            }
            s.push('\n');
            s.push_str(&self.lines[r1][..b1]);
            s
        };
        let tail = self.lines[r1][b1..].to_string();
        self.lines[r0].truncate(b0);
        let mut parts = ins.split('\n');
        self.lines[r0].push_str(parts.next().unwrap_or(""));
        let mut rest: Vec<String> = parts.map(str::to_string).collect();
        match rest.last_mut() {
            Some(last) => last.push_str(&tail),
            None => self.lines[r0].push_str(&tail),
        }
        self.lines.splice(r0 + 1..=r1, rest);
        removed
    }

    fn edit(&mut self, from: Pos, to: Pos, ins: &str, after: Pos, kind: Run) {
        let before = (self.row, self.col);
        let version_before = self.version;
        let merge = kind != Run::None
            && self.run == kind
            && self.undo.last().is_some_and(|last| {
                last.after == before
                    && match kind {
                        Run::Insert => {
                            last.removed.is_empty() && from.0 == last.after.0 && from == before
                        }
                        Run::Back => {
                            last.inserted.is_empty()
                                && !last.removed.contains('\n')
                                && last.start.0 == from.0
                                && last.start.1 == to.1
                        }
                        Run::None => false,
                    }
            });
        let removed = self.replace(from, to, ins);
        self.top_version += 1;
        self.version = self.top_version;
        if merge {
            let last = self.undo.last_mut().expect("merge implies a last edit");
            match kind {
                Run::Insert => last.inserted.push_str(ins),
                _ => {
                    last.removed.insert_str(0, &removed);
                    last.start = from;
                }
            }
            last.after = after;
            last.version_after = self.version;
        } else {
            self.push_undo(Edit {
                start: from,
                removed,
                inserted: ins.to_string(),
                before,
                after,
                version_before,
                version_after: self.version,
            });
            self.redo.clear();
        }
        self.run = kind;
        self.row = after.0;
        self.col = after.1;
        self.want_col = after.1;
    }

    fn push_undo(&mut self, e: Edit) {
        if self.undo.len() >= UNDO_CAP {
            self.undo.remove(0);
        }
        self.undo.push(e);
    }

    pub fn insert_char(&mut self, c: char) {
        if c == '\n' {
            self.newline();
            return;
        }
        let mut buf = [0u8; 4];
        let at = (self.row, self.col);
        self.edit(
            at,
            at,
            c.encode_utf8(&mut buf),
            (at.0, at.1 + 1),
            Run::Insert,
        );
    }

    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let text = s.replace("\r\n", "\n").replace('\r', "\n");
        let at = (self.row, self.col);
        self.edit(at, at, &text, end_of(at, &text), Run::None);
    }

    pub fn newline(&mut self) {
        let line = &self.lines[self.row];
        let indent: String = line
            .chars()
            .take(self.col)
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let text = format!("\n{indent}");
        let at = (self.row, self.col);
        self.edit(at, at, &text, end_of(at, &text), Run::None);
    }

    pub fn backspace(&mut self) {
        let (r, c) = (self.row, self.col);
        if c > 0 {
            self.edit((r, c - 1), (r, c), "", (r, c - 1), Run::Back);
        } else if r > 0 {
            let prev = self.line_len(r - 1);
            self.edit((r - 1, prev), (r, 0), "", (r - 1, prev), Run::None);
        }
    }

    pub fn delete(&mut self) {
        let (r, c) = (self.row, self.col);
        if c < self.line_len(r) {
            self.edit((r, c), (r, c + 1), "", (r, c), Run::None);
        } else if r + 1 < self.lines.len() {
            self.edit((r, c), (r + 1, 0), "", (r, c), Run::None);
        }
    }

    pub fn indent(&mut self) {
        let at = (self.row, self.col);
        self.edit(at, at, INDENT, (at.0, at.1 + INDENT.len()), Run::None);
    }

    pub fn outdent(&mut self) {
        let line = &self.lines[self.row];
        let n = if line.starts_with('\t') {
            1
        } else {
            line.chars()
                .take(INDENT.len())
                .take_while(|c| *c == ' ')
                .count()
        };
        if n == 0 {
            return;
        }
        let r = self.row;
        self.edit(
            (r, 0),
            (r, n),
            "",
            (r, self.col.saturating_sub(n)),
            Run::None,
        );
    }

    fn moved(&mut self, row: usize, col: usize, keep_want: bool) {
        self.run = Run::None;
        self.row = row;
        self.col = col;
        if !keep_want {
            self.want_col = col;
        }
    }

    fn col_for_want(&self, row: usize) -> usize {
        self.want_col.min(self.line_len(row))
    }

    pub fn move_left(&mut self) {
        if self.col > 0 {
            self.moved(self.row, self.col - 1, false);
        } else if self.row > 0 {
            self.moved(self.row - 1, self.line_len(self.row - 1), false);
        }
    }

    pub fn move_right(&mut self) {
        if self.col < self.line_len(self.row) {
            self.moved(self.row, self.col + 1, false);
        } else if self.row + 1 < self.lines.len() {
            self.moved(self.row + 1, 0, false);
        }
    }

    pub fn move_up(&mut self) {
        if self.row > 0 {
            let r = self.row - 1;
            self.moved(r, self.col_for_want(r), true);
        } else {
            self.moved(0, 0, false);
        }
    }

    pub fn move_down(&mut self) {
        if self.row + 1 < self.lines.len() {
            let r = self.row + 1;
            self.moved(r, self.col_for_want(r), true);
        } else {
            self.moved(self.row, self.line_len(self.row), false);
        }
    }

    pub fn home(&mut self) {
        let first = self.lines[self.row]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        let target = if self.col == first { 0 } else { first };
        self.moved(self.row, target, false);
    }

    pub fn end(&mut self) {
        self.moved(self.row, self.line_len(self.row), false);
    }

    pub fn page_up(&mut self, n: usize) {
        let r = self.row.saturating_sub(n);
        self.moved(r, self.col_for_want(r), true);
    }

    pub fn page_down(&mut self, n: usize) {
        let r = (self.row + n).min(self.lines.len() - 1);
        self.moved(r, self.col_for_want(r), true);
    }

    pub fn doc_start(&mut self) {
        self.moved(0, 0, false);
    }

    pub fn doc_end(&mut self) {
        let r = self.lines.len() - 1;
        self.moved(r, self.line_len(r), false);
    }

    pub fn undo(&mut self) -> bool {
        let Some(e) = self.undo.pop() else {
            return false;
        };
        let end = end_of(e.start, &e.inserted);
        self.replace(e.start, end, &e.removed);
        self.version = e.version_before;
        self.moved(e.before.0, e.before.1, false);
        self.redo.push(e);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(e) = self.redo.pop() else {
            return false;
        };
        let end = end_of(e.start, &e.removed);
        self.replace(e.start, end, &e.inserted);
        self.version = e.version_after;
        self.moved(e.after.0, e.after.1, false);
        self.push_undo(e);
        true
    }

    pub fn scroll_to_cursor(&self, scroll: &mut usize, view_h: usize) {
        if view_h == 0 {
            return;
        }
        if self.row < *scroll {
            *scroll = self.row;
        } else if self.row >= *scroll + view_h {
            *scroll = self.row + 1 - view_h;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn typed(b: &mut EditBuf, s: &str) {
        for c in s.chars() {
            b.insert_char(c);
        }
    }

    #[test]
    fn insert_and_delete() {
        let mut b = EditBuf::new("ac\n");
        b.set_cursor(0, 1);
        b.insert_char('b');
        assert_eq!(b.to_text(), "abc\n");
        assert_eq!(b.cursor(), (0, 2));
        b.delete();
        assert_eq!(b.to_text(), "ab\n");
        b.backspace();
        assert_eq!(b.to_text(), "a\n");
    }

    #[test]
    fn newline_auto_indents() {
        let mut b = EditBuf::new("    foo");
        b.doc_end();
        b.newline();
        assert_eq!(b.lines(), ["    foo", "    "]);
        assert_eq!(b.cursor(), (1, 4));
        b.set_cursor(0, 2);
        b.newline();
        assert_eq!(b.lines()[0], "  ");
        assert_eq!(b.lines()[1], "    foo");
    }

    #[test]
    fn join_lines() {
        let mut b = EditBuf::new("ab\ncd");
        b.set_cursor(1, 0);
        b.backspace();
        assert_eq!(b.lines(), ["abcd"]);
        assert_eq!(b.cursor(), (0, 2));
        b.set_cursor(0, 4);
        b.insert_str("\nx");
        b.set_cursor(0, 4);
        b.delete();
        assert_eq!(b.lines(), ["abcdx"]);
    }

    #[test]
    fn indent_outdent() {
        let mut b = EditBuf::new("x");
        b.indent();
        assert_eq!(b.lines()[0], "  x");
        assert_eq!(b.cursor(), (0, 2));
        b.outdent();
        assert_eq!(b.lines()[0], "x");
        assert_eq!(b.cursor(), (0, 0));
        b.outdent();
        assert_eq!(b.lines()[0], "x");
        let mut t = EditBuf::new("\tx");
        t.set_cursor(0, 1);
        t.outdent();
        assert_eq!(t.lines()[0], "x");
        assert_eq!(t.cursor(), (0, 0));
        let mut s = EditBuf::new("   x");
        s.outdent();
        assert_eq!(s.lines()[0], " x");
    }

    #[test]
    fn multi_line_paste() {
        let mut b = EditBuf::new("ab\n");
        b.set_cursor(0, 1);
        b.insert_str("1\n2\n3");
        assert_eq!(b.lines(), ["a1", "2", "3b"]);
        assert_eq!(b.cursor(), (2, 1));
        assert!(b.undo());
        assert_eq!(b.lines(), ["ab"]);
        assert_eq!(b.cursor(), (0, 1));
        assert!(b.redo());
        assert_eq!(b.lines(), ["a1", "2", "3b"]);
    }

    #[test]
    fn crlf_paste() {
        let mut b = EditBuf::new("");
        b.insert_str("a\r\nb\rc");
        assert_eq!(b.lines(), ["a", "b", "c"]);
    }

    #[test]
    fn undo_coalesces_typing() {
        let mut b = EditBuf::new("");
        typed(&mut b, "hello");
        b.newline();
        typed(&mut b, "yo");
        assert!(b.undo());
        assert_eq!(b.lines(), ["hello", ""]);
        assert!(b.undo());
        assert_eq!(b.lines(), ["hello"]);
        assert!(b.undo());
        assert_eq!(b.to_text(), "");
        assert!(!b.undo());
        assert!(b.redo());
        assert_eq!(b.lines(), ["hello"]);
    }

    #[test]
    fn cursor_move_breaks_run() {
        let mut b = EditBuf::new("");
        typed(&mut b, "ab");
        b.move_left();
        b.move_right();
        typed(&mut b, "cd");
        b.undo();
        assert_eq!(b.lines(), ["ab"]);
    }

    #[test]
    fn backspaces_coalesce() {
        let mut b = EditBuf::new("abcd");
        b.end();
        b.backspace();
        b.backspace();
        b.backspace();
        assert_eq!(b.lines(), ["a"]);
        assert!(b.undo());
        assert_eq!(b.lines(), ["abcd"]);
        assert_eq!(b.cursor(), (0, 4));
        assert!(b.redo());
        assert_eq!(b.lines(), ["a"]);
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut b = EditBuf::new("");
        typed(&mut b, "a");
        b.undo();
        b.insert_char('z');
        assert!(!b.redo());
        assert_eq!(b.lines(), ["z"]);
    }

    #[test]
    fn dirty_tracks_saved_state() {
        let mut b = EditBuf::new("a");
        assert!(!b.is_dirty());
        b.insert_char('x');
        assert!(b.is_dirty());
        b.undo();
        assert!(!b.is_dirty());
        b.redo();
        assert!(b.is_dirty());
        b.mark_saved();
        assert!(!b.is_dirty());
        b.undo();
        assert!(b.is_dirty());
        b.redo();
        assert!(!b.is_dirty());
        let v = b.version();
        b.insert_char('q');
        assert!(b.version() > v);
    }

    #[test]
    fn unicode_cursor_and_edits() {
        let mut b = EditBuf::new("é日本x\n");
        b.end();
        assert_eq!(b.cursor(), (0, 4));
        b.move_left();
        b.move_left();
        assert_eq!(b.cursor(), (0, 2));
        b.insert_char('é');
        assert_eq!(b.lines()[0], "é日é本x");
        b.backspace();
        b.backspace();
        assert_eq!(b.lines()[0], "é本x");
        b.delete();
        assert_eq!(b.lines()[0], "éx");
        b.undo();
        assert_eq!(b.lines()[0], "é本x");
        b.undo();
        assert_eq!(b.lines()[0], "é日é本x");
        b.undo();
        assert_eq!(b.lines()[0], "é日本x");
    }

    #[test]
    fn trailing_newline_round_trip() {
        for t in ["a\nb\n", "a\nb", "a\n\n", "a", ""] {
            assert_eq!(EditBuf::new(t).to_text(), t);
        }
        assert!(EditBuf::new("").to_text().is_empty());
    }

    #[test]
    fn vertical_moves_keep_want_col() {
        let mut b = EditBuf::new("abcdef\nab\nabcdef");
        b.set_cursor(0, 5);
        b.move_down();
        assert_eq!(b.cursor(), (1, 2));
        b.move_down();
        assert_eq!(b.cursor(), (2, 5));
        b.move_up();
        b.move_left();
        assert_eq!(b.cursor(), (1, 1));
        b.move_up();
        assert_eq!(b.cursor(), (0, 1));
    }

    #[test]
    fn home_is_smart() {
        let mut b = EditBuf::new("  ab");
        b.end();
        b.home();
        assert_eq!(b.cursor(), (0, 2));
        b.home();
        assert_eq!(b.cursor(), (0, 0));
        b.home();
        assert_eq!(b.cursor(), (0, 2));
    }

    #[test]
    fn page_moves_clamp() {
        let text: String = (0..10).map(|i| format!("l{i}\n")).collect();
        let mut b = EditBuf::new(&text);
        b.page_down(4);
        assert_eq!(b.cursor().0, 4);
        b.page_down(100);
        assert_eq!(b.cursor().0, 9);
        b.page_up(100);
        assert_eq!(b.cursor().0, 0);
        b.doc_end();
        assert_eq!(b.cursor(), (9, 2));
        b.doc_start();
        assert_eq!(b.cursor(), (0, 0));
    }

    #[test]
    fn scroll_keeps_cursor_visible() {
        let text: String = (0..50).map(|i| format!("l{i}\n")).collect();
        let mut b = EditBuf::new(&text);
        let mut scroll = 0;
        b.set_cursor(30, 0);
        b.scroll_to_cursor(&mut scroll, 10);
        assert_eq!(scroll, 21);
        b.set_cursor(5, 0);
        b.scroll_to_cursor(&mut scroll, 10);
        assert_eq!(scroll, 5);
        b.set_cursor(10, 0);
        b.scroll_to_cursor(&mut scroll, 10);
        assert_eq!(scroll, 5);
    }

    #[test]
    fn undo_depth_capped() {
        let mut b = EditBuf::new("");
        for _ in 0..600 {
            b.newline();
        }
        let mut n = 0;
        while b.undo() {
            n += 1;
        }
        assert_eq!(n, UNDO_CAP);
    }

    #[test]
    fn large_buffer_typing_is_fast() {
        let text: String = (0..5000).map(|i| format!("line number {i}\n")).collect();
        let mut b = EditBuf::new(&text);
        b.set_cursor(2500, 5);
        let t = Instant::now();
        for _ in 0..1000 {
            b.insert_char('x');
        }
        assert!(t.elapsed().as_millis() < 50, "{:?}", t.elapsed());
        assert_eq!(b.line_count(), 5000);
    }
}
