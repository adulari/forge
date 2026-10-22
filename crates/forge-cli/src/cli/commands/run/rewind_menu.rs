//! Step two of a rewind (docs/features/rewind.md): after the message is chosen, what to do with
//! it. Restore the code and the conversation (the old single action, still first so Enter Enter
//! does what Enter did), only the conversation, only the code, summarize from here, or go back.

use super::*;

/// A "summarize from here" the loop runs as a background task: it makes a model call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SummarizeRequest {
    pub seq: i64,
    pub focus: Option<String>,
}

/// The rewind menu's choices, in the order they are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RewindAction {
    All,
    Conversation,
    Code,
    Summarize,
    Back,
}

impl RewindAction {
    fn key(self) -> &'static str {
        match self {
            RewindAction::All => "all",
            RewindAction::Conversation => "chat",
            RewindAction::Code => "code",
            RewindAction::Summarize => "summary",
            RewindAction::Back => "back",
        }
    }

    fn parse(key: &str) -> Option<Self> {
        [
            RewindAction::All,
            RewindAction::Conversation,
            RewindAction::Code,
            RewindAction::Summarize,
            RewindAction::Back,
        ]
        .into_iter()
        .find(|a| a.key() == key)
    }

    /// Title and description. The list view gives a title 14 cells, so the titles stay short and
    /// the description says what it does.
    fn label(self) -> (&'static str, &'static str) {
        match self {
            RewindAction::All => (
                "Code and chat",
                "rewind the chat to before this message and put back the files changed since",
            ),
            RewindAction::Conversation => (
                "Chat only",
                "rewind the chat to before this message; files stay as they are",
            ),
            RewindAction::Code => (
                "Code only",
                "put back the files changed since this message; the chat stays",
            ),
            RewindAction::Summarize => (
                "Summarize",
                "fold this message and everything after it into a summary; files stay",
            ),
            RewindAction::Back => ("Never mind", "back to the message list"),
        }
    }
}

/// The action rows for the message at `seq`. The two code restores are only offered when a turn
/// from there on left file snapshots to put back.
fn action_rows(seq: i64, has_code: bool) -> Vec<forge_tui::PickerRow> {
    let actions: &[RewindAction] = if has_code {
        &[
            RewindAction::All,
            RewindAction::Conversation,
            RewindAction::Code,
            RewindAction::Summarize,
            RewindAction::Back,
        ]
    } else {
        &[
            RewindAction::Conversation,
            RewindAction::Summarize,
            RewindAction::Back,
        ]
    };
    actions
        .iter()
        .map(|a| {
            let (title, subtitle) = a.label();
            forge_tui::PickerRow {
                id: format!("{}@{seq}", a.key()),
                title: title.to_string(),
                subtitle: subtitle.to_string(),
            }
        })
        .collect()
}

fn parse_row(id: &str) -> Option<(RewindAction, i64)> {
    let (action, seq) = id.split_once('@')?;
    Some((RewindAction::parse(action)?, seq.parse().ok()?))
}

/// Open step two for the checkpoint row the user picked.
pub(crate) async fn open_rewind_actions(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    row: &forge_tui::PickerRow,
) -> Result<()> {
    let Ok(seq) = row.id.parse::<i64>() else {
        return Ok(());
    };
    let (has_code, prompt) = {
        let s = session.lock().await;
        (s.has_file_changes_from(seq), s.prompt_at(seq))
    };
    let about = prompt.unwrap_or_else(|| row.title.clone());
    let heading = format!(
        "restore from “{}”",
        forge_types::truncate_ellipsis(about.trim(), 48)
    );
    app.picker.open_with(
        forge_tui::PickerKind::RewindAction,
        &heading,
        action_rows(seq, has_code),
    );
    Ok(())
}

