//! `[git] commit_language`: the language the review dock's "generate commit message" button
//! writes in. The setting lives on [`crate::GitConfig`]; this module resolves it, including the
//! per-repository override in `.forge/config.toml`.

use std::path::Path;

pub const DEFAULT_COMMIT_LANGUAGE: &str = "English";

/// `configured`, or the default when it is blank.
pub fn commit_language_or_default(configured: &str) -> &str {
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        DEFAULT_COMMIT_LANGUAGE
    } else {
        trimmed
    }
}

/// `[git] commit_language` as `dir/.forge/config.toml` sets it, if it does. Read per call, since a
/// daemon's startup config came from no particular repository.
pub fn repo_commit_language(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(".forge").join("config.toml")).ok()?;
    let value: toml::Table = toml::from_str(&text).ok()?;
    let language = value.get("git")?.get("commit_language")?.as_str()?.trim();
    (!language.is_empty()).then(|| language.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_messages_default_to_english_and_blank_falls_back() {
        let parsed: crate::GitConfig = toml::from_str("").unwrap();
        assert_eq!(
            commit_language_or_default(&parsed.commit_language),
            "English"
        );
        let dutch: crate::GitConfig = toml::from_str("commit_language = \"Dutch\"").unwrap();
        assert_eq!(commit_language_or_default(&dutch.commit_language), "Dutch");
        let blank: crate::GitConfig = toml::from_str("commit_language = \"  \"").unwrap();
        assert_eq!(
            commit_language_or_default(&blank.commit_language),
            "English"
        );
    }

    #[test]
    fn a_repo_config_overrides_the_language() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(repo_commit_language(dir.path()), None);
        std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
        std::fs::write(
            dir.path().join(".forge/config.toml"),
            "[git]\ncommit_language = \"Dutch\"\n",
        )
        .unwrap();
        assert_eq!(repo_commit_language(dir.path()).as_deref(), Some("Dutch"));
        std::fs::write(
            dir.path().join(".forge/config.toml"),
            "[git]\ncommit_language = \"\"\n",
        )
        .unwrap();
        assert_eq!(repo_commit_language(dir.path()), None);
    }
}
