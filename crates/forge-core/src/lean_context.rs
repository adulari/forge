//! Cheap prompt-shape checks that decide whether optional per-session context earns its tokens.
//!
//! Every injected block rides along on every later model request of the session, so context that
//! only helps a broad or exploratory task is dead weight on a one-line, file-targeted edit.

use std::path::Path;

const SIMPLE_MAX_WORDS: usize = 45;
const SIMPLE_MAX_SENTENCES: usize = 2;

/// A short single-instruction request: no list, at most two sentences, under ~45 words. Standing
/// orchestration guidance (skill/subagent/MCP routing) adds nothing there.
pub(crate) fn prompt_is_simple(prompt: &str) -> bool {
    if prompt.split_whitespace().count() > SIMPLE_MAX_WORDS {
        return false;
    }
    !crate::review_gate::request_lists_multiple_requirements(prompt)
        && prompt
            .split(['.', '!', '?', '\n'])
            .filter(|s| s.split_whitespace().count() >= 3)
            .count()
            <= SIMPLE_MAX_SENTENCES
}

/// The prompt names a path that exists in the workspace (`src/store.rs`, `Cargo.toml:12`). The
/// model then reads that file directly, so retrieved neighbours are redundant.
pub(crate) fn prompt_names_workspace_file(prompt: &str, root: &Path) -> bool {
    prompt.split_whitespace().any(|raw| {
        let token = raw.trim_matches(|c: char| {
            matches!(
                c,
                '`' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']' | '<' | '>' | '.' | '!' | '?'
            )
        });
        let token = token.split(':').next().unwrap_or(token);
        let looks_like_path = token.contains('/')
            || Path::new(token).extension().is_some_and(|e| {
                (1..=5).contains(&e.len()) && e.to_string_lossy().chars().all(char::is_alphanumeric)
            });
        looks_like_path
            && !token.starts_with("http")
            && !token.contains("..")
            && !Path::new(token).is_absolute()
            && root.join(token).is_file()
    })
}

/// The project's `CLAUDE.md` already carries `agents_md` (identical text, or it imports
/// `@AGENTS.md`). A bridged claude loads `CLAUDE.md` itself, so injecting the same instructions
/// again doubles them on every request of its loop.
pub(crate) fn claude_md_covers(root: &Path, agents_md: &str) -> bool {
    [root.join("CLAUDE.md"), root.join(".claude/CLAUDE.md")]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .any(|claude| {
            claude.trim() == agents_md.trim() || claude.lines().any(|l| l.trim() == "@AGENTS.md")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_prompts_skip_orchestration_complex_ones_keep_it() {
        assert!(prompt_is_simple(
            "How many .rs files are in src/? Just count them."
        ));
        assert!(prompt_is_simple(
            "parse_duration(\"1h30m\") returns 11400 instead of 5400. Find the root cause and fix it, and add a regression test for it."
        ));
        assert!(!prompt_is_simple(
            "Make the retry delay a setting. Default it to 5s and use it in Store. Document it in the README."
        ));
        assert!(!prompt_is_simple("Do:\n- a\n- b"));
        assert!(!prompt_is_simple(&"word ".repeat(60)));
    }

    #[test]
    fn named_workspace_files_are_detected_only_when_they_exist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/store.rs"), "").unwrap();
        assert!(prompt_names_workspace_file(
            "hard-coded delay in src/store.rs.",
            dir.path()
        ));
        assert!(prompt_names_workspace_file(
            "see `src/store.rs:42`",
            dir.path()
        ));
        assert!(!prompt_names_workspace_file(
            "fix src/missing.rs",
            dir.path()
        ));
        assert!(!prompt_names_workspace_file(
            "explain how the store works",
            dir.path()
        ));
        assert!(!prompt_names_workspace_file(
            "read /etc/passwd or ../x.rs",
            dir.path()
        ));
    }

    #[test]
    fn claude_md_duplicate_detection() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!claude_md_covers(dir.path(), "rules"));
        std::fs::write(dir.path().join("CLAUDE.md"), "rules\n").unwrap();
        assert!(claude_md_covers(dir.path(), "rules"));
        assert!(!claude_md_covers(dir.path(), "other rules"));
        std::fs::write(dir.path().join("CLAUDE.md"), "# Project\n@AGENTS.md\n").unwrap();
        assert!(claude_md_covers(dir.path(), "other rules"));
    }
}
