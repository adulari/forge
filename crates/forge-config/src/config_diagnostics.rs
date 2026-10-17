//! Human-facing diagnostics for `config.toml`: where a bad value is (file, line, column) and which
//! keys Forge does not recognise. Neither is fatal to loading beyond what serde already enforces.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use figment::providers::{Format, Serialized, Toml};
use figment::Figment;
use toml_edit::Document;

use crate::toml_write::line_col;
use crate::Config;

/// Byte span of the item at `path` in `doc`, or of the deepest ancestor that exists.
fn span_of(doc: &Document<&str>, path: &[String]) -> Option<std::ops::Range<usize>> {
    let mut table: &dyn toml_edit::TableLike = doc.as_table();
    let mut best = None;
    let mut segments = path.iter();
    while let Some(segment) = segments.next() {
        let Some(item) = table.get(segment) else {
            break;
        };
        best = item.span().or(best);
        if let Some(next) = item.as_table_like() {
            table = next;
        } else if let Some(array) = item.as_array_of_tables() {
            let element = segments
                .next()
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| array.get(index));
            match element {
                Some(element) => {
                    best = element.span().or(best);
                    table = element;
                }
                None => break,
            }
        } else {
            break;
        }
    }
    best
}

/// `file:line:column` of the config key at `path`, searched in `files` in the order given (highest
/// precedence first). `None` when no file defines it, e.g. the value came from the environment.
pub(crate) fn locate(path: &[String], files: &[&Path]) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    for file in files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let Ok(doc) = Document::parse(text.as_str()) else {
            continue;
        };
        if !doc.as_table().contains_key(&path[0]) {
            continue;
        }
        if let Some(span) = span_of(&doc, path) {
            let (line, col) = line_col(&text, span.start);
            return Some(format!("{}:{line}:{col}", file.display()));
        }
    }
    None
}

/// Dotted paths in `text` that no `Config` field accepts, with their line in the file.
pub(crate) fn unknown_keys(text: &str) -> Vec<(String, Option<usize>)> {
    let Ok(value) = Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::string(text))
        .extract::<figment::value::Value>()
    else {
        return Vec::new();
    };
    let mut ignored = Vec::new();
    let _: Result<Config, _> = serde_ignored::deserialize(&value, |path| {
        ignored.push(path.to_string());
    });
    let doc = Document::parse(text).ok();
    ignored
        .into_iter()
        .map(|dotted| {
            let segments: Vec<String> = dotted.split('.').map(str::to_string).collect();
            let line = doc
                .as_ref()
                .and_then(|doc| span_of(doc, &segments))
                .map(|span| line_col(text, span.start).0);
            (dotted, line)
        })
        .collect()
}

static WARNED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Warn once per process about each unknown key in `file`; loading continues regardless.
pub(crate) fn warn_unknown_keys(file: &Path) {
    let Ok(text) = std::fs::read_to_string(file) else {
        return;
    };
    for (key, line) in unknown_keys(&text) {
        let at = line.map(|l| format!(":{l}")).unwrap_or_default();
        let message = format!(
            "unknown config key `{key}` in {}{at} is ignored (typo?)",
            file.display()
        );
        let mut guard = WARNED.lock().unwrap_or_else(|e| e.into_inner());
        if guard
            .get_or_insert_with(HashSet::new)
            .insert(message.clone())
        {
            tracing::warn!("{message}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_config_has_no_unknown_keys() {
        let text = toml::to_string(&Config::default()).unwrap();
        assert_eq!(unknown_keys(&text), Vec::new());
    }

    #[test]
    fn typos_are_reported_with_their_line() {
        let found = unknown_keys("permission_mode = \"default\"\n\n[mesh]\ncredit_mod = \"frugal\"\n\n[nonsense]\nx = 1\n");
        assert!(
            found.contains(&("mesh.credit_mod".to_string(), Some(4))),
            "{found:?}"
        );
        assert!(found.iter().any(|(k, _)| k == "nonsense"), "{found:?}");
    }

    #[test]
    fn locate_finds_a_nested_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "a = 1\n\n[mesh]\ncredit_mode = 5\n").unwrap();
        let at = locate(&["mesh".into(), "credit_mode".into()], &[&path]).unwrap();
        assert!(at.ends_with("config.toml:4:15"), "{at}");
    }
}
