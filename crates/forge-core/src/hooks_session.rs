//! Session lifecycle hooks with Claude Code's sources and reasons (docs/features/hooks.md):
//! `SessionStart` says how the session began (`startup`, `resume`, `clear`, `compact`) and its
//! matcher filters on that, `SessionEnd` says why it ended, and `InstructionsLoaded` reports the
//! instruction file the session put in front of the model. Surfaces call these instead of running
//! the hook lists themselves, so every surface reports the same source.

use std::path::PathBuf;

use forge_config::{HookConfig, HookEvent};
use forge_types::Role;

use crate::hooks_events::{run_event_hooks, EventMatch};
use crate::{CoreError, Session};

/// How a session began, for `SessionStart`'s `source` and its matcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSource {
    /// A new conversation.
    Startup,
    /// A stored conversation reopened (`--resume`, `--continue`, the sessions picker).
    Resume,
    /// `/new`: the previous conversation was set aside for an empty one.
    Clear,
    /// The context was compacted.
    Compact,
}

impl SessionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionSource::Startup => "startup",
            SessionSource::Resume => "resume",
            SessionSource::Clear => "clear",
            SessionSource::Compact => "compact",
        }
    }
}

/// What `SessionStart` needs, captured from a session so a surface holding the session behind a
/// mutex can run the hooks without keeping the lock.
#[derive(Debug, Clone)]
pub struct SessionStartPlan {
    hooks: Vec<HookConfig>,
    session_id: String,
    workspace: PathBuf,
    source: SessionSource,
    /// The instruction files in context, with Claude Code's `memory_type`.
    instruction_files: Vec<(PathBuf, &'static str)>,
}

impl SessionStartPlan {
    /// Fire `SessionStart` (and `InstructionsLoaded` for each instruction file) and return the
    /// context the hooks asked to put in front of the model.
    pub async fn run(&self) -> Vec<String> {
        if self.hooks.is_empty() {
            return Vec::new();
        }
        let context = crate::hooks::run_session_hooks_with(
            &self.hooks,
            HookEvent::SessionStart,
            &self.session_id,
            Some(&self.workspace),
            self.source.as_str(),
        )
        .await;
        let reason = match self.source {
            SessionSource::Compact => "compact",
            _ => "session_start",
        };
        for (file, memory_type) in &self.instruction_files {
            // Observe-only, as in Claude Code: the output is discarded.
            run_event_hooks(
                &self.hooks,
                HookEvent::InstructionsLoaded,
                &self.session_id,
                EventMatch::Value(reason),
                serde_json::json!({
                    "file_path": file.display().to_string(),
                    "memory_type": memory_type,
                    "load_reason": reason,
                    "cwd": self.workspace.display().to_string(),
                }),
            )
            .await;
        }
        context
    }
}

/// Fire `SessionStart` for a session a surface shares behind a mutex: the lock is held only to
/// read the session and to add the hooks' context, never while the hooks run. `None` picks
/// `startup` or `resume` from whether the session already holds a conversation.
pub async fn fire_session_start_shared(
    session: &tokio::sync::Mutex<Session>,
    source: Option<SessionSource>,
) {
    let plan = session.lock().await.session_start_plan(source);
    let context = plan.run().await;
    if !context.is_empty() {
        let mut session = session.lock().await;
        for text in &context {
            session.inject_hook_context(text);
        }
    }
}

impl Session {
    /// `Resume` when the session already holds a conversation, `Startup` otherwise.
    pub fn session_start_source(&self) -> SessionSource {
        let resumed = self
            .transcript
            .iter()
            .any(|m| matches!(m.role, Role::User | Role::Assistant));
        if resumed {
            SessionSource::Resume
        } else {
            SessionSource::Startup
        }
    }

    pub fn session_start_plan(&self, source: Option<SessionSource>) -> SessionStartPlan {
        SessionStartPlan {
            hooks: self.config.hooks.clone(),
            session_id: self.id.clone(),
            workspace: self.workspace.root().to_path_buf(),
            source: source.unwrap_or_else(|| self.session_start_source()),
            instruction_files: self.loaded_instruction_files(),
        }
    }

