# ADR-0012: hosted APNs push relay, opt-out default

- **Status:** Accepted
- **Date:** 2026-07-08
- **Deciders:** Floris Voskamp

## Context

Native iOS push (APNs) shipped this session (`crates/forge-cli/src/apns.rs`), but sending a
push requires a Team-scoped Apple Developer private key (`.p8` + Team ID + Key ID) that only
the app's owner has (Team ID `95VXXPD28Y`, bundle `dev.adulari.forge`). Requiring every
self-hosting end user to personally generate their own Apple Developer key just to get push
working is unreasonable setup friction for anyone who isn't the project owner — and Forge is
built for a real userbase, not a single operator.

This ADR was anticipated by ADR-0002, which already carved out the exception: *"team relay is
the only plausible future service, and it would be a separate component"* / *"Future
hosted-relay would be a new top-level component, not a refactor of this one."*

It also touches two existing project claims that need scoped, truthful updates rather than
silent contradiction: `docs/features/remote-control.md` currently says push is "self-hosted,
no relay" (written for Web Push, before APNs existed), and the App Store reviewer text in
`docs/mobile/APP_STORE_CHECKLIST.md` says "no backend service operated by us." Both are
updated alongside this ADR (see `docs/features/remote-control.md` §2f and
`APP_STORE_CHECKLIST.md` §6).

There is no rate-limiting/abuse-prevention precedent anywhere in this codebase — the relay is
new territory and must bring its own, sized to its actual (narrow) risk: an abuser can only
burn the operator's own Apple push quota against the operator's own app topic; they can never
reach arbitrary devices, and no session content, source code, or account data ever crosses
the relay.

## Options considered

**Hosting**
1. **Cloudflare Workers (TypeScript).** Free tier is effectively free at this volume; global
   edge; no server to patch. Rejected: requires re-deriving `apns.rs`'s already-tested ES256
   JWT construction (`p256`/RustCrypto) from scratch via `crypto.subtle`, with no shared test
   vector, and Workers' `fetch()` gives no explicit guaranteed HTTP/2-to-origin client while
   Apple's push gateway is documented to reject HTTP/1.1 — a foundational bet on an unverified
   platform capability for the one payload Apple will silently reject if subtly wrong.
2. **A new standalone Rust service, `crates/forge-relay`, on Fly.io.** Reuses `apns.rs`'s
   ~150 already-tested lines (JWT mint/cache, host-by-environment, POST shape) nearly
   verbatim. `crates/xtasks` is existing precedent for a second `[[bin]]` workspace member
   outside `forge-cli`.

**Default behavior**
1. **Opt-in relay** (env var required to enable). Preserves "local-first by default" most
   literally, but reproduces exactly the setup friction this feature exists to remove — a
   typical self-hoster gets no native push at all unless they read documentation closely
   enough to find and set a flag.
2. **Opt-out relay** (relay is the zero-config default; `FORGE_APNS_KEY_PATH` set still
   always wins for BYO-key operators; `FORGE_APNS_DISABLE_RELAY=1` disables native push
   entirely rather than silently downgrading). Loud startup banner names the relay and the
   opt-out flag so the default is disclosed, not silent.

**Abuse prevention**
1. **A shared secret baked into every distributed `forge-cli` binary.** Rejected: trivially
   extractable from a public OSS binary (`strings`/disassembly), so it only deters casual
   abuse, while rotating it breaks every already-installed binary until upgraded — real
   ongoing cost for weak-by-construction protection.
2. **Topic allowlist + payload validation + per-IP/per-token rate limiting (`governor`) +
   global daily send-cap circuit breaker**, all enforced relay-side before ever calling
   Apple. Confines all abuse to "free sends against the operator's own quota," matching the
   actual risk surface exactly.

## Decision

Adopt option 2 in every case: a dedicated **`crates/forge-relay`** Rust service deployed to
**Fly.io** (Cloudflare in front purely as CDN/TLS/edge-rate-limit, not as the compute layer),
an **opt-out relay default** with BYO-key always taking precedence, and **no baked-in shared
secret** — abuse prevention comes from a topic allowlist, payload validation, rate limiting,
and a daily send cap instead.

Wire protocol is a drop-in transport substitution for Apple's own API shape (`POST
/3/device/{token}`, same headers plus a new `apns-environment` header replacing the Apple
bearer JWT's implicit role, same JSON body forwarded opaquely, Apple's real HTTP status
proxied back verbatim) so `apns.rs`'s existing `Ok(410) => prune` logic needs zero changes.
`ApnsNotifier` gains an `ApnsBackend` enum (`Direct{auth}` / `Relay{base_url,relay_token}`);
only `send_one`'s body branches on it — every other caller in `apns.rs`/`serve.rs` is
unchanged.

## Rationale

- Reusing `apns.rs`'s already-tested JWT/HTTP logic in the same language eliminates the
  highest-risk unknown (a subtly-wrong signature Apple silently rejects) that a from-scratch
  TypeScript reimplementation would reintroduce.
- An opt-out default is the only choice that actually serves the stated goal — push should
  work like WhatsApp, with zero setup, for a userbase that isn't the project owner. An
  opt-in-only relay would ship a feature that doesn't fire by default for the exact audience
  it's for.
- Sizing abuse prevention to the real risk (relay can only spend the operator's own quota
  against one allowlisted topic, never reach arbitrary devices or data) avoids over-building
  security theater (a baked-in secret) whose cost (binary-wide rotation pain) exceeds what it
  actually buys.

## Consequences

- **Positive:** native push works out of the box for any self-hoster; BYO-key operators keep
  full independence; the relay's source lives in-tree, so anyone who doesn't want to trust
  the operator's instance can run their own and point `FORGE_APNS_RELAY_URL` at it.
- **Negative / trade-offs accepted:** the relay is a new deployable surface with real
  production consequences (the operator's live Apple key) and no existing deploy-workflow
  precedent to lean on — deploy is a manual runbook (`crates/forge-relay/README.md`) rather
  than CI-automated for now. The relay does see notification title/body text and Live
  Activity status fields (`busy`/`waiting`/`cost_usd`/`context_tokens`) in transit — never
  session content, source code, or credentials, but this is a real disclosure point, not
  "opaque token only," and both docs updates say so explicitly.
- **Follow-ups:** extract a shared `forge-apns-core` crate only if `forge-cli`'s and
  `forge-relay`'s copies of the JWT/HTTP logic need to change in lockstep often enough to make
  the duplication a genuine maintenance cost — not before. Add a CI deploy workflow (matching
  `app-web.yml`'s opt-in-Actions-variable pattern) only if manual `fly deploy` becomes
  frequent enough to be annoying.
