//! Core-side support for the persistent `python` kernel tool (`[harness] eval_kernel`,
//! docs/features/eval-kernel.md): the guidance block, failure recognition, the `!`/`%pip` magic
//! lines the permission floor checks, and the verification-ledger classification.

use serde_json::Value;

use crate::completion::VerificationObservation;

pub(crate) const GUIDANCE: &str = "python (persistent kernel):\n\
- Use `python` to explore data and files iteratively: load once (`df = pd.read_csv('x.csv')`), then inspect and transform it in later calls instead of repeating `python3 -c` that reloads the same input. A final bare expression is displayed.\n\
- Kernel state is not the deliverable: write final code and outputs to disk with the edit tools, then run the on-disk program or its tests with `shell` before you finish.\n\
- Run tests, builds and servers with `shell`, not `!` in the kernel.";

/// A kernel cell that errored, was interrupted, or whose kernel died or never started. A non-zero
/// `!` exit alone is not a failure (the header says `ok`).
pub(crate) fn cell_failed(result: &str) -> bool {
    let first = result.lines().next().unwrap_or("");
    [
        "python: error",
        "python: interrupted",
        "python: kernel ",
        "python: failed",
    ]
    .iter()
    .any(|prefix| first.starts_with(prefix))
}

pub(crate) fn cell_code(args: &Value) -> Option<&str> {
    args.get("code").and_then(Value::as_str)
}

/// Quote state across lines, so a `!`-looking line inside a triple-quoted string is not a magic.
fn triple_quote_open(line: &str, mut open: Option<&'static str>) -> Option<&'static str> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match open {
            Some(q) => {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if line[i..].starts_with(q) {
                    open = None;
                    i += 3;
                } else {
                    i += 1;
                }
            }
            None => match bytes[i] {
                b'#' => break,
                c @ (b'\'' | b'"') => {
                    let triple = if c == b'\'' { "'''" } else { "\"\"\"" };
                    if line[i..].starts_with(triple) {
                        open = Some(triple);
                        i += 3;
                    } else {
                        i += 1;
                        while i < bytes.len() && bytes[i] != c {
                            i += if bytes[i] == b'\\' { 2 } else { 1 };
                        }
                        i += 1;
                    }
                }
                _ => i += 1,
            },
        }
    }
    open
}

/// The `!cmd` and `%pip args` statement lines of a cell, as shell text.
pub(crate) fn magic_lines(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut open = None;
    for line in code.lines() {
        if open.is_none() {
            let stripped = line.trim_start();
            if let Some(cmd) = stripped.strip_prefix('!') {
                out.push(cmd.trim().to_string());
            } else if let Some(args) = stripped.strip_prefix("%pip") {
                if args.is_empty() || args.starts_with(char::is_whitespace) {
                    out.push(format!("pip {}", args.trim()));
                }
            }
        }
        open = triple_quote_open(line, open);
    }
    out
}

fn write_patterns() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r#"open\([^)]*,\s*(?:mode\s*=\s*)?['"][rbt]*[wax+][^'"]*['"]|open\([^)]*mode\s*=\s*['"][^'"]*[wax+]|\.write_text\(|\.write_bytes\(|\.to_csv\(|\.to_parquet\(|\.to_excel\(|json\.dump\(|pickle\.dump\(|shutil\.|os\.remove|os\.unlink|os\.rename|os\.replace|\.unlink\(|os\.makedirs|\.mkdir\("#,
        )
        .expect("write pattern")
    })
}

/// A kernel cell is a Mutation when it writes files or runs any magic, otherwise Generic, and
/// never a Check: `!pytest` passing is deliberately not credited, because a non-zero `!` exit
/// leaves the cell `ok`.
pub(crate) fn classify(code: &str) -> VerificationObservation {
    if !magic_lines(code).is_empty() || write_patterns().is_match(code) {
        VerificationObservation::Mutation
    } else {
        VerificationObservation::Generic
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_headers_are_recognised_and_success_is_not() {
        for failed in [
            "python: error KeyError (cell 3)\ntb",
            "python: interrupted after 60s; kernel state kept",
            "python: kernel died (exit 3); state lost",
            "python: kernel killed after ignoring interrupt; state lost",
            "python: failed to start (tried x): y; use shell",
        ] {
            assert!(cell_failed(failed), "{failed}");
        }
        assert!(!cell_failed(
            "python: ok (cell 1, 0.1s; a ! command exited 1)"
        ));
        assert!(!cell_failed("hi\npython: error"));
    }

    #[test]
    fn magic_lines_skip_strings_and_comments() {
        let code =
            "x = 1\n!ls -la\n  %pip install pandas\ns = '''\n!sudo true\n'''\n# !rm\nprint('!x')";
        assert_eq!(magic_lines(code), vec!["ls -la", "pip install pandas"]);
        assert!(magic_lines("a = 'su'\nopen('/etc/passwd').read()").is_empty());
    }

    #[test]
    fn classification_never_yields_a_check() {
        assert_eq!(
            classify("df.to_csv('o.csv')"),
            VerificationObservation::Mutation
        );
        assert_eq!(
            classify("open('f', 'w').write('x')"),
            VerificationObservation::Mutation
        );
        assert_eq!(classify("!pytest -q"), VerificationObservation::Mutation);
        assert_eq!(classify("df.describe()"), VerificationObservation::Generic);
        assert_eq!(
            classify("open('f').read()"),
            VerificationObservation::Generic
        );
    }
}
