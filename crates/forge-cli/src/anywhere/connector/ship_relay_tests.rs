//! Shipping over the relay, end to end and in process: a signed command from a paired device goes
//! through the host connector's durable-command path into a local daemon stand-in that runs the
//! real `ship_session` against a bare git remote and a fake `gh`. Nothing here talks to the real
//! Anywhere service, a real remote or a model.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::IntoResponse as _;
use axum::routing::{get, post};
use axum::{Json, Router};

use super::*;
use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_git::ship::{ship_session, ShipRequest};
use crate::serve_git::test_support::state;

const SENDER: [u8; 16] = [0x64; 16];
const HOST: [u8; 16] = [0x55; 16];

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Ship {
    _dir: tempfile::TempDir,
    work: PathBuf,
    remote: PathBuf,
    gh_log: PathBuf,
    state: Arc<DaemonState>,
    session: String,
    gh: PathBuf,
}

/// A session worktree on `feat/cart` with uncommitted edits, whose `origin` is a bare repository
/// on disk, and a `gh` that only logs and answers canned replies.
fn ship_fixture() -> Ship {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let remote = root.join("remote.git");
    let main = root.join("main");
    std::fs::create_dir_all(&main).unwrap();
    git(
        &root,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().unwrap(),
        ],
    );
    git(&main, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("remote.origin.url", "https://github.com/acme/shop.git"),
        ("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"),
    ] {
        git(&main, &["config", key, value]);
    }
    git(
        &main,
        &[
            "config",
            &format!("url.{}.insteadOf", remote.display()),
            "https://github.com/acme/shop.git",
        ],
    );
    std::fs::write(main.join("README.md"), "shop\n").unwrap();
    git(&main, &["add", "README.md"]);
    git(&main, &["commit", "-q", "-m", "init"]);
    git(&main, &["push", "-q", "-u", "origin", "main"]);
    let work = root.join("wt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/cart",
            work.to_str().unwrap(),
        ],
    );
    std::fs::write(work.join("cart.rs"), "fn cart() {}\n").unwrap();
    std::fs::write(work.join("README.md"), "shop\ncart\n").unwrap();

    let gh_log = root.join("gh.log");
    let gh = root.join("gh");
    std::fs::write(
        &gh,
        format!(
            r#"#!/bin/sh
echo "$*" >> '{log}'
case "$1 $2" in
  "repo view") echo '{{"viewerPermission":"WRITE","defaultBranchRef":{{"name":"main"}},"isArchived":false}}' ;;
  "pr list") echo '[]' ;;
  "pr create") cat >/dev/null; echo "https://github.com/acme/shop/pull/9" ;;
  *) echo "unexpected gh call: $*" >&2; exit 1 ;;
esac
"#,
            log = gh_log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();

    let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
    let session = store
        .create_session(&main.display().to_string(), "default")
        .unwrap();
    store
        .set_session_worktree(&session, &work.display().to_string())
        .unwrap();
    Ship {
        state: state(store, &main),
        _dir: dir,
        work,
        remote,
        gh_log,
        session,
        gh,
    }
}

impl Ship {
    fn gh_calls(&self) -> String {
        std::fs::read_to_string(&self.gh_log).unwrap_or_default()
    }

    fn commits_on_branch(&self) -> usize {
        git(&self.work, &["rev-list", "--count", "main..HEAD"])
            .parse()
            .unwrap()
    }

