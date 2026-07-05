//! `forge attach <id>` — a thin terminal client for a running `forge serve` daemon.
//!
//! It CONSUMES the daemon's public surface (it never imports the driver): `GET /api/sessions`
//! to discover/resolve a session, the per-session WebSocket `/ws?session=<id>&rev=<n>` for the
//! live [`crate::remote::Snapshot`] stream, and `RemoteInput` JSON back over the same socket to
//! submit prompts and answer permission prompts / questions. Auth is exactly the web PWA's: the
//! daemon token is the leading path segment (`/<token>/…`); a wrong token is a 404. Defaults
//! target the local daemon (loopback + the persisted `serve-token`); `--url` / `--token` override.

use std::io::Write as _;

use anyhow::{bail, Context, Result};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

/// One row of `GET /api/sessions` — only the fields the client renders (the daemon sends more).
#[derive(Debug, Clone, serde::Deserialize)]
struct SessionInfo {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    busy: bool,
    #[serde(default)]
    waiting: bool,
    #[serde(default)]
    cost_usd: f64,
    #[serde(default)]
    model: String,
}

/// A pending question's option (tappable button on the web page; numbered line here).
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct SnapOption {
    #[serde(default)]
    label: String,
}

/// The subset of the daemon's `Snapshot` this client deserializes and renders. Kept independent
/// of `crate::remote::Snapshot` on purpose: that type is `Serialize`-only (server → wire), and a
/// separate `Deserialize` view lets the client tolerate extra/absent fields across versions.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct ClientSnapshot {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    busy: bool,
    #[serde(default)]
    cost_usd: f64,
    #[serde(default)]
    transcript: Vec<String>,
    #[serde(default)]
    permission_prompt: Option<String>,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    question_options: Vec<SnapOption>,
    #[serde(default)]
    prompt_seq: u64,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    closed: bool,
}

/// What the session is currently blocked on — drives how the next typed line is interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    None,
    Permission(u64),
    Question(u64),
}

pub(crate) async fn attach_cmd(
    id: Option<String>,
    url: Option<String>,
    token: Option<String>,
    list: bool,
) -> Result<()> {
    let base = resolve_base_url(url);
    let token = resolve_token(token)?;
    let http = reqwest::Client::new();

    // One fetch does triple duty: connectivity check, token check, and session resolution.
    let sessions = fetch_sessions(&http, &base, &token).await?;

    let Some(id) = id else {
        print_session_list(&sessions);
        if !list && sessions.is_empty() {
            return Ok(());
        }
        if !list {
            println!("\nattach with:  forge attach <id>   (a unique prefix works)");
        }
        return Ok(());
    };

    if list {
        print_session_list(&sessions);
        return Ok(());
    }

    let full_id = resolve_session_id(&sessions, &id)?;
    run_attach(&base, &token, &full_id).await
}

/// Default base URL: the local daemon on the configured `[remote] port` (7420), loopback. The
/// LAN default binds self-signed HTTPS which a same-machine attach can't validate, so the
/// no-flag default is loopback HTTP — pair with `forge serve --local`, or pass `--url`.
fn resolve_base_url(url: Option<String>) -> String {
    let raw = url.unwrap_or_else(|| {
        let port = forge_config::load().unwrap_or_default().remote.serve_port();
        format!("http://127.0.0.1:{port}")
    });
    raw.trim_end_matches('/').to_string()
}

/// Default token: the persisted `serve-token` the daemon reads (never rotate from the client).
fn resolve_token(token: Option<String>) -> Result<String> {
    match token {
        Some(t) => Ok(t),
        None => crate::serve::daemon_token(false).context(
            "no --token given and no persisted daemon token found — start `forge serve` once, \
             or pass --token",
        ),
    }
}

