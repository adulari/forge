//! The rewind menu's actions on a chosen message: restore the code and the conversation, only the
//! conversation, only the code, or fold everything from that message on into a summary.
//!
//! Every action speaks the DB's `message.seq`, the identity checkpoints are keyed by
//! (docs/features/rewind.md, "One seq space").

use forge_types::MessageKind;

use super::*;
use crate::compaction_budget::CompactionBudget;
use crate::compaction_input::fold_inputs;
use crate::compaction_summary::focus_directive;

/// Heads the system message that takes the place of a "summarize from here" range.
pub const SUMMARIZED_FROM_HERE: &str =
    "[The conversation from this point on was summarized to save context]";

/// Heads the note the model gets after a code-only restore.
const FILES_PUT_BACK: &str = "[files restored by the user]";

/// What a rewind restores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewindScope {
    /// The conversation and the files its turns wrote: `/undo` and the menu's first choice.
    All,
    /// The conversation only. Files stay as they are.
    Conversation,
    /// The files only. The conversation is kept, and the model is told which files went back.
    Code,
}

/// Result of [`Session::summarize_from`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SummarizeOutcome {
    /// Messages folded into the summary. Zero when there was nothing at or after the message.
    pub folded: usize,
    /// The chosen message's prompt, which the UI puts back in the input box.
    pub rewound_prompt: Option<String>,
}

fn stored_to_message(m: forge_store::StoredMessage) -> Message {
    Message {
        reasoning: None,
        provider_items: m.provider_items,
        role: m.role,
        content: m.content,
        tool_calls: m.tool_calls,
        tool_call_id: m.tool_call_id,
        images: Vec::new(),
        visibility: m.visibility,
        kind: m.kind,
    }
}

impl Session {
    /// Rewind to the message at `db_seq` with `scope`. [`RewindScope::All`] and
    /// [`RewindScope::Conversation`] soft-delete the messages at and after it, reload the
    /// transcript from the store and hand back the message's prompt; `All` also restores the files
    /// those turns wrote. [`RewindScope::Code`] only restores the files.
    pub fn rewind_scoped(
        &mut self,
        db_seq: i64,
        scope: RewindScope,
    ) -> Result<RewindOutcome, CoreError> {
        let db_seq = db_seq.max(0);
        if scope == RewindScope::Code {
            let restore = self.restore_turn_files_from(db_seq);
            self.lsp_baselines.clear();
            self.tell_model_files_restored(db_seq, &restore)?;
            return Ok(RewindOutcome {
                restore,
                rewound_prompt: None,
            });
        }
        self.unfold_compaction_for(db_seq)?;
        // The message AT the boundary is the user prompt of the rewound-to turn; read it before
        // deactivation so the UI can re-offer it for editing/resubmitting.
        let rewound_prompt = self.store.user_prompt_at(&self.id, db_seq)?;
        let restore = match scope {
            RewindScope::All => self.restore_turn_files_from(db_seq),
            _ => snapshot::RestoreReport::default(),
        };
        self.store.deactivate_messages_from(&self.id, db_seq)?;
        // The model-facing transcript carries no DB seqs, and the DB seq space runs ahead of it
        // (usage anchor rows, UI notes, compaction), so truncating by index could not be right.
        // Rebuild it from the store instead: exactly what a fresh resume of this session sees.
        self.reload_transcript_from_store()?;
        self.after_history_rewrite();
        Ok(RewindOutcome {
            restore,
            rewound_prompt,
        })
    }

    /// Whether a turn at or after `db_seq` left file snapshots that a code restore would put back.
    pub fn has_file_changes_from(&self, db_seq: i64) -> bool {
        !snapshot::turn_seqs_from(&self.checkpoint_root, &self.id, db_seq.max(0)).is_empty()
    }

    /// The prompt of the active user message at `db_seq`.
    pub fn prompt_at(&self, db_seq: i64) -> Option<String> {
        self.store.user_prompt_at(&self.id, db_seq).ok().flatten()
    }

