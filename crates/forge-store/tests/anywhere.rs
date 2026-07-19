use forge_store::{ImportedSessionMetadata, RemoteSyncRecord, Store, SyncJournalOperation};
use forge_types::{Role, TaskTier, Usage};
use sha2::{Digest, Sha256};

fn remote_memory(
    cursor: i64,
    sender_device_id: [u8; 16],
    stable_id: &str,
    revision: u64,
    logical_clock: u64,
    text: &str,
) -> RemoteSyncRecord {
    let payload = serde_json::to_vec(&serde_json::json!({
        "id": stable_id,
        "scope": "project",
        "kind": "fact",
        "text": text,
        "source_session": "remote-session",
        "created_at": 10,
        "updated_at": 20,
        "salience": 0.75
    }))
    .expect("memory payload");
    RemoteSyncRecord {
        cursor,
        sender_device_id,
        record_kind: "memory".into(),
        stable_id: stable_id.into(),
        operation: "upsert".into(),
        revision,
        logical_clock,
        base_hash: None,
        content_hash: Sha256::digest(&payload).into(),
        payload,
    }
}

fn remote_history(
    cursor: i64,
    record_kind: &str,
    stable_id: &str,
    logical_clock: u64,
    payload: serde_json::Value,
) -> RemoteSyncRecord {
    let payload = serde_json::to_vec(&payload).expect("history payload");
    RemoteSyncRecord {
        cursor,
        sender_device_id: [0x70; 16],
        record_kind: record_kind.into(),
        stable_id: stable_id.into(),
        operation: "upsert".into(),
        revision: logical_clock,
        logical_clock,
        base_hash: None,
        content_hash: Sha256::digest(&payload).into(),
        payload,
    }
}

fn remote_portable(
    cursor: i64,
    sender_device_id: [u8; 16],
    record_kind: &str,
    stable_id: &str,
    logical_clock: u64,
    payload: &[u8],
) -> RemoteSyncRecord {
    RemoteSyncRecord {
        cursor,
        sender_device_id,
        record_kind: record_kind.into(),
        stable_id: stable_id.into(),
        operation: if payload.is_empty() {
            "tombstone"
        } else {
            "upsert"
        }
        .into(),
        revision: logical_clock,
        logical_clock,
        base_hash: None,
        content_hash: Sha256::digest(payload).into(),
        payload: payload.to_vec(),
    }
}

fn remote_file(
    cursor: i64,
    stable_id: &str,
    logical_clock: u64,
    base_hash: Option<[u8; 32]>,
    payload: &[u8],
) -> RemoteSyncRecord {
    RemoteSyncRecord {
        cursor,
        sender_device_id: [0x70; 16],
        record_kind: "file".into(),
        stable_id: stable_id.into(),
        operation: if payload.is_empty() {
            "tombstone"
        } else {
            "upsert"
        }
        .into(),
        revision: logical_clock,
        logical_clock,
        base_hash,
        content_hash: Sha256::digest(payload).into(),
        payload: payload.to_vec(),
    }
}

#[test]
fn sync_journal_is_idempotent_and_acknowledged_atomically() {
    let store = Store::open_in_memory().expect("open store");
    let payload = br#"{"id":"message-1"}"#;
    assert!(store
        .append_sync_journal(
            "message",
            "message-1",
            SyncJournalOperation::Upsert,
            1,
            7,
            payload,
        )
        .expect("append journal"));
    assert!(!store
        .append_sync_journal(
            "message",
            "message-1",
            SyncJournalOperation::Upsert,
            1,
            7,
            payload,
        )
        .expect("duplicate journal"));
    let pending = store.pending_sync_journal(10).expect("pending journal");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].stable_id, "message-1");
    assert_eq!(pending[0].payload, payload);
    assert_eq!(pending[0].content_hash, Sha256::digest(payload).as_slice());
    assert_eq!(
        store
            .mark_sync_journal_uploaded(&[pending[0].id], 123)
            .expect("mark uploaded"),
        1
    );
    assert!(store
        .pending_sync_journal(10)
        .expect("pending journal")
        .is_empty());
}

