//! Shared Forge mobile core: daemon HTTP/WS client + the view-model logic both native UIs share
//! (fleet ordering, markdown -> blocks, streaming accumulation, history paging).

mod markdown;
mod model;
mod stream;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

pub use markdown::{MdBlock, MdSpan};
pub use model::*;
use stream::StreamCmd;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("http: {0}")]
    Http(String),
    #[error("not streaming")]
    NotStreaming,
}

impl From<reqwest::Error> for CoreError {
    fn from(e: reqwest::Error) -> Self {
        CoreError::Http(e.to_string())
    }
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// `base_url` is the daemon origin plus its token path segment, e.g. `http://10.0.2.2:7420/<token>`.
#[derive(uniffi::Object)]
pub struct DaemonClient {
    base: String,
    http: reqwest::Client,
}

#[uniffi::export(async_runtime = "tokio")]
impl DaemonClient {
    #[uniffi::constructor]
    pub fn new(base_url: String) -> Arc<Self> {
        let _ = runtime();
        Arc::new(Self {
            base: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("http client"),
        })
    }

    /// Fleet, ordered the way the RN app orders it: needs-you first, then starred / forging / cool.
    pub async fn fleet(&self) -> Result<Vec<FleetEntry>, CoreError> {
        let rows: Vec<SessionRow> = self.http.get(format!("{}/api/sessions", self.base)).send().await?.json().await?;
        Ok(model::build_fleet(rows))
    }

    pub fn session(&self, session_id: String) -> Arc<SessionHandle> {
        Arc::new(SessionHandle {
            base: self.base.clone(),
            http: self.http.clone(),
            session_id,
            last_seq: Arc::new(AtomicI64::new(-1)),
            cmd: Mutex::new(None),
        })
    }
}

#[uniffi::export(with_foreign)]
pub trait SessionListener: Send + Sync {
    fn on_update(&self, update: SessionUpdate);
    /// Settled rows that arrived since the last history fetch (after a turn finished).
    fn on_items(&self, items: Vec<ChatItem>);
    fn on_connection(&self, state: ConnectionState);
}

#[derive(uniffi::Object)]
pub struct SessionHandle {
    base: String,
    http: reqwest::Client,
    session_id: String,
    last_seq: Arc<AtomicI64>,
    cmd: Mutex<Option<tokio::sync::mpsc::UnboundedSender<StreamCmd>>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl SessionHandle {
    /// Pages `/api/history` (newest first, `before` cursor) until `max` rows or the start, and
    /// returns them oldest-first with markdown already parsed to blocks.
    pub async fn load_history(&self, max: u32) -> Result<Vec<ChatItem>, CoreError> {
        let mut all: Vec<model::HistoryRow> = Vec::new();
        let mut before: Option<i64> = None;
        let page = 200usize;
        while all.len() < max as usize {
            let mut url = format!(
                "{}/api/history?session={}&limit={}&include_tools=1",
                self.base, self.session_id, page
            );
            if let Some(b) = before {
                url.push_str(&format!("&before={b}"));
            }
            let rows: Vec<model::HistoryRow> = self.http.get(url).send().await?.json().await?;
            let n = rows.len();
            before = rows.last().map(|r| r.seq);
            all.extend(rows);
            if n < page {
                break;
            }
        }
        all.truncate(max as usize);
        all.reverse();
        if let Some(m) = all.iter().map(|r| r.seq).max() {
            self.last_seq.fetch_max(m, Ordering::Relaxed);
        }
        Ok(all.into_iter().map(model::to_chat_item).collect())
    }
}

#[uniffi::export]
impl SessionHandle {
    /// Opens `/ws?session&rev` and pushes coalesced updates (<= ~60/s) to `listener`.
    pub fn start_stream(&self, listener: Arc<dyn SessionListener>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        *self.cmd.lock().unwrap() = Some(tx);
        let ctx = stream::Ctx {
            base: self.base.clone(),
            http: self.http.clone(),
            session_id: self.session_id.clone(),
            last_seq: self.last_seq.clone(),
        };
        runtime().spawn(stream::run(ctx, listener, rx));
    }

    pub fn stop_stream(&self) {
        if let Some(tx) = self.cmd.lock().unwrap().take() {
            let _ = tx.send(StreamCmd::Stop);
        }
    }

    pub fn send_prompt(&self, text: String) -> Result<(), CoreError> {
        match self.cmd.lock().unwrap().as_ref() {
            Some(tx) => tx.send(StreamCmd::Prompt(text)).map_err(|_| CoreError::NotStreaming),
            None => Err(CoreError::NotStreaming),
        }
    }
}

/// Exposed for the UIs' live bubble and for tests.
#[uniffi::export]
pub fn parse_markdown(text: String) -> Vec<MdBlock> {
    markdown::parse(&text)
}
