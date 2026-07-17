//! End-to-end encrypted replay-share creation.

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use forge_anywhere_protocol::{
    Envelope, EnvelopeKind, EnvelopeMetadata, RecipientKind, ReplayShare, ShareCompletion, ShareId,
    ShareReservation, ShareReserveRequest, ANYWHERE_OBJECT_MEDIA_TYPE, MAX_SHARE_ENVELOPE_BYTES,
    SHARE_VERSION,
};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, Url};
use sha2::{Digest as _, Sha256};

use super::{
    decode_base64_array, decode_hex_array, ensure_access_token, now_ms, LocalState, StateStore,
};
use crate::ShareExpiry;

const TRANSFER_ATTEMPTS: usize = 2;

struct PreparedShare {
    share_id: ShareId,
    key: [u8; 32],
    envelope: Vec<u8>,
    reserve: ShareReserveRequest,
    expires_at_ms: u64,
    reserve_idempotency_key: String,
    complete_idempotency_key: String,
}

pub(super) async fn create(session_prefix: &str, expiry: ShareExpiry) -> Result<()> {
    let state_store = StateStore::platform()?;
    let mut state = state_store.load()?;
    let access_token = ensure_access_token(&state_store, &mut state).await?;
    let session_id = resolve_session_id(session_prefix)?;
    let replay_entries = crate::open_store()?
        .load_replay(&session_id)
        .context("load replay for encrypted share")?;
    if replay_entries.is_empty() {
        bail!("session {session_prefix} has no replay history to share");
    }
    let replay: serde_json::Value =
        serde_json::from_str(&crate::replay::render_json(&session_id, &replay_entries))
            .context("encode replay export")?;
    validate_replay_export(&replay, &session_id)?;

    // Persist the sequence advance before using it in an envelope. A failed transfer can safely
    // consume a sequence, but a crash must never cause nonce/sequence reuse.
    let (state, sequence) = state_store.reserve_sequences(1)?;
    let prepared = prepare_share(
        replay,
        &session_id,
        expiry_seconds(expiry),
        sequence,
        now_ms(),
        rand::random(),
        rand::random(),
        rand::random(),
        &state,
    )?;

    let config = forge_config::load()?;
    let service_url = config.anywhere.service_url();
    let client = super::client()?;
    let reservation = reserve(&client, service_url, &access_token, &prepared).await?;
    upload(&client, &reservation.upload_url, &prepared.envelope).await?;
    let completion = complete(&client, service_url, &access_token, &prepared).await?;
    let url = public_url(service_url, &completion, &prepared)?;
    println!("{url}");
    Ok(())
}

fn resolve_session_id(prefix: &str) -> Result<String> {
    let mut matches = crate::open_store()?
        .matching_session_ids(prefix)
        .with_context(|| format!("resolving session {prefix}"))?;
    match matches.len() {
        0 => bail!("no session matches '{prefix}' — see `forge sessions`"),
        1 => Ok(matches.remove(0)),
        count => bail!("'{prefix}' is ambiguous ({count} sessions) — use more characters"),
    }
}