#[test]
fn sync_ciphertext_is_reused_until_the_journal_row_is_acknowledged() {
    let store = Store::open_in_memory().expect("open store");
    store
        .append_sync_journal(
            "message",
            "message-1",
            SyncJournalOperation::Upsert,
            1,
            1,
            b"plaintext",
        )
        .expect("append journal");
    let journal_id = store
        .pending_sync_journal(1)
        .expect("pending journal")
        .remove(0)
        .id;
    let first = store
        .store_sync_upload_envelope(journal_id, b"first ciphertext", [0x11; 32])
        .expect("store ciphertext");
    let raced = store
        .store_sync_upload_envelope(journal_id, b"different ciphertext", [0x22; 32])
        .expect("load winning ciphertext");
    assert_eq!(raced, first);
    assert_eq!(
        store
            .sync_upload_envelope(journal_id)
            .expect("load ciphertext"),
        Some(first)
    );
    store
        .mark_sync_journal_uploaded(&[journal_id], 123)
        .expect("acknowledge upload");
    assert!(store
        .sync_upload_envelope(journal_id)
        .expect("load cleared ciphertext")
        .is_none());
}

#[test]
fn remote_record_and_cursor_are_staged_atomically() {
    let store = Store::open_in_memory().expect("open store");
    let record = RemoteSyncRecord {
        cursor: 4,
        sender_device_id: [0x22; 16],
        record_kind: "message".into(),
        stable_id: "message-remote-1".into(),
        operation: "upsert".into(),
        revision: 1,
        logical_clock: 8,
        base_hash: None,
        content_hash: [0x33; 32],
        payload: b"remote plaintext cache".to_vec(),
    };
    assert!(store
        .stage_remote_sync_record(&record)
        .expect("stage remote record"));
    assert_eq!(store.sync_download_cursor().expect("cursor"), 4);
    assert!(!store
        .stage_remote_sync_record(&record)
        .expect("idempotent replay"));

    let mut conflict = record.clone();
    conflict.cursor = 5;
    conflict.payload = b"conflicting payload".to_vec();
    assert!(store.stage_remote_sync_record(&conflict).is_err());
    assert_eq!(
        store.sync_download_cursor().expect("cursor after conflict"),
        4,
        "a conflict must not advance the durable cursor"
    );
    store
        .advance_sync_download_cursor(9)
        .expect("advance past deleted ciphertext");
    assert_eq!(store.sync_download_cursor().expect("advanced cursor"), 9);
}

#[test]
fn staged_memory_uses_deterministic_lww_and_applies_tombstones() {
    let store = Store::open_in_memory().expect("open store");
    store
        .set_sync_journal_enabled(true)
        .expect("enable journal");
    let memory_id = store
        .add_memory_with_embedding(
            "project",
            "fact",
            "local winner",
            "local-session",
            &[0.25, 0.75],
        )
        .expect("add local memory");

    let tied = remote_memory(1, [0x70; 16], &memory_id, 1, 1, "remote tie loser");
    store
        .stage_remote_sync_record(&tied)
        .expect("stage tied record");
    let summary = store
        .apply_staged_memory_records([0x80; 16], 10)
        .expect("apply tied record");
    assert_eq!(summary.superseded, 1);
    assert_eq!(
        store.list_memories("project").expect("list memories")[0].text,
        "local winner"
    );

    let newer = remote_memory(2, [0x70; 16], &memory_id, 2, 2, "remote clock winner");
    store
        .stage_remote_sync_record(&newer)
        .expect("stage newer record");
    let summary = store
        .apply_staged_memory_records([0x80; 16], 10)
        .expect("apply newer record");
    assert_eq!(summary.applied, 1);
    assert_eq!(
        store.list_memories("project").expect("list memories")[0].text,
        "remote clock winner"
    );
    assert_eq!(
        store
            .recall_semantic("project", &[0.25, 0.75], 1)
            .expect("semantic recall")
            .len(),
        1,
        "remote metadata updates must preserve the device-local embedding"
    );

    let empty_hash: [u8; 32] = Sha256::digest([]).into();
    store
        .stage_remote_sync_record(&RemoteSyncRecord {
            cursor: 3,
            sender_device_id: [0x70; 16],
            record_kind: "memory".into(),
            stable_id: memory_id,
            operation: "tombstone".into(),
            revision: 3,
            logical_clock: 3,
            base_hash: None,
            content_hash: empty_hash,
            payload: Vec::new(),
        })
        .expect("stage tombstone");
    let summary = store
        .apply_staged_memory_records([0x80; 16], 10)
        .expect("apply tombstone");
    assert_eq!(summary.applied, 1);
    assert!(store
        .list_memories("project")
        .expect("list memories")
        .is_empty());
}

