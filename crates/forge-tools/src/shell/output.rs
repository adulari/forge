//! Model-facing rendering for shell command output.

pub(super) fn render_streams(out: &[u8], err: &[u8]) -> String {
    let mut parts = Vec::new();
    if let Some(text) = stream_text(out).filter(|text| !text.trim().is_empty()) {
        parts.push(text);
    }
    if let Some(text) = stream_text(err).filter(|text| !text.trim().is_empty()) {
        parts.push(format!("[stderr]\n{text}"));
    }
    parts.join("\n")
}

pub(super) fn stream_text(bytes: &[u8]) -> Option<String> {
    (!bytes.is_empty()).then(|| {
        if bytes.contains(&0) {
            format!("<binary output: {} bytes, not shown>", bytes.len())
        } else {
            strip_ansi(&String::from_utf8_lossy(bytes))
        }
    })
}

pub(super) fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&next) {
                    break;
                }
            }
        } else if chars.peek() == Some(&']') {
            chars.next();
            while let Some(next) = chars.next() {
                if next == '\x07' {
                    break;
                }
                if next == '\x1b' {
                    chars.next();
                    break;
                }
            }
        } else if let Some(next) = chars.next() {
            if ('\x20'..='\x2f').contains(&next) {
                chars.next();
            }
        }
    }
    out
}

pub(super) fn truncate_for_model(text: &str, budget: usize) -> (String, bool) {
    if text.len() <= budget {
        return (text.to_string(), false);
    }
    if budget == 0 {
        return (String::new(), true);
    }
    let mut marker_len = "\n… 0 bytes truncated …\n".len().min(budget);
    for _ in 0..3 {
        let content_budget = budget.saturating_sub(marker_len);
        let head_len = floor_boundary(text, content_budget / 2);
        let tail_len = floor_boundary_back(text, content_budget - head_len);
        let dropped = text.len() - head_len - tail_len;
        marker_len = format!("\n… {dropped} bytes truncated …\n")
            .len()
            .min(budget);
    }
    let content_budget = budget.saturating_sub(marker_len);
    let head_len = floor_boundary(text, content_budget / 2);
    let tail_len = floor_boundary_back(text, content_budget - head_len);
    let dropped = text.len() - head_len - tail_len;
    let marker = format!("\n… {dropped} bytes truncated …\n");
    (
        format!(
            "{}{}{}",
            &text[..head_len],
            marker,
            &text[text.len() - tail_len..]
        ),
        true,
    )
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}
fn floor_boundary_back(text: &str, len: usize) -> usize {
    let mut start = text.len().saturating_sub(len);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text.len() - start
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rendering_removes_terminal_control_data_and_preserves_stderr() {
        assert_eq!(strip_ansi("\x1b]0;title\x07\x1b[31mred\x1b[0m"), "red");
        assert_eq!(render_streams(b"out", b"err"), "out\n[stderr]\nerr");
        assert!(stream_text(&[0, 1]).unwrap().contains("binary output"));
    }
    #[test]
    fn truncation_is_utf8_safe_and_retains_both_ends() {
        let text = format!("start{}end", "é".repeat(10_000));
        let (truncated, did_truncate) = truncate_for_model(&text, 100);
        assert!(did_truncate);
        assert!(truncated.len() <= 100);
        assert!(truncated.starts_with("start"));
        assert!(truncated.ends_with("end"));
    }
}
