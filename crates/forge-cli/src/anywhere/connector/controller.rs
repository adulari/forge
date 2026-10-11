//! The controller half of the encrypted relay, for a Forge host talking to another of the
//! account's hosts (session listing and session-to-session messages).
//!
//! The phone app is the usual controller; this is the same protocol from Rust. A controller opens
//! its own relay socket with a ticket for its DEVICE id (the host connector's socket is keyed by
//! host id), seals one `BridgeRequest` to the target host with the Account Data Key and this
//! device's signing key, and waits for the sealed `BridgeResponse`. Everything the service sees
//! is the same opaque envelopes it relays for the app; no plaintext and no new service route.
//!
//! Responses that would not fit inline come back as relay blobs; this client does not follow
//! them (a session list never gets near the inline limit) and reports that instead.

use super::*;

#[derive(Serialize)]
struct ControllerTicketRequest {
    device_id: String,
}

struct ControllerContext<'a> {
    store: &'a StateStore,
    identity: &'a Identity,
    devices: &'a HashMap<[u8; 16], VerifyingKey>,
    http: &'a Client,
    service_url: &'a str,
    access_token: &'a str,
}

/// Everything a controller call needs, loaded from this device's Anywhere state.
pub(in crate::anywhere) struct HostCaller {
    store: StateStore,
    identity: Identity,
    devices: HashMap<[u8; 16], VerifyingKey>,
    http: Client,
    service_url: String,
    access_token: String,
    pub(in crate::anywhere) own_host_id: String,
}

impl HostCaller {
    pub(in crate::anywhere) async fn open() -> Result<Self> {
        let store = StateStore::platform()?;
        let mut state = store.load()?;
        let own_host_id = state
            .host_id
            .clone()
            .context("host is not registered; run `forge anywhere enable`")?;
        let access_token = ensure_access_token(&store, &mut state).await?;
        let config = forge_config::load()?;
        let service_url = config.anywhere.service_url().to_owned();
        refresh_account_epoch(&store, &mut state, &service_url, &access_token).await?;
        let identity = identity_from_state(&state)?;
        let http = client()?;
        let devices = active_device_keys(&http, &service_url, &access_token).await?;
        Ok(Self {
            store,
            identity,
            devices,
            http,
            service_url,
            access_token,
            own_host_id,
        })
    }

    /// `GET {service}{path}` with this device's access token.
    pub(in crate::anywhere) async fn service_get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T> {
        send_json(
            self.http
                .get(format!("{}{path}", self.service_url))
                .bearer_auth(&self.access_token),
        )
        .await
    }

    pub(in crate::anywhere) async fn call(
        &self,
        host_id: &str,
        host_device_id: Option<&str>,
        request: &BridgeRequest,
        timeout: Duration,
    ) -> Result<BridgeResponse> {
        let ctx = ControllerContext {
            store: &self.store,
            identity: &self.identity,
            devices: &self.devices,
            http: &self.http,
            service_url: &self.service_url,
            access_token: &self.access_token,
        };
        let host_id = decode_hex_array::<16>(host_id, "host id")?;
        let host_device_id = host_device_id
            .filter(|id| !id.is_empty())
            .map(|id| decode_hex_array::<16>(id, "host device id"))
            .transpose()?;
        exchange(&ctx, host_id, host_device_id, request, timeout).await
    }
}

/// Send `request` to `host_id` over the relay and return the host's response.
///
/// `host_device_id`, when known (the `/v1/hosts` row carries it), pins who may answer; without it
/// any active device of the account that signs a response is accepted, which is still
/// authenticated by the Account Data Key.
async fn exchange(
    ctx: &ControllerContext<'_>,
    host_id: [u8; 16],
    host_device_id: Option<[u8; 16]>,
    request: &BridgeRequest,
    timeout: Duration,
) -> Result<BridgeResponse> {
    tokio::time::timeout(
        timeout,
        exchange_inner(ctx, host_id, host_device_id, request),
    )
    .await
    .map_err(|_| anyhow::anyhow!("the host did not answer within {}s", timeout.as_secs()))?
}