#[test]
fn staged_memory_conflicts_never_overwrite_untracked_local_data() {
    let store = Store::open_in_memory().expect("open store");
    let memory_id = store
        .add_memory("project", "fact", "pre-Anywhere local", "local-session")
        .expect("add untracked memory");
    let remote = remote_memory(1, [0x90; 16], &memory_id, 1, 99, "unsafe overwrite");
    store
        .stage_remote_sync_record(&remote)
        .expect("stage remote overwrite");
    let summary = store
        .apply_staged_memory_records([0x10; 16], 10)
        .expect("classify conflict");
    assert_eq!(summary.conflicts, 1);
    assert_eq!(
        store.list_memories("project").expect("list memories")[0].text,
        "pre-Anywhere local"
    );

    let mut malformed = remote_memory(2, [0x90; 16], "new-memory", 1, 1, "bad hash");
    malformed.content_hash = [0xff; 32];
    store
        .stage_remote_sync_record(&malformed)
        .expect("stage malformed record");
    let summary = store
        .apply_staged_memory_records([0x10; 16], 10)
        .expect("classify malformed record");
    assert_eq!(summary.conflicts, 1);
    assert_eq!(summary.applied, 0);
}

#[test]
fn staged_history_applies_in_dependency_order_without_replacing_host_paths() {
    let store = Store::open_in_memory().expect("open store");
    store
        .set_sync_journal_enabled(true)
        .expect("enable journal");
    let session_id = store
        .create_session("/safe/local/repo", "accept_edits")
        .expect("create local session");

    let records = [
        remote_history(
            1,
            "session",
            &session_id,
            2,
            serde_json::json!({
                "id": session_id,
                "title": "Remote title",
                "cwd": "/unsafe/remote/path",
                "permission_mode": "danger_full_access",
                "total_cost_usd": 999.0,
                "parent_session_id": null,
                "forked_from": null,
                "forked_at_seq": null,
                "worktree_path": "/unsafe/remote/worktree",
                "archived": true,
                "view_snapshot": "{\"selected\":1}"
            }),
        ),
        remote_history(
            2,
            "message",
            "remote-message",
            1,
            serde_json::json!({
                "id": "remote-message",
                "session_id": session_id,
                "seq": 0,
                "role": "user",
                "content": "arrived from another device",
                "model": null,
                "tool_calls": [],
                "tool_call_id": null,
                "visibility": "llm"
            }),
        ),
        remote_history(
            3,
            "checkpoint",
            "remote-checkpoint",
            1,
            serde_json::json!({
                "id": "remote-checkpoint",
                "session_id": session_id,
                "label": "remote idle",
                "seq": 1
            }),
        ),
        remote_history(
            4,
            "tool_call",
            "remote-tool",
            1,
            serde_json::json!({
                "id": "remote-tool",
                "message_id": "remote-message",
                "tool_name": "read_file",
                "args_json": "{\"path\":\"src/lib.rs\"}",
                "result_json": "ok",
                "permission": "allowed",
                "status": "complete",
                "path": "src/lib.rs"
            }),
        ),
        remote_history(
            5,
            "routing_decision",
            "remote-routing",
            1,
            serde_json::json!({
                "id": "remote-routing",
                "message_id": "remote-message",
                "task_tier": "standard",
                "chosen_model": "provider::model",
                "rationale": "remote route"
            }),
        ),
        remote_history(
            6,
            "usage",
            "remote-usage",
            1,
            serde_json::json!({
                "id": "remote-usage",
                "message_id": "remote-message",
                "input_tokens": 10,
                "output_tokens": 5,
                "cached_input_tokens": 2,
                "cost_usd": 0.25
            }),
        ),
    ];
    for record in &records {
        store
            .stage_remote_sync_record(record)
            .expect("stage history record");
    }
    let summary = store
        .apply_staged_history_records([0x80; 16], 20)
        .expect("apply history graph");
    assert_eq!(summary.applied, records.len());
    assert_eq!(summary.conflicts, 0);
    assert_eq!(summary.deferred, 0);
    assert_eq!(
        store
            .session_cwd(&session_id)
            .expect("session cwd")
            .as_deref(),
        Some("/safe/local/repo"),
        "remote history must not replace the host-local workspace"
    );
    assert_eq!(
        store.session_mode(&session_id).expect("session mode"),
        "accept_edits",
        "remote history must not replace the host-local permission mode"
    );
    assert_eq!(
        store
            .session_title(&session_id)
            .expect("session title")
            .as_deref(),
        Some("Remote title")
    );
    assert!(store.session_archived(&session_id).expect("archived"));
    assert_eq!(
        store.load_messages(&session_id).expect("messages")[0].content,
        "arrived from another device"
    );
    assert_eq!(
        store
            .list_checkpoints(&session_id)
            .expect("checkpoints")
            .len(),
        1
    );
    assert_eq!(store.session_cost(&session_id).expect("session cost"), 0.25);
}

