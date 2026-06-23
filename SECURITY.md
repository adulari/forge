# Security Policy

## Supported versions

Forge is pre-1.0 and moves fast. Security fixes land on `main` and ship in the next release; the
latest released version is the only supported one.

## Reporting a vulnerability

**Please do not open a public issue for security vulnerabilities.**

Report privately via GitHub's [private vulnerability reporting](https://github.com/florisvoskamp/forge/security/advisories/new)
("Report a vulnerability" under the repo's **Security** tab). Include:

- a description of the issue and its impact,
- steps to reproduce (a minimal repro helps a lot),
- affected version (`forge --version`) and OS.

We aim to acknowledge reports within a few days and to ship a fix as quickly as is practical,
coordinating disclosure with you.

## Scope & handling of secrets

Forge handles credentials, so a few notes on the security model:

- **API keys and OAuth tokens** are stored in the OS keyring (with an encrypted-file fallback),
  never in config files or logs (ADR-0007).
- The **shell tool** runs behind a permission broker with an unoverridable denylist; an opt-in OS
  sandbox (Linux Landlock) is available via `[shell] sandbox`.
- **Web tools** are SSRF-guarded. **MCP** servers connect behind an allowlist.
- Forge never transmits your code or keys anywhere except the model/provider endpoints you
  configure, and (opt-in) a GitHub release check that sends no data.

If you find a gap in any of the above, that's exactly the kind of report we want.
