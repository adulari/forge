# forge-relay

The hosted APNs push relay — see [ADR-0012](../../docs/architecture/decisions/0012-hosted-apns-relay.md)
for why this exists and [`docs/features/remote-control.md`](../../docs/features/remote-control.md)
for the user-facing disclosure of what data crosses it.

A self-hosted `forge serve` daemon that hasn't configured its own Apple `.p8` key talks to this
service instead of Apple directly. This process holds the real Apple Developer credential
centrally; the relay never sees session content, source code, or a daemon's auth token — only
an opaque device token, an environment string, and the notification payload text.

This is a standalone deployable service, **not** part of `forge-cli`'s single-binary delivery
(ADR-0002's one anticipated exception). There is currently no CI deploy workflow — deploys are
manual, since the operator is the sole deployer today and a relay deploy has real production
consequences (a live Apple key). Revisit with an `app-web.yml`-style opt-in-gated workflow only
if manual deploys become frequent enough to be annoying.

## Configuration

| Env var | Required | Notes |
|---|---|---|
| `FORGE_APNS_TEAM_ID` | yes | Apple Developer Team ID |
| `FORGE_APNS_KEY_ID` | yes | The APNs Auth Key's Key ID (App Store Connect → Certificates, IDs & Profiles → Keys) |
| `FORGE_APNS_KEY_PEM` | one of this or `_KEY_PATH` | The `.p8` file's contents, set directly as a secret — preferred, since the key never touches the container filesystem |
| `FORGE_APNS_KEY_PATH` | one of this or `_KEY_PEM` | Path to a mounted `.p8` file, if you'd rather not put the PEM in an env var |
| `FORGE_RELAY_ALLOWED_TOPICS` | no | Comma-separated allowlist. Default: `dev.adulari.forge,dev.adulari.forge.push-type.liveactivity` |
| `PORT` | no | Default `8787` |
| `FORGE_RELAY_RATE_LIMIT` | no | Requests per window, per IP and per device token. Default `30` |
| `FORGE_RELAY_RATE_WINDOW_SECS` | no | Window length in seconds. Default `60` |
| `FORGE_RELAY_DAILY_SEND_CAP` | no | Global circuit breaker — total accepted sends/24h before the relay starts rejecting. Default `50000` |

## Deploy (Fly.io)

Run from the **workspace root** (the Dockerfile needs the whole workspace as build context):

```sh
fly launch --config crates/forge-relay/fly.toml --dockerfile crates/forge-relay/Dockerfile --no-deploy
fly secrets set --config crates/forge-relay/fly.toml \
  FORGE_APNS_TEAM_ID=95VXXPD28Y \
  FORGE_APNS_KEY_ID=<your key id> \
  FORGE_APNS_KEY_PEM="$(cat /path/to/AuthKey_XXXX.p8)"
fly deploy --config crates/forge-relay/fly.toml
```

Put a domain in front via Cloudflare (recommended — free-tier CDN/TLS/edge-rate-limiting as
defense-in-depth on top of this service's own limiter): point `relay.adulari.dev` at the Fly.io
app's IPv4/IPv6 (`fly ips list`), proxied (orange-cloud) through Cloudflare.

## Health check

```sh
curl https://relay.adulari.dev/health
# {"ok":true,"daily_sent":0}
```

## Rollback

```sh
fly releases --config crates/forge-relay/fly.toml
fly deploy --config crates/forge-relay/fly.toml --image <previous-image-ref>
```

## Monitoring

Watch `fly logs --config crates/forge-relay/fly.toml` for `rejected disallowed topic` (someone
probing with the wrong bundle id) and `rate limited` warnings. A daily-cap trip logs loudly
(`daily send cap reset (was N/CAP)`) rather than silently dropping — if `N` is ever close to
`CAP`, that's the signal to investigate before the cap becomes a real availability problem for
legitimate users.

## Testing locally

```sh
cargo test -p forge-relay
cargo run -p forge-relay  # needs FORGE_APNS_TEAM_ID/_KEY_ID/_KEY_PEM (or _KEY_PATH) set
```