/// Convert an `http(s)://` base to its `ws(s)://` scheme for the WebSocket route.
fn ws_scheme(base: &str) -> Result<String> {
    if let Some(rest) = base.strip_prefix("https://") {
        Ok(format!("wss://{rest}"))
    } else if let Some(rest) = base.strip_prefix("http://") {
        Ok(format!("ws://{rest}"))
    } else {
        bail!("--url must start with http:// or https:// (got {base})")
    }
}

async fn fetch_sessions(
    http: &reqwest::Client,
    base: &str,
    token: &str,
) -> Result<Vec<SessionInfo>> {
    let url = format!("{base}/{token}/api/sessions");
    let resp = http.get(&url).send().await.map_err(|e| {
        anyhow::anyhow!(
            "could not reach the forge serve daemon at {base} — is it running? \
             (start it with `forge serve --local`)  [{e}]"
        )
    })?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("daemon rejected the token (404) — wrong --token, or the daemon rotated it");
    }
    if !resp.status().is_success() {
        bail!("daemon returned {} for {url}", resp.status());
    }
    resp.json::<Vec<SessionInfo>>()
        .await
        .context("daemon /api/sessions response was not the expected JSON")
}

/// Resolve a full session id from an exact id or a unique prefix; ambiguity/absence is an error.
fn resolve_session_id(sessions: &[SessionInfo], needle: &str) -> Result<String> {
    if let Some(exact) = sessions.iter().find(|s| s.id == needle) {
        return Ok(exact.id.clone());
    }
    let matches: Vec<&SessionInfo> = sessions
        .iter()
        .filter(|s| s.id.starts_with(needle))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => {
            if sessions.is_empty() {
                bail!("no sessions are running on this daemon");
            }
            let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
            bail!("no session matches {needle:?}. running: {}", ids.join(", "))
        }
        many => {
            let ids: Vec<&str> = many.iter().map(|s| s.id.as_str()).collect();
            bail!("{needle:?} is ambiguous — matches: {}", ids.join(", "))
        }
    }
}

fn print_session_list(sessions: &[SessionInfo]) {
    if sessions.is_empty() {
        println!("no sessions are running on this daemon.");
        return;
    }
    println!("running sessions:");
    for s in sessions {
        let state = if s.waiting {
            "WAITING"
        } else if s.busy {
            "busy"
        } else {
            "idle"
        };
        let title = if s.title.is_empty() {
            "(untitled)"
        } else {
            s.title.as_str()
        };
        println!(
            "  {}  [{state}]  {title}  ({}, ${:.4})  {}",
            s.id, s.model, s.cost_usd, s.cwd
        );
    }
}

