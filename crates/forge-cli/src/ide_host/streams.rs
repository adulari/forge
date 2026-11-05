//! The long-lived sockets of IDE mode: the daemon terminal, check output followers and `$EDITOR`.

use std::time::Duration;

use forge_tui::board::ToastLevel;
use forge_tui::ide::wire::WireFile;
use forge_tui::ide::{IdeEvent, SaveFailure};
use tokio::sync::mpsc;

use super::{get, Api, CHECK_FOLLOW, TERMINAL_ID};
use crate::board::client::ws_url;
use crate::board::Ev;

// ---------------------------------------------------------------------------
// Terminal and check streams
// ---------------------------------------------------------------------------

pub(super) enum TermCmd {
    Input(Vec<u8>),
    Resize(u16, u16),
    Close,
}

pub(super) struct TermLink {
    pub(super) cmds: mpsc::UnboundedSender<TermCmd>,
    pub(super) task: tokio::task::JoinHandle<()>,
}

/// What the host keeps between actions: the terminal socket and the check output followers.
#[derive(Default)]
pub(super) struct Runtime {
    pub(super) term: Option<TermLink>,
    pub(super) checks: std::collections::HashMap<(String, String), tokio::task::JoinHandle<()>>,
}

impl Runtime {
    pub(super) fn term_cmd(&mut self, cmd: TermCmd) {
        if let Some(link) = &self.term {
            let _ = link.cmds.send(cmd);
        }
    }

    pub(super) fn close_terminal(&mut self) {
        if let Some(link) = self.term.take() {
            let _ = link.cmds.send(TermCmd::Close);
            let task = link.task;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                task.abort();
            });
        }
    }

    /// Leaving IDE mode: the shell keeps running on the daemon, only the sockets go.
    pub(super) fn shutdown(&mut self) {
        if let Some(link) = self.term.take() {
            link.task.abort();
        }
        for (_, handle) in self.checks.drain() {
            handle.abort();
        }
    }
}

pub(super) fn text_frame(value: serde_json::Value) -> tokio_tungstenite::tungstenite::Message {
    tokio_tungstenite::tungstenite::Message::Text(value.to_string().into())
}

pub(super) async fn term_task(
    base: String,
    token: String,
    session: String,
    size: (u16, u16),
    restart: bool,
    ev: mpsc::UnboundedSender<Ev>,
    mut cmds: mpsc::UnboundedReceiver<TermCmd>,
) {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let send = |event: IdeEvent| {
        let _ = ev.send(Ev::Ide(event));
    };
    let path = format!(
        "ws/terminal?session={session}&terminal={TERMINAL_ID}&cols={}&rows={}{}",
        size.0,
        size.1,
        if restart { "&restart=true" } else { "" }
    );
    let url = match ws_url(&base, &token, &path) {
        Ok(url) => url,
        Err(e) => {
            send(IdeEvent::Toast(ToastLevel::Error, e.to_string()));
            send(IdeEvent::TermEnded(None));
            return;
        }
    };
    let socket = match tokio_tungstenite::connect_async(&url).await {
        Ok((socket, _)) => socket,
        Err(e) => {
            send(IdeEvent::Toast(
                ToastLevel::Error,
                format!("could not open a terminal on the daemon: {e}"),
            ));
            send(IdeEvent::TermEnded(None));
            return;
        }
    };
    let (mut tx, mut rx) = socket.split();
    loop {
        tokio::select! {
            message = rx.next() => match message {
                Some(Ok(Message::Binary(bytes))) => send(IdeEvent::TermBytes(bytes.to_vec())),
                Some(Ok(Message::Text(text))) => {
                    let Ok(frame) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                    match frame["kind"].as_str() {
                        Some("status") if frame["status"].as_str() == Some("exited") => {
                            send(IdeEvent::TermEnded(frame["exit_code"].as_i64()));
                        }
                        Some("cleared") => send(IdeEvent::TermBytes(b"\x1b[2J\x1b[H".to_vec())),
                        _ => {}
                    }
                }
                Some(Ok(_)) => {}
                _ => {
                    send(IdeEvent::TermEnded(None));
                    return;
                }
            },
            command = cmds.recv() => match command {
                None => return,
                Some(TermCmd::Input(bytes)) => {
                    let data = String::from_utf8_lossy(&bytes).into_owned();
                    if tx.send(text_frame(serde_json::json!({ "kind": "input", "data": data }))).await.is_err() {
                        return;
                    }
                }
                Some(TermCmd::Resize(cols, rows)) => {
                    let _ = tx.send(text_frame(serde_json::json!({ "kind": "resize", "cols": cols, "rows": rows }))).await;
                }
                Some(TermCmd::Close) => {
                    let _ = tx.send(text_frame(serde_json::json!({ "kind": "close" }))).await;
                    return;
                }
            },
        }
    }
}

