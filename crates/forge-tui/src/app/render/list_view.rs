//! The one list component behind the palette, command center, `@file` picker, every picker,
//! the select takeovers and Help.
//!
//! A [`View`] is plain data (rows, query, selection, footer hints); [`render`] draws it as a
//! rounded frame with an optional filter row, group tabs, aligned rows (caret, matched characters
//! in brand, right-aligned metadata), a `… N more` row and a detail line. Selection is a raised
//! fill plus the `❯` caret plus bold, so it survives 16-colour and mono terminals.

use ratatui::widgets::Clear;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::chrome::Hint;
use super::*;
use crate::surface::SurfaceTone;
use crate::theme::glyphs::g;
use crate::theme::styles;

/// Cells of padding inside the frame on each side of a row.
const PAD: usize = 2;

/// One selectable row.
#[derive(Debug, Clone, Default)]
pub(crate) struct Row {
    pub label: String,
    /// Base style of the label (unselected). Defaults to `fg`.
    pub label_style: Option<Style>,
    pub desc: String,
    /// Right-aligned metadata (keybinding, tag, price).
    pub meta: String,
    pub meta_style: Option<Style>,
    /// Char indices of `label` that matched the query.
    pub marks: Vec<usize>,
}

#[derive(Debug, Clone)]
pub(crate) enum Item {
    Header(String),
    Row(Row),
}

/// The filter row: a label, what was typed so far and the placeholder shown while it is empty.
#[derive(Debug, Clone)]
pub(crate) struct Filter {
    pub label: &'static str,
    pub query: String,
    pub placeholder: &'static str,
}

/// Everything [`render`] needs.
#[derive(Debug, Clone)]
pub(crate) struct View {
    pub title: String,
    /// Right side of the top border (count, shortcut).
    pub context: String,
    pub filter: Option<Filter>,
    /// Group tabs and the active one.
    pub tabs: Option<(Vec<String>, usize)>,
    pub items: Vec<Item>,
    /// Index among the *rows* (headers do not count).
    pub selected: usize,
    pub detail: Option<Vec<Span<'static>>>,
    /// Footer hints drawn into the bottom border (modals only; attached popups leave the hints to
    /// the composer).
    pub footer: Vec<Hint>,
    pub empty: String,
    /// What follows `… N more` on the overflow row.
    pub more_hint: &'static str,
    /// One blank row of padding above and below the content (modals).
    pub padded: bool,
    pub tone: SurfaceTone,
}

impl Default for View {
    fn default() -> Self {
        Self {
            title: String::new(),
            context: String::new(),
            filter: None,
            tabs: None,
            items: Vec::new(),
            selected: 0,
            detail: None,
            footer: Vec::new(),
            empty: "nothing matches".to_string(),
            more_hint: "keep typing to filter",
            padded: false,
            tone: SurfaceTone::Tool,
        }
    }
}

impl View {
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.items.iter().filter_map(|i| match i {
            Item::Row(r) => Some(r),
            Item::Header(_) => None,
        })
    }

    /// Rows the frame needs to show everything, capped at `max_inner` content rows.
    pub fn desired_height(&self, max_inner: usize) -> u16 {
        let head = self.head_rows();
        let foot = self.foot_rows();
        let body = if self.items.is_empty() {
            1
        } else {
            self.items.len()
        };
        (body + head + foot).min(max_inner).max(1) as u16 + 2
    }

    fn head_rows(&self) -> usize {
        usize::from(self.padded)
            + usize::from(self.filter.is_some())
            + usize::from(self.tabs.is_some())
            + usize::from(self.padded && (self.filter.is_some() || self.tabs.is_some()))
    }

    fn foot_rows(&self) -> usize {
        match (&self.detail, self.padded) {
            (Some(_), true) => 2 + 1,
            (Some(_), false) => 2,
            (None, true) => 1,
            (None, false) => 0,
        }
    }
}

/// The geometry [`render`] used, for tests and mouse hit-testing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Geometry {
    pub first_row_y: u16,
    pub visible_items: usize,
    pub hidden_rows: usize,
}

/// Case-insensitive char positions of `query` in `text`: the contiguous run when it is a
/// substring, otherwise the leftmost subsequence. Empty when there is no match.
pub(crate) fn match_positions(text: &str, query: &str) -> Vec<usize> {
    let query = query.trim().trim_start_matches('/').to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let hay: Vec<char> = text.to_lowercase().chars().collect();
    let needle: Vec<char> = query.chars().collect();
    if let Some(start) = hay
        .windows(needle.len().max(1))
        .position(|w| w == needle.as_slice())
    {
        return (start..start + needle.len()).collect();
    }
    let mut out = Vec::new();
    let mut at = 0;
    for c in &needle {
        match hay[at..].iter().position(|h| h == c) {
            Some(p) => {
                out.push(at + p);
                at += p + 1;
            }
            None => return Vec::new(),
        }
    }
    out
}

