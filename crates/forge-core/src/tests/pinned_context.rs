//! The pinned session context block and what a compaction restores: project instructions,
//! memories, goal and task list ride on every request without being stored, and the files the
//! session was working on come back after a compaction.

use super::*;
use crate::file_restore::RESTORED_HEADER;
use forge_types::MessageKind;

fn summary() -> String {
    format!(
        "<analysis>scratch</analysis>\n<summary>\n1. Primary request and intent\nSUMMARY\n\
         2. Key technical concepts\n{}\n3. Files and code\nnone\n</summary>",
        "detail ".repeat(120)
    )
}

/// Records every main-loop request. Answers the summarizer with a well-formed summary, and a
/// scripted first reply with tool calls when `first_calls` is set.
#[derive(Default)]
pub(super) struct Recorder {
    pub(super) requests: Mutex<Vec<Vec<Message>>>,
    pub(super) first_calls: Mutex<Vec<forge_types::ToolCall>>,
}

impl Recorder {
    pub(super) fn requests(&self) -> Vec<Vec<Message>> {
        self.requests.lock().unwrap().clone()
    }
    pub(super) fn last(&self) -> Vec<Message> {
        self.requests().pop().expect("a request was made")
    }
}

#[async_trait::async_trait]
impl Provider for Recorder {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        self.requests.lock().unwrap().push(messages.to_vec());
        let calls = std::mem::take(&mut *self.first_calls.lock().unwrap());
        Ok(forge_provider::ModelResponse {
            content: if calls.is_empty() {
                "done".into()
            } else {
                String::new()
            },
            tool_calls: calls,
            ..Default::default()
        })
    }

    async fn complete_with(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        opts: &forge_provider::CompletionOptions,
        on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        if opts
            .prompt_cache_key
            .as_deref()
            .is_some_and(|key| key.ends_with(":compact"))
        {
            return Ok(forge_provider::ModelResponse {
                content: summary(),
                ..Default::default()
            });
        }
        // Side calls (memory capture, recap) carry a `<session>:<purpose>` key; only the main
        // loop's requests are recorded.
        if opts
            .prompt_cache_key
            .as_deref()
            .is_some_and(|key| key.contains(':'))
        {
            return Ok(forge_provider::ModelResponse {
                content: "NONE".into(),
                ..Default::default()
            });
        }
        self.complete(model, messages, tools, on_event).await
    }
}

pub(super) fn workspace(agents_md: Option<&str>) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("forge-pinned-{}", forge_types::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    if let Some(body) = agents_md {
        std::fs::write(dir.join("AGENTS.md"), body).unwrap();
    }
    dir
}

pub(super) fn quiet_config() -> Config {
    let mut config = Config::default();
    config.mesh.rate_limit_wait_secs = 0;
    config.mesh.auto_memory = false;
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.git.commit_nudge = false;
    config
}

fn router() -> Arc<FixedRouter> {
    Arc::new(FixedRouter {
        model: "m::x".into(),
        fallbacks: vec![],
    })
}