    /// Fold the message at `db_seq` and everything after it into one summary, keeping the earlier
    /// conversation as it is and leaving files alone. The folded rows stay in the store (inactive)
    /// so scrollback and replay still have them. `focus` steers the summary like `/compact
    /// <focus>`.
    pub async fn summarize_from(
        &mut self,
        db_seq: i64,
        focus: Option<&str>,
    ) -> Result<SummarizeOutcome, CoreError> {
        let db_seq = db_seq.max(0);
        self.unfold_compaction_for(db_seq)?;
        let messages: Vec<Message> = self
            .store
            .load_active_messages_from(&self.id, db_seq)?
            .into_iter()
            .map(stored_to_message)
            .collect();
        if messages.is_empty() {
            return Ok(SummarizeOutcome::default());
        }
        let rewound_prompt = self.store.user_prompt_at(&self.id, db_seq)?;
        let before = self.transcript.len();
        self.presenter
            .emit(PresenterEvent::CompactionStarted { auto: false });
        self.fire_lifecycle(
            forge_config::HookEvent::PreCompact,
            serde_json::json!({ "trigger": "manual" }),
        )
        .await;
        let budget = CompactionBudget::for_trigger(
            self.effective_compaction_trigger(None),
            self.pinned_tokens(),
        );
        // The rows come from the store, so tool results are already the full stored text.
        let folded = fold_inputs(&messages, &[]);
        let directive = focus
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(focus_directive);
        let (model, summary) = match self.fold_summary(&folded, directive, &budget).await {
            Ok(written) => written,
            Err(error) => {
                self.abandon_compaction(before);
                return Err(error);
            }
        };
        self.store.deactivate_messages_from(&self.id, db_seq)?;
        self.reload_transcript_from_store()?;
        self.push_note(
            MessageKind::Normal,
            format!("{SUMMARIZED_FROM_HERE}\n{summary}"),
        )?;
        self.after_history_rewrite();
        self.bridge_context_tokens = 0;
        self.refresh_gauge_after_history_change();
        let after = self.transcript.len();
        self.presenter
            .emit(PresenterEvent::CompactionFinished { before, after });
        self.presenter.emit(PresenterEvent::Warning(format!(
            "summarized {} messages from that point into one (summary via {model})",
            messages.len()
        )));
        self.fire_lifecycle(
            forge_config::HookEvent::PostCompact,
            serde_json::json!({ "before": before, "after": after }),
        )
        .await;
        Ok(SummarizeOutcome {
            folded: messages.len(),
            rewound_prompt,
        })
    }

    /// A compacted session's in-memory transcript is `[summary, recent tail]`. A target inside
    /// the folded-away history reactivates it first: acting on the tail alone would leave the
    /// model with nothing while the store kept the originals soft-deleted as "compacted".
    fn unfold_compaction_for(&mut self, db_seq: i64) -> Result<(), CoreError> {
        if self.was_compacted() {
            let first_active = self.store.min_active_seq(&self.id)?.unwrap_or(0);
            if db_seq <= first_active {
                self.store.uncompact_session_store(&self.id)?;
            }
        }
        Ok(())
    }

    /// Restore every snapshotted turn at or after `db_seq`, newest first so an earlier turn's
    /// blob (its pre-turn bytes) wins the final state. Turns are keyed by their user-message seq.
    fn restore_turn_files_from(&self, db_seq: i64) -> snapshot::RestoreReport {
        let mut restore = snapshot::RestoreReport::default();
        for seq in snapshot::turn_seqs_from(&self.checkpoint_root, &self.id, db_seq) {
            match snapshot::restore_turn(&self.checkpoint_root, &self.id, seq) {
                Ok(r) => {
                    restore.restored.extend(r.restored);
                    restore.warnings.extend(r.warnings);
                    restore.failed.extend(r.failed);
                }
                // Surface the failure instead of silently discarding it: a caller that only
                // checks `restore.restored` must still learn this turn's files may not have been
                // reverted.
                Err(e) => restore.failed.push(format!("turn {seq}: {e}")),
            }
        }
        restore
    }

    /// After a code-only restore the conversation still describes edits that are gone from disk.
    /// Say which files went back, so the model reads them again instead of editing from memory.
    fn tell_model_files_restored(
        &mut self,
        db_seq: i64,
        report: &snapshot::RestoreReport,
    ) -> Result<(), CoreError> {
        if report.restored.is_empty() {
            return Ok(());
        }
        let root = self.workspace.root().to_path_buf();
        let files: Vec<String> = report
            .restored
            .iter()
            .map(|p| {
                let path = std::path::Path::new(p);
                path.strip_prefix(&root)
                    .unwrap_or(path)
                    .display()
                    .to_string()
            })
            .collect();
        let anchor = match self.prompt_at(db_seq) {
            Some(prompt) => format!(
                "before the message \"{}\"",
                forge_types::truncate_ellipsis(prompt.trim(), 80)
            ),
            None => "to an earlier point".to_string(),
        };
        self.push_note(
            MessageKind::Normal,
            format!(
                "{FILES_PUT_BACK} The user put these files back the way they were {anchor}: {}. \
                 The conversation was kept, so earlier messages may describe edits that are no \
                 longer on disk. Read a file again before editing it.",
                files.join(", ")
            ),
        )
    }
}