/// Typographic punctuation swapped for its ASCII twin, so text that arrives with `—` or `…`
/// does not break the ASCII glyph tier. A no-op in the Unicode tier.
pub(crate) fn plain(s: &str) -> std::borrow::Cow<'_, str> {
    if g().unicode || s.is_ascii() {
        return s.into();
    }
    s.replace(['—', '–'], "-")
        .replace('…', "...")
        .replace('·', "-")
        .replace('→', "->")
        .replace('●', "*")
        .replace('○', "o")
        .replace(['“', '”'], "\"")
        .replace(['‘', '’'], "'")
        .into()
}

/// `s` clipped to `width` cells with a trailing ellipsis.
pub(crate) fn fit(s: &str, width: usize) -> String {
    let s = plain(s);
    let s = s.as_ref();
    if s.width() <= width {
        return s.to_string();
    }
    let ellipsis = g().ellipsis;
    let room = width.saturating_sub(ellipsis.width());
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > room {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push_str(ellipsis);
    out
}

fn pad_to(spans: &mut Vec<Span<'static>>, used: usize, width: usize) {
    if used < width {
        spans.push(Span::raw(" ".repeat(width - used)));
    }
}

fn row_line(
    row: &Row,
    selected: bool,
    width: usize,
    label_w: usize,
    meta_w: usize,
) -> TextLine<'static> {
    let caret = if selected {
        format!("{} ", g().prompt)
    } else {
        "  ".to_string()
    };
    let base = row.label_style.unwrap_or_else(styles::fg);
    let label_style = if selected { base.bold() } else { base };
    let mut spans: Vec<Span<'static>> = vec![Span::styled(caret, styles::brand().bold())];
    let label = fit(&row.label, label_w);
    let label_cells = label.width();
    let marks: std::collections::HashSet<usize> = row.marks.iter().copied().collect();
    if marks.is_empty() {
        spans.push(Span::styled(label, label_style));
    } else {
        for (i, c) in label.chars().enumerate() {
            let style = if marks.contains(&i) {
                styles::brand().bold()
            } else {
                label_style
            };
            spans.push(Span::styled(c.to_string(), style));
        }
    }
    let mut used = 2 + label_cells;
    if !row.desc.is_empty() || meta_w > 0 {
        let gap = label_w.saturating_sub(label_cells) + 2;
        spans.push(Span::raw(" ".repeat(gap)));
        used += gap;
        let meta_cells = if meta_w > 0 { meta_w + 2 } else { 0 };
        let desc_room = width.saturating_sub(used + meta_cells);
        let desc = fit(&row.desc, desc_room);
        let desc_cells = desc.width();
        if !desc.is_empty() {
            spans.push(Span::styled(desc, styles::muted()));
        }
        used += desc_cells;
        if meta_w > 0 {
            let meta = fit(&row.meta, meta_w);
            let meta_used = meta.width();
            let fill = width.saturating_sub(used + meta_used);
            spans.push(Span::raw(" ".repeat(fill)));
            spans.push(Span::styled(
                meta,
                row.meta_style.unwrap_or_else(styles::faint),
            ));
            used = width;
        }
    }
    pad_to(&mut spans, used, width);
    let mut line = TextLine::from(spans);
    if selected {
        line = line.style(styles::selected_row());
    }
    line
}

