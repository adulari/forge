//! Folds streamed deltas into whole Claude-Code-shaped `assistant` messages for `stream-json`.
//!
//! Forge streams an answer as many small deltas; `claude -p --output-format stream-json` emits one
//! `assistant` event per finished message and only exposes the deltas, as `stream_event` lines,
//! under `--include-partial-messages`. A message is one model call: it opens at the first delta
//! and closes when the call is over (its cost lands, a tool starts, a new request goes out, or the
//! turn ends). Tool calls of the same model call share its message id, as in Claude Code.

use serde_json::{json, Value};

#[derive(Debug)]
enum Block {
    Text(String),
    Thinking(String),
}

impl Block {
    fn kind(&self) -> &'static str {
        match self {
            Block::Text(_) => "text",
            Block::Thinking(_) => "thinking",
        }
    }

    fn to_content(&self) -> Value {
        match self {
            Block::Text(text) => json!({ "type": "text", "text": text }),
            Block::Thinking(thinking) => json!({ "type": "thinking", "thinking": thinking }),
        }
    }
}

/// What the presenter writes: a whole message, or a partial `stream_event` payload.
#[derive(Debug, PartialEq)]
pub(crate) enum Out {
    Assistant(Value),
    Partial(Value),
}

#[derive(Debug, Default)]
pub(crate) struct MessageAssembler {
    partials: bool,
    model: String,
    message_id: Option<String>,
    blocks: Vec<Block>,
    open: Option<Block>,
    started: bool,
    /// A text message went out since the last tool result, so the turn's answer is on the wire.
    answered: bool,
}

impl MessageAssembler {
    pub(crate) fn new(partials: bool) -> Self {
        Self {
            partials,
            ..Self::default()
        }
    }