#[test]
fn staged_history_defers_missing_parents_and_conflicts_on_sequence_collisions() {
    let store = Store::open_in_memory().expect("open store");
    let missing_parent = remote_history(
        1,
        "message",
        "orphan-message",
        1,
        serde_json::json!({
            "id": "orphan-message",
            "session_id": "missing-session",
            "seq": 0,
            "role": "user",
            "content": "wait for parent",
            "model": null,
            "tool_calls": [],
            "tool_call_id": null,
            "visibility": "llm"
        }),
    );
    store
        .stage_remote_sync_record(&missing_parent)
        .expect("stage orphan");
    let summary = store
        .apply_staged_history_records([0x80; 16], 10)
        .expect("defer orphan");
    assert_eq!(summary.deferred, 1);
    assert_eq!(summary.conflicts, 0);

    let session_id = store
        .create_session("/tmp/local", "accept_edits")
        .expect("create session");
    store
        .add_message(&session_id, 0, Role::User, "local sequence owner", None)
        .expect("add local message");
    let collision = remote_history(
        2,
        "message",
        "colliding-message",
        1,
        serde_json::json!({
            "id": "colliding-message",
            "session_id": session_id,
            "seq": 0,
            "role": "assistant",
            "content": "must not be remapped",
            "model": null,
            "tool_calls": [],
            "tool_call_id": null,
            "visibility": "llm"
        }),
    );
    store
        .stage_remote_sync_record(&collision)
        .expect("stage collision");
    let summary = store
        .apply_staged_history_records([0x80; 16], 10)
        .expect("classify collision");
    assert_eq!(summary.conflicts, 1);
    assert_eq!(summary.deferred, 1, "the earlier orphan remains retryable");
    let messages = store.load_messages(&session_id).expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "local sequence owner");
}

#[test]
fn staged_compaction_applies_and_rolls_back_without_resurrecting_unrelated_rows() {
    let store = Store::open_in_memory().expect("open store");
    store
        .set_sync_journal_enabled(true)
        .expect("enable journal");
    let session_id = store
        .create_session("/tmp/local", "accept_edits")
        .expect("create session");
    for seq in 0..4 {
        store
            .add_message(
                &session_id,
                seq,
                Role::User,
                &format!("message {seq}"),
                None,
            )
            .expect("add message");
    }
    let compaction = remote_history(
        1,
        "compaction",
        &session_id,
        1,
        serde_json::json!({
            "session_id": session_id,
            "summary": "remote summary",
            "keep_count": 2
        }),
    );
    store
        .stage_remote_sync_record(&compaction)
        .expect("stage compaction");
    let summary = store
        .apply_staged_history_records([0x80; 16], 10)
        .expect("apply compaction");
    assert_eq!(summary.applied, 1);
    assert!(store
        .session_has_compaction(&session_id)
        .expect("compaction"));
    assert_eq!(
        store.load_messages(&session_id).expect("active view").len(),
        3,
        "summary plus the two newest messages should remain active"
    );

    let empty_hash: [u8; 32] = Sha256::digest([]).into();
    store
        .stage_remote_sync_record(&RemoteSyncRecord {
            cursor: 2,
            sender_device_id: [0x70; 16],
            record_kind: "compaction".into(),
            stable_id: session_id.clone(),
            operation: "tombstone".into(),
            revision: 2,
            logical_clock: 2,
            base_hash: None,
            content_hash: empty_hash,
            payload: Vec::new(),
        })
        .expect("stage uncompact tombstone");
    let summary = store
        .apply_staged_history_records([0x80; 16], 10)
        .expect("apply uncompact tombstone");
    assert_eq!(summary.applied, 1);
    assert!(!store
        .session_has_compaction(&session_id)
        .expect("compaction"));
    assert_eq!(
        store.load_messages(&session_id).expect("active view").len(),
        4
    );
}

