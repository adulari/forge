//! What a shell command line *actually runs*.
//!
//! The permission broker matches rules against commands, so before it can decide anything it has
//! to unwrap the ways a second command hides inside one `command` string. This module owns that
//! decomposition — statement separators, heredocs, command substitution, `bash -c` wrappers and
//! no-op wrapper binaries — and nothing else: it takes a string and returns the segments plus
//! whether the scan fully succeeded. The decision itself stays with the broker.

/// Extract the effective command(s) from a shell command line so that arg-hidden danger
/// (`bash -c '...'`, wrapper binaries, `;`/`&&`/`|`/newline chains, `$(…)` substitution) is
/// unwrapped before matching. Returns the segments and whether parsing fully succeeded.
///
/// SECURITY NOTE — READ BEFORE TRUSTING THIS. This is an approximation, not a shell parser, and
/// it CANNOT be made exhaustive: `sh` resolves variables, aliases, functions, globs and encodings
/// that no static matcher sees. It closes the *obvious* ways a second command is smuggled into a
/// single `command` string — statement separators (`;`, `&&`, `||`, `|`, `&`, **newline**),
/// command substitution (`$(…)`, backticks), `bash -c` / `cmd /C` wrappers, and no-op wrapper
/// binaries (`env`, `nice`, …). Evasion classes that still get through, so nobody over-trusts the
/// built-in denylist:
///   * **indirection** — `X=rm; $X -rf ~`, `eval "$var"`, a shell function or alias, `$'\x72m'`;
///   * **encoding / staging** — `echo <base64> | base64 -d > f && sh f`, writing a script with
///     `write_file` and then running it;
///   * **wrappers we do not unwrap** — `xargs`, `find -exec`, `ssh host '<script>'`, `make`,
///     `python -c`, `perl -e`, `npm run <script>`;
///   * **quoting / nesting corners** — an unbalanced quote, a `)` inside `$( … )`, or a
///     backslash-escaped quote can make the scan give up part-way. It then sets `ok = false`,
///     which widens the literal-substring fallback in [`rule_matches`], but the per-segment globs
///     may still miss;
///   * **heredoc data fed to a non-shell** — a heredoc body is treated as data (so writing a file
///     with `cat <<EOF` is not denied for its contents); only a body piped into a shell
///     interpreter is scanned as a script.
///
/// The denylist is a floor against accidents and lazy attacks, NOT a sandbox. Real containment is
/// `shell.sandbox` or running Forge inside a container.
pub(super) fn effective_commands(cmd: &str) -> (Vec<String>, bool) {
    let mut out = Vec::new();
    let mut ok = true;
    collect_commands(cmd, 0, &mut out, &mut ok);
    if out.is_empty() {
        out.push(cmd.trim().to_string());
    }
    (out, ok)
}

fn collect_commands(cmd: &str, depth: usize, out: &mut Vec<String>, ok: &mut bool) {
    if depth > 4 {
        *ok = false;
        out.push(cmd.trim().to_string());
        return;
    }
    // Heredoc bodies are DATA, not statements — peel them off before any splitting so the lines of
    // `cat > f <<EOF … EOF` are not mistaken for a command chain (that would deny/prompt on honest
    // file writes). A body read by a shell interpreter (`bash <<EOF`) IS executed, so that one is
    // recursed into as a script.
    let (skeleton, heredocs) = strip_heredocs(cmd, ok);
    // Command substitution runs a command line of its own — including inside double quotes.
    for inner in command_substitutions(&skeleton) {
        collect_commands(&inner, depth + 1, out, ok);
    }
    // Split on shell operators into segments, then normalize each.
    for raw in split_operators(&skeleton) {
        let seg = raw.trim();
        if seg.is_empty() {
            continue;
        }
        let Some(tokens) = shell_words::split(seg).ok().filter(|t| !t.is_empty()) else {
            *ok = false;
            out.push(seg.to_string());
            continue;
        };
        let stripped = strip_wrappers(&tokens);
        // `bash -c "<script>"` / `sh -lc "<script>"`: recurse into the inner script.
        if let Some(inner) = inner_script(&stripped) {
            collect_commands(&inner, depth + 1, out, ok);
            continue;
        }
        out.push(stripped.join(" "));
    }
    for hd in heredocs {
        if hd.fed_to_shell {
            collect_commands(&hd.body, depth + 1, out, ok);
        }
    }
}