    pub(crate) fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
    }

    pub(crate) fn answered(&self) -> bool {
        self.answered
    }

    pub(crate) fn tool_finished(&mut self) {
        self.answered = false;
    }

    fn id(&mut self) -> String {
        self.message_id
            .get_or_insert_with(|| format!("msg_{}", forge_types::new_id().replace('-', "")))
            .clone()
    }

    /// A new model request: whatever the previous call said is complete.
    pub(crate) fn begin_call(&mut self, model: &str, usage: &Value) -> Vec<Out> {
        let out = self.flush(usage);
        self.message_id = None;
        self.set_model(model);
        out
    }

    pub(crate) fn text(&mut self, delta: &str) -> Vec<Out> {
        self.push(delta, false)
    }

    pub(crate) fn thinking(&mut self, delta: &str) -> Vec<Out> {
        self.push(delta, true)
    }

    fn push(&mut self, delta: &str, thinking: bool) -> Vec<Out> {
        let mut out = Vec::new();
        if delta.is_empty() {
            return out;
        }
        let same_kind = matches!(
            (&self.open, thinking),
            (Some(Block::Thinking(_)), true) | (Some(Block::Text(_)), false)
        );
        if !same_kind {
            out.extend(self.close_block());
            if !self.started && self.partials {
                let message = json!({
                    "id": self.id(), "type": "message", "role": "assistant",
                    "model": self.model, "content": [], "stop_reason": null,
                    "stop_sequence": null,
                    "usage": { "input_tokens": 0, "output_tokens": 0 }
                });
                out.push(Out::Partial(
                    json!({ "type": "message_start", "message": message }),
                ));
            }
            self.started = true;
            let block = if thinking {
                Block::Thinking(String::new())
            } else {
                Block::Text(String::new())
            };
            if self.partials {
                out.push(Out::Partial(json!({
                    "type": "content_block_start", "index": self.blocks.len(),
                    "content_block": block.to_content()
                })));
            }
            self.open = Some(block);
        }
        match self.open.as_mut() {
            Some(Block::Text(text)) => text.push_str(delta),
            Some(Block::Thinking(text)) => text.push_str(delta),
            None => {}
        }
        if self.partials {
            let delta = if thinking {
                json!({ "type": "thinking_delta", "thinking": delta })
            } else {
                json!({ "type": "text_delta", "text": delta })
            };
            out.push(Out::Partial(json!({
                "type": "content_block_delta", "index": self.blocks.len(), "delta": delta
            })));
        }
        out
    }

    fn close_block(&mut self) -> Vec<Out> {
        let Some(block) = self.open.take() else {
            return Vec::new();
        };
        let index = self.blocks.len();
        self.blocks.push(block);
        if self.partials {
            vec![Out::Partial(
                json!({ "type": "content_block_stop", "index": index }),
            )]
        } else {
            Vec::new()
        }
    }

    /// End the current message: emit it whole (plus the closing partial events).
    pub(crate) fn flush(&mut self, usage: &Value) -> Vec<Out> {
        let mut out = self.close_block();
        if self.blocks.is_empty() {
            return out;
        }
        if self.partials {
            out.push(Out::Partial(json!({
                "type": "message_delta",
                "delta": { "stop_reason": null, "stop_sequence": null },
                "usage": usage
            })));
            out.push(Out::Partial(json!({ "type": "message_stop" })));
        }
        let blocks = std::mem::take(&mut self.blocks);
        if blocks.iter().any(|b| b.kind() == "text") {
            self.answered = true;
        }
        out.push(Out::Assistant(
            self.message(blocks.iter().map(Block::to_content).collect(), usage),
        ));
        self.started = false;
        out
    }

    /// A whole `assistant` message carrying `content`, tied to the current model call.
    pub(crate) fn message(&mut self, content: Vec<Value>, usage: &Value) -> Value {
        json!({
            "id": self.id(), "type": "message", "role": "assistant", "model": self.model,
            "content": content, "stop_reason": null, "stop_sequence": null, "usage": usage
        })
    }

    /// A tool call: closes any text first, then one `tool_use` message on the same message id.
    pub(crate) fn tool_use(&mut self, block: Value, usage: &Value) -> Vec<Out> {
        let mut out = self.flush(usage);
        out.push(Out::Assistant(self.message(vec![block], usage)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_texts(out: &[Out]) -> Vec<Value> {
        out.iter()
            .filter_map(|o| match o {
                Out::Assistant(m) => Some(m["content"].clone()),
                Out::Partial(_) => None,
            })
            .collect()
    }

    #[test]
    fn deltas_become_one_whole_message() {
        let mut a = MessageAssembler::new(false);
        a.set_model("m");
        let usage = json!({});
        let mut out = Vec::new();
        for d in ["Let ", "me ", "look."] {
            out.extend(a.text(d));
        }
        assert!(
            out.is_empty(),
            "nothing is emitted mid-message without partials"
        );
        out.extend(a.flush(&usage));
        let contents = assistant_texts(&out);
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0][0]["text"], "Let me look.");
        assert!(a.answered());
        assert!(
            a.flush(&usage).is_empty(),
            "a flushed message is not sent twice"
        );
    }

    #[test]
    fn thinking_and_text_are_separate_blocks_of_one_message() {
        let mut a = MessageAssembler::new(false);
        let usage = json!({});
        a.thinking("hmm");
        a.text("answer");
        let out = a.flush(&usage);
        let contents = assistant_texts(&out);
        assert_eq!(contents[0][0]["type"], "thinking");
        assert_eq!(contents[0][1]["type"], "text");
    }

    #[test]
    fn partials_follow_the_anthropic_event_sequence() {
        let mut a = MessageAssembler::new(true);
        let usage = json!({ "output_tokens": 2 });
        let mut out = a.text("Hi");
        out.extend(a.text(" there"));
        out.extend(a.flush(&usage));
        let kinds: Vec<String> = out
            .iter()
            .map(|o| match o {
                Out::Partial(e) => e["type"].as_str().unwrap().to_string(),
                Out::Assistant(_) => "assistant".to_string(),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
                "assistant"
            ]
        );
        let Out::Partial(delta) = &out[2] else {
            panic!()
        };
        assert_eq!(delta["delta"]["type"], "text_delta");
        assert_eq!(delta["delta"]["text"], "Hi");
    }

    #[test]
    fn tool_calls_share_the_message_id_of_the_text_before_them() {
        let mut a = MessageAssembler::new(false);
        let usage = json!({});
        a.text("Reading.");
        let out = a.tool_use(json!({ "type": "tool_use", "name": "read_file" }), &usage);
        let ids: Vec<&Value> = out
            .iter()
            .filter_map(|o| match o {
                Out::Assistant(m) => Some(&m["id"]),
                Out::Partial(_) => None,
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], ids[1]);
        let next = a.begin_call("m", &usage);
        assert!(next.is_empty());
        a.text("Done.");
        let out = a.flush(&usage);
        let Out::Assistant(m) = &out[0] else { panic!() };
        assert_ne!(&m["id"], ids[0], "a new model call is a new message");
    }
}