#[test]
fn imported_session_provenance_round_trips() {
    let store = Store::open_in_memory().expect("open store");
    let session_id = store
        .create_session("/tmp/repo", "accept_edits")
        .expect("create session");
    let metadata = ImportedSessionMetadata {
        session_id: session_id.clone(),
        source_session_id: "source-session".into(),
        source_device_id: [0x22; 16],
        capsule_id: "capsule-1".into(),
        base_commit: "a".repeat(40),
        worktree_path: "/tmp/worktree".into(),
        imported_at: 456,
    };
    store
        .record_imported_session(&metadata)
        .expect("record import metadata");
    assert_eq!(
        store
            .imported_session_metadata(&session_id)
            .expect("load import metadata"),
        Some(metadata)
    );
}

#[test]
fn core_store_writes_enqueue_complete_sync_snapshots() {
    let store = Store::open_in_memory().expect("open store");
    store
        .set_sync_journal_enabled(true)
        .expect("enable sync journal");
    let session_id = store
        .create_session("/tmp/repo", "accept_edits")
        .expect("create session");
    store
        .set_session_title(&session_id, "Anywhere test")
        .expect("set title");
    let message_id = store
        .add_message(
            &session_id,
            0,
            Role::User,
            "encrypted after leaving this host",
            Some("provider::model"),
        )
        .expect("add message");
    store
        .record_routing(
            &message_id,
            TaskTier::Standard,
            "provider::model",
            "fits the task",
        )
        .expect("record routing");
    store
        .record_usage(
            &session_id,
            &message_id,
            &Usage {
                input_tokens: 10,
                output_tokens: 5,
                cached_input_tokens: 2,
                cost_usd: 0.01,
            },
        )
        .expect("record usage");
    store
        .record_tool_call(
            &message_id,
            "read_file",
            r#"{"path":"src/lib.rs"}"#,
            "result",
            "allowed",
            "complete",
        )
        .expect("record tool call");
    store
        .add_checkpoint(&session_id, Some("idle"), 1)
        .expect("add checkpoint");
    store
        .compact_session_store(&session_id, "summary", 1)
        .expect("compact session");
    store
        .add_memory_with_embedding(
            "project",
            "fact",
            "Forge Anywhere payloads are encrypted",
            &session_id,
            &[0.25, 0.75],
        )
        .expect("add memory");

    let entries = store.pending_sync_journal(100).expect("pending journal");
    let kinds = entries
        .iter()
        .map(|entry| entry.record_kind.as_str())
        .collect::<std::collections::HashSet<_>>();
    for expected in [
        "session",
        "message",
        "routing_decision",
        "usage",
        "tool_call",
        "checkpoint",
        "compaction",
        "memory",
    ] {
        assert!(kinds.contains(expected), "missing {expected} journal entry");
    }
    for entry in &entries {
        assert_eq!(
            entry.content_hash,
            Sha256::digest(&entry.payload).as_slice(),
            "hash mismatch for {}:{}",
            entry.record_kind,
            entry.stable_id
        );
        serde_json::from_slice::<serde_json::Value>(&entry.payload)
            .expect("upsert payload is valid JSON");
    }
    let memory = entries
        .iter()
        .find(|entry| entry.record_kind == "memory")
        .expect("memory journal entry");
    assert!(
        !String::from_utf8_lossy(&memory.payload).contains("embedding"),
        "local embeddings must never enter the sync journal"
    );
}

