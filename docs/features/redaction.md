# Secret redaction

Everything Forge writes out of its session store is passed through
`forge_core::redact::Redactor` first:

| Surface | Default |
|---|---|
| `forge replay <id>` (transcript, `--json`, `--html`, and the two-id diff) | redacted; `--no-redact` prints verbatim |
| TUI `/export [path]` | always redacted |
| `forge anywhere share` | always redacted (the share is published) |
| `forge bench swe --judge` / `bench integrity --judge` | the diff sent to the judge model is redacted |
| `forge bench export` | always redacted, then each field capped at 8 KB ([bench-tasks-and-export.md](bench-tasks-and-export.md)) |

Each masked value becomes `[redacted]`. The store itself is never rewritten.

## Layers

1. **Known secrets (the guarantee).** Every key Forge can resolve for a model provider, a search
   provider or Artificial Analysis — env vars (including numbered `_2`… rotation keys), the OS
   keyring and the encrypted fallback file — plus the value of any env var whose name has a
   `KEY`/`PAT`/`PASS` segment or a segment ending in `TOKEN`/`SECRET`/`PASSWORD`/`APIKEY`
   (`OPENAI_API_KEY`, `GITHUB_TOKEN`, `PGPASSWORD`). Values shorter than 8 characters, paths,
   booleans and anything containing whitespace are ignored so ordinary words are not masked.
   Matching is exact, longest secret first.
2. **Shapes.** `sk-…` (OpenAI, Anthropic `sk-ant-…`), `ghp_`/`gho_`/`ghu_`/`ghs_`/`ghr_`/
   `github_pat_`, `AKIA…`/`ASIA…`, `xox[abposr]-…`, `AIza…`, `hf_…`, JWTs (`eyJ….eyJ….…`),
   `-----BEGIN … PRIVATE KEY-----` blocks (certificates are left alone), `Bearer <16+ chars>`
   and `scheme://user:password@host` (the scheme, user and host are kept).
3. **Keyword-adjacent values.** `password`, `passwd`, `secret`, `token`, `api_key`/`apikey`,
   `access_key`, `private_key`, `client_secret`, optionally prefixed (`OPENAI_API_KEY=…`), followed
   by `:` or `=` and an optional quote (plain or JSON-escaped). Values that look like code or
   placeholders are kept: pure identifiers (`self.token`, `getpass`), pure numbers, and `$VAR` /
   `%VAR%` references.

Every shape and keyword rule is anchored on a prefix, so git SHAs, short SHAs, UUIDs and
`sha256:` digests are never masked (unit-tested).

## Limits

Regex layers cannot be complete; only layer 1 is a guarantee, and it only knows secrets that are
configured on the machine doing the export. A keyring-only MCP token is not enumerated (its shape
or a keyword usually catches it). Layer 3 keeps letters-only values to spare code, so a
passphrase like `password=correcthorse` is masked only if it is a configured secret (layer 1). Review a trajectory before publishing it anywhere public.

## Code

- `crates/forge-core/src/redact.rs` — `Redactor`, `redact()`, `known_secrets()`.
- `crates/forge-cli/src/cli/commands/replay.rs` — `forge replay` wiring and `--no-redact`.
- `crates/forge-cli/src/cli/commands/run/export.rs` — `/export`.
- `crates/forge-cli/src/anywhere/share.rs` — encrypted replay shares.