    /// The local daemon as the connector sees it: the real ship handler (with the fake `gh`) and
    /// the real PR Watch routes, behind the owner token path. `ship_calls` counts handler runs.
    async fn daemon(&self, ship_calls: Arc<AtomicUsize>) -> String {
        let gh = self.gh.clone();
        let ship_state = self.state.clone();
        let app = Router::new()
            .route(
                "/tok/api/sessions/{id}/ship",
                post(
                    |State((state, gh, calls)): State<(
                        Arc<DaemonState>,
                        PathBuf,
                        Arc<AtomicUsize>,
                    )>,
                     AxumPath(id): AxumPath<String>,
                     Json(request): Json<ShipRequest>| async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        match ship_session(&state, &id, &request, &gh).await {
                            Ok(response) => json_response(&response),
                            Err((status, message)) => err_response(status, &message),
                        }
                    },
                )
                .with_state((ship_state, gh, ship_calls)),
            )
            .merge(crate::serve_pr_watch::routes("/tok").with_state(self.state.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://127.0.0.1:{port}/tok")
    }
}

fn bridged(route: RouteId, method: &str, parameters: &[&str], body: &[u8]) -> BridgeRequest {
    BridgeRequest {
        request_id: [9; 16],
        route,
        method: method.into(),
        parameters: parameters.iter().map(|p| (*p).to_owned()).collect(),
        headers: vec![("content-type".into(), "application/json".into())],
        body: body.to_vec(),
        body_blob: None,
    }
}

fn json_body(response: &BridgeResponse) -> serde_json::Value {
    serde_json::from_slice(&response.body).unwrap_or(serde_json::Value::Null)
}

#[tokio::test]
async fn opt_in_off_refuses_ship_and_watch_writes_and_names_the_setting() {
    let fx = ship_fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let base = fx.daemon(calls.clone()).await;

    let ship = bridged(RouteId::SessionShip, "POST", &[&fx.session], b"{}");
    let watch = bridged(
        RouteId::PrWatchSet,
        "POST",
        &[],
        format!(r#"{{"session":"{}","mode":"merge"}}"#, fx.session).as_bytes(),
    );
    for request in [&ship, &watch] {
        let response = dispatch_http_with_policy(&base, request, false)
            .await
            .unwrap();
        assert_eq!(response.status, 403, "{:?}", request.route);
        let body = json_body(&response);
        assert_eq!(body["code"], "ship_over_relay_off");
        assert_eq!(body["setting"], "anywhere.allow_ship_over_relay");
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the daemon was never reached"
    );
    assert_eq!(fx.commits_on_branch(), 0);
    assert_eq!(fx.gh_calls(), "");
    let watches =
        dispatch_http_with_policy(&base, &bridged(RouteId::PrWatches, "GET", &[], b""), false)
            .await
            .unwrap();
    assert_eq!(
        watches.status, 200,
        "PR reads stay open when the opt-in is off"
    );
    assert_eq!(
        json_body(&watches)["watches"].as_array().map(Vec::len),
        Some(0)
    );
}

#[tokio::test]
async fn opt_in_on_ships_over_the_relay_and_arms_merge_mode() {
    let fx = ship_fixture();
    let base = fx.daemon(Arc::new(AtomicUsize::new(0))).await;
    let ship = bridged(
        RouteId::SessionShip,
        "POST",
        &[&fx.session],
        br#"{"message":"feat(cart): add cart","title":"feat(cart): add cart","body":"adds a cart"}"#,
    );
    let response = dispatch_http_with_policy(&base, &ship, true).await.unwrap();
    assert_eq!(response.status, 200);
    let body = json_body(&response);
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["pr"]["number"], 9);
    assert_eq!(body["watch"]["mode"], "merge");

    assert_eq!(fx.commits_on_branch(), 1);
    let sha = git(&fx.work, &["rev-parse", "HEAD"]);
    assert_eq!(git(&fx.remote, &["rev-parse", "refs/heads/feat/cart"]), sha);
    let files = git(&fx.work, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        files.contains("cart.rs") && files.contains("README.md"),
        "{files}"
    );
    let calls = fx.gh_calls();
    assert!(calls.contains("pr create"), "{calls}");
    assert!(!calls.contains("--force"), "{calls}");

    let watch = dispatch_http_with_policy(
        &base,
        &bridged(
            RouteId::PrWatchGet,
            "GET",
            &[&format!("?session={}", fx.session)],
            b"",
        ),
        true,
    )
    .await
    .unwrap();
    assert_eq!(json_body(&watch)["watch"]["mode"], "merge");
}

/// What the service keeps for the host: one queued command, listed on every poll, and an ack
/// endpoint that counts. The host's journal is the only thing between a re-listed command and a
/// second commit.
struct Service {
    url: String,
    acks: Arc<AtomicUsize>,
}

async fn service(command_id: CommandId, ciphertext: Vec<u8>, created_at_ms: u64) -> Service {
    let acks = Arc::new(AtomicUsize::new(0));
    let list = serde_json::to_value(QueuedCommandList {
        version: COMMAND_LIST_VERSION,
        commands: vec![QueuedCommandMetadata {
            command_id,
            sender_device_id: SENDER,
            created_at_ms,
            expires_at_ms: created_at_ms + forge_anywhere_protocol::COMMAND_EXPIRY_MS,
            ciphertext_bytes: ciphertext.len() as u64,
        }],
    })
    .unwrap();
    let host = hex::encode(HOST);
    let app =
        Router::new()
            .route(
                &format!("/v1/hosts/{host}/commands"),
                get(move || {
                    let list = list.clone();
                    async move { Json(list) }
                }),
            )
            .route(
                &format!("/v1/hosts/{host}/commands/{command_id}"),
                get(move || {
                    let ciphertext = ciphertext.clone();
                    async move {
                        ([(CONTENT_TYPE, "application/octet-stream")], ciphertext).into_response()
                    }
                }),
            )
            .route(
                &format!("/v1/hosts/{host}/commands/{command_id}/ack"),
                post({
                    let acks = acks.clone();
                    move || {
                        let acks = acks.clone();
                        async move {
                            acks.fetch_add(1, Ordering::SeqCst);
                            axum::http::StatusCode::NO_CONTENT
                        }
                    }
                }),
            )
            .route(
                "/v1/push/notifications",
                post(|| async { axum::http::StatusCode::ACCEPTED }),
            );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Service {
        url: format!("http://127.0.0.1:{port}"),
        acks,
    }
}

fn host_identity() -> Identity {
    Identity {
        account_id: [0x11; 16],
        device_id: [0x22; 16],
        host_id: HOST,
        data_key: [0x44; 32],
        data_key_epochs: BTreeMap::from([(3, [0x44; 32])]),
        key_epoch: 3,
        signing_key: SigningKey::from_bytes(&[0x33; 32]),
    }
}

fn host_store(dir: &Path) -> StateStore {
    let store = StateStore {
        path: dir.join("anywhere/state.json"),
    };
    store
        .save(&LocalState {
            version: super::super::STATE_VERSION,
            account_id: Some(hex::encode([0x11; 16])),
            device_id: Some(hex::encode([0x22; 16])),
            signing_private_key: Some(URL_SAFE_NO_PAD.encode([0x33; 32])),
            account_data_key: Some(URL_SAFE_NO_PAD.encode([0x44; 32])),
            key_epoch: Some(3),
            host_id: Some(hex::encode(HOST)),
            ..LocalState::default()
        })
        .unwrap();
    store
}

fn seal_command(request: &BridgeRequest, sender: &SigningKey, created_at_ms: u64) -> Vec<u8> {
    Envelope::seal(
        EnvelopeMetadata {
            kind: EnvelopeKind::Command,
            flags: 0,
            account_id: [0x11; 16],
            sender_device_id: SENDER,
            recipient_kind: RecipientKind::Host,
            recipient_id: HOST,
            key_epoch: 3,
            sequence: 1,
            created_at_ms,
            nonce: [0x84; 24],
        },
        &serde_json::to_vec(request).unwrap(),
        &[0x44; 32],
        sender,
    )
    .unwrap()
    .encode()
    .unwrap()
}

async fn poll_twice(
    fx: &Ship,
    allow_ship: bool,
    command: &BridgeRequest,
) -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let ship_calls = Arc::new(AtomicUsize::new(0));
    let base = fx.daemon(ship_calls.clone()).await;
    let sender = SigningKey::from_bytes(&[0x74; 32]);
    let created_at_ms = super::super::whole_second_ms(now_ms());
    let sealed = seal_command(command, &sender, created_at_ms);
    let service = service(CommandId::new([0xc1; 16]), sealed, created_at_ms).await;

