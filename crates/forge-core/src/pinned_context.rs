//! The pinned session context block.
//!
//! Project instructions, recalled memories, the active goal, the plan and the task list used to be
//! ordinary transcript messages injected once. Compaction folded them into the summary and nothing
//! put them back, so a long session ran without its own standing instructions. The block is built
//! fresh for every request and placed right after the system prompt; it is never stored, so it can
//! be neither folded nor duplicated, and it changes only when one of its inputs does.
//!
//! The task list is a snapshot taken at the start of a turn and after a compaction, not live: a
//! task update that rewrote the block would invalidate the provider's prompt cache for the whole
//! conversation behind it on every `update_tasks` call. `update_tasks` returns the full list, so
//! the newer state is in the conversation where the model made it.

use forge_types::{Message, MessageKind, PlanProposal, Role, TodoItem, TodoStatus};

use super::*;

/// Opens the memory section. Sessions stored before the block existed carry this text as a system
/// message; while they still do, the block leaves the section out.
pub(crate) const MEMORY_HEADER: &str = "Remembered from earlier sessions in this project (durable \
facts — use them, and don't re-ask what's already settled):\n";

const BLOCK_HEADER: &str = "[session context]\nStanding context for this session. The harness \
re-sends it with every request and keeps it current, so it is not repeated in the conversation \
and it survives compaction.";

/// Most recalled memories the block carries.
pub(crate) const RECALL_LIMIT: usize = 6;

/// The scope `forge memory add --global` writes to.
const GLOBAL_MEMORY_SCOPE: &str = "global";

/// Merge the two recalls into at most `limit` memories. Global memories get up to half the slots
/// so a long project list cannot crowd them out; slots a side does not need go to the other.
pub(crate) fn merge_recalled(
    project: Vec<forge_store::Memory>,
    global: Vec<forge_store::Memory>,
    limit: usize,
) -> Vec<forge_store::Memory> {
    let global: Vec<_> = global
        .into_iter()
        .filter(|g| project.iter().all(|p| p.id != g.id))
        .collect();
    let global_slots = global.len().min((limit / 2).max(1));
    let project_slots = project.len().min(limit - global_slots.min(limit));
    let global_slots = global.len().min(limit - project_slots);
    let mut merged: Vec<forge_store::Memory> = project.into_iter().take(project_slots).collect();
    merged.extend(global.into_iter().take(global_slots));
    merged
}

/// Tasks listed in the block; the rest are counted.
const LISTED_TASKS: usize = 60;

/// What the block carries besides the project instructions (those live in
/// `Session::cached_agents_md`, which the per-turn fingerprint check keeps current).
#[derive(Default)]
pub(crate) struct PinnedState {
    /// The memory block, rendered, from the last recall.
    pub(crate) memories: Option<String>,
    /// The task list as it stood at the start of the turn or at the last compaction.
    pub(crate) tasks: Option<String>,
    pub(crate) goal: Option<String>,
    pub(crate) plan: Option<String>,
    /// A recall is owed: the session is new or was resumed. The next turn runs it with its prompt.
    pub(crate) recall_due: bool,
    /// A recall already told the person it found memories; later refreshes stay quiet.
    pub(crate) recall_announced: bool,
    /// Hashes of the project-instruction and memory text last reported to the context pack.
    reported: (u64, u64),
    /// The project instructions are already in the project's `CLAUDE.md`, which a bridged claude
    /// loads itself.
    pub(crate) claude_md_covers_agents: bool,
}

fn status_word(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Done => "done",
        TodoStatus::InProgress => "in progress",
        TodoStatus::Pending => "pending",
    }
}