/// Split a command line into statements on `;`, `&&`, `||`, `|`, `&` and **newlines** (`\n`/`\r`),
/// outside of quotes and command substitutions.
///
/// Newlines are real statement separators to `sh -c`, and omitting them was a denylist bypass:
/// `"git status\nrm -rf ~"` collapsed into one flattened segment that matched neither the anchored
/// `rm -rf ~` deny nor its substring fallback. Three things keep honest multi-line commands intact:
/// a backslash-newline is a line continuation (not a separator), a newline inside quotes stays in
/// its segment, and heredoc bodies are removed by [`strip_heredocs`] before we get here.
fn split_operators(cmd: &str) -> Vec<String> {
    let mut segs = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            // Inside '…' a backslash is literal; inside "…" it escapes the next character (and a
            // backslash-newline is a line continuation).
            Some('\'') => {
                cur.push(c);
                if c == '\'' {
                    quote = None;
                }
                i += 1;
            }
            Some(q) => {
                if c == '\\' && i + 1 < chars.len() {
                    if let Some(next) = skip_line_continuation(&chars, i) {
                        i = next;
                        continue;
                    }
                    cur.push(c);
                    cur.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                // `$( … )` / backticks are opaque here: an operator inside a substitution must not
                // shred the surrounding segment (the substitution itself is collected separately).
                if let Some(end) = substitution_end(&chars, i) {
                    cur.extend(&chars[i..end]);
                    i = end;
                    continue;
                }
                cur.push(c);
                if c == q {
                    quote = None;
                }
                i += 1;
            }
            None => {
                if c == '\\' && i + 1 < chars.len() {
                    if let Some(next) = skip_line_continuation(&chars, i) {
                        i = next;
                        continue;
                    }
                    // Keep the escape so `\;` stays an argument rather than a separator.
                    cur.push(c);
                    cur.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if let Some(end) = substitution_end(&chars, i) {
                    cur.extend(&chars[i..end]);
                    i = end;
                    continue;
                }
                match c {
                    '\'' | '"' => {
                        quote = Some(c);
                        cur.push(c);
                    }
                    // A newline ends a statement exactly like `;` — this is the bypass being closed.
                    ';' | '\n' | '\r' => {
                        segs.push(std::mem::take(&mut cur));
                    }
                    '&' | '|' => {
                        // consume a possible doubled operator (&& / ||) and a single | .
                        segs.push(std::mem::take(&mut cur));
                        if i + 1 < chars.len() && chars[i + 1] == c {
                            i += 1;
                        }
                    }
                    _ => cur.push(c),
                }
                i += 1;
            }
        }
    }
    if !cur.trim().is_empty() {
        segs.push(cur);
    }
    segs
}

/// A backslash at `i` followed by a line ending: POSIX line continuation — both characters vanish
/// and the next physical line continues the SAME statement. Returns the index to resume at.
fn skip_line_continuation(chars: &[char], i: usize) -> Option<usize> {
    match chars.get(i + 1).copied() {
        Some('\n') => Some(i + 2),
        Some('\r') => Some(if chars.get(i + 2) == Some(&'\n') {
            i + 3
        } else {
            i + 2
        }),
        _ => None,
    }
}

/// A heredoc body lifted out of a command line, plus whether the line that introduced it fed the
/// body to a shell interpreter (in which case the body is a script, not data).
struct Heredoc {
    fed_to_shell: bool,
    body: String,
}

struct HeredocDelim {
    word: String,
    strip_tabs: bool,
}

/// Remove heredoc bodies (`<<EOF … EOF`, `<<-'EOF' … EOF`) from `cmd`, returning the remaining
/// command text and the bodies. Sets `*ok = false` on an unterminated heredoc so the caller widens
/// its literal-substring fallback.
fn strip_heredocs(cmd: &str, ok: &mut bool) -> (String, Vec<Heredoc>) {
    if !cmd.contains("<<") {
        return (cmd.to_string(), Vec::new());
    }
    let lines: Vec<&str> = cmd.split_inclusive('\n').collect();
    let mut skeleton = String::new();
    let mut heredocs = Vec::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        let (delims, next_quote) = scan_heredoc_intros(line, quote);
        quote = next_quote;
        skeleton.push_str(line);
        if delims.is_empty() {
            continue;
        }
        let fed_to_shell = invokes_shell(line);
        for d in delims {
            let mut body = String::new();
            let mut terminated = false;
            while i < lines.len() {
                let l = lines[i];
                i += 1;
                let bare = l.trim_end_matches(['\n', '\r']);
                let cmp = if d.strip_tabs {
                    bare.trim_start_matches('\t')
                } else {
                    bare
                };
                if cmp == d.word {
                    terminated = true;
                    break;
                }
                body.push_str(l);
            }
            if !terminated {
                *ok = false;
            }
            heredocs.push(Heredoc { fed_to_shell, body });
        }
    }
    (skeleton, heredocs)
}