async fn exchange_inner(
    ctx: &ControllerContext<'_>,
    host_id: [u8; 16],
    host_device_id: Option<[u8; 16]>,
    request: &BridgeRequest,
) -> Result<BridgeResponse> {
    let ticket: RelayTicketResponse = send_json(
        ctx.http
            .post(format!("{}/v1/relay/tickets", ctx.service_url))
            .bearer_auth(ctx.access_token)
            .json(&ControllerTicketRequest {
                device_id: hex::encode(ctx.identity.device_id),
            }),
    )
    .await
    .context("request one-time relay ticket")?;
    let url = relay_url(ctx.service_url, &ticket.ticket)?;
    let (relay, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .context("connect encrypted Anywhere relay")?;
    let (mut write, mut read) = relay.split();

    let plaintext = serde_json::to_vec(request).context("encode bridge request")?;
    let encoded = seal_for_recipient(
        ctx.store,
        EnvelopeKind::BridgeRequest,
        RecipientKind::Host,
        host_id,
        &plaintext,
    )?;
    write
        .send(Message::Binary(encoded.into()))
        .await
        .context("send bridge request")?;

    while let Some(frame) = read.next().await {
        match frame.context("read relay frame")? {
            Message::Binary(bytes) => {
                if let Some(response) =
                    open_response(ctx, host_device_id, request.request_id, bytes.as_ref())?
                {
                    let _ = write.close().await;
                    return Ok(response);
                }
            }
            Message::Ping(bytes) => write.send(Message::Pong(bytes)).await?,
            Message::Close(_) => bail!("relay closed the connection"),
            _ => {}
        }
    }
    bail!("relay closed the connection")
}

/// Authenticate one relay frame; `Some` only for the response to `request_id`.
fn open_response(
    ctx: &ControllerContext<'_>,
    host_device_id: Option<[u8; 16]>,
    request_id: [u8; 16],
    encoded: &[u8],
) -> Result<Option<BridgeResponse>> {
    let envelope = Envelope::decode(encoded).context("decode relay envelope")?;
    let meta = &envelope.metadata;
    if meta.account_id != ctx.identity.account_id {
        bail!("relay envelope account does not match this device");
    }
    if meta.recipient_kind != RecipientKind::Device || meta.recipient_id != ctx.identity.device_id {
        bail!("relay envelope is not addressed to this device");
    }
    if host_device_id.is_some_and(|expected| expected != meta.sender_device_id) {
        bail!("relay envelope came from a device other than the host that was asked");
    }
    let verifying_key = ctx
        .devices
        .get(&meta.sender_device_id)
        .context("relay envelope came from an unknown or revoked device")?;
    let data_key = ctx
        .identity
        .data_key_epochs
        .get(&meta.key_epoch)
        .context("relay envelope uses an unavailable Account Data Key epoch")?;
    let plaintext = envelope
        .open(data_key, verifying_key)
        .context("authenticate and decrypt relay envelope")?;
    accept_inbound_envelopes(
        ctx.store,
        meta.sender_device_id,
        meta.key_epoch,
        None,
        meta.sequence,
    )?;
    if meta.kind != EnvelopeKind::BridgeResponse {
        return Ok(None);
    }
    let response: BridgeResponse =
        serde_json::from_slice(&plaintext).context("decode bridge response")?;
    if response.request_id != request_id {
        return Ok(None);
    }
    if response.body_blob.is_some() {
        bail!("the host's response was too large for an inline relay reply");
    }
    Ok(Some(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
    use axum::routing::{get, post};
    use axum::Router;

    const CONTROLLER_DEVICE: [u8; 16] = [0x22; 16];
    const HOST_DEVICE: [u8; 16] = [0x66; 16];
    const HOST_ID: [u8; 16] = [0x77; 16];
    // One host device has one outbound counter across every connection it answers on.
    static NEXT_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    fn host_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[0x88; 32])
    }

    /// A stand-in relay + remote host: hands out tickets, and answers each sealed bridge request
    /// with `answer`, sealed as the host device. It speaks only through the protocol crate's
    /// envelopes, so it fails if the controller seals or addresses anything wrongly.
    async fn mock_relay(
        answer: impl Fn(BridgeRequest) -> BridgeResponse + Clone + Send + Sync + 'static,
    ) -> String {
        let router = Router::new()
            .route(
                "/v1/relay/tickets",
                post(
                    |axum::Json(body): axum::Json<serde_json::Value>| async move {
                        assert_eq!(body["device_id"], hex::encode(CONTROLLER_DEVICE));
                        axum::Json(serde_json::json!({ "ticket": "t-1" }))
                    },
                ),
            )
            .route(
                "/v1/relay",
                get(move |ws: WebSocketUpgrade| {
                    let answer = answer.clone();
                    async move { ws.on_upgrade(move |socket| host_side(socket, answer)) }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://127.0.0.1:{port}")
    }

    async fn host_side(mut socket: WebSocket, answer: impl Fn(BridgeRequest) -> BridgeResponse) {
        let controller_key = SigningKey::from_bytes(&[0x33; 32]).verifying_key();
        while let Some(Ok(AxumMessage::Binary(bytes))) = socket.recv().await {
            let envelope = Envelope::decode(&bytes).unwrap();
            assert_eq!(envelope.metadata.kind, EnvelopeKind::BridgeRequest);
            assert_eq!(envelope.metadata.recipient_kind, RecipientKind::Host);
            assert_eq!(envelope.metadata.recipient_id, HOST_ID);
            let plain = envelope.open(&[0x44; 32], &controller_key).unwrap();
            let request: BridgeRequest = serde_json::from_slice(&plain).unwrap();
            let response = answer(request);
            // A frame for some other request must be ignored by the controller.
            for rid in [[0xee; 16], response.request_id] {
                let mut reply = response.clone();
                reply.request_id = rid;
                let sequence = NEXT_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let sealed = Envelope::seal(
                    EnvelopeMetadata {
                        kind: EnvelopeKind::BridgeResponse,
                        flags: 0,
                        account_id: [0x11; 16],
                        sender_device_id: HOST_DEVICE,
                        recipient_kind: RecipientKind::Device,
                        recipient_id: CONTROLLER_DEVICE,
                        key_epoch: 3,
                        sequence,
                        created_at_ms: now_ms(),
                        nonce: rand::random::<[u8; 24]>(),
                    },
                    &serde_json::to_vec(&reply).unwrap(),
                    &[0x44; 32],
                    &host_signing_key(),
                )
                .unwrap();
                socket
                    .send(AxumMessage::Binary(sealed.encode().unwrap().into()))
                    .await
                    .unwrap();
            }
        }
    }

    fn context_for<'a>(
        store: &'a StateStore,
        identity: &'a Identity,
        devices: &'a HashMap<[u8; 16], VerifyingKey>,
        http: &'a Client,
        service_url: &'a str,
    ) -> ControllerContext<'a> {
        ControllerContext {
            store,
            identity,
            devices,
            http,
            service_url,
            access_token: "token",
        }
    }

    fn list_request() -> BridgeRequest {
        BridgeRequest {
            request_id: [5; 16],
            route: RouteId::ListSessions,
            method: "GET".into(),
            parameters: Vec::new(),
            headers: Vec::new(),
            body: Vec::new(),
            body_blob: None,
        }
    }

    #[tokio::test]
    async fn a_request_round_trips_sealed_through_a_relay_and_a_stand_in_host() {
        let service = mock_relay(|request| {
            assert_eq!(request.route, RouteId::ListSessions);
            BridgeResponse {
                request_id: request.request_id,
                status: 200,
                headers: Vec::new(),
                body: br#"[{"id":"abc"}]"#.to_vec(),
                body_blob: None,
            }
        })
        .await;
        let temp = tempfile::tempdir().unwrap();
        let store = StateStore {
            path: temp.path().join("anywhere/state.json"),
        };
        store
            .save(&LocalState {
                version: crate::anywhere::STATE_VERSION,
                account_id: Some(hex::encode([0x11; 16])),
                device_id: Some(hex::encode(CONTROLLER_DEVICE)),
                signing_private_key: Some(URL_SAFE_NO_PAD.encode([0x33; 32])),
                account_data_key: Some(URL_SAFE_NO_PAD.encode([0x44; 32])),
                key_epoch: Some(3),
                host_id: Some(hex::encode([0x55; 16])),
                ..LocalState::default()
            })
            .unwrap();
        let identity = identity_from_state(&store.load().unwrap()).unwrap();
        let devices = HashMap::from([(HOST_DEVICE, host_signing_key().verifying_key())]);
        let http = Client::new();
        let ctx = context_for(&store, &identity, &devices, &http, &service);

        let response = exchange(
            &ctx,
            HOST_ID,
            Some(HOST_DEVICE),
            &list_request(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, br#"[{"id":"abc"}]"#);

        // A response from a device other than the pinned host is refused.
        let err = exchange(
            &ctx,
            HOST_ID,
            Some([0x99; 16]),
            &list_request(),
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("other than the host"),
            "{err:#}"
        );

        // A key the account does not know cannot answer.
        let none = HashMap::new();
        let ctx = context_for(&store, &identity, &none, &http, &service);
        assert!(
            exchange(&ctx, HOST_ID, None, &list_request(), Duration::from_secs(5))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_unreachable_service_fails_fast_with_an_error_not_a_hang() {
        let temp = tempfile::tempdir().unwrap();
        let store = durable_controller_store(temp.path());
        let identity = identity_from_state(&store.load().unwrap()).unwrap();
        let devices = HashMap::new();
        let http = Client::new();
        let ctx = context_for(&store, &identity, &devices, &http, "http://127.0.0.1:1");
        assert!(
            exchange(&ctx, HOST_ID, None, &list_request(), Duration::from_secs(2))
                .await
                .is_err()
        );
    }

    fn durable_controller_store(dir: &std::path::Path) -> StateStore {
        let store = StateStore {
            path: dir.join("anywhere/state.json"),
        };
        store
            .save(&LocalState {
                version: crate::anywhere::STATE_VERSION,
                account_id: Some(hex::encode([0x11; 16])),
                device_id: Some(hex::encode(CONTROLLER_DEVICE)),
                signing_private_key: Some(URL_SAFE_NO_PAD.encode([0x33; 32])),
                account_data_key: Some(URL_SAFE_NO_PAD.encode([0x44; 32])),
                key_epoch: Some(3),
                host_id: Some(hex::encode([0x55; 16])),
                ..LocalState::default()
            })
            .unwrap();
        store
    }
}
