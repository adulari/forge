//! The upload outbox against a stand-in service that answers like the real one: errors are the
//! nested `{"version":1,"error":{"code":..}}` shape, which the old client never matched.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{post, put};
use axum::{Json, Router};
use forge_store::SyncJournalOperation;
use serde_json::{json, Value};

use super::*;
use crate::anywhere::sync_recover::ServiceRejected;

#[derive(Default)]
struct Service {
    reserve_conflict: Mutex<Option<&'static str>>,
    complete_conflict: Mutex<Option<&'static str>>,
    start_keys: Mutex<Vec<String>>,
    puts: AtomicUsize,
    port: AtomicUsize,
}

fn error(code: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::CONFLICT,
        Json(json!({"version":1,"error":{"code":code,"message":"refused"}})),
    )
}

async fn reserve(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    service.start_keys.lock().unwrap().push(
        headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
    );
    if let Some(code) = *service.reserve_conflict.lock().unwrap() {
        return error(code);
    }
    let port = service.port.load(Ordering::SeqCst);
    (
        StatusCode::OK,
        Json(json!({
            "upload_id": "00".repeat(16),
            "upload_url": format!("http://127.0.0.1:{port}/object"),
            "required_headers": {},
            "already_complete": false
        })),
    )
}

async fn object(State(service): State<Arc<Service>>) -> StatusCode {
    service.puts.fetch_add(1, Ordering::SeqCst);
    StatusCode::OK
}

async fn complete(State(service): State<Arc<Service>>) -> (StatusCode, Json<Value>) {
    match *service.complete_conflict.lock().unwrap() {
        Some(code) => error(code),
        None => (StatusCode::OK, Json(json!({"version":1,"cursor":1}))),
    }
}

struct Rig {
    store: Store,
    state_store: StateStore,
    service: Arc<Service>,
    url: String,
    _temp: tempfile::TempDir,
}

async fn rig() -> Rig {
    let service = Arc::new(Service::default());
    let app = Router::new()
        .route("/v1/sync/uploads", post(reserve))
        .route("/object", put(object))
        .route("/v1/sync/uploads/{id}/complete", post(complete))
        .with_state(service.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    service.port.store(port as usize, Ordering::SeqCst);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let temp = tempfile::tempdir().unwrap();
    let state_store = StateStore {
        path: temp.path().join("anywhere/state.json"),
    };
    state_store
        .save(&LocalState {
            version: super::super::STATE_VERSION,
            account_id: Some(hex::encode([0x11; 16])),
            device_id: Some(hex::encode([0x22; 16])),
            signing_private_key: Some(URL_SAFE_NO_PAD.encode([0x33; 32])),
            account_data_key: Some(URL_SAFE_NO_PAD.encode([0x44; 32])),
            key_epoch: Some(1),
            ..LocalState::default()
        })
        .unwrap();
    let store = Store::open_in_memory().unwrap();
    store.set_sync_journal_enabled(true).unwrap();
    Rig {
        store,
        state_store,
        service,
        url: format!("http://127.0.0.1:{port}"),
        _temp: temp,
    }
}

impl Rig {
    async fn upload(&self, entry: &SyncJournalEntry) -> Result<()> {
        upload_entry(
            &self.store,
            &self.state_store,
            &reqwest::Client::new(),
            &self.url,
            "token",
            entry,
        )
        .await
    }

    fn pending(&self) -> Vec<SyncJournalEntry> {
        self.store.pending_sync_journal(100).unwrap()
    }
}

/// The reported incident: the service answered the nested shape, the client looked for a top-level
/// `code`, so the supersede path never ran and the first row blocked the account forever.
#[tokio::test]
async fn a_revision_conflict_in_the_services_real_shape_supersedes_the_row() {
    let rig = rig().await;
    rig.store
        .write_portable_sync_record([0x22; 16], "command", "review", Some(b"{\"a\":1}"))
        .unwrap();
    *rig.service.reserve_conflict.lock().unwrap() = Some("sync_revision_conflict");
    let stuck = rig.pending().remove(0);

    rig.upload(&stuck)
        .await
        .expect("a revision conflict resolves");

    let pending = rig.pending();
    assert_eq!(
        pending.len(),
        1,
        "the retired row is gone, its successor waits"
    );
    assert_eq!(pending[0].revision, stuck.revision + 1);
    assert_eq!(pending[0].payload, stuck.payload, "no data is lost");
}

/// A completion the service refuses with `replay_detected` fails the same way on every retry of
/// the cached envelope, so the row is re-journaled at the next revision and its envelope dropped.
#[tokio::test]
async fn a_replayed_sequence_at_completion_rejournals_the_row() {
    let rig = rig().await;
    rig.store
        .append_sync_journal("message", "m1", SyncJournalOperation::Upsert, 1, 1, b"{}")
        .unwrap();
    let stuck = rig.pending().remove(0);
    *rig.service.complete_conflict.lock().unwrap() = Some("replay_detected");

    rig.upload(&stuck)
        .await
        .expect("replay is recovered, not retried");

    assert!(rig.store.sync_upload_envelope(stuck.id).unwrap().is_none());
    let pending = rig.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].revision, 2);
    assert_eq!(pending[0].payload, b"{}");

    *rig.service.complete_conflict.lock().unwrap() = None;
    rig.upload(&pending[0])
        .await
        .expect("the new revision uploads");
    assert!(rig.pending().is_empty());
    let keys = rig.service.start_keys.lock().unwrap().clone();
    assert_eq!(keys.len(), 2);
    assert_ne!(
        keys[0], keys[1],
        "a new revision and ciphertext get a new key"
    );
}