/// Heredoc delimiters introduced on one physical line, plus the quote state carried to the next.
fn scan_heredoc_intros(line: &str, mut quote: Option<char>) -> (Vec<HeredocDelim>, Option<char>) {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                }
                i += 1;
            }
            Some(q) => {
                if c == '\\' && i + 1 < chars.len() {
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
                i += 1;
            }
            None => match c {
                '\\' if i + 1 < chars.len() => i += 2,
                '\'' | '"' => {
                    quote = Some(c);
                    i += 1;
                }
                '<' if chars.get(i + 1) == Some(&'<') => {
                    // `<<<` is a here-STRING (one line, no body) — not a heredoc.
                    if chars.get(i + 2) == Some(&'<') {
                        i += 3;
                        continue;
                    }
                    let mut j = i + 2;
                    let strip_tabs = chars.get(j) == Some(&'-');
                    if strip_tabs {
                        j += 1;
                    }
                    while matches!(chars.get(j).copied(), Some(' ') | Some('\t')) {
                        j += 1;
                    }
                    let (word, next) = read_delimiter_word(&chars, j);
                    if !word.is_empty() {
                        out.push(HeredocDelim { word, strip_tabs });
                    }
                    i = next.max(i + 2);
                }
                _ => i += 1,
            },
        }
    }
    (out, quote)
}

/// Read a heredoc delimiter word starting at `j` — it may be quoted in whole or in part
/// (`EOF`, `'EOF'`, `"EOF"`, `E\OF`). Returns the unquoted word and the index just past it.
fn read_delimiter_word(chars: &[char], mut j: usize) -> (String, usize) {
    let mut word = String::new();
    let mut wq: Option<char> = None;
    while j < chars.len() {
        let c = chars[j];
        match wq {
            Some(q) => {
                if c == q {
                    wq = None;
                } else {
                    word.push(c);
                }
                j += 1;
            }
            None => {
                if c == '\'' || c == '"' {
                    wq = Some(c);
                    j += 1;
                } else if c == '\\' && j + 1 < chars.len() {
                    word.push(chars[j + 1]);
                    j += 2;
                } else if c.is_whitespace() || matches!(c, ';' | '|' | '&' | '<' | '>' | '(' | ')')
                {
                    break;
                } else {
                    word.push(c);
                    j += 1;
                }
            }
        }
    }
    (word, j)
}

/// Does this line hand a heredoc body to a shell interpreter (`bash <<EOF`)? Best effort: any
/// command word on the line whose basename is a POSIX shell counts.
fn invokes_shell(line: &str) -> bool {
    let tokens: Vec<String> = shell_words::split(line)
        .unwrap_or_else(|_| line.split_whitespace().map(str::to_string).collect());
    tokens.iter().any(|t| {
        let base = std::path::Path::new(t)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(t.as_str());
        matches!(base, "bash" | "sh" | "zsh" | "dash" | "ksh")
    })
}

/// Inner command lines of every `$( … )` / backtick substitution in `text`. Each runs as its own
/// command, so each must face the denylist — `echo $(rm -rf ~)` is an `rm`, not an `echo`.
/// Substitution happens inside double quotes too, but never inside single quotes.
fn command_substitutions(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                }
                i += 1;
            }
            Some(q) => {
                if c == '\\' && i + 1 < chars.len() {
                    i += 2;
                    continue;
                }
                if let Some((inner, next)) = read_substitution(&chars, i) {
                    push_substitution(&mut out, inner);
                    i = next;
                    continue;
                }
                if c == q {
                    quote = None;
                }
                i += 1;
            }
            None => {
                if c == '\\' && i + 1 < chars.len() {
                    i += 2;
                    continue;
                }
                if c == '\'' || c == '"' {
                    quote = Some(c);
                    i += 1;
                    continue;
                }
                if let Some((inner, next)) = read_substitution(&chars, i) {
                    push_substitution(&mut out, inner);
                    i = next;
                    continue;
                }
                i += 1;
            }
        }
    }
    out
}

fn push_substitution(out: &mut Vec<String>, inner: String) {
    if !inner.trim().is_empty() {
        out.push(inner);
    }
}

