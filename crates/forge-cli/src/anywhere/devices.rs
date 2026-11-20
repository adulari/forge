//! `forge anywhere devices`: list the account's devices and revoke one with a key rotation.
//!
//! Any active device may revoke another; only the owner device may revoke the owner device. The
//! service enforces that rule; the check here only saves typing the Recovery Kit for nothing.

use super::*;

pub(super) async fn devices(revoke: Option<&str>) -> Result<()> {
    let store = StateStore::platform()?;
    let mut state = store.load()?;
    let token = ensure_access_token(&store, &mut state).await?;
    let service_url = forge_config::load()?.anywhere.service_url().to_owned();
    refresh_account_epoch(&store, &mut state, &service_url, &token).await?;
    if let Some(device_id) = revoke {
        revoke_device(&store, &mut state, &service_url, &token, device_id).await?;
        return Ok(());
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
