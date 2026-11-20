//! `forge anywhere devices`: list the account's devices, revoke one with a key rotation, or make
//! another device the owner.
//!
//! Any active device may revoke another; only the owner device may revoke the owner device, and
//! only the owner device may hand ownership to another device. The service enforces both rules;
//! the checks here only explain them before a request is sent.

use super::*;

pub(super) async fn devices(
    revoke: Option<&str>,
    make_owner: Option<&str>,
    yes: bool,
) -> Result<()> {
    let store = StateStore::platform()?;
    let mut state = store.load()?;
    let token = ensure_access_token(&store, &mut state).await?;
    let service_url = forge_config::load()?.anywhere.service_url().to_owned();
    refresh_account_epoch(&store, &mut state, &service_url, &token).await?;
    if let Some(device_id) = revoke {
        revoke_device(&store, &mut state, &service_url, &token, device_id).await?;
        return Ok(());
    }
    if let Some(device_id) = make_owner {
        let this_device = state.device_id.clone();
        return make_owner_command(&service_url, &token, this_device.as_deref(), device_id, yes)
            .await;
    }

    let rows: DeviceList = send_json(
        client()?
            .get(format!("{service_url}/v1/devices"))
            .bearer_auth(token),
    )
    .await
    .context("list Anywhere devices")?;
    let this_device = state.device_id.clone();
    for device in rows.devices {
        let mut tags = Vec::new();
        if this_device.as_deref() == Some(device.id.as_str()) {
            tags.push("this device");
        }
        if device.owner {
            tags.push("owner");
        }
        println!(
            "{}  {}  enrolled {}{}{}",
            device.id,
            device.name,
            display_timestamp(&device.created_at),
            device
                .last_seen_at
                .map(|seen| format!("  last seen {}", display_timestamp(&seen)))
                .unwrap_or_default(),
            if tags.is_empty() {
                String::new()
            } else {
                format!("  [{}]", tags.join(", "))
            }
        );
    }
    println!(
        "\nRevoke one with `forge anywhere devices --revoke <id>` (asks for your Recovery Kit)."
    );
    if owner_transfer_supported(&service_url)
        .await
        .unwrap_or(false)
    {
        println!("Make another device the owner with `forge anywhere devices --make-owner <id>`.");
    }
    Ok(())
}

/// Whether the service advertises `features.owner_transfer`.
pub(super) async fn owner_transfer_supported(service_url: &str) -> Result<bool> {
    let capabilities: ServiceCapabilities =
        send_json(client()?.get(format!("{service_url}/v1/capabilities")))
            .await
            .context("check Forge Anywhere service capabilities")?;
    Ok(capabilities.features.owner_transfer)
}

/// Why this device cannot make `target` the owner, or `None` when it may. The service enforces
/// the same rule; this only explains it before a request is sent.
pub(super) fn make_owner_refusal(
    devices: &[DeviceRow],
    this_device: Option<&str>,
    target: &str,
) -> Option<String> {
    let Some(row) = devices.iter().find(|device| device.id == target) else {
        return Some(format!("device {target} is not an active enrolled device"));
    };
    if row.owner {
        return Some(format!("{} is already the owner device", row.name));
    }
    let this_is_owner = devices
        .iter()
        .any(|device| device.owner && this_device == Some(device.id.as_str()));
    if !this_is_owner {
        return Some(
            "only the owner device can hand ownership to another device; run this on the owner"
                .to_string(),
        );
    }
    None
}

/// `POST /v1/devices/{id}/make-owner`; the service answers with the updated device list.
pub(super) async fn request_make_owner(
    service_url: &str,
    access_token: &str,
    target: &str,
) -> Result<DeviceList> {
    send_json(
        client()?
            .post(format!("{service_url}/v1/devices/{target}/make-owner"))
            .bearer_auth(access_token),
    )
    .await
    .context("make the device the Anywhere owner")
}

