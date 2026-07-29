# Code archaeology: session catalog and transcript read models

## Summary

Three read-oriented store boundaries are now explicit: the session catalog, transcript reconstruction/history, and source-edit provenance. This replaces one broad extraction with cohesive owners while preserving resume affinity, soft-deletion semantics, compaction replay, tool-history ordering, and literal suffix matching.

## History and invariants

- `b3549011` introduced session listing and resume.
- `b7486d19` added continue/resume selection and excluded subagent children.
- `d44c21dc` changed selection and listing to most-recent message activity rather than creation time.
- `6c63abf1` excluded eager sessions with no real user message; `ce225d1d` retained sessions whose user message was later soft-deactivated.
- `bf19c4c6` added archived-session browsing and explicit unarchive-on-resume.
- `70ecdb87` blocked unarchive while an Anywhere handoff freezes the session.

## Boundaries

`session_catalog_store.rs` owns MRU discovery, archive state, worktree/title metadata, literal prefix resolution, and existence/count queries. `transcript_read_store.rs` owns model-context reconstruction, user-facing history pagination, compaction/uncompaction, and replay. `provenance_store.rs` owns edit attribution and nearest-turn context for `forge blame`.

Transcript insertion and sequencing remain with write lifecycle owners. Compaction remains transactional with its sync-journal revision; uncompaction only reactivates rows marked `compacted = 1`, never rows removed by undo.

## Interface as test surface

Catalog behavior is characterized by `list_sessions_newest_first_with_preview_and_count`, child/empty/assistant-only filters, soft-deleted-user retention, archive/resume tests, `source_handoff_freeze_survives_archive_controls_and_transfer`, literal-prefix tests, and the three `most_recent_session_id_*` tests.

Transcript behavior is characterized by active/all-message loading, `history_tool_result_finds_its_matching_interleaved_carrier`, the history page/epoch/tool-call ordering tests, compaction/uncompaction, and replay tests. Provenance is characterized by the two `file_edits_*` tests and `turn_context_finds_nearest_user_prompt_and_the_assistant_reply`.

## Leave alone

- `most_recent_session_id_skips_blank_and_archived_sessions` keeps implicit continue aligned with the real resumable catalog.
- A session counts as used if it ever received a user message, even after rewind.
- Archived sessions remain durable; Anywhere-frozen sessions cannot be unarchived.
- Model reconstruction excludes inactive rows and prepends the compaction summary; user history retains inactive rows.
- Tool-call carriers expand in persisted declaration/order semantics without inventing data.
- Uncompaction never resurrects undo-deactivated rows.
- Prefix and file-suffix matching escape SQL wildcard characters.