/// Follow a check's terminal: its history first, then live output, until the run ends.
pub(super) async fn stream_check(api: &Api, session: &str, action: &str, terminal_id: &str) {
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    let path = format!("ws/terminal?session={session}&terminal={terminal_id}");
    let Ok(url) = ws_url(&api.base, &api.token, &path) else {
        return;
    };
    let Ok((mut socket, _)) = tokio_tungstenite::connect_async(&url).await else {
        return;
    };
    let mut first = true;
    let mut deadline = tokio::time::Instant::now() + CHECK_FOLLOW;
    loop {
        let next = tokio::time::timeout_at(deadline, socket.next()).await;
        let Ok(Some(Ok(message))) = next else { break };
        match message {
            Message::Binary(bytes) => {
                api.send(IdeEvent::CheckOutput {
                    session_id: session.to_string(),
                    action: action.to_string(),
                    text: String::from_utf8_lossy(&bytes).into_owned(),
                    reset: std::mem::take(&mut first),
                });
            }
            Message::Text(text) => {
                let exited = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .is_some_and(|v| v["kind"] == "status" && v["status"] == "exited");
                // The status can arrive before the replayed history: read on until it goes quiet.
                if exited {
                    deadline =
                        deadline.min(tokio::time::Instant::now() + Duration::from_millis(600));
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    api.checks(session).await;
}

/// `E`: leave the screen, run the user's editor on a copy of the file, and save the result back
/// with the same hash check the built-in editor uses.
pub(super) async fn external_edit(
    api: &Api,
    session: &str,
    path: &str,
    refresh: &mpsc::UnboundedSender<()>,
) {
    let toast = |level: ToastLevel, text: String| api.send(IdeEvent::Toast(level, text));
    let loaded = match get::<WireFile>(
        &api.http,
        &api.base,
        &api.token,
        "api/workspace/file",
        &[("session", session), ("path", path)],
    )
    .await
    {
        Ok(file) => file,
        Err(e) => return toast(ToastLevel::Error, e.message),
    };
    if loaded.read_only {
        return toast(ToastLevel::Info, "this file is read-only".to_string());
    }
    let dir = std::env::temp_dir().join(format!("forge-ide-{}", std::process::id()));
    let name = std::path::Path::new(path)
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_os_string());
    let temp = dir.join(name);
    if std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&temp, &loaded.content))
        .is_err()
    {
        return toast(
            ToastLevel::Error,
            "could not write a temporary copy".to_string(),
        );
    }
    let editor = forge_tui::external_editor_command();
    let run_path = temp.clone();
    let ran = tokio::task::spawn_blocking(move || forge_tui::run_editor_on(&editor, &run_path))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    let edited = std::fs::read_to_string(&temp);
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = ran {
        return toast(ToastLevel::Error, e);
    }
    let Ok(edited) = edited else {
        return toast(
            ToastLevel::Error,
            "could not read the edited file".to_string(),
        );
    };
    if edited == loaded.content {
        return toast(ToastLevel::Info, "no changes".to_string());
    }
    match api
        .save_inner(session, path, edited, Some(loaded.hash))
        .await
    {
        Ok(_) => {
            toast(ToastLevel::Ok, format!("saved {path}"));
            api.file(session, path).await;
            let _ = refresh.send(());
        }
        Err(SaveFailure::Conflict) => toast(
            ToastLevel::Error,
            "agent changed this file while you edited: your copy was not saved".to_string(),
        ),
        Err(SaveFailure::Other(message)) => toast(ToastLevel::Error, message),
        Err(_) => toast(ToastLevel::Error, "could not save the file".to_string()),
    }
}
