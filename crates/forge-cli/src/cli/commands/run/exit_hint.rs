//! The line printed when an interactive chat ends: how to pick the conversation back up.

use forge_core::Session;

/// The width assumed when the terminal cannot be asked.
const FALLBACK_COLUMNS: usize = 80;

/// `Resume this session: forge chat --resume <id>`, with the session's name after it when it has
/// one, cut so the whole line fits `columns` (the command is never cut). The id is the eight
/// characters the rest of the interface shows.
pub(crate) fn resume_hint(session_id: &str, title: Option<&str>, columns: usize) -> String {
    let short: String = session_id.chars().take(8).collect();
    let command = format!("Resume this session: forge chat --resume {short}");
    let name = title.map(str::trim).filter(|t| !t.is_empty());
    let Some(name) = name else {
        return command;
    };
    // Two spaces and a pair of quotes around the name.
    let room = columns.saturating_sub(command.chars().count() + 4);
    if room < 8 {
        return command;
    }
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = if name.chars().count() > room {
        let cut: String = name.chars().take(room - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        name
    };
    format!("{command}  \"{name}\"")
}

/// The hint for `session`, or `None` when nothing was said in it: a conversation with no messages
/// is not worth coming back to, and resuming one has nothing to restore.
pub(crate) fn exit_hint(session: &Session) -> Option<String> {
    let id = session.session_id();
    if session.store.message_count(id).unwrap_or(0) == 0 {
        return None;
    }
    let title = session.store.session_title(id).ok().flatten();
    let columns = crossterm::terminal::size()
        .map(|(cols, _)| usize::from(cols))
        .unwrap_or(FALLBACK_COLUMNS);
    Some(resume_hint(id, title.as_deref(), columns))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "34a89baa-1111-4222-8333-444455556666";

    #[test]
    fn the_command_uses_the_short_id() {
        assert_eq!(
            resume_hint(ID, None, 120),
            "Resume this session: forge chat --resume 34a89baa"
        );
    }

    #[test]
    fn a_name_follows_the_command_on_the_same_line() {
        let line = resume_hint(ID, Some("Fix the failing test"), 120);
        assert_eq!(
            line,
            "Resume this session: forge chat --resume 34a89baa  \"Fix the failing test\""
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn a_long_name_is_cut_to_the_terminal_width_and_the_command_never_is() {
        let long = "Count slowly from 1 to 20, one number per line, adding a two-word fun fact";
        let line = resume_hint(ID, Some(long), 80);
        assert!(line.chars().count() <= 80, "{line}");
        assert!(line.starts_with("Resume this session: forge chat --resume 34a89baa  \""));
        assert!(line.ends_with("…\""), "{line}");
        // Too narrow for any useful part of the name: the command alone.
        let narrow = resume_hint(ID, Some(long), 55);
        assert_eq!(narrow, "Resume this session: forge chat --resume 34a89baa");
    }

    #[test]
    fn a_blank_or_multiline_name_collapses() {
        assert_eq!(
            resume_hint(ID, Some("   "), 120),
            "Resume this session: forge chat --resume 34a89baa"
        );
        let line = resume_hint(ID, Some("two\nlines  here"), 120);
        assert!(line.ends_with("\"two lines here\""), "{line}");
    }
}
