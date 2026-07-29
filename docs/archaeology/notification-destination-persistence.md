# Code archaeology: notification destination persistence

## Summary

Web Push subscriptions, APNs device registrations, and per-session Live Activity tokens are one notification-destination persistence domain. They now share a dedicated store module while preserving deduplication keys, stable delivery order, token replacement, and invalid-destination deletion.

## History and invariants

- `9f97bb6a` introduced Web Push subscriptions for actionable remote notifications. Browser re-subscription updates encryption keys by endpoint rather than creating duplicate deliveries.
- `9fafacb2` introduced APNs device registrations and Live Activity update tokens. APNs rows are keyed by device token; Live Activity rows are keyed by session.
- Migration 13 added the Web Push endpoint unique index, and migration 20 adds the APNs device-token unique index after deduplicating legacy rows, so concurrent upserts cannot race duplicate destinations.

## Boundary

`notification_store.rs` owns registration, replacement, lookup/listing, and deletion for every persisted notification destination. Push delivery, payload privacy, provider credentials, environment routing, and invalid-token response handling remain in their transport owners.

## Interface as test surface

- `push_subscription_crud_dedupes_by_endpoint` and `push_subscription_endpoint_has_a_unique_index` characterize stable identity and concurrent-safe deduplication.
- `apns_subscription_crud_dedupes_by_device_token`, `apns_device_token_has_a_unique_index`, and `migration_0020_dedupes_legacy_rows_and_rebuilds_the_identity_index` characterize atomic environment refresh, legacy repair, malformed-index replacement, and idempotent replay.
- `live_activity_token_upserts_and_replaces_by_session` characterizes one current token per session.

Deleting the module removes the notification registration API and breaks these direct persistence characterizations.

## Leave alone

- Web Push identity is the endpoint; key refresh retains the existing row id.
- APNs identity is the device token; environment refresh is an atomic upsert and does not create duplicates.
- Live Activity identity is the session; a new push token replaces the prior token.
- Listing order remains deterministic by creation time and id.
- Delete operations report whether a destination actually existed.
- Destination authentication material is stored here because delivery requires it; provider credentials and notification payload contents remain outside these rows.
