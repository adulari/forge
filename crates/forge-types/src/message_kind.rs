use serde::{Deserialize, Serialize};

/// Who wrote a message and how long it stays relevant. `Normal` is the person's words or durable
/// context. `Nudge` is a harness-authored user-role message (a continuation or failure push-back
/// stored as `user` because the next request must end on a legal user turn): it is never the
/// task, so it must not take a user message's place as the turn's anchor or in a summary. The
/// rest are ephemeral system notes where only the newest of each kind says anything the older
/// ones do not (a clock reading, a hint about the last tool result, a progress nudge), so the
/// request view keeps just that one. Persisted in the message row's `nudge` column.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    #[default]
    Normal,
    Nudge,
    Clock,
    Hint,
    Silence,
    LoopGuard,
    Continue,
    Deadline,
}

impl MessageKind {
    pub fn code(self) -> i64 {
        match self {
            MessageKind::Normal => 0,
            MessageKind::Nudge => 1,
            MessageKind::Clock => 2,
            MessageKind::Hint => 3,
            MessageKind::Silence => 4,
            MessageKind::LoopGuard => 5,
            MessageKind::Continue => 6,
            MessageKind::Deadline => 7,
        }
    }
    pub fn from_code(code: i64) -> MessageKind {
        match code {
            1 => MessageKind::Nudge,
            2 => MessageKind::Clock,
            3 => MessageKind::Hint,
            4 => MessageKind::Silence,
            5 => MessageKind::LoopGuard,
            6 => MessageKind::Continue,
            7 => MessageKind::Deadline,
            _ => MessageKind::Normal,
        }
    }
    pub fn is_default(&self) -> bool {
        matches!(self, MessageKind::Normal)
    }
    /// A system note whose older copies are superseded by the newest of its kind.
    pub fn is_ephemeral_note(self) -> bool {
        self.code() >= MessageKind::Clock.code()
    }
}
