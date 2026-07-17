# Forge Anywhere rollout checklists

These gates are cumulative. Record the date, client/service versions, test account, hosts, devices,
operator, result, and linked incident for every run. A checked box means evidence exists; it is not
a forecast. Never record plaintext, repository names, filenames, prompts, commands, or diffs in the
evidence.

## Before owner alpha

- [ ] Public protocol specification, threat model, licensing boundary, and Rust/TypeScript/service
  golden vectors agree.
- [ ] Wrong-key, corrupted-header, tamper, nonce, signature, and replay tests pass.
- [ ] GitHub device flow, expiry, token rotation/reuse detection, logout, and revocation tests pass.
- [ ] Paddle sandbox signature, duplicate, delayed/out-of-order, cancellation, renewal, failure,
  recovery, and entitlement-transition tests pass.
- [ ] Cross-account authorization and relay-isolation tests cover every API group.
- [ ] Quota race, abandoned upload, object mismatch, retention, deletion, and restore tests pass.
- [ ] Direct HTTP/WebSocket, LAN pairing, and user-managed `serve --anywhere` regression tests pass.
- [ ] Production and backup R2 buckets are separate; lifecycle policies match the published
  retention contract.
- [ ] SQLite WAL, foreign keys, busy timeout, bounded pool, Litestream replication, readiness, and
  metadata-only metrics are active.
- [ ] GitHub, Paddle, R2, and service credentials are systemd credentials, not arguments, env files
  in source control, or logs.
- [ ] Privacy inventory and AGPL service-boundary checklists are reviewed.
- [ ] Known-plaintext canary passes against captured logs, SQLite, R2 objects, and a restored backup.

## Owner alpha: minimum seven days

- [ ] Use two real hosts, one iPhone, and web daily for at least seven consecutive days.
- [ ] Complete login, bootstrap, QR pairing, revocation, reconnect, key-epoch rotation, and offline
  history flows.
- [ ] Exercise live control, a queued remote job, generic push, encrypted sync, conflict copies,
  handoff in both directions, share creation/revocation/expiry, and account export.
- [ ] Complete the documented recovery drill from a clean device without using server plaintext.
- [ ] Complete failed-handoff drills at upload, claim, import, acknowledgement, and cleanup points.
- [ ] Complete a Litestream restore into an isolated environment and run integrity/canary checks.
- [ ] Validate at least 500 concurrent WebSockets and 50 encrypted frames/second with p95 relay
  latency below 250 ms and service memory below 1.5 GiB on the intended VPS.
- [ ] Confirm alerts for readiness, 5xx rate, DB lock time, Paddle backlog, R2 errors, relay
  disconnects, backup lag, quota anomalies, disk, and memory.
- [ ] Confirm no data-loss defect and no unresolved security/privacy severity-1 or severity-2 issue.

**Exit gate:** seven days completed, recovery and failed-handoff drills pass, rollback paths are
understood, and the owner explicitly signs off the evidence.

## Closed beta: 20–30 individual developers

- [ ] Invite only individual developers; do not imply team/organization support.
- [ ] Show €10 monthly and €79 annual together; confirm the no-card trial starts on first host, once
  per GitHub account.
- [ ] Exercise real Paddle sandbox and limited production transitions, including cancellation and
  payment recovery.
- [ ] Measure only allowlisted funnel events and operational metadata; verify no marketing cookies
  or content collection.
- [ ] Track activation through first host, first remote session, and first handoff.
- [ ] Review availability, relay reconnects, sync lag/conflicts, failed handoffs, quota behavior,
  recovery success, support time, and deletion completion at least weekly.
- [ ] Send 30-day and 7-day retention warnings in a time-compressed test account.
- [ ] Sample cross-account isolation, object ownership, sequence replay, and idempotency tests on the
  deployed build.
- [ ] Achieve at least 99.5% beta availability excluding announced maintenance.
- [ ] Resolve every data-loss defect and review privacy, terms, refund wording, and AGPL separation.

**Exit gate:** no known data-loss defect, successful backup restoration, 99.5% beta availability,
reviewed legal/security boundaries, and support load judged sustainable.

## Paid launch

- [ ] Production GitHub, Paddle, R2, Cloudflare, nginx, systemd, Litestream, and alerting are tested
  with least-privilege credentials and documented rotation.
- [ ] `app.forge.adulari.dev` and `forge.adulari.dev/anywhere` have valid TLS, body/rate limits,
  origin restriction, readiness behavior, privacy, terms, refund, and account-control links.
- [ ] Homepage CTA says “Start 14-day trial with GitHub”; pricing defaults to annual while showing
  both options.
- [ ] Demo shows desktop work, iPhone response, and complete laptop handoff without exposing private
  content.
- [ ] Local Forge, LAN, direct pairing, and a user-managed tunnel work during a forced Anywhere
  outage and with an expired account.
- [ ] Production canary, restore drill, deletion audit, webhook replay, quota race, and load test pass.
- [ ] On-call owner, incident severity, rollback, customer notification, and support paths are named.
- [ ] The homepage CTA remains gated until all preceding checks have evidence.

## First 90 days

- [ ] Target 100 trials, at least 50 activated users, and 15 paid users.
- [ ] Keep monthly churn below 5% and support below one hour per customer per month.
- [ ] Review trial-to-paid conversion without collecting product content.
- [ ] If conversion is below 10%, interview activated and abandoned users before building team or
  enterprise features.
- [ ] Run and record one restore drill each month.
