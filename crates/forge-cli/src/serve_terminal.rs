//! `forge serve`'s terminal dock — `WS {base}/ws/terminal?session=<id>`.
//!
//! A real login shell on a real PTY, spawned in the session's own directory (its worktree when it
//! has one, else its cwd). Frames:
//! - client → server: `{"kind":"input","data":"<utf-8>"}` and `{"kind":"resize","cols":N,"rows":M}`
//! - server → client: binary frames of RAW pty bytes (terminal output is not valid UTF-8 in
//!   general — decoding server-side would corrupt escape sequences and split multi-byte glyphs)
//!
//! Security posture: this adds NO privilege. The daemon token is already full agent control over
//! the same machine (it can start a session that runs arbitrary shell commands), so the terminal is
//! gated by exactly that one token and nothing more. It refuses to open when the session has no
//! usable working directory, and it never accepts a directory from the request.
//!
//! Resource posture: output flows through a bounded channel and the reader uses a BLOCKING send, so
//! a runaway `yes` back-pressures into the PTY buffer and then into the child — the daemon's memory
//! ceiling is [`OUTPUT_CHANNEL_CHUNKS`] × [`READ_CHUNK_BYTES`], not the child's output rate.

use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;

use crate::serve::{err_response, DaemonState};

/// Bytes per PTY read. Big enough that a fast writer costs few syscalls, small enough that an
/// interactive keystroke echo is not delayed waiting for a full buffer.
const READ_CHUNK_BYTES: usize = 8 * 1024;

/// In-flight output chunks. With [`READ_CHUNK_BYTES`] this caps daemon-side buffering for one
/// terminal at ~512 KiB; past that the reader blocks and the child is back-pressured.
const OUTPUT_CHANNEL_CHUNKS: usize = 64;

/// Queued client keystroke/paste chunks. Small: input is human-rate, and an unbounded queue here
/// would let a scripted client buffer megabytes into the daemon.
const INPUT_CHANNEL_CHUNKS: usize = 32;

/// Largest single `input` payload accepted. A paste is legitimate; a megabyte is an attack on the
/// PTY writer.
const MAX_INPUT_BYTES: usize = 64 * 1024;

/// Terminal geometry bounds — a client-supplied `resize` reaches an ioctl, so it is clamped.
const MAX_COLS: u16 = 1_000;
const MAX_ROWS: u16 = 1_000;

#[derive(serde::Deserialize)]
pub(crate) struct TerminalQuery {
    #[serde(default)]
    session: String,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
}

/// Client → server frames.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TerminalFrame {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

/// The interactive shell to spawn. Unlike [`forge_tools`]'s one-shot `-c` invocation this must be
/// an INTERACTIVE shell (no command), so the user gets their prompt, aliases, and job control.
fn login_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn clamp_size(cols: Option<u16>, rows: Option<u16>) -> (u16, u16) {
    (
        cols.unwrap_or(80).clamp(1, MAX_COLS),
        rows.unwrap_or(24).clamp(1, MAX_ROWS),
    )
}

/// `WS {base}/ws/terminal?session=<id>[&cols=&rows=]`.
pub(crate) async fn terminal_ws(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<TerminalQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(handle) = state.registry.get(&params.session).await else {
        return err_response(axum::http::StatusCode::NOT_FOUND, "no such session");
    };
    let cwd = handle
        .worktree
        .clone()
        .unwrap_or_else(|| handle.cwd.clone());
    // No directory ⇒ no terminal. Never fall back to the daemon's cwd: the dock is scoped to the
    // session the client asked for, and silently landing elsewhere would be a surprise shell.
    if cwd.is_empty() || !std::path::Path::new(&cwd).is_dir() {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "session has no working directory on this host",
        );
    }
    let (cols, rows) = clamp_size(params.cols, params.rows);
    ws.on_upgrade(move |socket| pump_terminal(socket, cwd, cols, rows))
}

