//! Renders the `/worktree` and `/project` overlay: a centered card that reads the same at 80x24
//! and on a wide terminal, scrolling its list to keep the focused row in view.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::{Choice, Gh, Input, InputKind, Item, Kind, Screen, Tone, WorkspaceData, WorkspaceMenu};
use crate::app::App;
use crate::surface::{self, SurfaceTone};
use crate::theme::{glyphs::g, styles};

fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn trunc(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        if width(&out) + width(c.encode_utf8(&mut [0; 4])) + 1 > max {
            break;
        }
        out.push(c);
    }
    out.push_str(g().ellipsis);
    out
}

/// Keep the tail of a path, which is the part that tells worktrees apart.
fn trunc_left(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut tail = String::new();
    for c in chars.iter().rev() {
        if width(&tail) + width(c.encode_utf8(&mut [0; 4])) + 1 > max {
            break;
        }
        tail.insert(0, *c);
    }
    format!("{}{tail}", g().ellipsis)
}

fn pad(text: &str, to: usize) -> String {
    format!("{text}{}", " ".repeat(to.saturating_sub(width(text))))
}

fn wrap(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && width(&line) + 1 + width(word) > max {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        if width(word) > max {
            // A path longer than the card: break it rather than let the panel clip it.
            let mut piece = String::new();
            for c in word.chars() {
                if width(&piece) + width(c.encode_utf8(&mut [0; 4])) > max {
                    out.push(std::mem::take(&mut piece));
                }
                piece.push(c);
            }
            line = piece;
            continue;
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Ok => styles::ok(),
        Tone::Warn => styles::warn(),
        Tone::Err => styles::err(),
    }
}

fn tone_glyph(tone: Tone) -> &'static str {
    match tone {
        Tone::Ok => g().ok,
        Tone::Warn => g().warn,
        Tone::Err => g().fail,
    }
}

/// Card height for `content` rows: borders, plus the blank rows `padded_panel` adds when tall.
fn card_rows(content: u16) -> u16 {
    let padded = content + 2 + 2 * crate::overlays::PAD_Y;
    if padded >= crate::overlays::SHORT_REGION {
        padded
    } else {
        content + 2
    }
}

struct Body {
    lines: Vec<Line<'static>>,
    /// The line the list cursor is on, for scrolling.
    focus: Option<usize>,
}

fn row_line(focused: bool, mut spans: Vec<Span<'static>>) -> Line<'static> {
    let caret = if focused { g().prompt } else { " " };
    spans.insert(
        0,
        Span::styled(
            format!("{caret} "),
            if focused {
                styles::brand()
            } else {
                styles::faint()
            },
        ),
    );
    let mut line = Line::from(spans);
    if focused {
        line.style = styles::selected_row();
    }
    line
}

fn search_line(m: &WorkspaceMenu, w: usize, count: usize) -> Line<'static> {
    if m.searching {
        Line::from(vec![
            Span::styled("/ ", styles::brand()),
            Span::styled(m.filter.clone(), styles::fg()),
            Span::styled("\u{2581}", styles::brand()),
            Span::styled(format!("   {count} match(es)"), styles::faint()),
        ])
    } else if !m.filter.is_empty() {
        Line::from(vec![
            Span::styled("/ ", styles::faint()),
            Span::styled(trunc(&m.filter, w.saturating_sub(24)), styles::fg()),
            Span::styled(format!("   {count} match(es), Esc clears"), styles::faint()),
        ])
    } else {
        Line::from(Span::styled("/ to search", styles::faint()))
    }
}

fn worktree_rows(m: &WorkspaceMenu, data: &WorkspaceData, w: usize, body: &mut Body) {
    let items = m.items();
    if let Some(error) = &data.error {
        for part in wrap(error, w) {
            body.lines
                .push(Line::from(Span::styled(part, styles::warn())));
        }
        return;
    }
    if items.is_empty() {
        body.lines.push(Line::from(Span::styled(
            "No worktree matches.",
            styles::muted(),
        )));
        return;
    }
    let branch_w = data
        .worktrees
        .iter()
        .map(|r| width(&r.branch).max(8))
        .max()
        .unwrap_or(8)
        .min(w / 3);
    for (pos, item) in items.iter().enumerate() {
        let Item::Worktree(i) = item else { continue };
        let Some(row) = data.worktrees.get(*i) else {
            continue;
        };
        let mut tags: Vec<(&str, Style)> = Vec::new();
        if row.current {
            tags.push(("current", styles::ok()));
        }
        if row.main {
            tags.push(("main", styles::muted()));
        }
        if row.dirty {
            tags.push(("dirty", styles::warn()));
        }
        let tag_w: usize = tags.iter().map(|(t, _)| width(t) + 1).sum();
        let path_w = w.saturating_sub(2 + 2 + branch_w + 2 + tag_w + 1).max(8);
        let focused = pos == m.cursor;
        let mut spans = vec![
            Span::styled(format!("{} ", g().branch), styles::brand()),
            Span::styled(
                pad(&trunc(&row.branch, branch_w), branch_w),
                styles::fg().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                pad(&trunc_left(&row.display, path_w), path_w),
                styles::muted(),
            ),
        ];
        for (tag, style) in tags {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(tag.to_string(), style));
        }
        if focused {
            body.focus = Some(body.lines.len());
        }
        body.lines.push(row_line(focused, spans));
    }
}

