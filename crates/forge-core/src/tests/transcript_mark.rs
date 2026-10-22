//! A transcript position taken before a model loop must survive the compaction that loop runs.

use super::*;

/// Answers every summary request with a well-formed summary.
struct Summarizer;

#[async_trait::async_trait]
impl Provider for Summarizer {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        Ok(forge_provider::ModelResponse {
            content: format!(
                "<summary>\n1. Primary request and intent\nrename things\n2. Key technical \
                 concepts\n{}\n</summary>",
                "detail ".repeat(120)
            ),
            ..Default::default()
        })
    }
}

fn session() -> Session {
    fixed_session(
        Arc::new(Summarizer),
        Arc::new(FixedRouter {
            model: "direct::summarizer".into(),
            fallbacks: vec![],
        }),
    )
    .1
}

fn push_rounds(session: &mut Session, range: std::ops::Range<usize>) {
    for i in range {
        session
            .transcript
            .push(Message::user(format!("request {i}")));
        session
            .transcript
            .push(Message::assistant(format!("done {i}")));
    }
}

/// The measured panic: the completeness audit saved `transcript.len()` (16), its model loop
/// auto-compacted the transcript to 7 messages, and `&transcript[16..]` aborted the process.
#[tokio::test]
async fn a_mark_survives_a_compaction_that_shrinks_the_transcript_below_it() {
    let mut session = session();
    push_rounds(&mut session, 0..8);
    let mark = session.transcript_mark();
    let saved_len = session.transcript.len();
    session
        .transcript
        .push(Message::user("audit: search the repository"));
    session.transcript.push(Message::assistant("audit: done"));

    let (before, after) = session.compact(true).await.unwrap();
    assert!(after < before, "the transcript must have been compacted");
    assert!(
        session.transcript.len() < saved_len,
        "the old index ({saved_len}) now points past the end ({})",
        session.transcript.len()
    );

    let since: Vec<&str> = session
        .transcript_since(mark)
        .iter()
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(since, ["audit: search the repository", "audit: done"]);
}

#[tokio::test]
async fn a_mark_inside_the_folded_part_yields_only_the_kept_tail() {
    let mut session = session();
    push_rounds(&mut session, 0..2);
    let mark = session.transcript_mark();
    push_rounds(&mut session, 2..10);
    session.compact(true).await.unwrap();
    let since = session.transcript_since(mark);
    assert_eq!(since.len(), session.transcript.len() - 1);
    assert!(
        !since[0].content.contains("rename things"),
        "the summary also covers what came before the mark, so it is not 'since the mark'"
    );
}

#[tokio::test]
async fn a_rewind_invalidates_marks_instead_of_pointing_at_other_messages() {
    let mut session = session();
    push_rounds(&mut session, 0..4);
    let mark = session.transcript_mark();
    push_rounds(&mut session, 4..6);
    session.history_epoch += 1;
    assert!(session.transcript_since(mark).is_empty());
}
