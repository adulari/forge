//! Progress for an OpenAI-compatible tool call whose arguments stream in as chunks.

use std::time::{Duration, Instant};

use crate::StreamEvent;

/// Turns tool-call chunks into a throttled "writing <tool> call · N chars" report, so a long
/// argument payload reads as work in progress rather than an anonymous heartbeat.
pub(crate) struct ToolChunkProgress {
    name: String,
    chars: usize,
    last: Instant,
}

impl ToolChunkProgress {
    pub(crate) fn new() -> Self {
        Self {
            name: String::new(),
            chars: 0,
            last: Instant::now(),
        }
    }

    pub(crate) fn observe(&mut self, call: &genai::chat::ToolCall) -> StreamEvent {
        if !call.fn_name.is_empty() {
            self.name.clone_from(&call.fn_name);
        }
        self.chars += match &call.fn_arguments {
            serde_json::Value::String(s) => s.chars().count(),
            serde_json::Value::Null => 0,
            other => other.to_string().chars().count(),
        };
        if self.name.is_empty() || self.last.elapsed() < Duration::from_millis(250) {
            return StreamEvent::ProviderActivity;
        }
        self.last = Instant::now();
        StreamEvent::ProviderToolInput {
            name: self.name.clone(),
            chars: self.chars,
        }
    }
}