fn project_rows(m: &WorkspaceMenu, data: &WorkspaceData, w: usize, body: &mut Body) {
    let items = m.items();
    if items.is_empty() {
        body.lines.push(Line::from(Span::styled(
            "No entry matches.",
            styles::muted(),
        )));
        return;
    }
    for (pos, item) in items.iter().enumerate() {
        let (label, hint) = match item {
            Item::Home => (
                "Home (no project)".to_string(),
                "a scratch folder for chats that need no repository".to_string(),
            ),
            Item::Clone => (
                "Clone a repository...".to_string(),
                format!("into {}", data.clone_parent),
            ),
            Item::InitHere => (
                "Initialize git here".to_string(),
                "this folder is not a git repository".to_string(),
            ),
            Item::Rename => (
                "Rename this project".to_string(),
                format!("now {}", data.project_name),
            ),
            Item::Hide => (
                "Hide this project".to_string(),
                "from project lists; files stay".to_string(),
            ),
            Item::Worktree(_) => continue,
        };
        let focused = pos == m.cursor;
        if focused {
            body.focus = Some(body.lines.len());
        }
        let label_w = 26;
        body.lines.push(row_line(
            focused,
            vec![
                Span::styled(
                    pad(&label, label_w),
                    styles::fg().add_modifier(Modifier::BOLD),
                ),
                Span::styled(trunc(&hint, w.saturating_sub(label_w + 2)), styles::faint()),
            ],
        ));
    }
}

fn list_body(m: &WorkspaceMenu, w: usize) -> Body {
    let mut body = Body {
        lines: Vec::new(),
        focus: None,
    };
    let count = m.items().len();
    body.lines.push(search_line(m, w, count));
    body.lines.push(Line::default());
    let Some(data) = &m.data else {
        body.lines.push(Line::from(vec![
            Span::styled(
                g().spin(m.tick as usize).to_string(),
                styles::spinner(m.tick as usize),
            ),
            Span::styled(" reading the workspace", styles::muted()),
        ]));
        return body;
    };
    match m.kind {
        Kind::Worktrees => {
            if !data.is_git {
                body.lines.push(Line::from(Span::styled(
                    "This folder is not a git repository, so it has no worktrees.",
                    styles::muted(),
                )));
                body.lines.push(Line::from(vec![
                    Span::styled("/project", styles::brand()),
                    Span::styled(" can initialize one.", styles::muted()),
                ]));
            } else {
                worktree_rows(m, data, w, &mut body);
            }
        }
        Kind::Projects => {
            project_rows(m, data, w, &mut body);
            body.lines.push(Line::default());
            body.lines.push(Line::from(vec![
                Span::styled("Working in ", styles::faint()),
                Span::styled(
                    trunc_left(&data.root, w.saturating_sub(12)),
                    styles::muted(),
                ),
            ]));
        }
    }
    body
}