fn expiry_seconds(expiry: ShareExpiry) -> u64 {
    match expiry {
        ShareExpiry::Hours24 => 24 * 60 * 60,
        ShareExpiry::Days7 => 7 * 24 * 60 * 60,
        ShareExpiry::Days30 => 30 * 24 * 60 * 60,
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_share(
    replay: serde_json::Value,
    session_id: &str,
    expires_in_seconds: u64,
    sequence: u64,
    created_at_ms: u64,
    share_id_bytes: [u8; 16],
    key: [u8; 32],
    nonce: [u8; 24],
    state: &LocalState,
) -> Result<PreparedShare> {
    if !forge_anywhere_protocol::SHARE_EXPIRY_SECONDS.contains(&expires_in_seconds) {
        bail!("share expiry must be 24 hours, 7 days, or 30 days");
    }
    let account_id = decode_hex_array::<16>(
        state
            .account_id
            .as_deref()
            .context("account is not enrolled")?,
        "account id",
    )?;
    let sender_device_id = decode_hex_array::<16>(
        state
            .device_id
            .as_deref()
            .context("device is not enrolled")?,
        "device id",
    )?;
    let signing_key = SigningKey::from_bytes(&decode_base64_array(
        state
            .signing_private_key
            .as_deref()
            .context("device signing key is unavailable")?,
        "device signing key",
    )?);
    let expires_at_ms = created_at_ms
        .checked_add(
            expires_in_seconds
                .checked_mul(1000)
                .context("share expiry is too large")?,
        )
        .context("share expiry is too large")?;
    let plaintext = serde_json::to_vec(&ReplayShare {
        version: SHARE_VERSION,
        session_id: session_id.to_owned(),
        created_at_ms,
        expires_at_ms,
        replay,
    })
    .context("serialize encrypted replay share")?;
    let share_id = ShareId::new(share_id_bytes);
    let envelope = Envelope::seal(
        EnvelopeMetadata {
            kind: EnvelopeKind::Share,
            flags: 0,
            account_id,
            sender_device_id,
            recipient_kind: RecipientKind::Share,
            recipient_id: share_id_bytes,
            // A replay-share key is independent from every Account Data Key epoch.
            key_epoch: 0,
            sequence,
            created_at_ms,
            nonce,
        },
        &plaintext,
        &key,
        &signing_key,
    )?
    .encode()?;
    let ciphertext_bytes = u64::try_from(envelope.len()).context("share envelope is too large")?;
    if ciphertext_bytes > MAX_SHARE_ENVELOPE_BYTES {
        bail!(
            "encrypted replay is {} bytes; Forge Anywhere shares are limited to {} bytes",
            ciphertext_bytes,
            MAX_SHARE_ENVELOPE_BYTES
        );
    }
    let ciphertext_sha256: [u8; 32] = Sha256::digest(&envelope).into();
    Ok(PreparedShare {
        share_id,
        key,
        envelope,
        reserve: ShareReserveRequest {
            version: SHARE_VERSION,
            share_id,
            ciphertext_bytes,
            ciphertext_sha256,
            expires_in_seconds,
        },
        expires_at_ms,
        reserve_idempotency_key: idempotency_key(share_id, "reserve"),
        complete_idempotency_key: idempotency_key(share_id, "complete"),
    })
}

fn validate_replay_export(replay: &serde_json::Value, session_id: &str) -> Result<()> {
    let object = replay
        .as_object()
        .context("replay export is not a JSON object")?;
    if object.get("session_id").and_then(|value| value.as_str()) != Some(session_id) {
        bail!("replay export did not identify the selected session");
    }
    if !object.get("turns").is_some_and(serde_json::Value::is_array) {
        bail!("replay export does not contain a valid turn list");
    }
    Ok(())
}

fn idempotency_key(share_id: ShareId, operation: &str) -> String {
    format!("share-{share_id}-{operation}")
}

async fn reserve(
    client: &Client,
    service_url: &str,
    access_token: &str,
    prepared: &PreparedShare,
) -> Result<ShareReservation> {
    for attempt in 0..TRANSFER_ATTEMPTS {
        let result = client
            .post(format!("{service_url}/v1/shares"))
            .bearer_auth(access_token)
            .header("Idempotency-Key", &prepared.reserve_idempotency_key)
            .json(&prepared.reserve)
            .send()
            .await;
        match result {
            Ok(response)
                if response.status().is_server_error() && attempt + 1 < TRANSFER_ATTEMPTS =>
            {
                continue;
            }
            Ok(response) => {
                let reservation: ShareReservation = super::decode_response(response).await?;
                validate_reservation(&reservation, prepared)?;
                return Ok(reservation);
            }
            Err(_) if attempt + 1 < TRANSFER_ATTEMPTS => continue,
            Err(error) => return Err(error).context("reserve encrypted replay share"),
        }
    }
    unreachable!("the transfer attempt range is non-empty")
}

fn validate_reservation(reservation: &ShareReservation, prepared: &PreparedShare) -> Result<()> {
    if reservation.version != SHARE_VERSION
        || reservation.share_id != prepared.share_id
        || reservation.expires_at_ms == 0
    {
        bail!("Anywhere service returned mismatched share reservation metadata");
    }
    let upload_url = Url::parse(&reservation.upload_url).context("invalid share upload URL")?;
    if upload_url.fragment().is_some() || !matches!(upload_url.scheme(), "https" | "http") {
        bail!("Anywhere service returned an unsafe share upload URL");
    }
    Ok(())
}

async fn upload(client: &Client, upload_url: &str, envelope: &[u8]) -> Result<()> {
    for attempt in 0..TRANSFER_ATTEMPTS {
        let result = client
            .put(upload_url)
            .header(CONTENT_TYPE, ANYWHERE_OBJECT_MEDIA_TYPE)
            .body(envelope.to_vec())
            .send()
            .await;
        match result {
            Ok(response)
                if response.status().is_server_error() && attempt + 1 < TRANSFER_ATTEMPTS =>
            {
                continue;
            }
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                let status = response.status();
                bail!("encrypted share upload returned HTTP {}", status.as_u16());
            }
            Err(_) if attempt + 1 < TRANSFER_ATTEMPTS => continue,
            Err(error) => return Err(error).context("upload encrypted replay share"),
        }
    }
    unreachable!("the transfer attempt range is non-empty")
}

async fn complete(
    client: &Client,
    service_url: &str,
    access_token: &str,
    prepared: &PreparedShare,
) -> Result<ShareCompletion> {
    for attempt in 0..TRANSFER_ATTEMPTS {
        let result = client
            .post(format!(
                "{service_url}/v1/shares/{}/complete",
                prepared.share_id
            ))
            .bearer_auth(access_token)
            .header("Idempotency-Key", &prepared.complete_idempotency_key)
            .send()
            .await;
        match result {
            Ok(response)
                if response.status().is_server_error() && attempt + 1 < TRANSFER_ATTEMPTS =>
            {
                continue;
            }
            Ok(response) => {
                let completion: ShareCompletion = super::decode_response(response).await?;
                if completion.version != SHARE_VERSION
                    || completion.share_id != prepared.share_id
                    || completion.expires_at_ms != prepared.expires_at_ms
                {
                    bail!("Anywhere service returned mismatched completed-share metadata");
                }
                return Ok(completion);
            }
            Err(_) if attempt + 1 < TRANSFER_ATTEMPTS => continue,
            Err(error) => return Err(error).context("complete encrypted replay share"),
        }
    }
    unreachable!("the transfer attempt range is non-empty")
}

fn public_url(
    service_url: &str,
    completion: &ShareCompletion,
    prepared: &PreparedShare,
) -> Result<Url> {
    let expected_path = format!("/shares/{}", prepared.share_id);
    if completion.url_path != expected_path {
        bail!("Anywhere service returned an invalid public share path");
    }
    let mut url = Url::parse(service_url).context("invalid Anywhere service URL")?;
    url.set_path(&completion.url_path);
    url.set_query(None);
    url.set_fragment(Some(&format!(
        "key={}",
        super::URL_SAFE_NO_PAD.encode(prepared.key)
    )));
    Ok(url)
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use ed25519_dalek::VerifyingKey;

    use super::*;

    fn state() -> LocalState {
        LocalState {
            version: super::super::STATE_VERSION,
            account_id: Some(hex::encode([0x11; 16])),
            device_id: Some(hex::encode([0x22; 16])),
            signing_private_key: Some(super::super::URL_SAFE_NO_PAD.encode([0x33; 32])),
            ..LocalState::default()
        }
    }

    fn prepared() -> PreparedShare {
        prepare_share(
            serde_json::json!({"session_id":"session-1","summary":{},"turns":[]}),
            "session-1",
            86_400,
            9,
            1_750_000_000_000,
            [0x44; 16],
            [0x55; 32],
            [0x66; 24],
            &state(),
        )
        .expect("prepare share")
    }

    #[test]
    fn prepared_share_is_kind_seven_and_decrypts_only_with_share_key() {
        let prepared = prepared();
        let envelope = Envelope::decode(&prepared.envelope).expect("decode envelope");
        assert_eq!(envelope.metadata.kind, EnvelopeKind::Share);
        assert_eq!(envelope.metadata.recipient_kind, RecipientKind::Share);
        assert_eq!(envelope.metadata.recipient_id, [0x44; 16]);
        assert_eq!(envelope.metadata.key_epoch, 0);
        let verifying_key = VerifyingKey::from_bytes(
            SigningKey::from_bytes(&[0x33; 32])
                .verifying_key()
                .as_bytes(),
        )
        .expect("verifying key");
        let plaintext = envelope
            .open(&prepared.key, &verifying_key)
            .expect("open share");
        let share: ReplayShare<serde_json::Value> =
            serde_json::from_slice(&plaintext).expect("decode share plaintext");
        assert_eq!(share.session_id, "session-1");
        assert_eq!(share.expires_at_ms, 1_750_086_400_000);
        assert!(envelope.open(&[0x77; 32], &verifying_key).is_err());
    }

    #[test]
    fn public_url_keeps_the_key_only_in_the_fragment() {
        let prepared = prepared();
        let completion = ShareCompletion {
            version: SHARE_VERSION,
            share_id: prepared.share_id,
            url_path: format!("/shares/{}", prepared.share_id),
            expires_at_ms: prepared.expires_at_ms,
        };
        let url = public_url("https://app.forge.example/api", &completion, &prepared)
            .expect("public URL");
        assert_eq!(url.path(), completion.url_path);
        assert!(url.query().is_none());
        assert_eq!(
            url.fragment(),
            Some("key=VVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVU")
        );
        assert!(!url
            .as_str()
            .split('#')
            .next()
            .unwrap_or_default()
            .contains("VVVV"));
    }

    #[test]
    fn retries_reuse_exact_ciphertext_and_stable_idempotency_keys() {
        let prepared = prepared();
        assert_eq!(
            prepared.reserve_idempotency_key,
            "share-44444444444444444444444444444444-reserve"
        );
        assert_eq!(
            prepared.complete_idempotency_key,
            "share-44444444444444444444444444444444-complete"
        );
        assert_eq!(
            prepared.reserve.ciphertext_bytes as usize,
            prepared.envelope.len()
        );
        assert_eq!(
            prepared.reserve.ciphertext_sha256,
            <[u8; 32]>::from(Sha256::digest(&prepared.envelope))
        );
    }

    #[test]
    fn invalid_expiry_and_untrusted_public_path_are_rejected() {
        assert!(prepare_share(
            serde_json::json!({"session_id":"session-1","turns":[]}),
            "session-1",
            42,
            0,
            0,
            [1; 16],
            [2; 32],
            [3; 24],
            &state(),
        )
        .is_err());
        let prepared = prepared();
        let completion = ShareCompletion {
            version: SHARE_VERSION,
            share_id: prepared.share_id,
            url_path: "https://evil.example/steal".into(),
            expires_at_ms: prepared.expires_at_ms,
        };
        assert!(public_url("https://app.forge.example", &completion, &prepared).is_err());
    }
}