fn indent(mut line: TextLine<'static>) -> TextLine<'static> {
    let style = line.style;
    line.spans.insert(0, Span::raw(" ".repeat(PAD)));
    line.style = style;
    line
}

/// Draw `view` into `area` (frame included). Returns the geometry it used.
pub(crate) fn render(frame: &mut Frame, area: Rect, view: &View) -> Geometry {
    if area.width < 12 || area.height < 3 {
        return Geometry::default();
    }
    let footer = if view.footer.is_empty() {
        None
    } else {
        let hints =
            super::chrome::fit_hints(view.footer.clone(), (area.width as usize).saturating_sub(6));
        let mut spans = vec![Span::raw(" ")];
        for (i, h) in hints.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(format!(" {} ", g().sep), styles::faint()));
            }
            spans.extend(styles::hint_spans(&h.key, h.label));
        }
        spans.push(Span::raw(" "));
        Some(spans)
    };
    let title = Span::styled(
        format!(" {} ", view.title),
        styles::fg().add_modifier(Modifier::BOLD),
    );
    let context = (!view.context.is_empty())
        .then(|| Span::styled(format!(" {} ", view.context), styles::faint()));
    let inner = frame_it(frame, area, title, context, footer, view.tone);

    let width = (inner.width as usize).saturating_sub(PAD * 2);
    let mut head: Vec<TextLine<'static>> = Vec::new();
    if view.padded {
        head.push(TextLine::default());
    }
    if let Some(f) = &view.filter {
        let mut spans = vec![Span::styled(format!("{}  ", f.label), styles::muted())];
        if f.query.is_empty() {
            let bar = if g().unicode { "▏" } else { "_" };
            spans.push(Span::styled(bar, styles::brand()));
            spans.push(Span::styled(f.placeholder, styles::faint()));
        } else {
            spans.push(Span::styled(f.query.clone(), styles::fg()));
            spans.push(Span::styled(
                if g().unicode { "▏" } else { "_" },
                styles::brand(),
            ));
        }
        head.push(TextLine::from(spans));
    }
    if let Some((names, active)) = &view.tabs {
        let mut spans = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let style = if i == *active {
                styles::brand().bold().add_modifier(Modifier::UNDERLINED)
            } else {
                styles::muted()
            };
            spans.push(Span::styled(name.clone(), style));
            spans.push(Span::raw("  "));
        }
        head.push(TextLine::from(spans));
    }
    if view.padded && (view.filter.is_some() || view.tabs.is_some()) {
        head.push(TextLine::default());
    }
    let mut foot: Vec<TextLine<'static>> = Vec::new();
    if let Some(detail) = &view.detail {
        foot.push(TextLine::default());
        let fitted = super::span_fit::fit_spans(detail.clone(), width);
        foot.push(TextLine::from(
            fitted
                .into_iter()
                .map(|s| Span::styled(plain(&s.content).into_owned(), s.style))
                .collect::<Vec<_>>(),
        ));
    }
    if view.padded {
        foot.push(TextLine::default());
    }

    let inner_h = inner.height as usize;
    // Squeeze the padding before the list loses a row.
    while head.len() + foot.len() + 1 > inner_h && !foot.is_empty() {
        foot.pop();
    }
    while head.len() + foot.len() + 1 > inner_h && head.len() > 1 {
        head.pop();
    }
    let cap = inner_h.saturating_sub(head.len() + foot.len()).max(1);

    let mut body: Vec<TextLine<'static>> = Vec::new();
    let mut layout = Geometry {
        first_row_y: inner.y + head.len() as u16,
        ..Geometry::default()
    };
    if view.items.is_empty() {
        body.push(indent(TextLine::from(Span::styled(
            view.empty.clone(),
            styles::muted(),
        ))));
    } else {
        let label_w = view
            .rows()
            .map(|r| r.label.width())
            .max()
            .unwrap_or(0)
            .min(14)
            .min(width / 2)
            .max(4);
        let meta_w = view
            .rows()
            .map(|r| r.meta.width())
            .max()
            .unwrap_or(0)
            .min(width / 3);
        // Item index of the selected row.
        let mut seen = 0usize;
        let mut sel_item = 0usize;
        for (i, item) in view.items.iter().enumerate() {
            if let Item::Row(_) = item {
                if seen == view.selected {
                    sel_item = i;
                    break;
                }
                seen += 1;
            }
        }
        let total = view.items.len();
        let (shown, overflow) = if total > cap {
            (cap.saturating_sub(1).max(1), true)
        } else {
            (total, false)
        };
        let mut start = sel_item.saturating_sub(shown / 2);
        start = start.min(total.saturating_sub(shown));
        if start > 0
            && start == sel_item
            && matches!(view.items.get(start - 1), Some(Item::Header(_)))
        {
            start -= 1;
        }
        let end = (start + shown).min(total);
        let mut row_index = view.items[..start]
            .iter()
            .filter(|i| matches!(i, Item::Row(_)))
            .count();
        for item in &view.items[start..end] {
            match item {
                Item::Header(text) => body.push(indent(TextLine::from(Span::styled(
                    text.to_uppercase(),
                    styles::muted().add_modifier(Modifier::BOLD),
                )))),
                Item::Row(row) => {
                    body.push(indent(row_line(
                        row,
                        row_index == view.selected,
                        width,
                        label_w,
                        meta_w,
                    )));
                    row_index += 1;
                }
            }
        }
        layout.visible_items = end - start;
        if overflow {
            let hidden = view.items[..start]
                .iter()
                .chain(view.items[end..].iter())
                .filter(|i| matches!(i, Item::Row(_)))
                .count();
            layout.hidden_rows = hidden;
            let hint = if view.more_hint.is_empty() {
                String::new()
            } else {
                format!(" {} {}", g().sep, view.more_hint)
            };
            body.push(indent(TextLine::from(Span::styled(
                format!("{} {hidden} more{hint}", g().ellipsis),
                styles::faint(),
            ))));
        }
    }

    let mut lines: Vec<TextLine<'static>> = head.into_iter().map(indent).collect();
    lines.extend(body);
    // Detail and padding sit at the bottom of the frame, not right under a short list.
    let gap = inner_h.saturating_sub(lines.len() + foot.len());
    lines.extend(std::iter::repeat_with(TextLine::default).take(gap));
    lines.extend(foot.into_iter().map(indent));
    frame.render_widget(Paragraph::new(lines), inner);
    layout
}