fn input_body(m: &WorkspaceMenu, input: &Input, w: usize) -> Body {
    let mut lines = Vec::new();
    let (label, help) = match input.kind {
        InputKind::WorktreeName => (
            "Name",
            "Becomes the branch forge/<name> and the worktree folder.",
        ),
        InputKind::CloneUrl => (
            "Repository",
            "Paste an https, ssh or git URL, or pick one of your GitHub repositories.",
        ),
        InputKind::RenameProject => (
            "Project name",
            "Shown in project lists. Empty restores the folder name.",
        ),
    };
    for part in wrap(help, w) {
        lines.push(Line::from(Span::styled(part, styles::muted())));
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled(format!("{label}  "), styles::muted()),
        Span::styled(
            trunc_left(&input.value, w.saturating_sub(width(label) + 4)),
            styles::fg(),
        ),
        Span::styled("\u{2581}", styles::brand()),
    ]));
    if let Some(error) = &input.error {
        for part in wrap(error, w) {
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", g().fail), styles::err()),
                Span::styled(part, styles::err()),
            ]));
        }
    } else {
        lines.push(Line::default());
    }
    let mut focus = None;
    if input.kind == InputKind::CloneUrl {
        lines.push(Line::default());
        match &m.gh {
            Gh::Idle => {}
            Gh::Loading => lines.push(Line::from(vec![
                Span::styled(
                    g().spin(m.tick as usize).to_string(),
                    styles::spinner(m.tick as usize),
                ),
                Span::styled(" loading your GitHub repositories", styles::muted()),
            ])),
            Gh::Unavailable(note) => {
                for part in wrap(&format!("GitHub list unavailable: {note}"), w) {
                    lines.push(Line::from(Span::styled(part, styles::faint())));
                }
            }
            Gh::Ready(repos) => {
                let matches = m.repo_matches(&input.value);
                lines.push(Line::from(Span::styled(
                    format!(
                        "Your GitHub repositories ({} of {})",
                        matches.len(),
                        repos.len()
                    ),
                    styles::faint(),
                )));
                let name_w = matches
                    .iter()
                    .map(|i| width(&repos[*i].name))
                    .max()
                    .unwrap_or(10)
                    .min(w / 2);
                for i in matches {
                    let r = &repos[i];
                    let focused = input.sel == Some(i);
                    if focused {
                        focus = Some(lines.len());
                    }
                    let lock = if r.private { " private" } else { "" };
                    lines.push(row_line(
                        focused,
                        vec![
                            Span::styled(
                                pad(&trunc(&r.name, name_w), name_w),
                                styles::fg().add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(lock.to_string(), styles::warn()),
                            Span::styled(
                                format!(
                                    "  {}",
                                    trunc(&r.description, w.saturating_sub(name_w + 12))
                                ),
                                styles::faint(),
                            ),
                        ],
                    ));
                }
            }
        }
    }
    Body { lines, focus }
}

fn choice_body(choice: &Choice, w: usize) -> Body {
    let mut lines = Vec::new();
    for part in &choice.body {
        if part.is_empty() {
            lines.push(Line::default());
        }
        for wrapped in wrap(part, w) {
            lines.push(Line::from(Span::styled(wrapped, styles::muted())));
        }
    }
    lines.push(Line::default());
    let danger = if choice.danger {
        styles::err()
    } else {
        styles::brand()
    };
    let mut buttons = Vec::new();
    for (i, option) in choice.options.iter().enumerate() {
        let active = i == choice.selected;
        let style = if active {
            danger.add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            styles::muted()
        };
        buttons.push(Span::styled(format!(" {option} "), style));
        buttons.push(Span::raw("  "));
    }
    // Three long buttons overflow 70 columns; stack them instead.
    let total: usize = buttons.iter().map(|s| width(&s.content)).sum();
    if total > w {
        for (i, option) in choice.options.iter().enumerate() {
            let active = i == choice.selected;
            let style = if active {
                danger.add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                styles::muted()
            };
            lines.push(Line::from(Span::styled(format!(" {option} "), style)));
        }
    } else {
        lines.push(Line::from(buttons));
    }
    Body { lines, focus: None }
}

fn message_body(lines: &[String], tone: Option<Tone>, w: usize) -> Body {
    let style = tone.map_or_else(styles::muted, |_| styles::fg());
    let mut out = Vec::new();
    for line in lines {
        if line.is_empty() {
            out.push(Line::default());
        }
        for part in wrap(line, w) {
            out.push(Line::from(Span::styled(part, style)));
        }
    }
    Body {
        lines: out,
        focus: None,
    }
}

fn footer(m: &WorkspaceMenu) -> String {
    match &m.screen {
        Screen::List if m.searching => " type to filter  Enter done  Esc clear ".into(),
        Screen::List => match m.kind {
            Kind::Worktrees => {
                " Enter switch  n new  d remove  p pull  / search  Esc close ".into()
            }
            Kind::Projects => " Enter select  c clone  / search  Esc close ".into(),
        },
        Screen::Input(i) if i.kind == InputKind::CloneUrl => {
            " Enter clone  \u{2193} pick from list  Esc back ".into()
        }
        Screen::Input(_) => " Enter confirm  Esc back ".into(),
        Screen::Choice(_) => " \u{2190}\u{2192} choose  Enter confirm  y yes  Esc cancel ".into(),
        Screen::Busy(_) => " working...  Esc hides this ".into(),
        Screen::Message { .. } => " Enter back ".into(),
    }
}