/// End index (exclusive) of a `$( … )` / backtick substitution starting at `i`, if one starts there.
fn substitution_end(chars: &[char], i: usize) -> Option<usize> {
    read_substitution(chars, i).map(|(_, end)| end)
}

/// How far a substitution scan looks for its closing delimiter. Callers probe at EVERY character,
/// so an unbounded rescan makes a pathological `$($($(…` command quadratic in the broker — a cheap
/// self-inflicted stall on the hot permission path. Giving up past the cap returns `None`, which
/// treats the `$`/backtick as an ordinary character: the region is then split on operators as
/// usual, i.e. the conservative direction.
const MAX_SUBSTITUTION_SCAN: usize = 4096;

/// The inner text of a substitution starting at `i` plus the index just past its closing delimiter.
/// `$(( … ))` is arithmetic, not a command — it is skipped over with an empty inner.
fn read_substitution(chars: &[char], i: usize) -> Option<(String, usize)> {
    let limit = chars.len().min(i.saturating_add(MAX_SUBSTITUTION_SCAN));
    if chars[i] == '`' {
        let mut j = i + 1;
        let mut inner = String::new();
        while j < limit {
            if chars[j] == '\\' && j + 1 < chars.len() {
                inner.push(chars[j]);
                inner.push(chars[j + 1]);
                j += 2;
                continue;
            }
            if chars[j] == '`' {
                return Some((inner, j + 1));
            }
            inner.push(chars[j]);
            j += 1;
        }
        return None; // unterminated — treat as an ordinary character
    }
    if chars[i] != '$' || chars.get(i + 1) != Some(&'(') {
        return None;
    }
    let arithmetic = chars.get(i + 2) == Some(&'(');
    let mut depth = 0usize;
    let mut inner = String::new();
    let mut j = i + 1;
    while j < limit {
        match chars[j] {
            '(' => {
                depth += 1;
                if depth > 1 {
                    inner.push('(');
                }
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let text = if arithmetic { String::new() } else { inner };
                    return Some((text, j + 1));
                }
                inner.push(')');
            }
            other => inner.push(other),
        }
        j += 1;
    }
    None
}

/// Drop leading no-op wrapper binaries so `env X=1 nice rm ...` matches `rm ...`.
fn strip_wrappers(tokens: &[String]) -> Vec<String> {
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "nohup" | "nice" | "time" | "command" | "builtin" | "exec" => i += 1,
            // `env` followed by VAR=VAL assignments
            "env" => {
                i += 1;
                while i < tokens.len() && tokens[i].contains('=') && !tokens[i].starts_with('-') {
                    i += 1;
                }
            }
            _ => break,
        }
    }
    tokens[i..].to_vec()
}

/// If the command is `bash -c "<script>"` / `sh -lc "<script>"` / `cmd /C "<command>"` etc.,
/// return the inner script so catastrophic-deny patterns can be checked recursively.
fn inner_script(tokens: &[String]) -> Option<String> {
    if tokens.len() < 3 {
        return None;
    }
    let bin = std::path::Path::new(&tokens[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&tokens[0]);
    if matches!(bin, "bash" | "sh" | "zsh" | "dash") {
        // find a `-c` (possibly combined like `-lc`) and take the following token as the script.
        for (i, t) in tokens.iter().enumerate().skip(1) {
            if t.starts_with('-') && t.contains('c') {
                return tokens.get(i + 1).cloned();
            }
        }
        return None;
    }
    // Windows: `cmd /C <command>` — everything after /C is the inner command.
    if bin.eq_ignore_ascii_case("cmd") {
        for (i, t) in tokens.iter().enumerate().skip(1) {
            if t.eq_ignore_ascii_case("/C") && i + 1 < tokens.len() {
                return Some(tokens[i + 1..].join(" "));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_operators_treats_newlines_as_separators_but_respects_quoting() {
        let segs = |c: &str| -> Vec<String> {
            split_operators(c)
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        assert_eq!(segs("a\nb"), vec!["a", "b"]);
        assert_eq!(segs("a\r\nb"), vec!["a", "b"]);
        assert_eq!(segs("a \\\n b"), vec!["a  b"]);
        assert_eq!(segs("echo \"x\ny\""), vec!["echo \"x\ny\""]);
        assert_eq!(segs("echo $(a; b) c"), vec!["echo $(a; b) c"]);
        assert_eq!(segs("a; b && c | d"), vec!["a", "b", "c", "d"]);
    }
}