async fn pump_terminal(mut socket: WebSocket, cwd: String, cols: u16, rows: u16) {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    let size = PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    };
    let pair = match native_pty_system().openpty(size) {
        Ok(pair) => pair,
        Err(error) => {
            let _ = socket
                .send(WsMessage::Text(
                    format!("failed to open a pty: {error}\r\n").into(),
                ))
                .await;
            return;
        }
    };

    let mut command = CommandBuilder::new(login_shell());
    command.cwd(&cwd);
    // Without a TERM the shell assumes "dumb" and emits no colour or line editing.
    command.env("TERM", "xterm-256color");
    let mut child = match pair.slave.spawn_command(command) {
        Ok(child) => child,
        Err(error) => {
            let _ = socket
                .send(WsMessage::Text(
                    format!("failed to start a shell in {cwd}: {error}\r\n").into(),
                ))
                .await;
            return;
        }
    };
    // Drop our copy of the slave so the master sees EOF as soon as the child exits.
    drop(pair.slave);

    let (mut reader, writer) = match (pair.master.try_clone_reader(), pair.master.take_writer()) {
        (Ok(reader), Ok(writer)) => (reader, writer),
        _ => {
            let _ = child.kill();
            let _ = socket
                .send(WsMessage::Text("failed to attach to the pty\r\n".into()))
                .await;
            return;
        }
    };
    let mut killer = child.clone_killer();
    let master = pair.master;

    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(OUTPUT_CHANNEL_CHUNKS);
    let reader_task = tokio::task::spawn_blocking(move || {
        let mut buffer = [0u8; READ_CHUNK_BYTES];
        loop {
            match std::io::Read::read(&mut reader, &mut buffer) {
                Ok(0) => break,
                // `blocking_send` is the whole back-pressure story: a slow/stalled client stops
                // this read loop, the PTY buffer fills, and the child blocks on write — exactly
                // what a real terminal does, and it cannot grow the daemon's heap.
                Ok(n) => {
                    if out_tx.blocking_send(buffer[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break, // EIO once the master fd is closed
            }
        }
    });

    let (in_tx, mut in_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(INPUT_CHANNEL_CHUNKS);
    let writer_task = tokio::task::spawn_blocking(move || {
        let mut writer = writer;
        while let Some(chunk) = in_rx.blocking_recv() {
            use std::io::Write;
            if writer.write_all(&chunk).is_err() || writer.flush().is_err() {
                break;
            }
        }
    });

    let (exit_tx, mut exit_rx) = tokio::sync::mpsc::channel::<()>(1);
    let wait_task = tokio::task::spawn_blocking(move || {
        let _ = child.wait();
        let _ = exit_tx.blocking_send(());
    });

    loop {
        tokio::select! {
            chunk = out_rx.recv() => {
                let Some(chunk) = chunk else { break };
                if socket.send(WsMessage::Binary(chunk.into())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(WsMessage::Text(text))) => {
                        match serde_json::from_str::<TerminalFrame>(&text) {
                            Ok(TerminalFrame::Input { data }) => {
                                if data.len() > MAX_INPUT_BYTES {
                                    continue;
                                }
                                if in_tx.send(data.into_bytes()).await.is_err() {
                                    break;
                                }
                            }
                            Ok(TerminalFrame::Resize { cols, rows }) => {
                                let (cols, rows) = clamp_size(Some(cols), Some(rows));
                                let _ = master.resize(PtySize {
                                    rows,
                                    cols,
                                    pixel_width: 0,
                                    pixel_height: 0,
                                });
                            }
                            // An unparseable frame is a client bug, not a reason to drop a shell
                            // the user may have work in.
                            Err(_) => continue,
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
            _ = exit_rx.recv() => break,
        }
    }

    // Tear the child down on disconnect: an orphaned interactive shell would otherwise sit on the
    // daemon's process table forever, holding the worktree open.
    let _ = killer.kill();
    drop(master);
    drop(in_tx);
    reader_task.abort();
    writer_task.abort();
    wait_task.abort();
    let _ = socket.send(WsMessage::Close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_clamped_to_sane_geometry() {
        assert_eq!(clamp_size(None, None), (80, 24));
        assert_eq!(clamp_size(Some(0), Some(0)), (1, 1));
        assert_eq!(clamp_size(Some(9_000), Some(9_000)), (MAX_COLS, MAX_ROWS));
    }

    #[test]
    fn frames_parse_by_kind_tag() {
        let input: TerminalFrame =
            serde_json::from_str(r#"{"kind":"input","data":"ls\n"}"#).unwrap();
        assert!(matches!(input, TerminalFrame::Input { data } if data == "ls\n"));
        let resize: TerminalFrame =
            serde_json::from_str(r#"{"kind":"resize","cols":120,"rows":40}"#).unwrap();
        assert!(matches!(
            resize,
            TerminalFrame::Resize {
                cols: 120,
                rows: 40
            }
        ));
        assert!(serde_json::from_str::<TerminalFrame>(r#"{"kind":"exec"}"#).is_err());
    }
}
