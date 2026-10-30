//! Error text for failed `edit_file` / `multi_edit` matches: the closest region with a similarity
//! score and the first differing line as a `-`/`+` pair, or a list of the places an ambiguous
//! `old` matched. Pure functions over the file text; the model gets enough to fix the call
//! without another read.

const MIN_SIMILARITY: u32 = 50;
const MAX_SCAN_WINDOWS: usize = 40_000;
const MAX_OCCURRENCES_SHOWN: usize = 5;
const PREVIEW_CHARS: usize = 100;

fn bigrams(s: &str) -> Vec<(char, char)> {
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    chars.windows(2).map(|w| (w[0], w[1])).collect()
}

/// Dice coefficient over character bigrams, ignoring whitespace, 0..=100.
fn line_similarity(a: &str, b: &str) -> u32 {
    let (a, b) = (a.trim(), b.trim());
    if a == b {
        return 100;
    }
    let (ga, mut gb) = (bigrams(a), bigrams(b));
    if ga.is_empty() || gb.is_empty() {
        return 0;
    }
    let total = ga.len() + gb.len();
    let mut shared = 0;
    for g in &ga {
        if let Some(i) = gb.iter().position(|x| x == g) {
            gb.swap_remove(i);
            shared += 1;
        }
    }
    (200 * shared / total) as u32
}

pub(super) struct ClosestMatch {
    pub line: usize,
    pub diff_line: usize,
    pub similarity: u32,
    pub old_line: String,
    pub actual_line: String,
}

pub(super) fn closest_match(content: &str, old: &str) -> Option<ClosestMatch> {
    let old_lines: Vec<&str> = old.lines().collect();
    let lines: Vec<&str> = content.lines().collect();
    if old_lines.is_empty() || lines.is_empty() {
        return None;
    }
    let width = old_lines.len().min(lines.len());
    let windows = lines.len() - width + 1;
    if windows * width > MAX_SCAN_WINDOWS {
        return None;
    }
    let mut best: Option<(u32, usize)> = None;
    for start in 0..windows {
        let sum: u32 = (0..width)
            .map(|i| line_similarity(old_lines[i], lines[start + i]))
            .sum();
        let score = sum / width as u32;
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, start));
        }
    }
    let (similarity, start) = best?;
    if similarity < MIN_SIMILARITY {
        return None;
    }
    let offset = (0..width)
        .find(|&i| old_lines[i].trim_end() != lines[start + i].trim_end())
        .unwrap_or(0);
    Some(ClosestMatch {
        line: start + 1,
        diff_line: start + offset + 1,
        similarity,
        old_line: old_lines[offset].to_string(),
        actual_line: lines[start + offset].to_string(),
    })
}

pub(super) fn closest_match_message(m: &ClosestMatch) -> String {
    let line = m.line;
    let sim = m.similarity;
    let at = m.diff_line;
    format!(
        "closest match ({sim}% similar) starts at line {line}; first difference at line {at}:\n\
         - {}\n+ {}\nCopy the real text (`+` line) into `old`.",
        m.old_line, m.actual_line
    )
}

fn preview(line: &str) -> String {
    let t = line.trim();
    if t.chars().count() > PREVIEW_CHARS {
        let cut: String = t.chars().take(PREVIEW_CHARS).collect();
        format!("{cut}...")
    } else {
        t.to_string()
    }
}

/// Describe where an `old` that matched `count` times sits: up to five `line N: preview` rows.
pub(super) fn ambiguous_message(content: &str, old: &str, count: usize) -> String {
    let mut rows = Vec::new();
    for (at, _) in content.match_indices(old).take(MAX_OCCURRENCES_SHOWN) {
        let line = content[..at].matches('\n').count() + 1;
        let text = content[at..].lines().next().unwrap_or("");
        rows.push(format!("  line {line}: {}", preview(text)));
    }
    let shown = rows.len();
    let header = if count > shown {
        format!("`old` is ambiguous: {count} occurrences (showing first {shown}):")
    } else {
        format!("`old` is ambiguous: {count} occurrences:")
    };
    format!(
        "{header}\n{}\nAdd surrounding lines to make it unique, or pass `replace_all: true` if \
         every occurrence should change.",
        rows.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "fn a() {\n    let x = compute(1, 2);\n    println!(\"{x}\");\n}\n\nfn b() {\n    let x = 3;\n}\n";

    #[test]
    fn closest_match_reports_similarity_line_and_first_difference() {
        let old = "fn a() {\n    let x = compute(1, 3);\n    println!(\"{x}\");\n}";
        let m = closest_match(FILE, old).unwrap();
        assert_eq!(m.line, 1);
        assert!(m.similarity >= 80, "{}", m.similarity);
        assert_eq!(m.old_line, "    let x = compute(1, 3);");
        assert_eq!(m.actual_line, "    let x = compute(1, 2);");
        let msg = closest_match_message(&m);
        assert!(msg.contains("first difference at line 2"), "{msg}");
        assert!(msg.contains("- ") && msg.contains("+ "), "{msg}");
    }

    #[test]
    fn unrelated_text_gets_no_candidate() {
        assert!(closest_match(FILE, "zzzz qqqq wwww\nkkkk jjjj").is_none());
    }

    #[test]
    fn ambiguous_lists_up_to_five_occurrences_with_line_numbers() {
        let content = "x = 1\n".repeat(8);
        let msg = ambiguous_message(&content, "x = 1", 8);
        assert!(msg.contains("8 occurrences (showing first 5)"), "{msg}");
        assert!(msg.contains("line 1: x = 1") && msg.contains("line 5: x = 1"));
        assert!(!msg.contains("line 6"), "{msg}");
        assert!(msg.contains("Add surrounding lines to make it unique"));
    }
}
