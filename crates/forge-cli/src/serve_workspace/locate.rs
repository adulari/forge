//! Finding the file a chat path chip means when it is not at `<root>/<path>`.
//!
//! Models write `SOLVER.md` for `notes/SOLVER.md`, or name a file they wrote by absolute path in
//! another directory. A file inside the workspace is found by its path suffix. A file outside it
//! opens read-only, and only when all of these hold: the caller may write the workspace (a
//! view-only device never reaches outside it), the path appears in something the user typed or in
//! the arguments of a tool call (never in tool output or fetched content, which anyone can
//! influence), and it is not a credential, key, Forge state file or repository internal.

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use super::{sensitive, skipped_search_directory, wire_path, MAX_SEARCH_FILES};

pub(super) enum Located {
    /// Workspace-relative wire path of the one file the suffix names.
    Inside(String),
    /// Absolute path outside the workspace that the session's transcript names.
    Outside(PathBuf),
}

/// `raw` with `./` and surrounding slashes removed, or `None` when it cannot name a file.
fn suffix_of(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches("./").trim_matches('/');
    if trimmed.is_empty()
        || trimmed
            .split('/')
            .any(|part| part == ".." || part.is_empty())
    {
        return None;
    }
    Some(trimmed.to_string())
}

/// The workspace file whose path ends with `raw` (`SOLVER.md` → `notes/SOLVER.md`). Several
/// matches pick the shallowest, then the shortest path.
pub(super) fn inside_by_suffix(root: &Path, raw: &str) -> Option<String> {
    let suffix = suffix_of(raw)?;
    let tail = format!("/{suffix}");
    let mut best: Option<String> = None;
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .filter_entry(|entry| !skipped_search_directory(entry));
    for (scanned, entry) in builder.build().flatten().enumerate() {
        if scanned > MAX_SEARCH_FILES {
            break;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let path = wire_path(relative);
        if path != suffix && !path.ends_with(&tail) {
            continue;
        }
        let better = best.as_ref().is_none_or(|current| {
            let depth = |p: &str| p.matches('/').count();
            (depth(&path), path.len()) < (depth(current), current.len())
        });
        if better {
            best = Some(path);
        }
    }
    best
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | '@' | '~')
}

/// Absolute paths in `text` (`/a/b.md`, `~/a/b.md`), with trailing sentence dots dropped.
fn absolute_paths(text: &str, home: Option<&Path>) -> Vec<PathBuf> {
    text.split(|c: char| !is_path_char(c))
        .filter_map(|token| {
            let token = token.trim_end_matches('.');
            if let Some(rest) = token.strip_prefix("~/") {
                return home.map(|home| home.join(rest));
            }
            (token.starts_with('/') && token.len() > 1).then(|| PathBuf::from(token))
        })
        .collect()
}

fn string_leaves<'a>(value: &'a serde_json::Value, out: &mut Vec<&'a str>) {
    match value {
        serde_json::Value::String(text) => out.push(text),
        serde_json::Value::Array(items) => items.iter().for_each(|item| string_leaves(item, out)),
        serde_json::Value::Object(map) => map.values().for_each(|item| string_leaves(item, out)),
        _ => {}
    }
}

/// The newest absolute path in `texts` (oldest first) that matches `raw` and is a regular file.
/// An absolute `raw` must appear verbatim; a relative one matches as a path suffix.
pub(super) fn referenced_file<'a>(
    texts: impl DoubleEndedIterator<Item = &'a str>,
    raw: &str,
    home: Option<&Path>,
) -> Option<PathBuf> {
    let raw = raw.trim();
    let wanted: Box<dyn Fn(&Path) -> bool> = if let Some(rest) = raw.strip_prefix("~/") {
        let exact = home?.join(rest);
        Box::new(move |path| path == exact)
    } else if raw.starts_with('/') {
        let exact = PathBuf::from(raw);
        Box::new(move |path| path == exact)
    } else {
        let tail = format!("/{}", suffix_of(raw)?);
        Box::new(move |path| path.to_string_lossy().ends_with(&tail))
    };
    for text in texts.rev() {
        let mut found = absolute_paths(text, home);
        found.reverse();
        if let Some(path) = found
            .into_iter()
            .find(|path| wanted(path) && path.is_file())
        {
            return Some(path);
        }
    }
    None
}