/// Checks the capability and the owner rule, then transfers. Returns the new owner's name.
pub(super) async fn make_owner_checked(
    service_url: &str,
    access_token: &str,
    this_device: Option<&str>,
    target: &str,
) -> Result<String> {
    if !owner_transfer_supported(service_url).await? {
        bail!("this Forge Anywhere service cannot change the owner device yet");
    }
    let rows: DeviceList = send_json(
        client()?
            .get(format!("{service_url}/v1/devices"))
            .bearer_auth(access_token),
    )
    .await
    .context("list Anywhere devices")?;
    if let Some(reason) = make_owner_refusal(&rows.devices, this_device, target) {
        bail!(reason);
    }
    let updated = request_make_owner(service_url, access_token, target).await?;
    let Some(owner) = updated.devices.iter().find(|device| device.owner) else {
        bail!("the service did not report an owner device after the transfer");
    };
    if owner.id != target {
        bail!("the service kept {} as the owner device", owner.name);
    }
    Ok(owner.name.clone())
}

async fn make_owner_command(
    service_url: &str,
    access_token: &str,
    this_device: Option<&str>,
    target: &str,
    yes: bool,
) -> Result<()> {
    if !yes {
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            bail!("pass --yes to make a device the owner without a terminal to confirm at");
        }
        eprint!(
            "Make {target} the owner? Only the owner can revoke the owner or hand ownership on, \
             and this device stops being the owner. [y/N] "
        );
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("Nothing changed.");
            return Ok(());
        }
    }
    let name = make_owner_checked(service_url, access_token, this_device, target).await?;
    println!("{name} is now the owner device of this Forge Anywhere account.");
    Ok(())
}

