//! Drawing the diff overlay: a file list beside the diff on wide terminals, the diff in unified
//! or side-by-side form, and a one-row key hint. Takes over the whole frame, like the output viewer.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{DiffLine, DiffOverlay, LineKind, OverlayFile, Row};
use crate::theme::glyphs::g;
use crate::theme::{styles, t};

/// The file list opens beside the diff from this width.
const LIST_MIN_WIDTH: u16 = 100;
const LIST_WIDTH: u16 = 32;
/// Side-by-side needs two readable columns.
const SPLIT_MIN_WIDTH: u16 = 70;

fn fit(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > width {
            if used < width {
                out.push_str(g().ellipsis);
            }
            return out;
        }
        out.push(if ch == '\t' { ' ' } else { ch });
        used += w;
    }
    out
}

fn pad(text: String, width: usize) -> String {
    let used = text.width();
    if used >= width {
        text
    } else {
        format!("{text}{}", " ".repeat(width - used))
    }
}

fn line_style(kind: LineKind) -> Style {
    let p = t().palette;
    match kind {
        LineKind::Add => p.diff_add_bg.style().patch(styles::fg()),
        LineKind::Del => p.diff_del_bg.style().patch(styles::fg()),
        LineKind::Context => styles::muted(),
    }
}

fn sigil(kind: LineKind) -> (&'static str, Style) {
    match kind {
        LineKind::Add => ("+", styles::ok()),
        LineKind::Del => (g().diff_minus, styles::err()),
        LineKind::Context => (" ", styles::faint()),
    }
}

fn number(n: Option<u32>, width: usize) -> String {
    match n {
        Some(n) => format!("{n:>width$}"),
        None => " ".repeat(width),
    }
}

fn unified_row(line: &DiffLine, gutter: usize, width: usize) -> Line<'static> {
    let (mark, mark_style) = sigil(line.kind);
    let body = width.saturating_sub(gutter * 2 + 4);
    let style = line_style(line.kind);
    Line::from(vec![
        Span::styled(
            format!(
                "{} {} ",
                number(line.old_no, gutter),
                number(line.new_no, gutter)
            ),
            styles::faint(),
        ),
        Span::styled(format!("{mark} "), mark_style.patch(style)),
        Span::styled(pad(fit(&line.text, body), body), style),
    ])
}

fn half(line: Option<&DiffLine>, gutter: usize, width: usize, old: bool) -> Vec<Span<'static>> {
    let body = width.saturating_sub(gutter + 3);
    let Some(line) = line else {
        return vec![Span::raw(" ".repeat(width))];
    };
    let no = if old { line.old_no } else { line.new_no };
    let (mark, mark_style) = sigil(line.kind);
    let style = line_style(line.kind);
    vec![
        Span::styled(format!("{} ", number(no, gutter)), styles::faint()),
        Span::styled(format!("{mark} "), mark_style.patch(style)),
        Span::styled(pad(fit(&line.text, body), body), style),
    ]
}

fn body_lines(app: &DiffOverlay, file: &OverlayFile, area: Rect) -> Vec<Line<'static>> {
    let width = usize::from(area.width);
    let split = app.split && area.width >= SPLIT_MIN_WIDTH;
    let max_no = file
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .filter_map(|l| l.old_no.max(l.new_no))
        .max()
        .unwrap_or(1);
    let gutter = max_no.to_string().len().max(3);
    let (rows, _) = super::layout(file, split);
    let mut out = Vec::new();
    if let Some(note) = &file.note {
        out.push(Line::from(Span::styled(format!(" {note}"), styles::warn())));
    }
    let skip = app.scroll().saturating_sub(0);
    let reserved = out.len();
    for row in rows
        .iter()
        .skip(skip)
        .take(usize::from(area.height).saturating_sub(reserved))
    {
        out.push(match row {
            Row::Hunk(h) => Line::from(Span::styled(
                pad(fit(&format!(" {}", file.hunks[*h].header), width), width),
                styles::brand().patch(styles::surface()),
            )),
            Row::Line(h, l) => unified_row(&file.hunks[*h].lines[*l], gutter, width),
            Row::Pair(h, left, right) => {
                let lines = &file.hunks[*h].lines;
                let col = (width.saturating_sub(1)) / 2;
                let mut spans = half(left.map(|i| &lines[i]), gutter, col, true);
                spans.push(Span::styled(
                    if g().unicode { "│" } else { "|" },
                    styles::line(),
                ));
                spans.extend(half(
                    right.map(|i| &lines[i]),
                    gutter,
                    width - col - 1,
                    false,
                ));
                Line::from(spans)
            }
        });
    }
    out
}