/// Clear and frame `area` with the title on the left of the top border, the context on the right
/// and the footer on the right of the bottom border; returns the inside.
fn frame_it(
    frame: &mut Frame,
    area: Rect,
    title: Span<'static>,
    context: Option<Span<'static>>,
    footer: Option<Vec<Span<'static>>>,
    tone: SurfaceTone,
) -> Rect {
    let border = styles::line();
    let h = g().border.horizontal_top;
    let mut block = Block::bordered()
        .border_set(g().border)
        .border_style(border)
        .style(styles::surface())
        .title(TextLine::from(vec![Span::styled(h, border), title]));
    if let Some(context) = context {
        block = block.title(TextLine::from(vec![context, Span::styled(h, border)]).right_aligned());
    }
    if let Some(mut footer) = footer {
        footer.push(Span::styled(g().border.horizontal_bottom, border));
        block = block.title_bottom(TextLine::from(footer).right_aligned());
    }
    let _ = tone;
    crate::surface::note_panel(area);
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(styles::surface()), area);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn row(label: &str, desc: &str, meta: &str) -> Row {
        Row {
            label: label.into(),
            desc: desc.into(),
            meta: meta.into(),
            ..Row::default()
        }
    }

    fn draw(view: &View, w: u16, h: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|f| {
                render(f, f.area(), view);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn sample(n: usize) -> View {
        View {
            title: "Commands".into(),
            context: format!("{n}"),
            items: (0..n)
                .map(|i| Item::Row(row(&format!("/cmd{i}"), "does a thing", "Alt+T")))
                .collect(),
            ..View::default()
        }
    }

    #[test]
    fn rows_align_and_the_selected_one_has_the_caret() {
        for width in [80u16, 160] {
            let mut view = sample(3);
            view.selected = 1;
            let lines = draw(&view, width, 8);
            let sel = lines.iter().find(|l| l.contains("/cmd1")).unwrap();
            assert!(sel.contains(&format!("{} /cmd1", g().prompt)), "{sel}");
            let other = lines.iter().find(|l| l.contains("/cmd0")).unwrap();
            assert!(other.contains("  /cmd0"), "{other}");
            assert!(sel.trim_end().ends_with("│"), "{sel}");
            assert!(sel.contains("Alt+T"));
            assert_eq!(sel.chars().count(), width as usize);
        }
    }

    #[test]
    fn overflow_shows_a_more_row_and_keeps_the_selection_visible() {
        let mut view = sample(40);
        view.selected = 30;
        let lines = draw(&view, 80, 10);
        let text = lines.join("\n");
        assert!(text.contains("/cmd30"), "{text}");
        assert!(text.contains("more"), "{text}");
        assert!(text.contains("keep typing to filter"), "{text}");
    }

    #[test]
    fn a_detail_line_sits_at_the_bottom_and_an_empty_list_says_why() {
        let mut view = sample(2);
        view.detail = Some(vec![Span::raw("usage  /cmd0 [x]")]);
        let lines = draw(&view, 80, 9);
        assert!(lines[7].contains("usage  /cmd0 [x]"), "{lines:?}");
        let mut empty = sample(0);
        empty.empty = "no commands match".into();
        assert!(draw(&empty, 80, 6).join("\n").contains("no commands match"));
    }

    #[test]
    fn descriptions_clip_with_an_ellipsis_instead_of_wrapping() {
        let mut view = sample(1);
        view.items = vec![Item::Row(row("/x", &"long ".repeat(40), "F1"))];
        let lines = draw(&view, 60, 5);
        let l = lines.iter().find(|l| l.contains("/x")).unwrap();
        assert!(l.contains('…') || !g().unicode, "{l}");
        assert!(l.contains("F1"));
    }

    #[test]
    fn match_positions_prefer_a_contiguous_run_then_a_subsequence() {
        assert_eq!(match_positions("/models", "mo"), vec![1, 2]);
        assert_eq!(match_positions("compact", "cpt"), vec![0, 3, 6]);
        assert!(match_positions("help", "zz").is_empty());
        assert!(match_positions("help", "").is_empty());
    }

    #[test]
    fn tiny_areas_draw_nothing_instead_of_panicking() {
        let view = sample(3);
        let _ = draw(&view, 80, 3);
        let mut terminal = Terminal::new(TestBackend::new(8, 2)).unwrap();
        terminal
            .draw(|f| {
                render(f, f.area(), &view);
            })
            .unwrap();
    }
}