/// The texts that may vouch for a path, oldest first: what the user typed, and the string values
/// of the assistant's tool-call arguments (where a written file's absolute path usually is).
/// Tool results and assistant prose can echo attacker-controlled content, so they never count.
pub(super) fn evidence_texts(messages: &[forge_store::StoredMessage]) -> Vec<String> {
    let mut texts: Vec<&str> = Vec::new();
    for message in messages {
        if message.role == forge_types::Role::User {
            texts.push(message.content.as_str());
        }
        for call in &message.tool_calls {
            string_leaves(&call.args, &mut texts);
        }
    }
    texts.into_iter().map(str::to_owned).collect()
}

/// Where `raw` really is: inside `root` by path or suffix first, which needs no transcript, then
/// (only when `allow_outside`) a file the evidence names. `evidence` is called at most once, and
/// only when the cheap checks miss.
pub(super) fn locate(
    root: &Path,
    raw: &str,
    allow_outside: bool,
    evidence: impl FnOnce() -> Vec<String>,
    home: Option<&Path>,
) -> Option<Located> {
    let raw_trimmed = raw.trim();
    let named = if let Some(rest) = raw_trimmed.strip_prefix("~/") {
        home.map(|home| home.join(rest))
    } else if raw_trimmed.starts_with('/') {
        Some(PathBuf::from(raw_trimmed))
    } else {
        None
    };
    match named {
        Some(path) => {
            if let Ok(rest) = path.strip_prefix(root) {
                let relative = wire_path(rest);
                if !relative.is_empty() && root.join(rest).is_file() {
                    return Some(Located::Inside(relative));
                }
            }
        }
        None => {
            if let Some(relative) = inside_by_suffix(root, raw_trimmed) {
                return Some(Located::Inside(relative));
            }
        }
    }
    if !allow_outside {
        return None;
    }
    let texts = evidence();
    let path = referenced_file(texts.iter().map(String::as_str), raw_trimmed, home)?;
    match path.strip_prefix(root) {
        Ok(rest) => Some(Located::Inside(wire_path(rest))),
        Err(_) if sensitive::is_sensitive(&path, home) => None,
        Err(_) => Some(Located::Outside(path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn ev(texts: &[&str]) -> impl FnOnce() -> Vec<String> {
        let owned: Vec<String> = texts.iter().map(|t| t.to_string()).collect();
        move || owned
    }

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("notes/deep")).unwrap();
        fs::write(dir.path().join("notes/SOLVER.md"), "a").unwrap();
        fs::write(dir.path().join("notes/deep/SOLVER.md"), "b").unwrap();
        fs::write(dir.path().join("main.rs"), "c").unwrap();
        dir
    }

    #[test]
    fn a_bare_name_finds_the_shallowest_workspace_file() {
        let dir = workspace();
        assert_eq!(
            inside_by_suffix(dir.path(), "SOLVER.md").as_deref(),
            Some("notes/SOLVER.md")
        );
        assert_eq!(
            inside_by_suffix(dir.path(), "deep/SOLVER.md").as_deref(),
            Some("notes/deep/SOLVER.md")
        );
        assert_eq!(
            inside_by_suffix(dir.path(), "./main.rs").as_deref(),
            Some("main.rs")
        );
        assert_eq!(inside_by_suffix(dir.path(), "OLVER.md"), None);
        assert_eq!(inside_by_suffix(dir.path(), "../main.rs"), None);
    }

    #[test]
    fn a_file_outside_the_workspace_needs_the_transcript_to_name_it() {
        let dir = workspace();
        let elsewhere = tempfile::tempdir().unwrap();
        let outside = elsewhere.path().join("work/SOLVER.md");
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, "x").unwrap();
        let other = elsewhere.path().join("REPORT.md");
        fs::write(&other, "y").unwrap();

        let mention = format!("appended a line to {}.", outside.display());
        let texts = ["unrelated", mention.as_str()];
        match locate(dir.path(), "work/SOLVER.md", true, ev(&texts), None) {
            Some(Located::Outside(path)) => assert_eq!(path, outside),
            _ => panic!("expected the transcript's file"),
        }
        assert!(locate(dir.path(), "REPORT.md", true, ev(&texts), None).is_none());
        let exact = outside.to_string_lossy().into_owned();
        assert!(matches!(
            locate(dir.path(), &exact, true, ev(&texts), None),
            Some(Located::Outside(_))
        ));
        let unnamed = other.to_string_lossy().into_owned();
        assert!(locate(dir.path(), &unnamed, true, ev(&texts), None).is_none());
    }

    #[test]
    fn the_workspace_wins_over_the_transcript_and_absolute_inside_paths_become_relative() {
        let dir = workspace();
        let texts = ["/somewhere/else/SOLVER.md"];
        assert!(matches!(
            locate(dir.path(), "SOLVER.md", true, ev(&texts), None),
            Some(Located::Inside(path)) if path == "notes/SOLVER.md"
        ));
        let absolute = dir.path().join("main.rs").to_string_lossy().into_owned();
        assert!(matches!(
            locate(dir.path(), &absolute, true, Vec::new, None),
            Some(Located::Inside(path)) if path == "main.rs"
        ));
    }

    #[test]
    fn tilde_paths_expand_against_home_and_the_newest_mention_wins() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join("a")).unwrap();
        fs::create_dir_all(home.path().join("b")).unwrap();
        fs::write(home.path().join("a/NOTES.md"), "1").unwrap();
        fs::write(home.path().join("b/NOTES.md"), "2").unwrap();
        let texts = ["wrote ~/a/NOTES.md", "then ~/b/NOTES.md:12"];
        assert_eq!(
            referenced_file(texts.iter().copied(), "NOTES.md", Some(home.path())),
            Some(home.path().join("b/NOTES.md"))
        );
    }

    #[test]
    fn tool_call_argument_strings_are_scanned() {
        let call = serde_json::json!({"code": "await tools.shell({command: 'echo x >> /srv/x/SOLVER.md'})", "n": 3});
        let mut out = Vec::new();
        string_leaves(&call, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(
            absolute_paths(out[0], None),
            vec![PathBuf::from("/srv/x/SOLVER.md")]
        );
    }

    #[test]
    fn a_view_only_caller_never_reaches_outside_the_workspace() {
        let dir = workspace();
        let elsewhere = tempfile::tempdir().unwrap();
        let outside = elsewhere.path().join("notes.md");
        fs::write(&outside, "x").unwrap();
        let mention = format!("wrote {}", outside.display());
        let raw = outside.to_string_lossy().into_owned();
        let texts = [mention.as_str()];
        assert!(locate(dir.path(), &raw, false, ev(&texts), None).is_none());
        assert!(locate(dir.path(), &raw, true, ev(&texts), None).is_some());
    }

    #[test]
    fn credentials_named_in_the_evidence_are_still_refused() {
        let dir = workspace();
        let elsewhere = tempfile::tempdir().unwrap();
        let secret = elsewhere.path().join(".env");
        fs::write(&secret, "TOKEN=1").unwrap();
        let git = elsewhere.path().join("repo/.git/config");
        fs::create_dir_all(git.parent().unwrap()).unwrap();
        fs::write(&git, "url").unwrap();
        for path in [&secret, &git] {
            let mention = format!("saw {}", path.display());
            let raw = path.to_string_lossy().into_owned();
            let texts = [mention.as_str()];
            assert!(locate(dir.path(), &raw, true, ev(&texts), None).is_none());
        }
    }

    #[test]
    fn a_workspace_hit_never_loads_the_transcript() {
        let dir = workspace();
        let loaded = std::cell::Cell::new(false);
        let found = locate(
            dir.path(),
            "SOLVER.md",
            true,
            || {
                loaded.set(true);
                Vec::new()
            },
            None,
        );
        assert!(matches!(found, Some(Located::Inside(_))));
        assert!(
            !loaded.get(),
            "the suffix match answers without the transcript"
        );
    }

    #[test]
    fn only_user_text_and_tool_call_arguments_vouch_for_a_path() {
        use forge_types::{Role, ToolCall};
        let message = |role, content: &str, calls: Vec<ToolCall>| forge_store::StoredMessage {
            role,
            content: content.to_string(),
            model: None,
            tool_calls: calls,
            tool_call_id: None,
            visibility: forge_types::Visibility::Llm,
            provider_items: None,
            kind: forge_types::MessageKind::Normal,
        };
        let call = ToolCall {
            id: "1".into(),
            name: "write_file".into(),
            args: serde_json::json!({"path": "/srv/a/written.md"}),
        };
        let messages = [
            message(Role::User, "open /srv/a/typed.md", Vec::new()),
            message(Role::Tool, "ls shows /home/u/.ssh/id_rsa", Vec::new()),
            message(Role::Assistant, "the page mentions /etc/passwd", vec![call]),
        ];
        let texts = evidence_texts(&messages);
        let joined = texts.join("\n");
        assert!(joined.contains("typed.md") && joined.contains("written.md"));
        assert!(!joined.contains("id_rsa") && !joined.contains("passwd"));
    }
}