#[test]
fn failed_store_write_does_not_leave_a_journal_row() {
    let store = Store::open_in_memory().expect("open store");
    assert!(store
        .add_message("missing-session", 0, Role::User, "nope", None)
        .is_err());
    assert!(store
        .pending_sync_journal(10)
        .expect("pending journal")
        .is_empty());
}

#[test]
fn entirely_remote_session_materializes_in_dependency_order_with_local_safety_fields() {
    let store = Store::open_in_memory().expect("open store");
    let message = remote_history(
        1,
        "message",
        "remote-only-message",
        1,
        serde_json::json!({
            "id": "remote-only-message",
            "session_id": "remote-only-session",
            "seq": 0,
            "role": "user",
            "content": "offline work",
            "model": null,
            "tool_calls": [],
            "tool_call_id": null,
            "visibility": "llm"
        }),
    );
    let session = remote_history(
        2,
        "session",
        "remote-only-session",
        2,
        serde_json::json!({
            "id": "remote-only-session",
            "title": "Remote only",
            "cwd": "/remote/must/not/win",
            "permission_mode": "danger_full_access",
            "archived": false,
            "view_snapshot": null
        }),
    );
    store
        .stage_remote_sync_record(&message)
        .expect("stage child first");
    store
        .stage_remote_sync_record(&session)
        .expect("stage parent second");

    let summary = store
        .apply_staged_history_records([0x80; 16], 10)
        .expect("apply remote graph");
    assert_eq!(summary.applied, 2);
    assert_eq!(
        store.session_mode("remote-only-session").unwrap(),
        "accept_edits"
    );
    assert_ne!(
        store.session_cwd("remote-only-session").unwrap().as_deref(),
        Some("/remote/must/not/win")
    );
    assert_eq!(
        store.load_messages("remote-only-session").unwrap()[0].content,
        "offline work"
    );
}

#[test]
fn remote_history_tombstones_are_idempotent_and_cascade_only_synced_rows() {
    let store = Store::open_in_memory().expect("open store");
    for record in [
        remote_history(
            1,
            "session",
            "delete-session",
            1,
            serde_json::json!({
                "id": "delete-session", "title": null, "archived": false,
                "view_snapshot": null
            }),
        ),
        remote_history(
            2,
            "message",
            "delete-message",
            1,
            serde_json::json!({
                "id": "delete-message", "session_id": "delete-session", "seq": 0,
                "role": "user", "content": "temporary", "model": null,
                "tool_calls": [], "tool_call_id": null, "visibility": "llm"
            }),
        ),
    ] {
        store.stage_remote_sync_record(&record).unwrap();
    }
    assert_eq!(
        store
            .apply_staged_history_records([0x80; 16], 10)
            .unwrap()
            .applied,
        2
    );
    let message_tombstone = RemoteSyncRecord {
        cursor: 3,
        sender_device_id: [0x70; 16],
        record_kind: "message".into(),
        stable_id: "delete-message".into(),
        operation: "tombstone".into(),
        revision: 2,
        logical_clock: 2,
        base_hash: None,
        content_hash: Sha256::digest([]).into(),
        payload: Vec::new(),
    };
    store.stage_remote_sync_record(&message_tombstone).unwrap();
    assert_eq!(
        store
            .apply_staged_history_records([0x80; 16], 10)
            .unwrap()
            .applied,
        1
    );
    assert!(store.load_messages("delete-session").unwrap().is_empty());
    assert!(!store.stage_remote_sync_record(&message_tombstone).unwrap());
}