    /// The instruction files the session carries, each with Claude Code's `memory_type`.
    fn loaded_instruction_files(&self) -> Vec<(PathBuf, &'static str)> {
        use crate::instructions::Origin;
        if !self.has_hooks_for(HookEvent::InstructionsLoaded) {
            return Vec::new();
        }
        let root = self.workspace.root();
        self.instructions
            .files
            .iter()
            .filter(|file| !file.body.trim().is_empty())
            .map(|file| {
                let scope = match file.origin {
                    Origin::User => "User",
                    Origin::Local => "Local",
                    Origin::Project => "Project",
                    Origin::Rule if file.path.starts_with(root) => "Project",
                    Origin::Rule => "User",
                };
                (file.path.clone(), scope)
            })
            .collect()
    }

    /// Fire `SessionStart` and put what the hooks returned in front of the model.
    pub async fn fire_session_start(&mut self, source: Option<SessionSource>) {
        let context = self.session_start_plan(source).run().await;
        for text in &context {
            self.inject_hook_context(text);
        }
    }

    /// Fire `SessionEnd` with Claude Code's `reason` (`clear`, `resume`, `prompt_input_exit`,
    /// `other` …). Observe-only.
    pub async fn fire_session_end(&mut self, reason: &str) {
        if !self.has_hooks_for(HookEvent::SessionEnd) {
            return;
        }
        let workspace = self.workspace.root().to_path_buf();
        crate::hooks::run_session_hooks_with(
            &self.config.hooks,
            HookEvent::SessionEnd,
            &self.id,
            Some(&workspace),
            reason,
        )
        .await;
    }

    /// `UserPromptSubmit` for `prompt`, with this session's model hooks available: the prompt to
    /// run (rewritten or extended by the hooks), or why a hook blocked it.
    pub async fn run_prompt_submit_hooks(&mut self, prompt: &str) -> Result<String, String> {
        if !self.has_hooks_for(HookEvent::UserPromptSubmit) {
            return Ok(prompt.to_string());
        }
        let runner = self.llm_hook_runner().await;
        let workspace = self.workspace.root().to_path_buf();
        crate::hooks_llm::with_runner(
            runner,
            crate::hooks::run_prompt_hooks_for(
                &self.config.hooks,
                prompt,
                Some(&workspace),
                &self.id,
            ),
        )
        .await
    }

    /// `UserPromptExpansion` for a typed `/command` or skill that expanded into a prompt (the
    /// matcher filters `command_name`; `command_source` is `user`, `project` or `builtin`). `Ok`
    /// carries context to add next to the expanded prompt; `Err` why a hook blocked it.
    pub async fn run_expansion_hooks(
        &mut self,
        command_name: &str,
        command_args: &str,
        command_source: &str,
        prompt: &str,
    ) -> Result<Vec<String>, String> {
        if !self.has_hooks_for(HookEvent::UserPromptExpansion) {
            return Ok(Vec::new());
        }
        let runner = self.llm_hook_runner().await;
        let out = crate::hooks_llm::with_runner(
            runner,
            run_event_hooks(
                &self.config.hooks,
                HookEvent::UserPromptExpansion,
                &self.id,
                EventMatch::Value(command_name),
                serde_json::json!({
                    "expansion_type": "slash_command",
                    "command_name": command_name,
                    "command_args": command_args,
                    "command_source": command_source,
                    "prompt": prompt,
                    "cwd": self.workspace.display(),
                    "permission_mode": self.mode.key(),
                }),
            ),
        )
        .await;
        for note in out.notes {
            self.presenter
                .emit(crate::PresenterEvent::Warning(format!("⎇ hook: {note}")));
        }
        match out.blocked.or(out.halt) {
            Some(reason) => Err(reason),
            None => Ok(out.context),
        }
    }

    /// `/new` with its hooks: `SessionEnd` (`clear`) for the conversation being left, then
    /// `SessionStart` (`clear`) for the empty one.
    pub async fn reset_fresh_hooked(&mut self, cwd: &str) -> Result<(), CoreError> {
        self.fire_session_end("clear").await;
        self.reset_fresh(cwd)?;
        self.fire_session_start(Some(SessionSource::Clear)).await;
        Ok(())
    }

    /// `/resume` with its hooks: `SessionEnd` (`resume`) for the conversation being left, then
    /// `SessionStart` (`resume`) for the reopened one.
    pub async fn reset_resumed_hooked(&mut self, session_id: &str) -> Result<(), CoreError> {
        if self.store.session_exists(session_id)? {
            self.fire_session_end("resume").await;
        }
        self.reset_resumed(session_id)?;
        self.fire_session_start(Some(SessionSource::Resume)).await;
        Ok(())
    }
}