fn list_lines(app: &DiffOverlay, area: Rect) -> Vec<Line<'static>> {
    let files = app.files();
    let height = usize::from(area.height);
    let start = (app.file_index() + 1).saturating_sub(height);
    let width = usize::from(area.width);
    files
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, f)| {
            let selected = i == app.file_index();
            let mark = if app.is_reviewed(f) { g().ok } else { " " };
            let caret = if selected { g().prompt } else { " " };
            let stat = format!("+{} {}{}", f.adds, g().diff_minus, f.dels);
            let name_w = width.saturating_sub(stat.width() + 7);
            let name = fit(&f.path, name_w);
            let fill = if selected {
                styles::selected_row()
            } else {
                Style::default()
            };
            let name_style = if app.is_reviewed(f) {
                styles::faint()
            } else {
                styles::fg()
            };
            Line::from(vec![
                Span::styled(format!("{caret} "), styles::brand().patch(fill)),
                Span::styled(format!("{mark} "), styles::ok().patch(fill)),
                Span::styled(
                    format!("{} ", f.status.letter()),
                    styles::muted().patch(fill),
                ),
                Span::styled(pad(name, name_w), name_style.patch(fill)),
                Span::styled(format!(" {stat}"), styles::faint().patch(fill)),
            ])
        })
        .collect()
}

fn header(app: &DiffOverlay, width: usize) -> Line<'static> {
    let source = &app.sources()[app.source_index()];
    let total = app.files().len();
    let mut spans = vec![
        Span::styled(" Diff ", styles::brand().bold()),
        Span::styled(source.label.clone(), styles::fg()),
    ];
    if total > 0 {
        spans.push(Span::styled(
            format!(
                "  file {}/{}  reviewed {}/{}",
                app.file_index() + 1,
                total,
                app.reviewed_count(),
                total
            ),
            styles::muted(),
        ));
    }
    let (adds, dels) = app
        .files()
        .iter()
        .fold((0, 0), |(a, d), f| (a + f.adds, d + f.dels));
    if total > 0 {
        spans.push(Span::styled(format!("  +{adds}"), styles::ok()));
        spans.push(Span::styled(
            format!(" {}{dels}", g().diff_minus),
            styles::err(),
        ));
    }
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let view = if app.split { "split" } else { "unified" };
    if used + view.len() + 2 < width {
        spans.push(Span::raw(" ".repeat(width - used - view.len() - 1)));
        spans.push(Span::styled(view, styles::faint()));
    }
    Line::from(spans)
}

fn empty_message(app: &DiffOverlay) -> String {
    if app.loading {
        return "reading git…".to_string();
    }
    if let Some(error) = &app.error {
        return error.clone();
    }
    if app.source_index() == super::TURN {
        "the agent has not changed any file this turn".to_string()
    } else {
        "no uncommitted changes".to_string()
    }
}

pub fn render_diff_overlay(frame: &mut Frame, app: &DiffOverlay) {
    if !app.open {
        return;
    }
    let full = frame.area();
    frame.render_widget(Clear, full);
    if full.height < 4 || full.width < 20 {
        return;
    }
    let width = usize::from(full.width);
    frame.render_widget(
        Paragraph::new(header(app, width)),
        Rect { height: 1, ..full },
    );
    let body = Rect {
        y: full.y + 1,
        height: full.height - 2,
        ..full
    };
    let hint = styles::hint_row(&[
        ("j/k", "scroll"),
        ("]/[", "hunk"),
        ("n/p", "file"),
        ("v", "split"),
        ("d", "source"),
        ("m", "reviewed"),
        ("Esc", "close"),
    ]);
    frame.render_widget(
        Paragraph::new(Line::from(hint)),
        Rect {
            y: full.y + full.height - 1,
            height: 1,
            ..full
        },
    );
    app.set_view_height(usize::from(body.height));
    let Some(file) = app.current() else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {}", empty_message(app)),
                styles::muted(),
            ))),
            body,
        );
        return;
    };
    let (list, diff) = if full.width >= LIST_MIN_WIDTH {
        (
            Some(Rect {
                width: LIST_WIDTH,
                ..body
            }),
            Rect {
                x: body.x + LIST_WIDTH + 1,
                width: body.width - LIST_WIDTH - 1,
                ..body
            },
        )
    } else {
        (None, body)
    };
    if let Some(list) = list {
        frame.render_widget(Paragraph::new(list_lines(app, list)), list);
        let rule = if g().unicode { "│" } else { "|" };
        let rules: Vec<Line> = (0..body.height)
            .map(|_| Line::from(Span::styled(rule, styles::line())))
            .collect();
        frame.render_widget(
            Paragraph::new(rules),
            Rect {
                x: body.x + LIST_WIDTH,
                width: 1,
                ..body
            },
        );
    }
    // The file's own name leads the diff when there is no list to say which file this is.
    let mut diff = diff;
    if list.is_none() {
        let reviewed = if app.is_reviewed(file) {
            format!(" {}", g().ok)
        } else {
            String::new()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {} ", file.status.letter()), styles::muted()),
                Span::styled(file.path.clone(), styles::fg().bold()),
                Span::styled(reviewed, styles::ok()),
            ])),
            Rect { height: 1, ..diff },
        );
        diff = Rect {
            y: diff.y + 1,
            height: diff.height.saturating_sub(1),
            ..diff
        };
    }
    let narrow = diff.width < SPLIT_MIN_WIDTH;
    app.set_diff_area(
        usize::from(diff.height).saturating_sub(usize::from(file.note.is_some())),
        narrow,
    );
    frame.render_widget(Paragraph::new(body_lines(app, file, diff)), diff);
}
