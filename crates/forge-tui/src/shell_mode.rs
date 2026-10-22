//! `!command` in the composer: Claude Code's shell mode. The line runs through the shell tool
//! instead of going to the model; see `docs/features/tui-interaction.md`.

/// The shell command of a composer line that starts with `!`, trimmed. `Some("")` for a bare `!`.
/// A line that begins with a space before the `!` is an ordinary prompt, so a message that starts
/// with `!` can still be sent to the model.
pub fn bang_command(line: &str) -> Option<&str> {
    line.strip_prefix('!').map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_bang_is_a_shell_command() {
        assert_eq!(bang_command("!git status"), Some("git status"));
        assert_eq!(bang_command("! ls -la  "), Some("ls -la"));
        assert_eq!(bang_command("!"), Some(""));
        assert_eq!(bang_command("!echo a\necho b"), Some("echo a\necho b"));
    }

    #[test]
    fn a_shell_line_never_opens_the_palette_or_the_file_picker() {
        let line = "!ls /etc @HEAD";
        assert_eq!(crate::slash_token_at(line, 8), None);
        assert_eq!(crate::at_token_at(line, line.len()), None);
        assert!(crate::slash_token_at("look at /etc", 12).is_some());
    }

    #[test]
    fn anything_else_is_a_prompt() {
        assert_eq!(bang_command("fix it!"), None);
        assert_eq!(bang_command(" !important: keep this a prompt"), None);
        assert_eq!(bang_command("/help"), None);
    }
}