async fn run_attach(base: &str, token: &str, id: &str) -> Result<()> {
    let ws_base = ws_scheme(base)?;
    let ws_url = format!("{ws_base}/{token}/ws?session={id}&rev=0");
    let (ws, _resp) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| map_ws_error(e, id))?;
    let (mut ws_tx, mut ws_rx) = ws.split();

    println!("⚒ attached to session {id} — type a prompt and press Enter.");
    println!("   commands: /q quit · /i interrupt (stop the current turn)");
    println!("   when a permission prompt appears, answer with  y  or  n\n");

    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut line = String::new();
    let mut renderer = Renderer::default();
    let mut pending = Pending::None;

    loop {
        line.clear();
        tokio::select! {
            biased;
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(t))) => {
                        match serde_json::from_str::<ClientSnapshot>(&t) {
                            Ok(snap) => {
                                if snap.closed {
                                    renderer.render(&snap, &mut pending);
                                    println!("\n⚒ session closed by the daemon — detaching.");
                                    break;
                                }
                                renderer.render(&snap, &mut pending);
                            }
                            Err(e) => eprintln!("⚠ skipped an unparseable frame: {e}"),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        println!("\n⚒ connection closed — detaching.");
                        break;
                    }
                    Some(Ok(_)) => {} // ping/pong/binary — ignored
                    Some(Err(e)) => {
                        eprintln!("\n⚠ websocket error: {e}");
                        break;
                    }
                }
            }
            read = read_line(&mut stdin, &mut line) => {
                match read {
                    Ok(0) => {
                        // stdin EOF (Ctrl-D / piped input drained): detach, leave the session running.
                        println!("\n⚒ input closed — detaching (the session keeps running).");
                        break;
                    }
                    Ok(_) => {
                        if let Some(msg) = interpret_input(line.trim(), &pending) {
                            match msg {
                                Action::Quit => {
                                    println!("⚒ detaching (the session keeps running).");
                                    break;
                                }
                                Action::Send(json) => {
                                    let text = json.to_string();
                                    if ws_tx.send(Message::Text(text.into())).await.is_err() {
                                        println!("\n⚒ connection dropped while sending — detaching.");
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("⚠ stdin error: {e}");
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

async fn read_line(
    stdin: &mut tokio::io::BufReader<tokio::io::Stdin>,
    buf: &mut String,
) -> std::io::Result<usize> {
    use tokio::io::AsyncBufReadExt;
    stdin.read_line(buf).await
}

/// What a typed line resolves to.
enum Action {
    Quit,
    Send(serde_json::Value),
}

/// Turn a typed line into a `RemoteInput` JSON (matching the daemon's tagged enum) given what the
/// session is currently blocked on. Returns `None` for a no-op (blank line, or a bad answer while
/// a prompt is pending — with a printed hint).
fn interpret_input(line: &str, pending: &Pending) -> Option<Action> {
    if line.is_empty() {
        return None;
    }
    match line {
        "/q" | "/quit" | "/exit" => return Some(Action::Quit),
        "/i" | "/interrupt" => {
            return Some(Action::Send(serde_json::json!({ "kind": "interrupt" })));
        }
        _ => {}
    }
    match pending {
        Pending::Permission(seq) => {
            let yes = matches!(line.to_ascii_lowercase().as_str(), "y" | "yes" | "allow");
            let no = matches!(line.to_ascii_lowercase().as_str(), "n" | "no" | "deny");
            if !yes && !no {
                println!("  (a permission prompt is pending — answer  y  or  n)");
                return None;
            }
            Some(Action::Send(serde_json::json!({
                "kind": "allow",
                "yes": yes,
                "seq": seq,
            })))
        }
        Pending::Question(seq) => Some(Action::Send(serde_json::json!({
            "kind": "answer",
            "text": line,
            "seq": seq,
        }))),
        Pending::None => Some(Action::Send(serde_json::json!({
            "kind": "prompt",
            "text": line,
        }))),
    }
}

/// Incremental line-oriented renderer: the daemon re-sends a full (bounded, sliding) snapshot each
/// frame, so this prints only what's new since the last frame and never re-prints scrollback.
#[derive(Default)]
struct Renderer {
    header_shown: bool,
    printed_transcript: Vec<String>,
    printed_notes: Vec<String>,
    shown_prompt_seq: Option<u64>,
    was_busy: bool,
}

impl Renderer {
    fn render(&mut self, snap: &ClientSnapshot, pending: &mut Pending) {
        if !self.header_shown {
            let title = if snap.title.is_empty() {
                "(untitled)"
            } else {
                snap.title.as_str()
            };
            println!("── {} · {title} · {} ──", snap.session_id, snap.cwd);
            println!("   model: {}   cost: ${:.4}\n", snap.model, snap.cost_usd);
            self.header_shown = true;
        }

        for l in new_suffix(&self.printed_transcript, &snap.transcript) {
            println!("{l}");
        }
        self.printed_transcript = snap.transcript.clone();

        for n in new_suffix(&self.printed_notes, &snap.notes) {
            println!("· {n}");
        }
        self.printed_notes = snap.notes.clone();

        // Turn boundary: announce when a turn finishes so the operator knows it's their move.
        if self.was_busy
            && !snap.busy
            && snap.permission_prompt.is_none()
            && snap.question.is_none()
        {
            println!("— turn complete (${:.4}) —", snap.cost_usd);
        }
        self.was_busy = snap.busy;

        // Permission prompt: print once per distinct prompt_seq; a new seq replaces the old.
        if let Some(body) = &snap.permission_prompt {
            if self.shown_prompt_seq != Some(snap.prompt_seq) {
                println!("\n⚠ permission needed:");
                for l in body.lines() {
                    println!("    {l}");
                }
                println!("  answer:  y  (allow)   ·   n  (deny)\n");
                self.shown_prompt_seq = Some(snap.prompt_seq);
            }
            *pending = Pending::Permission(snap.prompt_seq);
        } else if let Some(q) = &snap.question {
            if self.shown_prompt_seq != Some(snap.prompt_seq) {
                println!("\n❓ {q}");
                for (i, opt) in snap.question_options.iter().enumerate() {
                    println!("    {}. {}", i + 1, opt.label);
                }
                println!("  answer by typing a number or free text\n");
                self.shown_prompt_seq = Some(snap.prompt_seq);
            }
            *pending = Pending::Question(snap.prompt_seq);
        } else {
            self.shown_prompt_seq = None;
            *pending = Pending::None;
        }
        let _ = std::io::stdout().flush();
    }
}

/// The lines of `next` that follow everything already printed. Because the transcript/notes are a
/// bounded window that slides, alignment is by the last-printed line's most recent position in the
/// new window; if it slid off entirely, print the whole new window.
fn new_suffix(prev: &[String], next: &[String]) -> Vec<String> {
    let Some(last) = prev.last() else {
        return next.to_vec();
    };
    match next.iter().rposition(|l| l == last) {
        Some(pos) => next[pos + 1..].to_vec(),
        None => next.to_vec(),
    }
}

/// Turn a WS handshake failure into an operator-facing message (unknown session / bad token vs.
/// daemon down), so the client exits cleanly instead of dumping a tungstenite error.
fn map_ws_error(e: tokio_tungstenite::tungstenite::Error, id: &str) -> anyhow::Error {
    use tokio_tungstenite::tungstenite::Error as WsErr;
    match e {
        WsErr::Http(resp) if resp.status() == 404 => anyhow::anyhow!(
            "the daemon has no session {id:?} (or the token is wrong) — run `forge attach` to list"
        ),
        WsErr::Http(resp) => {
            anyhow::anyhow!("daemon refused the websocket: HTTP {}", resp.status())
        }
        other => anyhow::anyhow!("could not open the websocket: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prompts the client sends must deserialize into the daemon's OWN `RemoteInput` — this is
    /// the load-bearing protocol contract. Round-trip our JSON through `crate::remote::RemoteInput`.
    #[test]
    fn typed_input_matches_daemon_remote_input() {
        use crate::remote::RemoteInput;

        let prompt = interpret_json(interpret_input("fix the bug", &Pending::None));
        assert_eq!(
            serde_json::from_value::<RemoteInput>(prompt).unwrap(),
            RemoteInput::Prompt {
                text: "fix the bug".into()
            }
        );

        let allow = interpret_json(interpret_input("y", &Pending::Permission(7)));
        assert_eq!(
            serde_json::from_value::<RemoteInput>(allow).unwrap(),
            RemoteInput::Allow { yes: true, seq: 7 }
        );

        let deny = interpret_json(interpret_input("no", &Pending::Permission(7)));
        assert_eq!(
            serde_json::from_value::<RemoteInput>(deny).unwrap(),
            RemoteInput::Allow { yes: false, seq: 7 }
        );

        let answer = interpret_json(interpret_input("2", &Pending::Question(3)));
        assert_eq!(
            serde_json::from_value::<RemoteInput>(answer).unwrap(),
            RemoteInput::Answer {
                text: "2".into(),
                seq: 3
            }
        );

        let interrupt = interpret_json(interpret_input("/i", &Pending::None));
        assert_eq!(
            serde_json::from_value::<RemoteInput>(interrupt).unwrap(),
            RemoteInput::Interrupt
        );
    }

    fn interpret_json(a: Option<Action>) -> serde_json::Value {
        match a {
            Some(Action::Send(v)) => v,
            _ => panic!("expected a Send action"),
        }
    }

    #[test]
    fn quit_and_blank_and_bad_answer_are_handled() {
        assert!(matches!(
            interpret_input("/q", &Pending::None),
            Some(Action::Quit)
        ));
        assert!(interpret_input("", &Pending::None).is_none());
        // A non-y/n line while a permission prompt is pending is a no-op (not sent as a prompt).
        assert!(interpret_input("maybe", &Pending::Permission(1)).is_none());
    }

    /// A real daemon `Snapshot` JSON must parse into the client's view with the fields it renders.
    #[test]
    fn client_snapshot_parses_daemon_frame() {
        let frame = serde_json::json!({
            "protocol": 7,
            "session_id": "abc123",
            "title": "fix parser",
            "cwd": "/repo",
            "model": "sonnet",
            "busy": true,
            "cost_usd": 0.0123,
            "streaming": "work…",
            "transcript": ["user: hi", "assistant: hello"],
            "permission_prompt": "run write_file on src/x.rs?",
            "prompt_seq": 4,
            "notes": ["stale answer ignored"],
            "revision": 12,
            "closed": false,
            "extra_future_field": 99
        })
        .to_string();
        let snap: ClientSnapshot = serde_json::from_str(&frame).unwrap();
        assert_eq!(snap.session_id, "abc123");
        assert_eq!(snap.transcript.len(), 2);
        assert_eq!(
            snap.permission_prompt.as_deref(),
            Some("run write_file on src/x.rs?")
        );
        assert_eq!(snap.prompt_seq, 4);
        assert!(!snap.closed);
    }

    #[test]
    fn new_suffix_handles_growth_and_sliding_window() {
        // Growth: only the appended tail is new.
        let prev = vec!["a".to_string(), "b".to_string()];
        let next = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(new_suffix(&prev, &next), vec!["c".to_string()]);
        // Sliding window: "b" is the anchor; everything after it is new.
        let next2 = vec!["b".to_string(), "c".to_string(), "d".to_string()];
        assert_eq!(
            new_suffix(&prev, &next2),
            vec!["c".to_string(), "d".to_string()]
        );
        // Window slid off entirely: print all.
        let next3 = vec!["x".to_string(), "y".to_string()];
        assert_eq!(new_suffix(&prev, &next3), next3);
        // Nothing printed yet: everything is new.
        assert_eq!(new_suffix(&[], &next), next);
    }

    #[test]
    fn ws_scheme_maps_http_and_https() {
        assert_eq!(
            ws_scheme("http://127.0.0.1:7420").unwrap(),
            "ws://127.0.0.1:7420"
        );
        assert_eq!(ws_scheme("https://host:9").unwrap(), "wss://host:9");
        assert!(ws_scheme("ftp://nope").is_err());
    }

    #[test]
    fn resolve_session_id_exact_prefix_and_errors() {
        let sessions = vec![
            SessionInfo {
                id: "aaa111".into(),
                title: String::new(),
                cwd: String::new(),
                busy: false,
                waiting: false,
                cost_usd: 0.0,
                model: String::new(),
            },
            SessionInfo {
                id: "aab222".into(),
                title: String::new(),
                cwd: String::new(),
                busy: false,
                waiting: false,
                cost_usd: 0.0,
                model: String::new(),
            },
        ];
        assert_eq!(resolve_session_id(&sessions, "aaa111").unwrap(), "aaa111");
        assert_eq!(resolve_session_id(&sessions, "aab").unwrap(), "aab222");
        assert!(resolve_session_id(&sessions, "aa").is_err()); // ambiguous
        assert!(resolve_session_id(&sessions, "zzz").is_err()); // none
    }
}