/// The task list as the model reads it, one task per line.
pub fn render_task_list(tasks: &[TodoItem]) -> String {
    let mut out = tasks
        .iter()
        .take(LISTED_TASKS)
        .map(|t| {
            let owner = t
                .assignee
                .as_deref()
                .map(|a| format!(" (assigned to {a})"))
                .unwrap_or_default();
            format!("- [{}] {}{owner}", status_word(t.status), t.title)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let more = tasks.len().saturating_sub(LISTED_TASKS);
    if more > 0 {
        out.push_str(&format!("\n- (and {more} more)"));
    }
    out
}

pub(crate) fn render_plan(plan: &PlanProposal) -> String {
    let mut out = format!("{}\n", plan.title.trim());
    for (i, step) in plan.steps.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, step.title.trim()));
        let detail = step.detail.trim();
        if !detail.is_empty() {
            out.push_str(&format!("   - {detail}\n"));
        }
    }
    if let Some(notes) = plan
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        out.push_str(&format!("Notes: {notes}\n"));
    }
    out.trim_end().to_string()
}

pub(crate) fn render_memories(memories: &[forge_store::Memory]) -> String {
    let mut block = String::from(MEMORY_HEADER);
    for m in memories {
        block.push_str(&format!("- [{}] {}\n", m.kind, m.text));
    }
    block
}

