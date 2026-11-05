use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::Message;

use crate::model::{self, ConnectionState, HistoryRow, SessionUpdate, Snapshot};
use crate::{markdown, SessionListener};

pub enum StreamCmd {
    Prompt(String),
    Stop,
}

pub struct Ctx {
    pub base: String,
    pub http: reqwest::Client,
    pub session_id: String,
    pub last_seq: Arc<std::sync::atomic::AtomicI64>,
}

/// UTF-16 accumulation of the reply tail, mirrors `mobile/src/lib/streamingText.ts`.
#[derive(Default)]
struct Acc {
    base: usize,
    text: Vec<u16>,
}

impl Acc {
    fn fold(&mut self, tail: &str, offset: usize) {
        let tail16: Vec<u16> = tail.encode_utf16().collect();
        if offset == 0 {
            self.base = 0;
            self.text = tail16;
            return;
        }
        let keep = offset as isize - self.base as isize;
        if keep < 0 || keep as usize > self.text.len() {
            self.base = offset;
            self.text = tail16;
        } else {
            self.text.truncate(keep as usize);
            self.text.extend(tail16);
        }
    }
    fn string(&self) -> String {
        let s = String::from_utf16_lossy(&self.text);
        if self.base > 0 && !s.is_empty() { format!("…{s}") } else { s }
    }
}

pub async fn run(ctx: Ctx, listener: Arc<dyn SessionListener>, mut cmds: UnboundedReceiver<StreamCmd>) {
    let ws_base = ctx.base.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
    let mut rev: u64 = 0;
    let mut attempt: u32 = 0;
    loop {
        listener.on_connection(if attempt == 0 { ConnectionState::Connecting } else { ConnectionState::Reconnecting });
        let url = format!("{ws_base}/ws?session={}&rev={rev}", ctx.session_id);
        let conn = tokio_tungstenite::connect_async(url).await;
        let Ok((mut ws, _)) = conn else {
            attempt += 1;
            let delay = Duration::from_millis((250u64 << attempt.min(5)).min(8000));
            tokio::select! {
                _ = tokio::time::sleep(delay) => continue,
                c = cmds.recv() => { if matches!(c, Some(StreamCmd::Stop) | None) { break } else { continue } }
            }
        };
        attempt = 0;
        listener.on_connection(ConnectionState::Open);

        let mut acc = Acc::default();
        let mut dirty: Option<Snapshot> = None;
        let mut was_busy = false;
        let mut tick = tokio::time::interval(Duration::from_millis(16));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut stop = false;
        loop {
            tokio::select! {
                msg = ws.next() => {
                    match msg {
                        Some(Ok(Message::Text(t))) => {
                            let Ok(snap) = serde_json::from_str::<Snapshot>(&t) else { continue };
                            let Some(r) = snap.revision else { continue };
                            rev = r;
                            acc.fold(&snap.streaming, snap.streaming_offset.unwrap_or(0));
                            if snap.closed { stop = true; break; }
                            if was_busy && !snap.busy {
                                dirty = Some(snap);
                                flush(&mut dirty, &acc, &listener);
                                refresh_items(&ctx, &listener).await;
                                was_busy = false;
                                continue;
                            }
                            was_busy = snap.busy;
                            dirty = Some(snap);
                        }
                        Some(Ok(Message::Ping(p))) => { let _ = ws.send(Message::Pong(p)).await; }
                        Some(Ok(_)) => {}
                        _ => break,
                    }
                }
                _ = tick.tick() => flush(&mut dirty, &acc, &listener),
                c = cmds.recv() => match c {
                    Some(StreamCmd::Prompt(text)) => {
                        let frame = serde_json::json!({"kind":"prompt","text":text,"client_msg_id":format!("n{}", rev)});
                        let _ = ws.send(Message::Text(frame.to_string().into())).await;
                    }
                    _ => { stop = true; break; }
                },
            }
        }
        if stop {
            let _ = ws.close(None).await;
            break;
        }
        attempt += 1;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    listener.on_connection(ConnectionState::Closed);
}

fn flush(dirty: &mut Option<Snapshot>, acc: &Acc, listener: &Arc<dyn SessionListener>) {
    let Some(s) = dirty.take() else { return };
    let text = acc.string();
    listener.on_update(SessionUpdate {
        revision: s.revision.unwrap_or(0),
        title: s.title,
        busy: s.busy,
        activity: s.activity,
        model: s.model,
        cost_usd: s.cost_usd,
        streaming_chars: text.chars().count() as u32,
        streaming_blocks: if text.is_empty() { vec![] } else { markdown::parse(&text) },
    });
}

/// After a turn ends, pull rows newer than the last seen seq so the settled reply replaces the live bubble.
async fn refresh_items(ctx: &Ctx, listener: &Arc<dyn SessionListener>) {
    let url = format!("{}/api/history?session={}&limit=20&include_tools=1", ctx.base, ctx.session_id);
    let Ok(resp) = ctx.http.get(url).send().await else { return };
    let Ok(rows) = resp.json::<Vec<HistoryRow>>().await else { return };
    let seen = ctx.last_seq.load(std::sync::atomic::Ordering::Relaxed);
    let mut fresh: Vec<HistoryRow> = rows.into_iter().filter(|r| r.seq > seen).collect();
    fresh.reverse();
    if let Some(m) = fresh.iter().map(|r| r.seq).max() {
        ctx.last_seq.fetch_max(m, std::sync::atomic::Ordering::Relaxed);
    }
    if !fresh.is_empty() {
        listener.on_items(fresh.into_iter().map(model::to_chat_item).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::Acc;

    #[test]
    fn accumulates_tail_by_utf16_offset() {
        let mut a = Acc::default();
        a.fold("hello ", 0);
        a.fold("lo wörld", 3);
        assert_eq!(a.string(), "hello wörld");
        a.fold("tail", 500);
        assert_eq!(a.string(), "…tail");
    }
}
