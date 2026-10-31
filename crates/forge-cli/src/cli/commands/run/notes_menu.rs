//! The TUI side of `/notes`: loads the project's rows from the store and performs what
//! `forge_tui::notes_overlay::NotesAction` asks for. "Start thread" asks the local `forge serve`
//! daemon for a session seeded with the item, so the thread shows up on every connected surface.

use forge_store::ScratchpadSave;
use forge_tui::notes_overlay::{NotesAction, NotesSnapshot, TodoView, Tone};
use forge_tui::{App, KeyKind};
use serde_json::json;
use tokio::sync::mpsc;

use crate::cli::commands::dispatch::Daemon;
use crate::cli::commands::replay::open_store;

enum Msg {
    Thread {
        id: String,
        result: Result<String, String>,
    },
}

pub(crate) struct NotesMenuDriver {
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
    starting: bool,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn snapshot(project: &str) -> Result<NotesSnapshot, String> {
    let store = open_store().map_err(|e| e.to_string())?;
    let notes = store.project_notes(project).map_err(|e| e.to_string())?;
    Ok(NotesSnapshot {
        project: notes.project,
        body: notes.body,
        updated_at: notes.updated_at,
        todos: notes
            .todos
            .into_iter()
            .map(|todo| TodoView {
                id: todo.id,
                text: todo.text,
                done: todo.done,
                thread: todo
                    .session_id
                    .map(|id| id.chars().take(8).collect::<String>()),
            })
            .collect(),
    })
}

async fn start_thread(project: String, text: String) -> Result<String, String> {
    let token = crate::attach::resolve_token(None)
        .map_err(|e| format!("{e} Start the daemon with: forge serve --local"))?;
    let http = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let daemon = Daemon::new(http, crate::attach::resolve_base_url(None), token);
    let title: String = text
        .lines()
        .next()
        .unwrap_or(&text)
        .chars()
        .take(60)
        .collect();
    let body = json!({ "cwd": project, "title": title, "prompt": text });
    let reply = daemon
        .post("sessions", &body)
        .await
        .map_err(|e| e.to_string())?;
    reply
        .get("id")
        .and_then(|id| id.as_str())
        .map(str::to_string)
        .ok_or_else(|| "the daemon did not return a session id".to_string())
}

impl NotesMenuDriver {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            starting: false,
        }
    }

    /// Apply finished work and load rows the overlay asked for. True when a redraw is due.
    ///
    /// A started thread is linked to its to-do whether or not the overlay is still open: the
    /// session already exists on the daemon, and dropping the reply would make the next Enter
    /// start a duplicate. Only the on-screen notice depends on the overlay.
    pub(crate) fn tick(&mut self, app: &mut App) -> bool {
        let open = app.notes_overlay.open;
        let mut dirty = false;
        if open && app.notes_overlay.take_reload() {
            self.reload(app);
            dirty = true;
        }
        while let Ok(Msg::Thread { id, result }) = self.rx.try_recv() {
            self.starting = false;
            dirty |= open;
            match result {
                Ok(session) => {
                    let linked = open_store()
                        .and_then(|s| Ok(s.link_project_todo_session(&id, &session, now_ms())?));
                    if open {
                        match linked {
                            Err(error) => app.notes_overlay.notify(
                                Tone::Warn,
                                format!("Thread started, but not linked: {error}"),
                            ),
                            Ok(_) => {
                                let short: String = session.chars().take(8).collect();
                                app.notes_overlay.notify(
                                    Tone::Ok,
                                    format!("Thread {short} started. Open it in the app or forge board."),
                                );
                            }
                        }
                        self.reload(app);
                    }
                }
                Err(error) if open => app.notes_overlay.notify(Tone::Warn, error),
                Err(_) => {}
            }
        }
        dirty
    }

    fn reload(&mut self, app: &mut App) {
        let project = app.notes_overlay.data.project.clone();
        match snapshot(&project) {
            Ok(snap) => app.notes_overlay.apply(snap),
            Err(error) => app.notes_overlay.notify(Tone::Warn, error),
        }
    }

    pub(crate) fn key(&mut self, app: &mut App, key: KeyKind) {
        let action = app.notes_overlay.key(key);
        let project = app.notes_overlay.data.project.clone();
        let store = || open_store().map_err(|e| e.to_string());
        let result: Result<(), String> = match action {
            NotesAction::None | NotesAction::Close => return,
            NotesAction::Reload => {
                self.reload(app);
                return;
            }
            NotesAction::Add(text) => store().and_then(|s| {
                s.add_project_todo(&project, &text, now_ms())
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }),
            NotesAction::Edit { id, text } => store().and_then(|s| {
                s.update_project_todo(&id, Some(&text), None, now_ms())
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }),
            NotesAction::Toggle { id, done } => store().and_then(|s| {
                s.update_project_todo(&id, None, Some(done), now_ms())
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }),
            NotesAction::Delete(id) => store().and_then(|s| {
                s.delete_project_todo(&id)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }),
            NotesAction::ClearDone => store().and_then(|s| {
                s.clear_done_project_todos(&project)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }),
            NotesAction::SavePad {
                body,
                expected_updated_at,
            } => {
                let saved = store().and_then(|s| {
                    s.set_project_scratchpad_if(
                        &project,
                        &body,
                        Some(expected_updated_at),
                        now_ms(),
                    )
                    .map_err(|e| e.to_string())
                });
                match saved {
                    Ok(ScratchpadSave::Saved { .. }) => Ok(()),
                    Ok(ScratchpadSave::Conflict { updated_at, .. }) => {
                        app.notes_overlay.pad_conflict(body, updated_at);
                        return;
                    }
                    Err(error) => Err(error),
                }
            }
            NotesAction::StartThread { id, text } => {
                if self.starting {
                    return;
                }
                self.starting = true;
                app.notes_overlay.notify(Tone::Ok, "Starting a thread...");
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = start_thread(project, text).await;
                    let _ = tx.send(Msg::Thread { id, result });
                });
                return;
            }
        };
        match result {
            Ok(()) => self.reload(app),
            Err(error) => app.notes_overlay.notify(Tone::Warn, error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_that_finishes_after_the_overlay_closed_is_still_linked() {
        let project = format!("/w/notes-menu-test-{}", std::process::id());
        let store = open_store().unwrap();
        let todo = store.add_project_todo(&project, "ship it", 1).unwrap();

        let mut driver = NotesMenuDriver::new();
        let mut app = App::default();
        assert!(!app.notes_overlay.open, "the overlay was closed mid-start");
        driver.starting = true;
        driver
            .tx
            .send(Msg::Thread {
                id: todo.id.clone(),
                result: Ok("session-abcdef123".into()),
            })
            .unwrap();
        driver.tick(&mut app);

        assert!(!driver.starting);
        let linked = store.project_todo(&todo.id).unwrap().unwrap();
        assert_eq!(linked.session_id.as_deref(), Some("session-abcdef123"));
    }

    #[test]
    fn a_failed_start_while_closed_links_nothing() {
        let project = format!("/w/notes-menu-fail-{}", std::process::id());
        let store = open_store().unwrap();
        let todo = store.add_project_todo(&project, "retry me", 1).unwrap();
        let mut driver = NotesMenuDriver::new();
        let mut app = App::default();
        driver
            .tx
            .send(Msg::Thread {
                id: todo.id.clone(),
                result: Err("daemon down".into()),
            })
            .unwrap();
        driver.tick(&mut app);
        assert!(store
            .project_todo(&todo.id)
            .unwrap()
            .unwrap()
            .session_id
            .is_none());
    }
}
