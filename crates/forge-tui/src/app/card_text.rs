//! Plain-text helpers for tool rows: paths shown relative or under `~`, cell widths, wrapping
//! and truncation that count terminal cells, and number labels.

use crate::theme::glyphs::g;

/// How a path is shown: relative to the call's own `cwd` when it sits under it, else with the
/// home directory folded to `~`. A tool call's path is almost always inside the workspace, and
/// repeating that prefix on every row costs the width the interesting tail needs.
pub(crate) fn display_path(path: &str, cwd: Option<&str>) -> String {
    // A bridged claude call carries absolute paths and no `cwd` of its own; the TUI's directory is
    // the session's, so it stands in.
    let own = std::env::current_dir().ok();
    let own = own.as_deref().and_then(std::path::Path::to_str);
    for cwd in [cwd, own].into_iter().flatten().filter(|c| !c.is_empty()) {
        if let Ok(rest) = std::path::Path::new(path).strip_prefix(cwd) {
            let rest = rest.to_string_lossy();
            if !rest.is_empty() {
                return rest.into_owned();
            }
        }
    }
    shorten(path)
}

/// Replace the home directory with `~` so a path row reads as a path, not as a margin-eating
/// absolute prefix repeated on every call.
pub(crate) fn shorten(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

/// Every home-directory path in `text` (a shell command, a log line) with the home folded to
/// `~`: `cd /home/u/wt && ls /home/u` reads `cd ~/wt && ls ~`. A longer name that merely starts
/// with the home (`/home/user2`) is left alone.
pub(crate) fn shorten_in(text: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if home.len() > 1 => fold_home(text, home.trim_end_matches('/')),
        _ => text.to_string(),
    }
}

fn fold_home(text: &str, home: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(home) {
        let after = &rest[at + home.len()..];
        let before = rest[..at].chars().next_back();
        let starts =
            before.is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/')));
        let ends = after
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.')));
        out.push_str(&rest[..at]);
        out.push_str(if starts && ends { "~" } else { home });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Collapse a multi-line value onto one line for the header row.
pub(crate) fn oneline(s: &str) -> String {
    let mut out = String::new();
    for (i, part) in s
        .split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .enumerate()
    {
        if i > 0 {
            out.push_str(" ⏎ ");
        }
        out.push_str(part);
    }
    out
}

/// Hard-wrap plain text to `width` cells. Never returns an empty vector, so a blank line in tool
/// output still occupies a row (blank lines are structure in most command output).
pub(crate) fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cells = 0usize;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(1);
        if cells + w > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cells = 0;
        }
        cur.push(ch);
        cells += w;
    }
    out.push(cur);
    out
}

/// The display width of a string in terminal cells.
pub(crate) fn cells(s: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    s.chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(1))
        .sum()
}

/// Truncate to `cells` display columns with an ellipsis, counting terminal cells (a CJK glyph is
/// two) so the header's right-hand status column stays put on any content.
pub(crate) fn truncate_cells(s: &str, cells: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if cells == 0 {
        return String::new();
    }
    let total: usize = s
        .chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(1))
        .sum();
    if total <= cells {
        return s.to_string();
    }
    let ellipsis = g().ellipsis;
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(1);
        if used + w > cells.saturating_sub(ellipsis.chars().count()) {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push_str(ellipsis);
    out
}

/// `1234567` → `1,234,567`.
pub(crate) fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `5,012 lines · 212 KB`.
pub(crate) fn size_label(lines: usize, bytes: usize) -> String {
    let size = match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b.div_ceil(1 << 10)),
        b => format!("{b} B"),
    };
    let unit = if lines == 1 { "line" } else { "lines" };
    format!("{} {unit} · {size}", group(lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_paths_fold_to_a_tilde_wherever_they_sit_in_a_command() {
        assert_eq!(
            fold_home(
                "cd /home/u/wt && tail -6 /home/u/.log; ls /home/u",
                "/home/u"
            ),
            "cd ~/wt && tail -6 ~/.log; ls ~"
        );
        assert_eq!(
            fold_home("ls /home/u2/x /x/home/u", "/home/u"),
            "ls /home/u2/x /x/home/u"
        );
        assert_eq!(fold_home("echo \"/home/u/a\"", "/home/u"), "echo \"~/a\"");
    }
}
