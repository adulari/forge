# Security Policy

## Supported versions

Forge is post-1.0 and releases frequently. Security fixes land on `main` and ship in the next
release. We support the **latest released minor series** with security patches:

| Version | Supported |
| --- | --- |
| Latest released `2.x` (current: `2.0.x`) | Yes — security + bug fixes |
| `1.x` | No — upgrade to the latest `2.x` |
| `0.x` | No — pre-1.0, unsupported |

Always upgrade to the latest release before reporting (`forge --version`, or re-run the installer /
`brew upgrade forge`). Stability guarantees for the CLI, config, and output formats are documented
in [`docs/STABILITY.md`](docs/STABILITY.md).

## Reporting a vulnerability

**Please do not open a public issue for security vulnerabilities.**

Report privately via GitHub's
[private vulnerability reporting](https://github.com/Adulari/forge/security/advisories/new)
("Report a vulnerability" under the repository's **Security** tab). If that is unavailable to you,
email the maintainer at **fvoskamp2005@gmail.com** with `[forge-security]` in the subject.

Please include:

- a description of the issue and its impact,
- steps to reproduce (a minimal repro helps a lot),
- the affected version (`forge --version`) and your OS,
- any suggested remediation, if you have one.

### What to expect

- **Acknowledgement** within 3 business days.
- An initial assessment (severity + whether we can reproduce) within about a week.
- A fix released as quickly as is practical for the severity, with disclosure coordinated with you.
  We credit reporters in the release notes unless you prefer to remain anonymous.

## Supply-chain & dependencies

- Dependencies are monitored continuously: `cargo audit` (RUSTSEC advisories) and `cargo deny`
  (licenses, banned/duplicate crates, source pinning) run in CI on every PR and weekly
  (`.github/workflows/security.yml`, `deny.toml`).
- Dependency updates are proposed automatically via Dependabot (`.github/dependabot.yml`).

## Scope & handling of secrets

Forge handles credentials, so a few notes on the security model:

- **API keys and OAuth tokens** are stored in the OS keyring (with an encrypted-file fallback),
  never in config files or logs (ADR-0007).
- The **shell tool** runs behind a permission broker with an unoverridable denylist; an opt-in
  OS sandbox (Linux Landlock) is available via `[shell] sandbox`.
- **Web tools** are SSRF-guarded. **MCP** servers connect behind an allowlist.
- Forge transmits your code and keys only to the model/provider endpoints you configure, plus
  (opt-in) a GitHub release check that sends no data.

## Remote access (`forge serve`)

The daemon's owner token (`serve-token`) is the full-control credential. Paired devices instead
hold their own token (`forge serve pair`), stored hashed in `serve-clients.json`, with scopes
`view`, `prompt`, `approve`, `terminal`, `workspace-write` and `manage-providers`. The `viewer`
preset holds `view` only. The local CLI (`forge attach`, `forge board`, `forge send`) reads the
owner token from the config directory and therefore runs with owner scope. Every mutation,
approval, refusal and WebSocket open is appended to `remote-audit.jsonl` in the data directory
(client id, route, session, decision; never a token).

| Threat | Mitigation | Residual risk |
| --- | --- | --- |
| A lost or shared phone | `forge serve clients revoke ID` or Revoke in Remote access; the token dies at once and open sockets are cut within seconds | The owner token cannot be revoked per device; `forge serve --rotate-token` replaces it |
| A leaked pairing link | Links expire unused after `--ttl` (default 10 minutes, at most 24 hours) | Anyone who opens it first becomes the device; pair on a trusted network |
| A read-only device acting | Scopes are checked before routing and on each WebSocket input; refusals are typed (`scope_denied`) and audited | Scopes are coarse: `prompt` lets an agent run whatever its own permission mode allows |
| DNS rebinding to a local daemon | Requests whose `Host` is not an IP, loopback or single-label name, `*.local`, `*.ts.net`, a known tunnel domain, the configured tunnel host or `[remote] allowed_hosts` are refused | A custom public hostname must be added to `allowed_hosts` |
| Spoofed client address | `X-Forwarded-For` is trusted only from loopback or `[remote] trusted_proxies`, and only feeds the audit log | The address is informational, never an access decision |
| The token in a WebSocket URL (history, proxy logs) | `POST /api/ws-ticket` issues a 30 second single-use ticket usable as `/ws?ticket=...` | Clients that still put the token in the path keep that exposure |
| The legacy control page leaking the owner token | `/`, `/app.js`, `/manifest.webmanifest`, `/sw.js` and device management are owner-only | None known |
| Audit log tampering | The log is append-only from the daemon and 0600 | A local user with the same account can edit it |

If you find a gap in any of the above, that's exactly the kind of report we want.