fn hash(text: Option<&str>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

impl Session {
    /// Replace the project instructions the block carries. Whether a bridged claude already loads
    /// them from its own `CLAUDE.md` is settled here, once, not on every request.
    pub(crate) fn set_agents_md(&mut self, body: Option<String>) {
        self.pinned.claude_md_covers_agents = body
            .as_deref()
            .is_some_and(|body| lean_context::claude_md_covers(self.workspace.root(), body));
        self.cached_agents_md = body;
    }

    /// Tell the block which goal the session is working on (`None` clears it).
    pub fn set_active_goal(&mut self, goal: Option<&str>) {
        self.pinned.goal = goal
            .map(str::trim)
            .filter(|goal| !goal.is_empty())
            .map(str::to_string);
    }

    /// Carry the task list into the block as it stands now.
    pub(crate) fn refresh_pinned_tasks(&mut self) {
        self.pinned.tasks = (!self.tasks.is_empty()).then(|| render_task_list(&self.tasks));
    }

    /// An exact system message the transcript already carries: the copy a session stored before
    /// the block existed, which the block must not repeat.
    fn transcript_carries(&self, matches: impl Fn(&str) -> bool) -> bool {
        self.transcript
            .iter()
            .any(|m| m.role == Role::System && matches(&m.content))
    }

    /// The block for a request to `model` (`None` when it only has to be sized): `None` when
    /// nothing is pinned.
    pub(crate) fn pinned_context_message(&self, model: Option<&str>) -> Option<Message> {
        let mut sections: Vec<String> = Vec::new();

        let bridged_claude = model.is_some_and(|m| m.starts_with("claude-cli::"));
        if let Some(body) = self
            .cached_agents_md
            .as_deref()
            .filter(|body| !body.trim().is_empty())
            .filter(|_| !(bridged_claude && self.pinned.claude_md_covers_agents))
            .filter(|body| !self.transcript_carries(|text| text == *body))
        {
            sections.push(format!("## Project instructions\n{}", body.trim_end()));
        }
        if let Some(memories) = self
            .pinned
            .memories
            .as_deref()
            .filter(|_| !self.transcript_carries(|text| text.starts_with(MEMORY_HEADER)))
        {
            sections.push(format!("## Memory\n{}", memories.trim_end()));
        }
        if let Some(goal) = &self.pinned.goal {
            sections.push(format!("## Active goal\n{goal}"));
        }
        if let Some(plan) = &self.pinned.plan {
            sections.push(format!("## Plan\n{plan}"));
        }
        if let Some(tasks) = &self.pinned.tasks {
            sections.push(format!(
                "## Task list (as of the start of this turn; an update_tasks result later in the \
                 conversation is newer)\n{tasks}"
            ));
        }
        if sections.is_empty() {
            return None;
        }
        Some(
            Message::system(format!("{BLOCK_HEADER}\n\n{}", sections.join("\n\n")))
                .with_kind(MessageKind::Pinned),
        )
    }

    /// Tokens the block adds to every request.
    pub(crate) fn pinned_tokens(&self) -> usize {
        self.pinned_context_message(None)
            .map_or(0, |m| context_pipeline::message_tokens(&m))
    }

    /// Record in the turn's context pack the project instructions and memories that entered the
    /// block since it last said so.
    pub(crate) fn audit_pinned_context(&mut self, pack: &mut context_pack::ContextPack) {
        let agents = self
            .cached_agents_md
            .clone()
            .filter(|body| !body.trim().is_empty());
        let agents_hash = hash(agents.as_deref());
        if agents_hash != self.pinned.reported.0 {
            self.pinned.reported.0 = agents_hash;
            if let Some(body) = agents {
                pack.push(
                    context_pack::ContextSource::ProjectInstructions,
                    "project instructions (pinned session context)",
                    &body,
                );
            }
        }
        let memories_hash = hash(self.pinned.memories.as_deref());
        if memories_hash != self.pinned.reported.1 {
            self.pinned.reported.1 = memories_hash;
            if let Some(block) = &self.pinned.memories {
                pack.push(
                    context_pack::ContextSource::Memory,
                    "relevant durable project memories (pinned session context)",
                    block,
                );
            }
        }
    }

    /// What a recall searches for: the goal and the person's latest request.
    pub(crate) fn recall_query(&self, request: &str) -> String {
        match &self.pinned.goal {
            Some(goal) => format!("{goal}\n{request}"),
            None => request.to_string(),
        }
    }

    /// The person's latest request still in the transcript, for a recall that has no new prompt.
    pub(crate) fn latest_user_request(&self) -> String {
        self.transcript
            .iter()
            .rev()
            .find(|m| m.is_real_user() && !m.content.trim().is_empty())
            .map(|m| m.content.clone())
            .unwrap_or_default()
    }

    /// Recall the memories most relevant to `query` and put them in the block, replacing what an
    /// earlier recall found. Auto-memory off, or a store error, leaves the block as it was.
    pub(crate) async fn recall_pinned_memories(&mut self, query: &str) {
        if !self.config.mesh.auto_memory {
            return;
        }
        memory_scope::align_memory_scopes(&self.store, self.workspace.root());
        let scope = memory_scope_at(self.workspace.root());
        let embedding = embed_one(&self.config.lattice.embeddings, query).await;
        let recall = |scope: &str, limit: usize| match embedding.as_deref() {
            Some(embedding) => self.store.recall_semantic(scope, embedding, limit),
            None => self.store.recall_memories(scope, query, limit),
        };
        let Ok(project) = recall(&scope, RECALL_LIMIT) else {
            return;
        };
        // `forge memory add --global` writes to the `global` scope: recall it alongside the
        // project's. A failed global read must not hide the project memories.
        let global = recall(GLOBAL_MEMORY_SCOPE, RECALL_LIMIT).unwrap_or_default();
        let memories = merge_recalled(project, global, RECALL_LIMIT);
        if memories.is_empty() {
            self.pinned.memories = None;
            return;
        }
        self.pinned.memories = Some(render_memories(&memories));
        if !self.pinned.recall_announced {
            self.pinned.recall_announced = true;
            self.presenter.emit(PresenterEvent::Warning(format!(
                "💭 recalled {} memories from past sessions",
                memories.len()
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str, status: TodoStatus) -> TodoItem {
        TodoItem {
            title: title.to_string(),
            status,
            assignee: None,
        }
    }

    #[test]
    fn the_task_list_renders_every_task_with_its_status() {
        let tasks = vec![
            task("write the parser", TodoStatus::Done),
            task("wire it in", TodoStatus::InProgress),
            TodoItem {
                assignee: Some("worker-2".into()),
                ..task("document it", TodoStatus::Pending)
            },
        ];
        assert_eq!(
            render_task_list(&tasks),
            "- [done] write the parser\n- [in progress] wire it in\n- [pending] document it \
             (assigned to worker-2)"
        );
    }

    #[test]
    fn a_long_task_list_is_cut_with_a_count() {
        let tasks: Vec<TodoItem> = (0..LISTED_TASKS + 5)
            .map(|i| task(&format!("t{i}"), TodoStatus::Pending))
            .collect();
        let text = render_task_list(&tasks);
        assert!(text.ends_with("- (and 5 more)"), "{text}");
    }
}
