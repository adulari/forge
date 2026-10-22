//! `/context`: what a request to the model is made of, by category.
//!
//! The status-line gauge says how full the window was on the last request; this says why. Counts
//! use the same tokenizer and per-message framing as the trimmer (`tokens.rs`), over what the
//! next request would carry before window fitting: the system prompt, the tool definitions the
//! model is shown, the pinned session block and the stored conversation.

use forge_types::{Role, Visibility};

use super::*;

/// One row of the breakdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCategory {
    pub label: &'static str,
    pub tokens: usize,
    /// Messages, tools or files behind the row (0 when it is one block).
    pub items: usize,
}

/// The whole breakdown, in request order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextReport {
    pub categories: Vec<ContextCategory>,
}

impl ContextReport {
    pub fn total(&self) -> usize {
        self.categories.iter().map(|c| c.tokens).sum()
    }
}

impl Session {
    /// The estimated size of the next request, split into what fills it.
    pub fn context_report(&self) -> ContextReport {
        let system: usize = self
            .system_preamble()
            .iter()
            .map(context_pipeline::message_tokens)
            .sum();
        let specs = self.tool_specs();
        let pinned = self.pinned_tokens();
        let instructions = self
            .cached_agents_md()
            .as_deref()
            .map_or(0, tokens::count_text)
            .min(pinned);
        let mut categories = vec![
            row("System prompt", system, 0),
            row(
                "Tool definitions",
                context_pipeline::tool_spec_tokens(&specs),
                specs.len(),
            ),
            row("Project instructions", instructions, 0),
            row("Memory, goal and tasks", pinned - instructions, 0),
        ];
        let mut conversation = [
            row("Your messages", 0, 0),
            row("Assistant replies", 0, 0),
            row("Tool results", 0, 0),
            row("Harness notes", 0, 0),
        ];
        for message in self
            .transcript
            .iter()
            .filter(|m| m.visibility != Visibility::UiOnly)
        {
            let slot = match message.role {
                Role::User => 0,
                Role::Assistant => 1,
                Role::Tool => 2,
                Role::System => 3,
            };
            conversation[slot].tokens += context_pipeline::message_tokens(message);
            conversation[slot].items += 1;
        }
        categories.extend(conversation);
        ContextReport { categories }
    }
}

fn row(label: &'static str, tokens: usize, items: usize) -> ContextCategory {
    ContextCategory {
        label,
        tokens,
        items,
    }
}
