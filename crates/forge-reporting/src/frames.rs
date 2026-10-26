//! Turn a rendered backtrace into a handful of normalised in-crate frames.
//!
//! Release builds are stripped, so frames can be empty; the panic `Location` (always present) is
//! the fallback. Only symbols from Forge's own crates (`forge_*`) are kept: std/tokio/dependency
//! frames say nothing about which Forge code is at fault and are noise in a fingerprint.

use regex::Regex;
use std::sync::OnceLock;

use crate::scrub;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub frames: Vec<String>,
    /// `crate/src/file.rs:line` of the first in-crate frame that carried one.
    pub location: Option<String>,
}

const MAX_FRAMES: usize = 12;

fn hash_suffix() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"::h[0-9a-f]{16}$").expect("static regex"))
}

fn generics() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<([^<>]*)>").expect("static regex"))
}

/// `<forge_core::Session>::run_turn::{{closure}}::h0123456789abcdef` -> `forge_core::Session::run_turn`.
/// `None` when the symbol is not Forge's own.
pub fn normalize_symbol(symbol: &str) -> Option<String> {
    let mut s = hash_suffix().replace(symbol.trim(), "").into_owned();
    for _ in 0..6 {
        let at_start = s.starts_with('<');
        let next = generics()
            .replace(&s, |c: &regex::Captures| {
                if at_start && c.get(0).is_some_and(|m| m.start() == 0) {
                    c[1].split(" as ").next().unwrap_or_default().to_string()
                } else {
                    String::new()
                }
            })
            .into_owned();
        if next == s {
            break;
        }
        s = next;
    }
    let cleaned: Vec<&str> = s
        .split("::")
        .filter(|seg| !seg.is_empty() && !seg.starts_with('{') && !seg.starts_with('<'))
        .collect();
    let joined = cleaned.join("::");
    // The reporting crate's own hook frames say nothing about the bug.
    if !joined.starts_with("forge_") || joined.starts_with("forge_reporting::") {
        return None;
    }
    Some(scrub::truncate(&joined, 120))
}

pub fn parse_backtrace(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut awaiting_location = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some((index, symbol)) = trimmed.split_once(": ") {
            if index.chars().all(|c| c.is_ascii_digit()) && !index.is_empty() {
                awaiting_location = false;
                if let Some(frame) = normalize_symbol(symbol) {
                    if !parsed.frames.contains(&frame) && parsed.frames.len() < MAX_FRAMES {
                        parsed.frames.push(frame);
                    }
                    awaiting_location = parsed.location.is_none();
                }
                continue;
            }
        }
        if awaiting_location {
            if let Some(at) = trimmed.strip_prefix("at ") {
                let loc = scrub::normalize_location(at);
                if loc.contains("/src/") {
                    parsed.location = Some(loc);
                }
                awaiting_location = false;
            }
        }
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_forge_frames_and_normalises_them() {
        let bt = "\
   0: forge_reporting::panic::capture
   0: forge_core::turn::{closure#0}::inner
   0: std::panicking::begin_panic
             at /rustc/abc/library/std/src/panicking.rs:1:1
   1: forge_core::session::run_turn::{{closure}}::h0123456789abcdef
             at /home/alice/work/forge/crates/forge-core/src/session.rs:412:9
   2: <forge_core::Session as forge_core::Turns>::drive::h0123456789abcdef
             at ./crates/forge-core/src/lib.rs:90:1
   3: tokio::runtime::task::core::Core<T,S>::poll
   4: forge_cli::main
";
        let parsed = parse_backtrace(bt);
        assert_eq!(
            parsed.frames,
            vec![
                "forge_core::turn::inner",
                "forge_core::session::run_turn",
                "forge_core::Session::drive",
                "forge_cli::main"
            ]
        );
        assert_eq!(
            parsed.location.as_deref(),
            Some("forge-core/src/session.rs:412")
        );
        assert!(!parsed.frames.iter().any(|f| f.contains("alice")));
    }

    #[test]
    fn stripped_backtraces_yield_nothing() {
        assert_eq!(
            parse_backtrace("   0: <unknown>\n   1: <unknown>\n"),
            Parsed::default()
        );
    }
}
