//! Refuses a fixed `sleep N` of half a minute or more at the top level of a command. A blind
//! wait finishes late when the job was ready early and early when it was not; the model should
//! poll for the condition instead (poll mode or a background job).

const REFUSE_FROM_SECS: f64 = 30.0;

struct Segment {
    start: usize,
    end: usize,
    sep_end: usize,
    backgrounded: bool,
}

/// The refusal text when `command` holds a standalone `sleep` of at least 30s outside any loop,
/// group, subshell or background job; `None` when the command may run.
pub(super) fn refuse_fixed_wait(command: &str) -> Option<String> {
    let segments = top_level_segments(command);
    for seg in &segments {
        if seg.backgrounded {
            continue;
        }
        let text = command[seg.start..seg.end].trim();
        let Some(secs) = sleep_secs(text) else {
            continue;
        };
        if secs < REFUSE_FROM_SECS {
            continue;
        }
        return Some(message(command, seg, text));
    }
    None
}

fn message(command: &str, seg: &Segment, sleep_text: &str) -> String {
    let rest = if seg.sep_end > seg.end {
        format!("{}{}", &command[..seg.start], &command[seg.sep_end..])
    } else {
        command[..seg.start].to_string()
    };
    let rest = rest
        .trim()
        .trim_end_matches([';', '&', '|'])
        .trim()
        .to_string();
    let check = if rest.is_empty() || rest.len() > 200 {
        "ssh host 'test -f /path/done'".to_string()
    } else {
        rest
    };
    format!(
        "refused: a fixed '{sleep_text}' waits blindly (it returns late if the job finished \
         early, and early if it did not). Poll for the condition instead: call shell with \
         \"poll_until_exit_zero\": true, \"poll_interval_secs\": 15 and a command that succeeds \
         only when the result is ready, e.g. {check:?} rewritten to exit 0 when done. It returns \
         as soon as the condition holds, or says to call again. For a long job, start it with \
         \"background\": true and poll its log. Nothing was run."
    )
}

fn sleep_secs(segment: &str) -> Option<f64> {
    let words = words(segment);
    let mut iter = words.iter();
    if iter.next()?.as_str() != "sleep" {
        return None;
    }
    let mut total = 0.0;
    let mut any = false;
    for arg in iter {
        total += duration(arg)?;
        any = true;
    }
    any.then_some(total)
}

fn duration(word: &str) -> Option<f64> {
    let (number, unit) = match word.char_indices().last()? {
        (i, c) if c.is_ascii_alphabetic() => (&word[..i], c),
        _ => (word, 's'),
    };
    let value: f64 = number
        .parse()
        .ok()
        .filter(|v: &f64| v.is_finite() && *v >= 0.0)?;
    let scale = match unit {
        's' => 1.0,
        'm' => 60.0,
        'h' => 3600.0,
        'd' => 86400.0,
        _ => return None,
    };
    Some(value * scale)
}

fn words(segment: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = segment.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    started = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, c) => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// Splits on `;`, `&&`, `||`, `|`, `&` and newlines that sit outside quotes, `$(...)`,
/// backticks, `(...)` and `{...}` groups and loop/`if`/`case` bodies.
fn top_level_segments(command: &str) -> Vec<Segment> {
    let bytes = command.as_bytes();
    let mut segments = Vec::new();
    let mut quote: Option<u8> = None;
    let mut depth = 0usize;
    let mut compound = 0usize;
    let mut seg_start = 0usize;
    let mut i = 0usize;
    let mut finish = |start: usize, end: usize, sep_end: usize, bg: bool, compound: &mut usize| {
        let text = &command[start..end];
        let first = text.split_whitespace().next().unwrap_or("");
        match first {
            "while" | "until" | "for" | "select" | "if" | "case" => *compound += 1,
            "done" | "fi" | "esac" => *compound = compound.saturating_sub(1),
            _ => {}
        }
        if *compound == 0 && !matches!(first, "done" | "fi" | "esac" | "") {
            segments.push(Segment {
                start,
                end,
                sep_end,
                backgrounded: bg,
            });
        }
    };
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                i += 1;
            } else if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'\\' => i += 1,
            b'\'' | b'"' | b'`' => quote = Some(b),
            b'(' | b'{' if b == b'(' || is_group_open(bytes, i) => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'}' if depth > 0 => depth -= 1,
            b';' | b'\n' | b'&' | b'|' if depth == 0 => {
                let mut sep_end = i + 1;
                let mut bg = false;
                if (b == b'&' || b == b'|') && bytes.get(i + 1) == Some(&b) {
                    sep_end += 1;
                } else if b == b'&' {
                    bg = true;
                }
                if b == b'&' && i > 0 && matches!(bytes[i - 1], b'>' | b'<') {
                    i += 1;
                    continue;
                }
                finish(seg_start, i, sep_end, bg, &mut compound);
                seg_start = sep_end;
                i = sep_end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    finish(seg_start, bytes.len(), bytes.len(), false, &mut compound);
    segments
}

