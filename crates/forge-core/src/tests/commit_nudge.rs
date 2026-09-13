//! Commit discipline (`git_hygiene.rs`) wired through a real turn: the mid-turn reminder lands
//! after the edit that made it due, and the next turn starts by naming what is still uncommitted.

use super::*;

/// Every call writes `notes.md`; call `final_on` answers instead.
struct Editor {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    final_on: usize,
}

#[async_trait::async_trait]
impl Provider for Editor {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n >= self.final_on {
            return Ok(forge_provider::ModelResponse {
                reasoning: String::new(),
                content: "done".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            });
        }
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            content: String::new(),
            tool_calls: vec![forge_types::ToolCall {
                id: forge_types::new_id(),
                name: "write_file".into(),
                args: serde_json::json!({"path": "notes.md", "content": format!("edit {n}\n")}),
            }],
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

fn git(root: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn repo_session(final_on: usize, every: u32) -> (tempfile::TempDir, Session) {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("README.md"), "hi\n").unwrap();
    git(dir.path(), &["add", "README.md"]);
    git(dir.path(), &["commit", "-q", "-m", "init"]);
    let mut config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    config.git.commit_nudge_edits = every;
    let session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(Editor {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            final_on,
        }),
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(CapturePresenter {
            attended: true,
            ..Default::default()
        }),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    (dir, session)
}

fn git_notes(session: &Session) -> Vec<usize> {
    session
        .transcript
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::System && m.content.starts_with("[git]"))
        .map(|(i, _)| i)
        .collect()
}

#[tokio::test]
async fn the_mid_turn_reminder_lands_after_the_edit_that_made_it_due() {
    let (_dir, mut session) = repo_session(3, 2);
    session.run_turn("take notes").await.unwrap();
    let tool_results: Vec<usize> = session
        .transcript
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::Tool)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(tool_results.len(), 3, "three edits ran");
    let notes = git_notes(&session);
    assert_eq!(
        notes.len(),
        1,
        "one reminder for three edits at every-2: {notes:?}"
    );
    assert_eq!(
        notes[0],
        tool_results[1] + 1,
        "the reminder sits right after the second edit's result"
    );
    let text = &session.transcript[notes[0]].content;
    assert!(text.contains("notes.md"), "{text}");
    assert!(text.contains("uncommitted"), "{text}");
}

#[tokio::test]
async fn the_next_turn_opens_by_naming_the_uncommitted_files_until_they_are_committed() {
    let (dir, mut session) = repo_session(1, 0);
    session.run_turn("take notes").await.unwrap();
    assert!(
        git_notes(&session).is_empty(),
        "every=0 → no mid-turn reminder"
    );

    session.run_turn("more").await.unwrap();
    let notes = git_notes(&session);
    assert_eq!(notes.len(), 1, "{notes:?}");
    let user_idx = session
        .transcript
        .iter()
        .rposition(|m| m.role == Role::User && m.content == "more")
        .unwrap();
    assert!(
        notes[0] > user_idx,
        "reminder is part of the new turn's context"
    );
    assert!(session.transcript[notes[0]].content.contains("notes.md"));

    // The user commits outside Forge; the next turn is silent about it.
    git(dir.path(), &["add", "notes.md"]);
    git(dir.path(), &["commit", "-q", "-m", "notes"]);
    session.run_turn("again").await.unwrap();
    assert_eq!(
        git_notes(&session).len(),
        1,
        "no new reminder once the file is committed"
    );
}

#[tokio::test]
async fn disabling_commit_nudge_silences_both_reminders() {
    let (_dir, mut session) = repo_session(3, 1);
    session.config.git.commit_nudge = false;
    session.run_turn("take notes").await.unwrap();
    session.run_turn("more").await.unwrap();
    assert!(git_notes(&session).is_empty());
}
