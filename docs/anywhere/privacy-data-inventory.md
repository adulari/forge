# Forge Anywhere privacy data inventory

Forge Anywhere collects the minimum metadata needed to authenticate, bill, route encrypted data,
enforce quotas, operate the service, and understand the conversion funnel. It must never collect
product content for analytics or support.

## Data inventory

| Category | Examples | Purpose | Retention/control |
| --- | --- | --- | --- |
| Account identity | Immutable GitHub user ID, login name, account ID | One trial per account, login, ownership | Until account deletion; live removal within 24 hours |
| Device/host metadata | Opaque IDs, public signing/exchange keys, user-supplied host/device display name, last heartbeat | Enrollment, routing, revocation | Until revoked/deleted; display names are account-visible only and never analytics dimensions |
| Authentication | Hashed opaque access/refresh tokens, token family, expiry, reuse status | Session security and reuse detection | Access 15 minutes; rotating refresh family 30 days; revoke on logout/reuse |
| Billing | Paddle customer/subscription IDs, state, paid-through/occurrence times, processed event ID | Checkout and entitlement | Required billing/legal period; no payment-card data is stored by Forge |
| Routing/security metadata | Envelope kind, account/sender/recipient IDs, key epoch, sequence, time, ciphertext size, IP/security event | Delivery, replay rejection, abuse defense | Minimize and aggregate; metadata-only audit retention documented operationally |
| Encrypted objects | Sync records, blobs, capsules, shares, queued commands | User-requested cloud features | Per published object and entitlement retention policies |
| Quota/operations | Bytes, object count, daily request/frame counters, latency/error buckets, readiness/backup lag | Limits, reliability, abuse control | Aggregate wherever possible; no content-derived labels |
| Push | Device push token, platform/environment, generic event class | Notify that attention is needed | Until revoked/invalid; never sync into history or log the token |
| Funnel analytics | Allowlisted event and coarse timestamp/source | Measure activation and conversion | First-party only; no content or stable cross-site advertising identifier |

Backups contain encrypted objects and the same service metadata, never decryption keys. Deleted live
data is removed within 24 hours and encrypted backups expire within 30 days. Expired/suspended
subscription data follows the 90-day lifecycle with 30-day and 7-day warnings.

## Content denylist

The service, logs, metrics, analytics, push, audit events, and support tooling must not record:

- prompts, responses, transcript text, messages, tool input/output, commands, or error text derived
  from user work;
- filenames, paths, repository/project names, remotes, branches, source code, diffs, capsule entries,
  or session titles;
- provider credentials, API keys, keyring content, local daemon tokens, device private keys,
  Account Data Keys, recovery words, share-fragment keys, or push tokens;
- decrypted sync/history/share/capsule content, embeddings, or content-derived classifications;
- full request/response bodies for encrypted or authentication endpoints.

Opaque IDs must not be joined to marketing profiles or exported to ad networks. Security support
requests use opaque envelope/object/account identifiers and categorical errors only.

## Funnel-event allowlist

Only this ordered vocabulary is accepted:

```text
landing_view
trial_start
first_host
first_remote_session
first_handoff
checkout
paid
```

Allowed properties are event schema version, coarse timestamp, first-party page/campaign source,
anonymous pre-account visit ID, opaque account ID after authentication, selected billing interval,
client platform/version, and categorical success/failure. Drop free-form values and unknown
properties at ingestion. Do not attach GitHub login, IP-derived location, host/device display name,
repository/session/object IDs, ciphertext size, prompt/command/file data, or failure text.

`landing_view` may use Cloudflare Web Analytics. Product funnel events are first-party. No
marketing-cookie banner is needed until non-essential tracking, cross-site identifiers, replay
analytics, or advertising tools are introduced; adding any of those requires a new privacy review.

## Review checklist

- [ ] Every new database column, log field, metric label, push field, and analytics property has an
  owner, purpose, retention, and deletion behavior.
- [ ] Free-form strings from clients are rejected or kept inside ciphertext.
- [ ] Reverse-proxy access logs exclude query strings, credentials, bodies, and sensitive routes.
- [ ] Known-plaintext canaries scan logs, SQLite, R2 captures, and restored backups.
- [ ] Account export contains documented metadata/ciphertext; deletion covers DB, R2, tokens, push,
  pending uploads, shares, capsules, and billing linkage as legally permitted.
- [ ] Privacy/terms name GitHub, Paddle, Cloudflare/R2, backup processing, and retention accurately.