fn is_group_open(bytes: &[u8], i: usize) -> bool {
    let after = bytes.get(i + 1).is_none_or(|n| n.is_ascii_whitespace());
    let before = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'$';
    after && before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flagged(command: &str) -> bool {
        refuse_fixed_wait(command).is_some()
    }

    #[test]
    fn flags_fixed_waits() {
        assert!(flagged("sleep 540; ssh -o BatchMode=yes root@h 'check'"));
        assert!(flagged("sleep 560 && ssh h x"));
        assert!(flagged("sleep 10m && make"));
        assert!(flagged("make; sleep 120"));
        assert!(flagged("sleep 30"));
        assert!(flagged("sleep 1m 30s"));
        assert!(flagged("echo start\nsleep 600\necho done"));
        assert!(flagged("sleep 0.5h"));
    }

    #[test]
    fn leaves_short_loops_and_background() {
        assert!(!flagged("sleep 5; x"));
        assert!(!flagged("sleep 29 && x"));
        assert!(!flagged("until ssh h test -f d; do sleep 15; done"));
        assert!(!flagged("until ssh h test -f d; do sleep 600; done"));
        assert!(!flagged("while true; do x; sleep 60; done"));
        assert!(!flagged("for i in 1 2 3; do x; sleep 60; done"));
        assert!(!flagged("if x; then sleep 60; fi"));
        assert!(!flagged("echo 'sleep 600'"));
        assert!(!flagged("echo \"a; sleep 600\""));
        assert!(!flagged("(sleep 40; touch /tmp/d) &"));
        assert!(!flagged("sleep 600 &"));
        assert!(!flagged("sleep 600 & echo started"));
        assert!(!flagged("{ sleep 600; touch d; } &"));
        assert!(!flagged("x=$(sleep 60; echo hi)"));
        assert!(!flagged("sleep $N"));
        assert!(!flagged("sleep infinity"));
        assert!(!flagged("ssh h 'sleep 600'"));
        assert!(!flagged("while true; do\n  sleep 60\ndone"));
    }

    #[test]
    fn code_after_a_loop_is_still_checked() {
        assert!(flagged("while x; do y; done; sleep 120"));
    }

    #[test]
    fn refusal_keeps_the_check_and_names_poll_mode() {
        let text = refuse_fixed_wait("sleep 540; ssh -o BatchMode=yes root@h 'ls /done'").unwrap();
        assert!(text.starts_with("refused: a fixed 'sleep 540' waits blindly"));
        assert!(text.contains("ssh -o BatchMode=yes root@h 'ls /done'"));
        assert!(text.contains("\"poll_until_exit_zero\": true"));
        assert!(text.contains("\"background\": true"));
        assert!(text.contains("Nothing was run."));
        let trailing = refuse_fixed_wait("make check; sleep 120").unwrap();
        assert!(trailing.contains("\"make check\""));
        let bare = refuse_fixed_wait("sleep 90").unwrap();
        assert!(bare.contains("test -f /path/done"));
    }
}
