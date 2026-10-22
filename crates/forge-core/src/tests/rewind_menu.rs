//! The rewind menu's actions: restore only the conversation, only the code, or summarize from a
//! message (docs/features/rewind.md, "The action menu").

use super::*;
use crate::session_rewind::SUMMARIZED_FROM_HERE;

fn quiet(mut config: Config) -> Config {
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    config
}

fn writing_session(dir: &std::path::Path, file: &std::path::Path) -> Session {
    let config = quiet(Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    });
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(WritingProvider {
            path: file.to_string_lossy().to_string(),
            content: "MODEL-EDIT".into(),
        }),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(HeadlessPresenter::new(false)),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap();
    session.set_checkpoint_root(dir.join("snaps"));
    session
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("forge-{tag}-{}", forge_types::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn turn_seq(session: &Session, prompt: &str) -> i64 {
    session
        .checkpoints()
        .unwrap()
        .into_iter()
        .find(|c| c.label.as_deref() == Some(prompt))
        .unwrap_or_else(|| panic!("no checkpoint {prompt:?}"))
        .seq
}

fn live_matches_store(session: &Session) {
    let stored: Vec<String> = session
        .store
        .load_messages(session.session_id())
        .unwrap()
        .into_iter()
        .map(|m| m.content)
        .collect();
    let live: Vec<String> = session
        .transcript
        .iter()
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(
        live, stored,
        "the live transcript must equal what a resume loads"
    );
}

#[tokio::test]
async fn restoring_the_conversation_only_leaves_the_files_alone() {
    let dir = temp_dir("rew-chat");
    let file = dir.join("f.txt");
    std::fs::write(&file, "ORIGINAL").unwrap();
    let mut session = writing_session(&dir, &file);
    session.run_turn("edit the file").await.unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "MODEL-EDIT");
    let seq = turn_seq(&session, "edit the file");
    assert!(session.has_file_changes_from(seq));

    let outcome = session
        .rewind_scoped(seq, RewindScope::Conversation)
        .unwrap();

    assert_eq!(outcome.rewound_prompt.as_deref(), Some("edit the file"));
    assert!(outcome.restore.is_empty(), "{:?}", outcome.restore);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "MODEL-EDIT");
    assert!(!session
        .transcript
        .iter()
        .any(|m| m.role == Role::User && m.content == "edit the file"));
    live_matches_store(&session);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn restoring_the_code_only_keeps_the_conversation_and_tells_the_model() {
    let dir = temp_dir("rew-code");
    let file = dir.join("f.txt");
    std::fs::write(&file, "ORIGINAL").unwrap();
    let mut session = writing_session(&dir, &file);
    session.run_turn("edit the file").await.unwrap();
    let seq = turn_seq(&session, "edit the file");
    let before = session.transcript.len();

    let outcome = session.rewind_scoped(seq, RewindScope::Code).unwrap();

    assert_eq!(outcome.rewound_prompt, None, "the input box is left alone");
    assert!(
        !outcome.restore.restored.is_empty(),
        "{:?}",
        outcome.restore
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "ORIGINAL");
    assert!(
        session
            .transcript
            .iter()
            .any(|m| m.role == Role::User && m.content == "edit the file"),
        "the conversation is kept"
    );
    assert_eq!(session.transcript.len(), before + 1, "one note was added");
    let note = &session.transcript.last().unwrap().content;
    assert!(note.contains("f.txt"), "{note}");
    assert!(note.contains("\"edit the file\""), "{note}");
    assert!(
        !session.has_file_changes_from(seq),
        "the snapshots were used up by the restore"
    );
    live_matches_store(&session);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_code_restore_with_nothing_to_put_back_adds_no_note() {
    let dir = temp_dir("rew-none");
    let file = dir.join("f.txt");
    std::fs::write(&file, "ORIGINAL").unwrap();
    let mut session = writing_session(&dir, &file);
    session.run_turn("edit the file").await.unwrap();
    let next = session
        .store
        .next_seq_for_session(session.session_id())
        .unwrap();
    let before = session.transcript.len();
    let outcome = session.rewind_scoped(next, RewindScope::Code).unwrap();
    assert!(outcome.restore.is_empty());
    assert_eq!(session.transcript.len(), before);
    std::fs::remove_dir_all(&dir).ok();
}

/// Answers turns with plain text and every call without tools (the summarizer, side calls) with
/// a well-formed summary, recording the system prompt.
#[derive(Default)]
struct SummaryProvider {
    summary_systems: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Provider for SummaryProvider {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let content = if tools.is_empty() {
            self.summary_systems
                .lock()
                .unwrap()
                .push(messages[0].content.clone());
            format!(
                "<analysis>scratch</analysis>\n<summary>\n1. Primary request and intent\n\
                 SUMMARY-MARKER\n2. Key technical concepts\n{}\n3. Files and code\nnone\n</summary>",
                "detail ".repeat(120)
            )
        } else {
            let asked = messages
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .map(|m| m.content.clone())
                .unwrap_or_default();
            format!("answer to {asked}")
        };
        Ok(forge_provider::ModelResponse {
            content,
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn summarize_from_here_folds_the_tail_and_keeps_the_head() {
    let provider = Arc::new(SummaryProvider::default());
    let store = Arc::new(Store::open_in_memory().unwrap());
    let config = quiet(Config::default());
    let mut session = Session::start(
        Arc::clone(&store),
        provider.clone(),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(HeadlessPresenter::new(false)),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    for prompt in ["one", "two", "three"] {
        session.run_turn(prompt).await.unwrap();
    }
    let id = session.session_id().to_string();
    let two = turn_seq(&session, "two");

    let outcome = session
        .summarize_from(two, Some("keep the error messages"))
        .await
        .unwrap();

    assert_eq!(outcome.rewound_prompt.as_deref(), Some("two"));
    assert!(outcome.folded >= 4, "two turns folded: {outcome:?}");
    let systems = provider.summary_systems.lock().unwrap().clone();
    assert!(
        systems
            .iter()
            .any(|s| s.contains("keep the error messages")),
        "the focus reached the summarizer"
    );
    let rows = store.message_state_rows(&id).unwrap();
    for (seq, role, active, content) in &rows {
        if *seq >= two && role != "system" {
            assert!(!active, "row {seq} ({role}: {content}) must be folded");
        }
        if *seq < two && role == "user" {
            assert!(active, "row {seq} before the point stays");
        }
    }
    let summary = session.transcript.last().unwrap();
    assert_eq!(summary.role, Role::System);
    assert!(summary.content.starts_with(SUMMARIZED_FROM_HERE));
    assert!(summary.content.contains("SUMMARY-MARKER"));
    assert!(session
        .transcript
        .iter()
        .any(|m| m.role == Role::User && m.content == "one"));
    assert!(!session
        .transcript
        .iter()
        .any(|m| m.role == Role::User && (m.content == "two" || m.content == "three")));
    live_matches_store(&session);

    // The conversation carries on after the summary, on a fresh seq.
    session.run_turn("four").await.unwrap();
    assert!(session
        .transcript
        .iter()
        .any(|m| m.role == Role::User && m.content == "four"));
    live_matches_store(&session);
}

#[tokio::test]
async fn summarize_from_past_the_end_does_nothing() {
    let provider = Arc::new(SummaryProvider::default());
    let config = quiet(Config::default());
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        provider.clone(),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(HeadlessPresenter::new(false)),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    session.run_turn("one").await.unwrap();
    let next = session
        .store
        .next_seq_for_session(session.session_id())
        .unwrap();
    let before = session.transcript.len();
    let calls = provider.summary_systems.lock().unwrap().len();
    let outcome = session.summarize_from(next, None).await.unwrap();
    assert_eq!(outcome, SummarizeOutcome::default());
    assert_eq!(session.transcript.len(), before);
    assert_eq!(
        provider.summary_systems.lock().unwrap().len(),
        calls,
        "no summary call was made"
    );
}