fn title(m: &WorkspaceMenu) -> (&str, &'static str) {
    match (&m.screen, m.kind) {
        (Screen::Choice(c), Kind::Worktrees) => (&c.title, " /worktree "),
        (Screen::Choice(c), Kind::Projects) => (&c.title, " /project "),
        (Screen::Message { title, .. }, Kind::Worktrees) => (title, " /worktree "),
        (Screen::Message { title, .. }, Kind::Projects) => (title, " /project "),
        (Screen::Input(i), _) => match i.kind {
            InputKind::WorktreeName => ("New worktree", " /worktree "),
            InputKind::CloneUrl => ("Clone a repository", " /project "),
            InputKind::RenameProject => ("Rename project", " /project "),
        },
        (_, Kind::Worktrees) => ("Worktrees", " /worktree "),
        (_, Kind::Projects) => ("Project", " /project "),
    }
}

pub(crate) fn render_workspace_menu(f: &mut Frame, app: &App, region: Rect) {
    let m = &app.workspace_menu;
    if !m.open {
        return;
    }
    surface::dim_backdrop(f, region);
    let card_w = region.width.saturating_sub(2).clamp(1, 100);
    let w = (card_w as usize).saturating_sub(2 + 2 * crate::overlays::PAD_X as usize);
    let body = match &m.screen {
        Screen::List => list_body(m, w),
        Screen::Input(input) => input_body(m, input, w),
        Screen::Choice(choice) => choice_body(choice, w),
        Screen::Busy(label) => Body {
            lines: vec![Line::from(vec![
                Span::styled(
                    g().spin(m.tick as usize).to_string(),
                    styles::spinner(m.tick as usize),
                ),
                Span::styled(format!(" {label}"), styles::fg()),
            ])],
            focus: None,
        },
        Screen::Message { lines, tone, .. } => message_body(lines, *tone, w),
    };
    let notice_rows = u16::from(m.notice.is_some());
    let want = card_rows(body.lines.len() as u16 + notice_rows);
    let card_h = want.max(7).min(region.height);
    let card = Rect {
        x: region.x + region.width.saturating_sub(card_w) / 2,
        y: region.y + region.height.saturating_sub(card_h) / 2,
        width: card_w,
        height: card_h,
    };
    let (name, context) = title(m);
    let tone = match &m.screen {
        Screen::Choice(c) if c.danger => SurfaceTone::Warning,
        _ => SurfaceTone::Brand,
    };
    let inner = crate::overlays::padded_panel(
        f,
        card,
        surface::title(name, tone),
        Some(Span::styled(context, styles::faint())),
        Some(Span::styled(footer(m), styles::faint())),
        tone,
    );
    if inner.width < 20 || inner.height < 2 {
        return;
    }
    let body_h = inner.height.saturating_sub(notice_rows) as usize;
    let total = body.lines.len();
    let mut offset = 0;
    if let Some(focus) = body.focus {
        // The search line stays put; the list scrolls under it.
        if focus + 2 > body_h {
            offset = (focus + 2).saturating_sub(body_h);
        }
    }
    offset = offset.min(total.saturating_sub(body_h));
    let area = Rect {
        height: body_h as u16,
        ..inner
    };
    let visible: Vec<Line> = body
        .lines
        .iter()
        .skip(offset)
        .take(body_h)
        .cloned()
        .collect();
    f.render_widget(Paragraph::new(visible), area);
    let edge = area.x + area.width;
    for (shown, y, glyph) in [
        (offset > 0, area.y, g().up),
        (
            offset + body_h < total,
            area.y + area.height.saturating_sub(1),
            g().down,
        ),
    ] {
        if shown && edge < f.area().width {
            f.buffer_mut()[(edge, y)]
                .set_symbol(glyph)
                .set_style(styles::muted());
        }
    }
    if let Some((tone, text)) = &m.notice {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                trunc(
                    &format!("{} {text}", tone_glyph(*tone)),
                    inner.width as usize,
                ),
                tone_style(*tone),
            ))),
            Rect {
                y: inner.y + inner.height - 1,
                height: 1,
                ..inner
            },
        );
    }
}