async fn revoke_device(
    store: &StateStore,
    state: &mut LocalState,
    service_url: &str,
    access_token: &str,
    target_device_id: &str,
) -> Result<()> {
    let rows: DeviceList = send_json(
        client()?
            .get(format!("{service_url}/v1/devices"))
            .bearer_auth(access_token),
    )
    .await
    .context("load active devices before revocation")?;
    let Some(target) = rows
        .devices
        .iter()
        .find(|device| device.id == target_device_id)
    else {
        bail!("device {target_device_id} is not an active enrolled device");
    };
    // The service enforces this too; checking first saves typing the Recovery Kit for nothing.
    let this_is_owner = rows
        .devices
        .iter()
        .any(|device| device.owner && state.device_id.as_deref() == Some(device.id.as_str()));
    if target.owner && !this_is_owner {
        bail!(
            "{} is the owner device of this account; only the owner device can revoke it, which \
             keeps a recovery path if another device is lost or compromised",
            target.name
        );
    }
    let account_id = decode_hex_array::<16>(
        state
            .account_id
            .as_deref()
            .context("Anywhere account id is missing")?,
        "account id",
    )?;
    let sender_device_id = decode_hex_array::<16>(
        state
            .device_id
            .as_deref()
            .context("Anywhere device id is missing")?,
        "device id",
    )?;
    let signing_private = decode_base64_array::<32>(
        state
            .signing_private_key
            .as_deref()
            .context("Anywhere signing key is missing")?,
        "signing private key",
    )?;
    let signing_key = SigningKey::from_bytes(&signing_private);
    let exchange_private = decode_base64_array::<32>(
        state
            .exchange_private_key
            .as_deref()
            .context("Anywhere exchange key is missing")?,
        "exchange private key",
    )?;
    let current_data_key = decode_base64_array::<32>(
        state
            .account_data_key
            .as_deref()
            .context("Anywhere account data key is missing")?,
        "account data key",
    )?;
    let current_epoch = state.key_epoch.context("Anywhere key epoch is missing")?;
    let new_epoch = current_epoch
        .checked_add(1)
        .context("Anywhere key epoch is exhausted")?;

    let current_recovery: RecoveryWrapResponse = send_json(
        client()?
            .get(format!(
                "{service_url}/v1/key-epochs/{current_epoch}/wraps/recovery"
            ))
            .bearer_auth(access_token),
    )
    .await
    .context("load the current encrypted recovery wrap")?;
    if current_recovery.epoch != current_epoch {
        bail!("service returned the wrong recovery key epoch");
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        bail!("revoking a device needs the Recovery Kit typed at a terminal; run this in an interactive shell");
    }
    let recovery_words = rpassword::prompt_password(
        "Recovery Kit (12 or 24 words, or the path to a .forge-recovery file): ",
    )?;
    let recovery_words = recovery_words.trim();
    let old_envelope = Envelope::decode(
        &URL_SAFE_NO_PAD
            .decode(&current_recovery.recovery_wrap_envelope)
            .context("decode current recovery wrap")?,
    )?;
    let old_recovery_key =
        recovery_wrap_key_from_input(recovery_words, service_url, &account_id, current_epoch)?;
    let old_signing_key = ed25519_dalek::VerifyingKey::from_bytes(&decode_base64_array(
        &current_recovery.signing_public_key,
        "recovery-wrap signing key",
    )?)?;
    let recovered = old_envelope.open(old_recovery_key.as_bytes(), &old_signing_key)?;
    if recovered.as_slice() != current_data_key {
        bail!("recovery phrase does not match the current account key; no device was revoked");
    }

    let new_data_key = rand::random::<[u8; 32]>();
    let wrap_count = rows.devices.len().saturating_sub(1).saturating_add(1);
    let (reserved_state, mut next_sequence) = store.reserve_sequences(wrap_count)?;
    *state = reserved_state;
    let mut device_wraps = Vec::with_capacity(rows.devices.len().saturating_sub(1));
    for device in &rows.devices {
        if device.id == target_device_id {
            continue;
        }
        let recipient_id = decode_hex_array::<16>(&device.id, "recipient device id")?;
        let recipient_exchange_key = decode_base64_array::<32>(
            device
                .exchange_public_key
                .as_deref()
                .context("service omitted a device exchange key")?,
            "device exchange public key",
        )?;
        let wrap_key = derive_device_wrap_key(
            &exchange_private,
            &recipient_exchange_key,
            &account_id,
            new_epoch,
        )?;
        let envelope = key_wrap_envelope(
            new_data_key,
            *wrap_key.as_bytes(),
            account_id,
            sender_device_id,
            RecipientKind::Device,
            recipient_id,
            new_epoch,
            next_sequence,
            &signing_key,
        )?;
        next_sequence = next_sequence
            .checked_add(1)
            .context("Anywhere sequence is exhausted")?;
        device_wraps.push(RotationDeviceWrap {
            device_id: device.id.clone(),
            envelope: URL_SAFE_NO_PAD.encode(envelope),
        });
    }
    let recovery_wrap_key =
        recovery_wrap_key_from_input(recovery_words, service_url, &account_id, new_epoch)?;
    let recovery_wrap = key_wrap_envelope(
        new_data_key,
        *recovery_wrap_key.as_bytes(),
        account_id,
        sender_device_id,
        RecipientKind::Account,
        account_id,
        new_epoch,
        next_sequence,
        &signing_key,
    )?;
    next_sequence = next_sequence
        .checked_add(1)
        .context("Anywhere sequence is exhausted")?;

    debug_assert_eq!(next_sequence, state.next_sequence);
    let response: RevokeDeviceResponse = send_json(
        client()?
            .post(format!(
                "{service_url}/v1/devices/{target_device_id}/revoke"
            ))
            .bearer_auth(access_token)
            .header("Idempotency-Key", idempotency_key())
            .json(&RevokeDeviceRequest {
                epoch: new_epoch,
                recovery_wrap_envelope: URL_SAFE_NO_PAD.encode(recovery_wrap),
                device_wraps,
            }),
    )
    .await
    .context("atomically revoke device and rotate the account key")?;
    if response.epoch != new_epoch {
        bail!("service acknowledged the wrong replacement key epoch");
    }
    let target_is_local = state.device_id.as_deref() == Some(target_device_id);
    let encoded_data_key = URL_SAFE_NO_PAD.encode(new_data_key);
    *state = store.update(|latest| {
        if target_is_local {
            latest.clear_tokens();
        } else if latest.key_epoch.unwrap_or(0) < new_epoch {
            latest.account_data_key = Some(encoded_data_key.clone());
            latest.key_epoch = Some(new_epoch);
            latest.data_key_epochs.insert(new_epoch, encoded_data_key);
        }
        Ok(())
    })?;
    println!("Revoked device {target_device_id} and rotated encrypted data to epoch {new_epoch}.");
    account_sync::rotate::after_revoke(state, target_is_local);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::extract::{Path, State};
    use axum::http::HeaderMap;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::{json, Value};

    use super::*;

    fn row(id: &str, owner: bool) -> DeviceRow {
        DeviceRow {
            id: id.into(),
            name: format!("device {id}"),
            created_at: "1".into(),
            last_seen_at: None,
            signing_public_key: None,
            exchange_public_key: None,
            owner,
        }
    }

    #[test]
    fn only_the_owner_may_hand_ownership_to_another_active_device() {
        let rows = [row("aa", true), row("bb", false)];
        assert_eq!(make_owner_refusal(&rows, Some("aa"), "bb"), None);
        assert!(make_owner_refusal(&rows, Some("bb"), "bb")
            .unwrap()
            .contains("only the owner device"));
        assert!(make_owner_refusal(&rows, Some("aa"), "aa")
            .unwrap()
            .contains("already the owner"));
        assert!(make_owner_refusal(&rows, Some("aa"), "cc")
            .unwrap()
            .contains("not an active enrolled device"));
        assert!(make_owner_refusal(&rows, None, "bb").is_some());
    }

    #[derive(Default)]
    struct Seen {
        owner: String,
        calls: Vec<(String, Option<String>)>,
    }

    async fn service(owner_transfer: Option<bool>) -> (String, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen {
            owner: "aa".into(),
            ..Seen::default()
        }));
        fn list(owner: &str) -> Value {
            json!({"version": 1, "devices": [
                {"id": "aa", "name": "desk", "created_at": "1", "owner": owner == "aa"},
                {"id": "bb", "name": "phone", "created_at": "2", "owner": owner == "bb"},
            ]})
        }
        let mut features = json!({"account_bound_enrollment": true, "recovery_kit_v2": true});
        if let Some(flag) = owner_transfer {
            features["owner_transfer"] = json!(flag);
        }
        let capabilities = json!({
            "version": 1, "protocol_version": 2, "maximum_client_major": 2, "ready": true,
            "features": features,
        });
        let app = Router::new()
            .route(
                "/v1/capabilities",
                get(move || {
                    let capabilities = capabilities.clone();
                    async move { Json(capabilities) }
                }),
            )
            .route(
                "/v1/devices",
                get(|State(seen): State<Arc<Mutex<Seen>>>| async move {
                    Json(list(&seen.lock().unwrap().owner))
                }),
            )
            .route(
                "/v1/devices/{id}/make-owner",
                post(
                    |State(seen): State<Arc<Mutex<Seen>>>,
                     Path(id): Path<String>,
                     headers: HeaderMap| async move {
                        let mut seen = seen.lock().unwrap();
                        let bearer = headers
                            .get("authorization")
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_owned);
                        seen.calls.push((id.clone(), bearer));
                        seen.owner = id;
                        Json(list(&seen.owner))
                    },
                ),
            )
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{address}"), seen)
    }

    #[tokio::test]
    async fn make_owner_posts_with_the_access_token_and_reads_back_the_new_owner() {
        let (url, seen) = service(Some(true)).await;
        let name = make_owner_checked(&url, "access-1", Some("aa"), "bb")
            .await
            .unwrap();
        assert_eq!(name, "phone");
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.calls,
            vec![("bb".to_string(), Some("Bearer access-1".to_string()))]
        );
    }

    #[tokio::test]
    async fn an_older_service_without_the_capability_is_never_asked() {
        for flag in [None, Some(false)] {
            let (url, seen) = service(flag).await;
            assert!(!owner_transfer_supported(&url).await.unwrap());
            let error = make_owner_checked(&url, "access-1", Some("aa"), "bb")
                .await
                .unwrap_err();
            assert!(error.to_string().contains("cannot change the owner"));
            assert!(seen.lock().unwrap().calls.is_empty());
        }
    }

    #[tokio::test]
    async fn a_non_owner_is_refused_before_any_request() {
        let (url, seen) = service(Some(true)).await;
        let error = make_owner_checked(&url, "access-1", Some("bb"), "aa")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already the owner"), "{error}");
        let error = make_owner_checked(&url, "access-1", Some("zz"), "bb")
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("only the owner device"),
            "{error}"
        );
        assert!(seen.lock().unwrap().calls.is_empty());
    }
}