#[test]
fn portable_settings_commands_skills_agents_and_workflows_use_lww_and_tombstones() {
    let store = Store::open_in_memory().expect("open store");
    let kinds = ["user_setting", "command", "skill", "agent", "workflow"];
    for (index, kind) in kinds.iter().enumerate() {
        store
            .stage_remote_sync_record(&remote_portable(
                index as i64 + 1,
                [0x70; 16],
                kind,
                &format!("{kind}/one"),
                1,
                format!("portable {kind}").as_bytes(),
            ))
            .unwrap();
    }
    assert_eq!(
        store
            .apply_staged_portable_records([0x80; 16], 10)
            .unwrap()
            .applied,
        kinds.len()
    );
    for kind in kinds {
        let record = store
            .portable_sync_record(kind, &format!("{kind}/one"))
            .unwrap()
            .expect("materialized portable record");
        assert!(!record.deleted);
        assert!(String::from_utf8(record.payload).unwrap().contains(kind));
    }

    store
        .stage_remote_sync_record(&remote_portable(
            6,
            [0x70; 16],
            "user_setting",
            "user_setting/one",
            2,
            &[],
        ))
        .unwrap();
    assert_eq!(
        store
            .apply_staged_portable_records([0x80; 16], 10)
            .unwrap()
            .applied,
        1
    );
    assert!(
        store
            .portable_sync_record("user_setting", "user_setting/one")
            .unwrap()
            .unwrap()
            .deleted
    );

    assert!(store
        .write_portable_sync_record(
            [0x80; 16],
            "user_setting",
            "provider_api_key",
            Some(b"must never sync"),
        )
        .is_err());
}

#[test]
fn local_portable_write_and_file_base_hash_are_journaled_atomically() {
    let store = Store::open_in_memory().expect("open store");
    store.set_sync_journal_enabled(true).unwrap();
    store
        .write_portable_sync_record([0x80; 16], "skill", "reviewer", Some(b"review safely"))
        .unwrap();
    let empty_hash: [u8; 32] = Sha256::digest([]).into();
    store
        .write_sync_file([0x80; 16], "commands/review.md", empty_hash, Some(b"local"))
        .unwrap();
    let pending = store.pending_sync_journal(10).unwrap();
    assert!(pending.iter().any(|entry| entry.record_kind == "skill"));
    let file = pending
        .iter()
        .find(|entry| entry.record_kind == "file")
        .unwrap();
    assert_eq!(file.base_hash, Some(empty_hash));
    assert_eq!(file.payload, b"local");
}

#[test]
fn divergent_files_create_visible_conflict_copies_instead_of_overwriting() {
    let store = Store::open_in_memory().expect("open store");
    store.set_sync_journal_enabled(true).unwrap();
    let empty_hash: [u8; 32] = Sha256::digest([]).into();
    store
        .write_sync_file([0x80; 16], "workflow/a.js", empty_hash, Some(b"local"))
        .unwrap();
    store
        .stage_remote_sync_record(&remote_file(
            1,
            "workflow/a.js",
            2,
            Some(empty_hash),
            b"remote divergent",
        ))
        .unwrap();
    let summary = store.apply_staged_file_records([0x80; 16], 10).unwrap();
    assert_eq!(summary.conflicts, 1);
    assert_eq!(store.sync_file("workflow/a.js").unwrap().unwrap(), b"local");
    let copies = store.sync_file_conflicts("workflow/a.js").unwrap();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].payload, b"remote divergent");
    assert_eq!(store.sync_apply_conflicts(10).unwrap().len(), 1);

    let local_hash: [u8; 32] = Sha256::digest(b"local").into();
    store
        .stage_remote_sync_record(&remote_file(
            2,
            "workflow/a.js",
            3,
            Some(local_hash),
            b"remote based on local",
        ))
        .unwrap();
    assert_eq!(
        store
            .apply_staged_file_records([0x80; 16], 10)
            .unwrap()
            .applied,
        1
    );
    assert_eq!(
        store.sync_file("workflow/a.js").unwrap().unwrap(),
        b"remote based on local"
    );
}

#[test]
fn sender_identity_collision_does_not_advance_the_staging_cursor() {
    let store = Store::open_in_memory().expect("open store");
    let first = remote_portable(1, [0x11; 16], "skill", "same-revision", 1, b"one");
    store.stage_remote_sync_record(&first).unwrap();
    let mut collision = first;
    collision.cursor = 2;
    collision.sender_device_id = [0x22; 16];
    assert!(store.stage_remote_sync_record(&collision).is_err());
    assert_eq!(store.sync_download_cursor().unwrap(), 1);
}
