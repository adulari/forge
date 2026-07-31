//! Durable command validation, claiming, and acknowledgement policy.

use super::*;

pub(super) fn verify_command_envelope(
    metadata: &QueuedCommandMetadata,
    encoded: &[u8],
    identity: &Identity,
    devices: &HashMap<[u8; 16], VerifyingKey>,
) -> Result<VerifiedCommand> {
    if encoded.len() as u64 != metadata.ciphertext_bytes {
        bail!("durable command length does not match its metadata");
    }
    let envelope = Envelope::decode(encoded).context("decode durable command envelope")?;
    if envelope.metadata.kind != EnvelopeKind::Command {
        bail!("durable command envelope has the wrong kind");
    }
    validate_inbound_routing(&envelope, identity)?;
    if envelope.metadata.sender_device_id != metadata.sender_device_id
        || envelope.metadata.created_at_ms != metadata.created_at_ms
    {
        bail!("durable command envelope does not match its queue metadata");
    }
    let sender_device_id = envelope.metadata.sender_device_id;
    let verifying_key = devices
        .get(&sender_device_id)
        .context("durable command came from an unknown or revoked device")?;
    let data_key = identity
        .data_key_epochs
        .get(&envelope.metadata.key_epoch)
        .context("durable command uses an unavailable Account Data Key epoch")?;
    let plaintext = envelope
        .open(data_key, verifying_key)
        .context("authenticate and decrypt durable command")?;
    Ok(VerifiedCommand {
        sender_device_id,
        key_epoch: envelope.metadata.key_epoch,
        sequence: envelope.metadata.sequence,
        plaintext,
    })
}

pub(super) fn decode_command_request(plaintext: &[u8]) -> Result<BridgeRequest> {
    let request: BridgeRequest =
        serde_json::from_slice(plaintext).context("decode durable bridge request")?;
    if serde_json::to_vec(&request).context("canonicalize durable bridge request")? != plaintext {
        bail!("durable bridge request is not canonical JSON");
    }
    Ok(request)
}

pub(super) fn validate_command_request(
    request: &BridgeRequest,
) -> std::result::Result<(), CommandErrorCode> {
    if request.body_blob.is_some() {
        return Err(CommandErrorCode::InvalidCommand);
    }
    if request.headers.iter().any(|(name, _)| {
        !matches!(
            name.to_ascii_lowercase().as_str(),
            "accept" | "content-type"
        )
    }) {
        return Err(CommandErrorCode::PermissionDenied);
    }
    if matches!(
        request.route,
        RouteId::WebSocket | RouteId::TerminalWebSocket
    ) {
        return Err(CommandErrorCode::PermissionDenied);
    }
    if request.route == RouteId::Health {
        if request.method != Method::GET.as_str()
            || !request.parameters.is_empty()
            || !request.headers.is_empty()
            || !request.body.is_empty()
        {
            return Err(CommandErrorCode::InvalidCommand);
        }
        return Ok(());
    }
    let target = route_target(request).map_err(|_| CommandErrorCode::PermissionDenied)?;
    let method = Method::from_bytes(request.method.as_bytes())
        .map_err(|_| CommandErrorCode::InvalidCommand)?;
    if method != target.method {
        return Err(CommandErrorCode::PermissionDenied);
    }
    Ok(())
}

pub(super) fn command_result_for_status(status: u16) -> CommandResult {
    if status < 400 {
        CommandResult::Success
    } else {
        CommandResult::Error {
            code: CommandErrorCode::ExecutionFailed,
            retryable: status >= 500,
        }
    }
}

pub(super) fn command_journal_status(
    store: &StateStore,
    metadata: &QueuedCommandMetadata,
    worker_id: &str,
    now: u64,
) -> Result<CommandJournalStatus> {
    let state = store.load()?;
    let Some(entry) = state.command_journal.get(&metadata.command_id.to_string()) else {
        return Ok(CommandJournalStatus::New);
    };
    validate_journal_metadata(entry, metadata)?;
    journal_entry_status(entry, worker_id, now)
}

pub(super) fn journal_entry_status(
    entry: &CommandJournalEntry,
    _worker_id: &str,
    now: u64,
) -> Result<CommandJournalStatus> {
    match &entry.state {
        CommandJournalState::DispatchStarted { lease_until_ms, .. } if *lease_until_ms > now => {
            Ok(CommandJournalStatus::Busy)
        }
        CommandJournalState::DispatchStarted { .. } => Ok(CommandJournalStatus::DispatchUncertain),
        CommandJournalState::AcknowledgementReady {
            result,
            envelope,
            idempotency_key,
        } => Ok(CommandJournalStatus::AcknowledgementReady(
            PendingAcknowledgement {
                envelope: URL_SAFE_NO_PAD
                    .decode(envelope)
                    .context("decode persisted command acknowledgement")?,
                idempotency_key: idempotency_key.clone(),
                result: *result,
            },
        )),
        CommandJournalState::Acked { .. } => Ok(CommandJournalStatus::Acked),
    }
}

pub(super) fn validate_journal_metadata(
    entry: &CommandJournalEntry,
    metadata: &QueuedCommandMetadata,
) -> Result<()> {
    if entry.sender_device_id != hex::encode(metadata.sender_device_id)
        || entry.created_at_ms != metadata.created_at_ms
        || entry.expires_at_ms != metadata.expires_at_ms
        || entry.ciphertext_bytes != metadata.ciphertext_bytes
    {
        bail!("durable command metadata changed after it was journaled");
    }
    Ok(())
}

