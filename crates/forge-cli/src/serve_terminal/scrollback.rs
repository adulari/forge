//! Read access to a terminal's history for `read_terminal` and the `@terminal` mention.

use super::TerminalRegistry;

impl TerminalRegistry {
    /// The tail of a session's terminal history as raw bytes, for `read_terminal` and the
    /// `@terminal` mention. Without a `terminal_id` the lexically first terminal of the session
    /// answers. Returns the terminal id and the bytes.
    pub(crate) async fn scrollback(
        &self,
        session_id: &str,
        terminal_id: Option<&str>,
    ) -> Option<(String, Vec<u8>)> {
        let handle = {
            let terminals = self.terminals.lock().await;
            terminals
                .iter()
                .filter(|(key, _)| {
                    key.session_id == session_id
                        && terminal_id.is_none_or(|id| key.terminal_id == id)
                })
                .min_by(|a, b| a.0.terminal_id.cmp(&b.0.terminal_id))
                .map(|(_, handle)| handle.clone())?
        };
        let (_, bytes) = handle.history.lock().await.snapshot();
        Some((handle.key.terminal_id.clone(), bytes))
    }
}