    let dir = tempfile::tempdir().unwrap();
    let store = host_store(dir.path());
    let identity = host_identity();
    let devices = HashMap::from([(SENDER, sender.verifying_key())]);
    let http = Client::new();
    let host_id = hex::encode(HOST);
    let client = DurableCommandClient {
        store: &store,
        identity: &identity,
        devices: &devices,
        local_base_url: &base,
        http: &http,
        service_url: &service.url,
        access_token: "token",
        host_id: &host_id,
        worker_id: "worker-a",
        allow_ship_override: Some(allow_ship),
    };
    client.poll().await.unwrap();
    // The service keeps listing the command until it is acknowledged; a replay of the same signed
    // command, or a poll after a crash, must find it in the journal and run nothing.
    client.poll().await.unwrap();
    client.poll().await.unwrap();
    (ship_calls, service.acks)
}

#[tokio::test]
async fn replaying_the_same_signed_ship_command_commits_and_opens_one_pr() {
    let fx = ship_fixture();
    let command = bridged(
        RouteId::SessionShip,
        "POST",
        &[&fx.session],
        br#"{"message":"feat(cart): add cart","title":"feat(cart): add cart","body":"adds a cart"}"#,
    );
    let (ship_calls, acks) = poll_twice(&fx, true, &command).await;

    assert_eq!(ship_calls.load(Ordering::SeqCst), 1, "dispatched once");
    assert_eq!(acks.load(Ordering::SeqCst), 1, "acknowledged once");
    assert_eq!(fx.commits_on_branch(), 1, "one commit");
    let creates = fx
        .gh_calls()
        .lines()
        .filter(|line| line.starts_with("pr create"))
        .count();
    assert_eq!(creates, 1, "one PR");
}

#[tokio::test]
async fn a_signed_ship_command_is_dropped_by_the_host_when_the_opt_in_is_off() {
    let fx = ship_fixture();
    let command = bridged(RouteId::SessionShip, "POST", &[&fx.session], b"{}");
    let (ship_calls, acks) = poll_twice(&fx, false, &command).await;

    assert_eq!(ship_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        acks.load(Ordering::SeqCst),
        1,
        "the refusal is acknowledged once"
    );
    assert_eq!(fx.commits_on_branch(), 0);
    assert_eq!(fx.gh_calls(), "");
}