#[tokio::test]
async fn a_failed_verification_is_retried_before_the_row_is_rejournaled() {
    let rig = rig().await;
    // The strike counter is process-wide and keyed by journal id, which every in-memory store
    // starts counting from 1; a distinct id keeps parallel tests from resetting each other.
    for n in 0..3_000 {
        rig.store
            .append_sync_journal(
                "message",
                &format!("pad-{n}"),
                SyncJournalOperation::Upsert,
                1,
                1,
                b"{}",
            )
            .unwrap();
    }
    let pad: Vec<i64> = (1..=3_000).collect();
    rig.store.mark_sync_journal_uploaded(&pad, 1).unwrap();
    rig.store
        .append_sync_journal("message", "m2", SyncJournalOperation::Upsert, 1, 1, b"{}")
        .unwrap();
    let stuck = rig.pending().remove(0);
    *rig.service.complete_conflict.lock().unwrap() = Some("upload_verification_failed");

    for _ in 0..2 {
        let error = rig
            .upload(&stuck)
            .await
            .expect_err("retried while it may still heal");
        let rejected = error
            .downcast_ref::<ServiceRejected>()
            .expect("typed error");
        assert_eq!(rejected.code.as_deref(), Some("upload_verification_failed"));
        assert!(format!("{error:#}").contains("upload_verification_failed"));
        assert_eq!(rig.pending().len(), 1);
        assert_eq!(rig.pending()[0].id, stuck.id);
    }
    rig.upload(&stuck)
        .await
        .expect("the third failure rejournals");
    assert_eq!(rig.pending().len(), 1);
    assert_eq!(rig.pending()[0].revision, 2);
}

#[tokio::test]
async fn an_unhandled_conflict_names_its_code_and_keeps_the_row() {
    let rig = rig().await;
    rig.store
        .append_sync_journal("message", "m3", SyncJournalOperation::Upsert, 1, 1, b"{}")
        .unwrap();
    let stuck = rig.pending().remove(0);
    *rig.service.reserve_conflict.lock().unwrap() = Some("sync_revision_conflict");

    let error = rig
        .upload(&stuck)
        .await
        .expect_err("message rows are not superseded");
    assert!(
        format!("{error:#}").contains("sync_revision_conflict"),
        "{error:#}"
    );
    assert_eq!(rig.pending().len(), 1);
}