/// Act on a step-two row. The restores run here; a summary is handed back for the loop to spawn,
/// with the message's prompt already back in the input box.
pub(crate) async fn apply_rewind_action(
    row: &forge_tui::PickerRow,
    session: &Arc<tokio::sync::Mutex<Session>>,
    tui: Option<&mut forge_tui::Tui>,
    app: &mut forge_tui::App,
) -> Result<Option<SummarizeRequest>> {
    let Some((action, seq)) = parse_row(&row.id) else {
        return Ok(None);
    };
    let scope = match action {
        RewindAction::Back => {
            open_checkpoint_picker(session, app, "rewind to a message").await?;
            return Ok(None);
        }
        RewindAction::Summarize => {
            if let Some(prompt) = session.lock().await.prompt_at(seq) {
                app.input_cursor = prompt.len();
                app.input = prompt;
            }
            let focus = app.picker.last_query.trim().to_string();
            app.note("● summarizing from that point — files stay as they are");
            return Ok(Some(SummarizeRequest {
                seq,
                focus: (!focus.is_empty()).then_some(focus),
            }));
        }
        RewindAction::All => forge_core::RewindScope::All,
        RewindAction::Conversation => forge_core::RewindScope::Conversation,
        RewindAction::Code => forge_core::RewindScope::Code,
    };
    let (items, outcome) = {
        let mut s = session.lock().await;
        let outcome = s
            .rewind_scoped(seq, scope)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        (s.replay_items(), outcome)
    };
    if scope == forge_core::RewindScope::Code {
        app.note("● restored the files to that point — the conversation is unchanged");
        note_restore(app, &outcome.restore);
        if outcome.restore.is_empty() {
            app.note("no file changes from that point were left to restore");
        }
        return Ok(None);
    }
    if let Some(tui) = tui {
        tui.clear_screen();
    }
    app.clear_transcript();
    app.note(if scope == forge_core::RewindScope::All {
        "● rewound to that point"
    } else {
        "● rewound the conversation to that point — files stay as they are"
    });
    app.replay_history(&items);
    note_restore(app, &outcome.restore);
    // Put the rewound-to message back in the input box so it can be edited/resubmitted, with the
    // cursor after it: left at its old offset, typed text landed in the middle of the restored
    // prompt and Ctrl-U cleared nothing.
    if let Some(prompt) = outcome.rewound_prompt {
        app.input_cursor = prompt.len();
        app.input = prompt;
    }
    Ok(None)
}

/// Spawn a "summarize from here" as a background task, like `/compact`: the spinner ticks while
/// the summary is written.
pub(crate) fn spawn_summarize_from(
    request: SummarizeRequest,
    session: &Arc<tokio::sync::Mutex<Session>>,
    done_tx: &std::sync::mpsc::Sender<u64>,
    gen: u64,
    app: &mut forge_tui::App,
    busy: &mut bool,
    busy_since: &mut std::time::Instant,
) -> tokio::task::JoinHandle<()> {
    app.done = false;
    app.tick = 0;
    *busy = true;
    *busy_since = std::time::Instant::now();
    let s = session.clone();
    let dt = done_tx.clone();
    tokio::spawn(async move {
        let _done = DoneGuard(dt, gen);
        let mut sess = s.lock().await;
        match sess
            .summarize_from(request.seq, request.focus.as_deref())
            .await
        {
            Ok(outcome) if outcome.folded == 0 => {
                sess.notify_warning("nothing to summarize from that point");
            }
            Ok(_) => {}
            Err(e) => sess.notify_error(&format!("summarize failed: {e}")),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_round_trip_their_action_and_seq() {
        let rows = action_rows(42, true);
        let parsed: Vec<_> = rows.iter().map(|r| parse_row(&r.id).unwrap()).collect();
        assert_eq!(
            parsed,
            [
                (RewindAction::All, 42),
                (RewindAction::Conversation, 42),
                (RewindAction::Code, 42),
                (RewindAction::Summarize, 42),
                (RewindAction::Back, 42),
            ]
        );
        assert_eq!(rows[0].title, "Code and chat");
        assert_eq!(parse_row("nonsense"), None);
        assert_eq!(parse_row("all@x"), None);
    }

    #[test]
    fn without_file_changes_only_the_conversation_choices_are_offered() {
        let titles: Vec<String> = action_rows(7, false).into_iter().map(|r| r.title).collect();
        assert_eq!(titles, ["Chat only", "Summarize", "Never mind"]);
    }
}