pub(super) fn begin_command(
    store: &StateStore,
    metadata: &QueuedCommandMetadata,
    verified: &VerifiedCommand,
    worker_id: &str,
    now: u64,
) -> Result<CommandJournalStatus> {
    let command_id = metadata.command_id.to_string();
    let mut status = CommandJournalStatus::New;
    store.update(|state| {
        if let Some(entry) = state.command_journal.get(&command_id) {
            validate_journal_metadata(entry, metadata)?;
            if entry.sender_device_id != hex::encode(verified.sender_device_id)
                || entry.key_epoch != verified.key_epoch
                || entry.sequence != verified.sequence
            {
                bail!("durable command envelope changed after it was journaled");
            }
            status = journal_entry_status(entry, worker_id, now)?;
            return Ok(());
        }
        let sender_device_id = hex::encode(verified.sender_device_id);
        if state.command_journal.iter().any(|(other_id, entry)| {
            other_id != &command_id
                && entry.sender_device_id == sender_device_id
                && entry.key_epoch == verified.key_epoch
                && entry.sequence == verified.sequence
        }) {
            bail!("durable command reused an authenticated sender/epoch/sequence tuple");
        }
        let lease_until_ms = now
            .checked_add(COMMAND_WORKER_LEASE_MS)
            .context("durable command worker lease overflow")?;
        state.command_journal.insert(
            command_id,
            CommandJournalEntry {
                sender_device_id,
                key_epoch: verified.key_epoch,
                sequence: verified.sequence,
                created_at_ms: metadata.created_at_ms,
                expires_at_ms: metadata.expires_at_ms,
                ciphertext_bytes: metadata.ciphertext_bytes,
                state: CommandJournalState::DispatchStarted {
                    worker_id: worker_id.to_owned(),
                    lease_until_ms,
                },
            },
        );
        status = CommandJournalStatus::Claimed;
        Ok(())
    })?;
    Ok(status)
}

pub(super) fn ensure_command_acknowledgement(
    store: &StateStore,
    command_id: CommandId,
    result: CommandResult,
) -> Result<PendingAcknowledgement> {
    let journal_key = command_id.to_string();
    let state = store.load()?;
    let entry = state
        .command_journal
        .get(&journal_key)
        .context("durable command is missing from its journal")?;
    match &entry.state {
        CommandJournalState::AcknowledgementReady {
            result,
            envelope,
            idempotency_key,
        } => {
            return Ok(PendingAcknowledgement {
                envelope: URL_SAFE_NO_PAD
                    .decode(envelope)
                    .context("decode persisted command acknowledgement")?,
                idempotency_key: idempotency_key.clone(),
                result: *result,
            });
        }
        CommandJournalState::Acked { .. } => bail!("durable command is already acknowledged"),
        CommandJournalState::DispatchStarted { .. } => {}
    }
    let recipient_device_id =
        decode_hex_array(&entry.sender_device_id, "command sender device id")?;
    let plaintext = serde_json::to_vec(&CommandAcknowledgement { command_id, result })
        .context("encode durable command acknowledgement")?;
    let envelope = seal_for_recipient(
        store,
        EnvelopeKind::Acknowledgement,
        RecipientKind::Device,
        recipient_device_id,
        &plaintext,
    )?;
    let pending = PendingAcknowledgement {
        envelope,
        idempotency_key: idempotency_key(),
        result,
    };
    let mut persisted = pending.clone();
    store.update(|state| {
        let entry = state
            .command_journal
            .get_mut(&journal_key)
            .context("durable command disappeared from its journal")?;
        match &entry.state {
            CommandJournalState::AcknowledgementReady {
                result,
                envelope,
                idempotency_key,
            } => {
                persisted = PendingAcknowledgement {
                    envelope: URL_SAFE_NO_PAD
                        .decode(envelope)
                        .context("decode persisted command acknowledgement")?,
                    idempotency_key: idempotency_key.clone(),
                    result: *result,
                };
            }
            CommandJournalState::DispatchStarted { .. } => {
                entry.state = CommandJournalState::AcknowledgementReady {
                    result,
                    envelope: URL_SAFE_NO_PAD.encode(&pending.envelope),
                    idempotency_key: pending.idempotency_key.clone(),
                };
            }
            CommandJournalState::Acked { .. } => {
                bail!("durable command was acknowledged concurrently")
            }
        }
        Ok(())
    })?;
    Ok(persisted)
}

pub(super) fn mark_command_acked(
    store: &StateStore,
    command_id: CommandId,
    acked_at_ms: u64,
) -> Result<()> {
    let journal_key = command_id.to_string();
    store.update(|state| {
        let entry = state
            .command_journal
            .get_mut(&journal_key)
            .context("acknowledged durable command is missing from its journal")?;
        match entry.state {
            CommandJournalState::AcknowledgementReady { .. } => {
                entry.state = CommandJournalState::Acked { acked_at_ms };
            }
            CommandJournalState::Acked { .. } => {}
            CommandJournalState::DispatchStarted { .. } => {
                bail!("durable command acknowledgement was not persisted")
            }
        }
        Ok(())
    })?;
    Ok(())
}

pub(super) fn prune_command_journal(store: &StateStore, now: u64) -> Result<()> {
    store.update(|state| {
        state.command_journal.retain(|_, entry| {
            entry
                .expires_at_ms
                .checked_add(COMMAND_JOURNAL_RETENTION_MS)
                .is_some_and(|retain_until| retain_until > now)
        });
        Ok(())
    })?;
    Ok(())
}
