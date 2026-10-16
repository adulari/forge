//! The `/commit` turn (docs/features/commit-discipline.md): the model commits the session's
//! uncommitted work through the ordinary shell tool and its permission gate; Forge never pushes.

pub(super) fn commit_prompt(hint: &str) -> String {
    let hint = if hint.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nThe user's hint for the commit: {}", hint.trim())
    };
    format!(
        "Commit the uncommitted work in this repository. Run `git status` and `git diff` first, \
group the changes into one or more focused commits (one coherent unit each), stage the specific \
files for each (name the paths explicitly: never `git add -A`, `git add .` or `git commit -a`; skip files you did not change unless they clearly belong to \
the same change, and never stage secrets, credentials, logs or build output), and write \
conventional-commit messages (feat/fix/refactor/docs/test/chore) that say what and why. Never \
pass `--no-verify` (or `-n`) or otherwise skip hooks: if a pre-commit hook fails, fix what it \
reports and commit again. Do NOT push. Finish by listing the commits you made with their hashes.{hint}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hint_rides_along_and_pushing_is_forbidden() {
        let p = commit_prompt("  split the proto change out ");
        assert!(p.contains("hint for the commit: split the proto change out"));
        assert!(p.contains("Do NOT push"));
        assert!(p.contains("Never pass `--no-verify`"));
        assert!(p.contains("never `git add -A`"));
        assert!(!commit_prompt("").contains("hint for the commit"));
    }
}
