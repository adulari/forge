//! The header row (fullscreen only): where am I. Brand, repository and branch, remote-control
//! dot, and the version when there is room. Inline shows no header; `/status` lists the same
//! facts.

use unicode_width::UnicodeWidthStr;

use super::*;
use crate::git_location::GitHead;
use crate::theme::glyphs::g;
use crate::theme::styles;

/// Width from which the version joins the header.
pub(crate) const VERSION_MIN_WIDTH: u16 = 120;

/// 1 in fullscreen, 0 inline.
pub(crate) fn header_height(app: &App) -> u16 {
    u16::from(app.fullscreen)
}

fn version_label() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// The branch chip: `repo ⑂ branch`, with a worktree note and a detached-HEAD warning.
fn location_spans(app: &App, budget: usize) -> Vec<Span<'static>> {
    let glyphs = g();
    let Some(location) = &app.git_location else {
        return match &app.repo_name {
            Some(repo) => vec![Span::styled(
                format!("   {}", middle_truncate(repo, budget.saturating_sub(3))),
                styles::fg(),
            )],
            None => Vec::new(),
        };
    };
    let (branch, branch_style) = match &location.head {
        GitHead::Branch(name) => (name.clone(), styles::muted()),
        GitHead::Detached(_) => (location.branch_label(), styles::warn()),
    };
    let worktree = if location.worktree { " worktree" } else { "" };
    let repo = middle_truncate(&location.repo, 24);
    for with_repo in [true, false] {
        let repo_cells = if with_repo { repo.width() + 1 } else { 0 };
        let fixed = 3 + repo_cells + glyphs.branch.width() + 1 + worktree.width();
        let room = budget.saturating_sub(fixed);
        if room < 6 {
            continue;
        }
        let mut spans = vec![Span::raw("   ")];
        if with_repo {
            spans.push(Span::styled(format!("{repo} "), styles::fg()));
        }
        spans.push(Span::styled(format!("{} ", glyphs.branch), styles::faint()));
        spans.push(Span::styled(
            middle_truncate(&branch, room.min(40)),
            branch_style,
        ));
        if location.worktree {
            spans.push(Span::styled(worktree, styles::faint()));
        }
        return spans;
    }
    Vec::new()
}

/// `feat/tool-output-viewer` in 14 cells is `feat/to…viewer`: a branch is recognised by both ends.
pub(crate) fn middle_truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return g().ellipsis.to_string();
    }
    let keep = max - 1;
    let tail = keep / 2;
    let head = keep - tail;
    let head: String = chars[..head].iter().collect();
    let tail: String = chars[chars.len() - tail..].iter().collect();
    format!("{head}{}{tail}", g().ellipsis)
}

/// The header's spans for `width` cells: left cluster, then the right cluster, padded apart.
pub(crate) fn header_spans(app: &App, width: u16) -> Vec<Span<'static>> {
    let glyphs = g();
    let width = width as usize;
    let mut right: Vec<Span<'static>> = Vec::new();
    if app.remote_active {
        right.push(Span::styled(
            format!("{} remote", glyphs.live_dot),
            styles::ok(),
        ));
    }
    if width >= VERSION_MIN_WIDTH as usize {
        if !right.is_empty() {
            right.push(Span::raw("   "));
        }
        right.push(Span::styled(version_label(), styles::faint()));
    }
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let mut left = vec![
        Span::styled(format!(" {} ", glyphs.brand), styles::brand().bold()),
        Span::styled("forge", styles::brand().bold()),
    ];
    let left_brand_w: usize = left.iter().map(|s| s.content.width()).sum();
    let budget = width.saturating_sub(left_brand_w + right_w + if right_w > 0 { 3 } else { 1 });
    left.extend(location_spans(app, budget));
    let left_w: usize = left.iter().map(|s| s.content.width()).sum();
    let mut out = left;
    if right_w > 0 {
        let pad = width.saturating_sub(left_w + right_w + 1);
        out.push(Span::raw(" ".repeat(pad)));
        out.extend(right);
    }
    out
}

pub(crate) fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(TextLine::from(header_spans(app, area.width))),
        Rect { height: 1, ..area },
    );
}