pub(super) fn start(
    store: &Arc<Store>,
    dir: &std::path::Path,
    provider: &Arc<Recorder>,
    config: Config,
) -> Session {
    Session::start(
        Arc::clone(store),
        provider.clone(),
        router(),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(HeadlessPresenter::new(false)),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap()
}

fn resume(
    store: &Arc<Store>,
    dir: &std::path::Path,
    provider: &Arc<Recorder>,
    id: &str,
    config: Config,
) -> Session {
    Session::resume(
        Arc::clone(store),
        provider.clone(),
        router(),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(HeadlessPresenter::new(false)),
        config,
        id,
    )
    .unwrap()
}

fn pinned(request: &[Message]) -> Option<&Message> {
    request.iter().find(|m| m.kind == MessageKind::Pinned)
}

fn holding(request: &[Message], needle: &str) -> usize {
    request
        .iter()
        .filter(|m| m.content.contains(needle))
        .count()
}

fn fill(session: &mut Session, pairs: usize) {
    for i in 0..pairs {
        session
            .transcript
            .push(Message::user(format!("request {i}")));
        session
            .transcript
            .push(Message::assistant(format!("done {i}")));
    }
}

fn stored_has(store: &Store, id: &str, needle: &str) -> bool {
    store
        .load_all_messages(id)
        .unwrap()
        .iter()
        .any(|m| m.content.contains(needle))
}

#[tokio::test]
async fn the_pinned_block_follows_the_system_prompt_and_is_never_stored() {
    let dir = workspace(Some("PROJECT-RULES: use tabs\n"));
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut session = start(&store, &dir, &provider, quiet_config());
    session.run_turn("first prompt").await.unwrap();

    let request = provider.last();
    let at = request
        .iter()
        .position(|m| m.kind == MessageKind::Pinned)
        .expect("the block is in the request");
    assert!(at >= 2, "after the prompt and the env block: {at}");
    assert!(
        request[..=at].iter().all(|m| m.role == Role::System),
        "it sits in the leading system run"
    );
    assert!(request[at].content.contains("PROJECT-RULES: use tabs"));
    assert_eq!(holding(&request, "PROJECT-RULES"), 1);
    assert!(
        session
            .transcript
            .iter()
            .all(|m| !m.content.contains("PROJECT-RULES")),
        "not in the in-memory transcript"
    );
    assert!(
        !stored_has(&store, session.id(), "PROJECT-RULES"),
        "not in the stored transcript"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_block_is_still_there_after_a_compaction_and_after_a_resume() {
    let dir = workspace(Some("PROJECT-RULES: use tabs\n"));
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut session = start(&store, &dir, &provider, quiet_config());
    let id = session.id().to_string();
    session.run_turn("first prompt").await.unwrap();
    fill(&mut session, 8);
    session.compact(false).await.unwrap();
    assert!(
        session.transcript[0]
            .content
            .starts_with(crate::compaction_summary::SUMMARY_PREFIX),
        "the compaction ran"
    );

    session.run_turn("after the compaction").await.unwrap();
    let request = provider.last();
    assert_eq!(
        holding(&request, "PROJECT-RULES"),
        1,
        "once, not folded and not doubled"
    );
    assert!(pinned(&request).is_some());

    let mut resumed = resume(&store, &dir, &provider, &id, quiet_config());
    resumed.run_turn("after the resume").await.unwrap();
    let request = provider.last();
    assert_eq!(holding(&request, "PROJECT-RULES"), 1);
    assert!(
        !stored_has(&store, &id, "PROJECT-RULES"),
        "nothing about the instructions was ever stored"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_session_stored_before_the_block_existed_is_not_sent_its_instructions_twice() {
    let body = "PROJECT-RULES: legacy copy\n";
    let dir = workspace(Some(body));
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let id = store
        .create_session(dir.to_str().unwrap(), "default")
        .unwrap();
    store.add_message(&id, 0, Role::System, body, None).unwrap();
    store
        .add_message(&id, 1, Role::User, "an old request", None)
        .unwrap();
    store
        .add_message(&id, 2, Role::Assistant, "an old answer", None)
        .unwrap();

    let mut session = resume(&store, &dir, &provider, &id, quiet_config());
    session.run_turn("a new request").await.unwrap();
    assert_eq!(
        holding(&provider.last(), "PROJECT-RULES"),
        1,
        "the stored copy is the only one while it is still in the transcript"
    );

    session.transcript.retain(|m| m.content != body);
    session.run_turn("another request").await.unwrap();
    assert_eq!(
        holding(&provider.last(), "PROJECT-RULES"),
        1,
        "once a compaction folds the stored copy away, the block carries it"
    );
    assert!(pinned(&provider.last()).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_agents_md_edited_mid_session_replaces_the_pinned_text() {
    let dir = workspace(Some("RULES v1\n"));
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut session = start(&store, &dir, &provider, quiet_config());
    session.run_turn("first").await.unwrap();
    assert_eq!(holding(&provider.last(), "RULES v1"), 1);

    std::fs::write(dir.join("AGENTS.md"), "RULES v2 — revised\n").unwrap();
    session.run_turn("second").await.unwrap();
    let request = provider.last();
    assert_eq!(holding(&request, "RULES v2"), 1);
    assert_eq!(
        holding(&request, "RULES v1"),
        0,
        "the old text is gone, not stacked"
    );
    assert!(
        session
            .last_context_pack()
            .entries()
            .iter()
            .any(|e| { e.source() == context_pack::ContextSource::ProjectInstructions }),
        "the change is recorded in the turn's context pack"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_agents_md_written_mid_session_reaches_the_next_turn() {
    // The session that writes the file (or runs `/init`) must pick it up on its next turn, and an
    // unchanged file must not pile up a copy per turn.
    let dir = workspace(None);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut session = start(&store, &dir, &provider, quiet_config());
    session.run_turn("first prompt").await.unwrap();
    assert!(
        pinned(&provider.last()).is_none(),
        "nothing pinned before the file exists"
    );

    std::fs::write(dir.join("AGENTS.md"), "PROJECT-RULES v1\n").unwrap();
    session.run_turn("second prompt").await.unwrap();
    assert_eq!(holding(&provider.last(), "PROJECT-RULES v1"), 1);
    session.run_turn("third prompt").await.unwrap();
    assert_eq!(holding(&provider.last(), "PROJECT-RULES v1"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_bridged_claude_that_loads_claude_md_is_not_sent_the_instructions() {
    let dir = workspace(Some("PROJECT-RULES: shared\n"));
    std::fs::write(dir.join("CLAUDE.md"), "@AGENTS.md\n").unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let session = start(&store, &dir, &provider, quiet_config());
    assert!(session
        .pinned_context_message(Some("claude-cli::sonnet"))
        .is_none());
    let direct = session
        .pinned_context_message(Some("m::x"))
        .expect("a direct provider still gets them");
    assert!(direct.content.contains("PROJECT-RULES: shared"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn memories_are_recalled_again_after_a_compaction() {
    let dir = workspace(None);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut config = quiet_config();
    config.mesh.auto_memory = true;
    let mut session = start(&store, &dir, &provider, config);
    session.run_turn("first prompt").await.unwrap();
    assert!(
        session.pinned.memories.is_none() && pinned(&provider.last()).is_none(),
        "nothing to recall yet"
    );

    store
        .add_memory(
            &memory_scope_at(session.workspace.root()),
            "decision",
            "deploy with blue green",
            "s0",
        )
        .unwrap();
    fill(&mut session, 8);
    session
        .transcript
        .push(Message::user("how do we deploy this?"));
    session.transcript.push(Message::assistant("let me check"));
    assert_eq!(session.latest_user_request(), "how do we deploy this?");
    session.set_active_goal(Some("ship the release"));
    assert_eq!(
        session.recall_query("how do we deploy this?"),
        "ship the release\nhow do we deploy this?"
    );
    session.compact(false).await.unwrap();
    assert!(
        session
            .pinned
            .memories
            .as_deref()
            .is_some_and(|m| m.contains("deploy with blue green")),
        "the recall ran again after the compaction: {:?}",
        session.pinned.memories
    );

    session.run_turn("go on").await.unwrap();
    let request = provider.last();
    assert_eq!(holding(&request, "deploy with blue green"), 1);
    assert!(
        session
            .transcript
            .iter()
            .all(|m| !m.content.contains("deploy with blue green")),
        "the memories are not stored in the transcript"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_resumed_session_recalls_memories_on_its_first_turn() {
    let dir = workspace(None);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut config = quiet_config();
    config.mesh.auto_memory = true;
    let mut session = start(&store, &dir, &provider, config.clone());
    let id = session.id().to_string();
    session.run_turn("first prompt").await.unwrap();
    store
        .add_memory(
            &memory_scope_at(session.workspace.root()),
            "preference",
            "prefers small commits",
            "s0",
        )
        .unwrap();

    let mut resumed = resume(&store, &dir, &provider, &id, config);
    resumed.run_turn("what about commits?").await.unwrap();
    assert_eq!(holding(&provider.last(), "prefers small commits"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_goal_and_the_task_list_ride_in_the_block() {
    let dir = workspace(None);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut session = start(&store, &dir, &provider, quiet_config());
    session.set_active_goal(Some("ship the exporter"));
    session.tasks = vec![
        forge_types::TodoItem {
            title: "write the exporter".into(),
            status: forge_types::TodoStatus::InProgress,
            assignee: None,
        },
        forge_types::TodoItem {
            title: "add its tests".into(),
            status: forge_types::TodoStatus::Pending,
            assignee: None,
        },
    ];
    session.run_turn("continue").await.unwrap();
    let request = provider.last();
    let block = &pinned(&request).expect("block").content;
    assert!(
        block.contains("## Active goal\nship the exporter"),
        "{block}"
    );
    assert!(
        block.contains("- [in progress] write the exporter"),
        "{block}"
    );
    assert!(block.contains("- [pending] add its tests"), "{block}");

    // The per-turn note keeps its instruction but no longer quotes the list a second time.
    let notes: Vec<&Message> = request
        .iter()
        .filter(|m| m.content.starts_with("[task list]"))
        .collect();
    assert_eq!(notes.len(), 1);
    assert!(!notes[0].content.contains("write the exporter"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn update_tasks_returns_the_full_list_and_the_block_does_not_move_mid_turn() {
    let dir = workspace(None);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    *provider.first_calls.lock().unwrap() = vec![forge_types::ToolCall {
        id: "t1".into(),
        name: "update_tasks".into(),
        args: serde_json::json!({ "tasks": [
            { "title": "read the code", "status": "done" },
            { "title": "write the fix", "status": "done" },
        ]}),
    }];
    let mut session = start(&store, &dir, &provider, quiet_config());
    session.run_turn("fix the bug").await.unwrap();

    let result = session
        .transcript
        .iter()
        .find(|m| m.role == Role::Tool && m.content.starts_with("task list updated"))
        .expect("update_tasks answered");
    assert_eq!(
        result.content,
        "task list updated: 2 task(s) — 2 done, 0 in progress\n- [done] read the code\n- [done] write the fix"
    );

    let requests = provider.requests();
    assert!(requests.len() >= 2, "the tool call and the answer");
    assert!(
        requests.iter().all(|request| pinned(request).is_none()),
        "the task list was written mid-turn; the block waits for the next turn so the provider's \
         cached prefix stays intact"
    );

    session.run_turn("and now?").await.unwrap();
    let last = provider.last();
    let block = &pinned(&last).expect("block").content;
    assert!(block.contains("- [done] write the fix"), "{block}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_read_file_call_is_remembered_and_comes_back_after_a_compaction() {
    let dir = workspace(None);
    std::fs::write(dir.join("a.rs"), "fn a() { /* v1 */ }\n").unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    *provider.first_calls.lock().unwrap() = vec![forge_types::ToolCall {
        id: "r1".into(),
        name: "read_file".into(),
        args: serde_json::json!({ "path": "a.rs" }),
    }];
    let mut session = start(&store, &dir, &provider, quiet_config());
    session.run_turn("look at a.rs").await.unwrap();
    assert_eq!(
        session.recent_files.newest_first(),
        [session.workspace.root().join("a.rs")]
    );

    std::fs::write(dir.join("a.rs"), "fn a() { /* v2 */ }\n").unwrap();
    fill(&mut session, 8);
    session.compact(false).await.unwrap();
    let restored = session.transcript.last().expect("the note");
    assert_eq!(restored.kind, MessageKind::FilesRestored);
    assert_eq!(restored.role, Role::System);
    assert!(restored.content.starts_with(RESTORED_HEADER));
    assert!(
        restored.content.contains("fn a() { /* v2 */ }") && !restored.content.contains("v1"),
        "the text on disk now, not what was read: {}",
        restored.content
    );
    assert!(
        stored_has(&store, session.id(), RESTORED_HEADER),
        "the note is stored, so a resume has it too"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_second_compaction_replaces_the_restored_files_rather_than_stacking_them() {
    let dir = workspace(None);
    std::fs::write(dir.join("a.rs"), "fn a() { /* first */ }\n").unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut session = start(&store, &dir, &provider, quiet_config());
    session.note_file_use("a.rs");
    fill(&mut session, 8);
    session.compact(false).await.unwrap();

    std::fs::write(dir.join("a.rs"), "fn a() { /* second */ }\n").unwrap();
    session.transcript.push(Message::user("one more request"));
    session.transcript.push(Message::assistant("done"));
    fill(&mut session, 2);
    session.compact(false).await.unwrap();

    session.run_turn("go on").await.unwrap();
    let request = provider.last();
    assert_eq!(
        holding(&request, RESTORED_HEADER),
        1,
        "one copy in the request"
    );
    assert_eq!(holding(&request, "/* second */"), 1);
    assert_eq!(holding(&request, "/* first */"), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_bridged_session_is_only_told_which_files_to_read_again() {
    let dir = workspace(None);
    std::fs::write(dir.join("a.rs"), "fn a() { /* SECRET-BODY */ }\n").unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut session = start(&store, &dir, &provider, quiet_config());
    session.pin_model(Some("claude-cli::sonnet".into()));
    session.note_file_use("a.rs");
    fill(&mut session, 8);
    session.compact(false).await.unwrap();
    let note = session.transcript.last().expect("the note");
    assert_eq!(note.kind, MessageKind::FilesRestored);
    assert!(note.content.contains("- a.rs"), "{}", note.content);
    assert!(!note.content.contains("SECRET-BODY"), "the paths only");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn nothing_is_restored_when_no_recent_file_is_left_on_disk() {
    let dir = workspace(None);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut session = start(&store, &dir, &provider, quiet_config());
    session.note_file_use("gone.rs");
    fill(&mut session, 8);
    session.compact(false).await.unwrap();
    assert!(session
        .transcript
        .iter()
        .all(|m| m.kind != MessageKind::FilesRestored));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_block_counts_toward_the_gauge_and_the_window_budget() {
    let dir = workspace(Some(&"rule ".repeat(400)));
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let session = start(&store, &dir, &provider, quiet_config());
    let pinned_tokens = session.pinned_tokens();
    assert!(pinned_tokens >= 400, "{pinned_tokens}");
    assert_eq!(
        session.estimated_transcript_tokens(),
        pinned_tokens as u64,
        "an empty transcript still costs what rides on every request"
    );

    let none = workspace(None);
    let bare = start(&store, &none, &provider, quiet_config());
    let with_block = session.transcript_with_preamble_and_tools("m::x", &[]);
    let without = bare.transcript_with_preamble_and_tools("m::x", &[]);
    assert_eq!(with_block.len(), without.len() + 1);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&none);
}

#[tokio::test]
async fn a_stored_memory_message_keeps_the_recalled_memories_out_of_the_block() {
    let dir = workspace(None);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut session = start(&store, &dir, &provider, quiet_config());
    session.pinned.memories = Some(format!(
        "{}- [fact] uses tabs\n",
        crate::pinned_context::MEMORY_HEADER
    ));
    assert!(session
        .pinned_context_message(None)
        .is_some_and(|m| m.content.contains("uses tabs")));

    session
        .transcript
        .push(Message::system(session.pinned.memories.clone().unwrap()));
    assert!(
        session.pinned_context_message(None).is_none(),
        "the session already carries its memories as a stored message"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_recall_embedding_starts_before_routing_only_when_there_is_something_to_recall() {
    let dir = workspace(None);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut config = quiet_config();
    config.mesh.auto_memory = true;
    config.lattice.embeddings.enabled = false;
    let mut session = start(&store, &dir, &provider, config);
    session.prefetch_recall_embedding("how do we deploy?");
    assert!(
        session.pinned.recall_prefetch.is_none(),
        "an empty memory store has nothing to recall, so nothing is embedded"
    );

    store
        .add_memory(
            &memory_scope_at(session.workspace.root()),
            "fact",
            "DEPLOYFACT deploys go through the blue cluster",
            "s0",
        )
        .unwrap();
    session.prefetch_recall_embedding("how do we deploy?");
    assert!(session.pinned.recall_prefetch.is_some());
    session.run_turn("how do we deploy?").await.unwrap();
    assert!(
        session.pinned.recall_prefetch.is_none(),
        "the recall consumed the embedding started before routing"
    );
    assert_eq!(holding(&provider.last(), "DEPLOYFACT"), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn global_memories_are_recalled_alongside_the_projects() {
    let dir = workspace(None);
    let store = Arc::new(Store::open_in_memory().unwrap());
    store
        .add_memory(
            "global",
            "fact",
            "GLOBALFACT the staging cluster is blue",
            "manual",
        )
        .unwrap();
    store
        .add_memory(
            &crate::memory_scope_at(&dir),
            "fact",
            "PROJFACT the build uses green runners",
            "manual",
        )
        .unwrap();
    let provider = Arc::new(Recorder::default());
    let mut config = quiet_config();
    config.mesh.auto_memory = true;
    let mut session = start(&store, &dir, &provider, config);
    session
        .run_turn("which cluster and runners?")
        .await
        .unwrap();

    let request = provider.last();
    assert_eq!(holding(&request, "GLOBALFACT"), 1, "global memory recalled");
    assert_eq!(
        holding(&request, "PROJFACT"),
        1,
        "project memory still recalled"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn merge_recalled_reserves_room_for_global_memories_without_wasting_slots() {
    fn mem(id: &str) -> forge_store::Memory {
        forge_store::Memory {
            id: id.into(),
            scope: String::new(),
            kind: "fact".into(),
            text: id.into(),
            source_session: String::new(),
            created_at: 0,
            updated_at: 0,
            salience: 0.5,
        }
    }
    let ids = |v: Vec<forge_store::Memory>| v.into_iter().map(|m| m.id).collect::<Vec<_>>();
    let many: Vec<_> = (0..6).map(|i| mem(&format!("p{i}"))).collect();
    let merged = crate::pinned_context::merge_recalled(many.clone(), vec![mem("g0"), mem("g1")], 6);
    assert_eq!(ids(merged), ["p0", "p1", "p2", "p3", "g0", "g1"]);
    // Nothing global: the project keeps every slot. Nothing project: global fills them.
    assert_eq!(
        crate::pinned_context::merge_recalled(many.clone(), vec![], 6).len(),
        6
    );
    let globals: Vec<_> = (0..8).map(|i| mem(&format!("g{i}"))).collect();
    assert_eq!(
        crate::pinned_context::merge_recalled(vec![], globals, 6).len(),
        6
    );
    // The same row in both scopes is listed once.
    let both = crate::pinned_context::merge_recalled(vec![mem("x")], vec![mem("x")], 6);
    assert_eq!(ids(both), ["x"]);
}

fn words(n: usize) -> String {
    (0..n).map(|i| format!("w{i} ")).collect()
}

fn small_window_session(
    store: &Arc<Store>,
    dir: &std::path::Path,
    provider: &Arc<Recorder>,
    window: u32,
    cap: u64,
) -> Session {
    let mut config = quiet_config();
    config.mesh.compact_cap_tokens = cap;
    config.mesh.context_windows.insert("m::x".into(), window);
    let mut session = start(store, dir, provider, config);
    session.pin_model(Some("m::x".into()));
    session
}

#[tokio::test]
async fn a_compaction_with_large_restored_files_lands_under_a_small_trigger() {
    for (window, cap) in [
        (1_000_000u32, 24_000u64),
        (32_000, 217_600),
        (64_000, 24_000),
    ] {
        let dir = workspace(Some("always run the tests"));
        for name in ["a.rs", "b.rs", "c.rs", "d.rs"] {
            std::fs::write(dir.join(name), words(5_000)).unwrap();
            assert!(std::fs::metadata(dir.join(name)).unwrap().len() > 20_000);
        }
        let store = Arc::new(Store::open_in_memory().unwrap());
        let provider = Arc::new(Recorder::default());
        let mut session = small_window_session(&store, &dir, &provider, window, cap);
        for name in ["a.rs", "b.rs", "c.rs", "d.rs"] {
            session.note_file_use(name);
        }
        fill(&mut session, 12);
        let trigger = session.effective_compaction_trigger(None);
        assert!(trigger <= 25_600, "{window}/{cap}: {trigger}");
        session.compact(true).await.unwrap();
        let after = session.context_pressure_tokens();
        assert!(
            after <= trigger / 2 + trigger / 20,
            "{window}/{cap}: {after} tokens left of a {trigger} trigger"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[tokio::test]
async fn a_big_window_still_restores_its_files_in_full() {
    let dir = workspace(None);
    std::fs::write(dir.join("a.rs"), words(5_000)).unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut session = small_window_session(&store, &dir, &provider, 1_000_000, 10_000_000);
    session.note_file_use("a.rs");
    fill(&mut session, 12);
    session.compact(true).await.unwrap();
    let note = session.transcript.last().expect("the note");
    assert_eq!(note.kind, MessageKind::FilesRestored);
    assert!(note.content.contains("w4999"), "whole file restored");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn auto_compaction_waits_for_regrowth_instead_of_compacting_back_to_back() {
    let dir = workspace(None);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let provider = Arc::new(Recorder::default());
    let mut session = small_window_session(&store, &dir, &provider, 200_000, 24_000);
    let trigger = session.effective_compaction_trigger(None);
    assert_eq!(trigger, 24_000);
    fill(&mut session, 10);
    session.transcript.push(Message::user(words(12_000)));
    session.transcript.push(Message::assistant("ok"));
    let size = session.context_pressure_tokens();
    assert!(size > trigger, "{size}");

    session
        .compact_guard
        .note_compaction(&session.id, size - 500);
    let epoch = session.history_epoch;
    session.auto_compact_if_needed("m::x").await;
    assert_eq!(
        session.history_epoch, epoch,
        "no regrowth, no second summary"
    );

    session
        .compact_guard
        .note_compaction(&session.id, size.saturating_sub(trigger / 4 + 1));
    session.auto_compact_if_needed("m::x").await;
    assert!(
        session.history_epoch > epoch,
        "regrown by a quarter of the trigger"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
