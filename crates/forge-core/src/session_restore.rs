//! What a session says about itself that comes from the store rather than a live turn: the state
//! shown on resume before the next turn runs, and the title derived from what the user typed.

use super::*;

impl Session {
    /// What a resumed session should show before its next turn runs: how the last turn ended (only
    /// a final answer is recoverable from the store; anything else stays unknown) and the model it
    /// last ran on.
    ///
    /// The finished-turn COUNT is deliberately not restored: `turns_finished` is a per-process
    /// counter that the dispatch supervisor compares against its own baseline to notice a worker
    /// finishing a turn, and to re-drive a restored worker that has not run since the daemon
    /// started. Seeding it would report restored workers as already finished.
    pub fn restored_turn_state(&self) -> RestoredTurnState {
        let last_is_answer = self.store.session_last_is_answer(&self.id).unwrap_or(false);
        let model = self
            .pinned_model
            .as_ref()
            .and_then(|set| set.first())
            .cloned()
            .or_else(|| self.store.session_last_model(&self.id).ok().flatten());
        RestoredTurnState {
            last_stop_reason: last_is_answer.then_some(forge_types::StopReason::FinalAnswer),
            model,
        }
    }

    /// Title this session from what the user typed, if it has no title yet. Call with the ORIGINAL
    /// text, before `UserPromptSubmit` hooks rewrite or append to it: the stored first user
    /// message is the hooked prompt, so a title derived from it later carries hook context (or a
    /// hook's replacement) instead of the request.
    pub fn name_from_prompt(&self, original: &str) {
        if let Some(title) = prompt_title(original) {
            let _ = self.store.set_session_title_if_unset(&self.id, &title);
        }
    }
}

/// One readable line for a session title: whitespace collapsed, bounded to 64 characters. `None`
/// for blank input.
pub(crate) fn prompt_title(original: &str) -> Option<String> {
    const MAX: usize = 64;
    let collapsed: String = original.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() > MAX {
        Some(format!(
            "{}…",
            collapsed.chars().take(MAX - 1).collect::<String>()
        ))
    } else {
        Some(collapsed)
    }
}

#[cfg(test)]
mod prompt_title_tests {
    use super::prompt_title;

    #[test]
    fn titles_collapse_whitespace_and_bound_length() {
        assert_eq!(
            prompt_title("  fix\n the   parser "),
            Some("fix the parser".into())
        );
        assert_eq!(prompt_title("   \n"), None);
        let long = "x".repeat(200);
        assert_eq!(prompt_title(&long).unwrap().chars().count(), 64);
    }
}

/// See [`Session::restored_turn_state`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredTurnState {
    pub last_stop_reason: Option<forge_types::StopReason>,
    pub model: Option<String>,
}
