//! Mid-intent stop detection: classifier accuracy over a labelled corpus plus the model-loop
//! behaviour built on it.
//!
//! `mid_intent_corpus.txt` holds `+`/`-` lines (label TAB reply tail, newlines escaped as `\n`).
//! `-` lines are the last ~300 chars of final assistant replies (no tool call) that the user
//! followed with an unrelated request. `+` lines are replies the model ended on an announced-but-
//! undone action: replies after which the user typed `continue`/`you stopped`, relabelled by hand
//! from final replies that were followed by a topic change but plainly stop mid-sentence, plus
//! announce-then-call tails from the same store as a proxy for the wording of a stalled reply.
//! Paths, addresses, hashes and URLs were scrubbed.

use super::*;
use crate::mid_intent::ends_mid_intent;

const CORPUS: &str = include_str!("mid_intent_corpus.txt");

fn corpus() -> Vec<(bool, String)> {
    CORPUS
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (label, text) = l.split_once('\t').expect("label<TAB>text");
            (label == "+", text.replace("\\n", "\n"))
        })
        .collect()
}

#[test]
fn classifier_meets_precision_and_recall_on_the_corpus() {
    let (mut tp, mut fp, mut fneg, mut tn) = (0usize, 0usize, 0usize, 0usize);
    for (label, text) in corpus() {
        match (label, ends_mid_intent(&text)) {
            (true, true) => tp += 1,
            (true, false) => fneg += 1,
            (false, true) => fp += 1,
            (false, false) => tn += 1,
        }
    }
    let precision = tp as f64 / (tp + fp).max(1) as f64;
    let recall = tp as f64 / (tp + fneg).max(1) as f64;
    eprintln!("mid-intent corpus: tp={tp} fp={fp} fn={fneg} tn={tn} precision={precision:.3} recall={recall:.3}");
    assert!(tp + fneg >= 60 && fp + tn >= 60, "corpus shrank");
    assert!(recall >= 0.90, "recall {recall:.3} (tp={tp}, fn={fneg})");
    assert!(
        precision >= 0.95,
        "precision {precision:.3} (tp={tp}, fp={fp})"
    );
}

#[test]
fn announced_but_undone_work_is_mid_intent() {
    for text in [
        "Syntax passes. Let me verify a few runtime issues that syntax would not catch:",
        "The file is written. I'll now run the browser checks...",
        "Now the worker/engine side — modes, actions, and what the dashboard can actually drive.",
        "I’m checking whether Rust already has that path, then I’ll wire Android login to it.",
        "Good, now let's look at the config loader",
        "Moving on to the store migration.",
        "Next, I'll update the tests.",
        "Decoding the v4 password flow now.",
        "Done with the parser.\n\nNow I need to update the `login` function:",
        "I've finished the map; now I'm pinning the refactor shape before touching code.",
    ] {
        assert!(ends_mid_intent(text), "missed: {text}");
    }
}

#[test]
fn answers_questions_offers_and_summaries_are_not_mid_intent() {
    for text in [
        "",
        "What changed:",
        "I will keep this constraint in mind.",
        "Implemented and verified the fix: all targeted tests pass.",
        "Should I run the full suite now?",
        "Want me to run the tests?",
        "Let me know if you want the full diff.",
        "If you'd like, I can also add a regression test.",
        "Say the word and I'll implement it.",
        "Tests pass. Now the build is green and the branch is clean.",
        "Done. I read the manifest and the workspace looks healthy.",
        "Next steps:\n- run the suite\n- open the PR",
        "All set:\n\n```text\ncargo test: ok\n```",
        "Paste the login code here and I'll enter it right away.",
        "I'll wait for your approval before touching anything.",
    ] {
        assert!(!ends_mid_intent(text), "false positive: {text}");
    }
}

/// Replays a `(content, tool_calls)` script, repeating the last entry once it runs out.
struct ScriptedProvider {
    script: Vec<(String, Vec<forge_types::ToolCall>)>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for ScriptedProvider {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let step = self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (content, tool_calls) = self
            .script
            .get(step)
            .or_else(|| self.script.last())
            .cloned()
            .unwrap_or_default();
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

fn list_dir_call() -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: "call-list".into(),
        name: "list_dir".into(),
        args: serde_json::json!({"path": "."}),
    }
}

/// Runs one turn against the script; returns (main-loop provider calls, intent-nudge warnings,
/// final text). Side calls the session makes around a turn (title, recap) are subtracted by
/// measuring a one-reply baseline first.
async fn run_script(script: Vec<(&str, Vec<forge_types::ToolCall>)>) -> (usize, usize, String) {
    let (raw, nudges, text) = run_raw(script).await;
    let (baseline, _, _) = run_raw(vec![("Nothing to add.", vec![])]).await;
    (raw + 1 - baseline, nudges, text)
}

async fn run_raw(script: Vec<(&str, Vec<forge_types::ToolCall>)>) -> (usize, usize, String) {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let store = Arc::new(Store::open_in_memory().unwrap());
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let mut session = Session::start(
        store,
        Arc::new(ScriptedProvider {
            script: script
                .into_iter()
                .map(|(t, c)| (t.to_string(), c))
                .collect(),
            calls: Arc::clone(&calls),
        }),
        Arc::new(FixedRouter {
            model: "direct::scripted".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(capture),
        Config::default(),
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap();
    let outcome = session.run_turn("look around").await.unwrap();
    let nudges = events
        .lock()
        .unwrap()
        .iter()
        .filter(
            |e| matches!(e, PresenterEvent::Warning(w) if w.contains("announced another action")),
        )
        .count();
    (
        calls.load(std::sync::atomic::Ordering::Relaxed),
        nudges,
        outcome.text,
    )
}

#[tokio::test]
async fn intent_only_ending_is_nudged_then_proceeds_to_a_final_answer() {
    let (calls, nudges, text) = run_script(vec![
        ("Now let me look at the workspace layout:", vec![]),
        ("", vec![list_dir_call()]),
        ("The workspace is empty; nothing to change.", vec![]),
    ])
    .await;
    assert_eq!(nudges, 1);
    assert_eq!(calls, 3);
    assert_eq!(text, "The workspace is empty; nothing to change.");
}

#[tokio::test]
async fn nudges_rearm_after_progress_but_idle_answers_end_the_turn() {
    // stall, tool, stall, tool, stall x3: the first two stalls are each nudged (progress re-arms
    // the budget in between); then two consecutive idle answers exhaust it and the third stands.
    let (calls, nudges, _) = run_script(vec![
        ("Let me look around.", vec![]),
        ("", vec![list_dir_call()]),
        ("Next, I'll check the layout.", vec![]),
        ("", vec![list_dir_call()]),
        ("Now let me summarise what I found.", vec![]),
    ])
    .await;
    assert_eq!(nudges, 4, "two re-armed stretches of two nudges each");
    assert_eq!(calls, 7);
}

#[tokio::test]
async fn model_that_ignores_two_nudges_in_a_row_stops() {
    let (calls, nudges, text) = run_script(vec![("Let me look at the layout.", vec![])]).await;
    assert_eq!(nudges, 2);
    assert_eq!(
        calls, 3,
        "initial reply plus one per nudge, then the turn ends"
    );
    assert_eq!(text, "Let me look at the layout.");
}

#[tokio::test]
async fn question_to_the_user_is_not_nudged() {
    let (calls, nudges, text) = run_script(vec![(
        "Should I look at the layout now, or leave it?",
        vec![],
    )])
    .await;
    assert_eq!((calls, nudges), (1, 0));
    assert_eq!(text, "Should I look at the layout now, or leave it?");
}
