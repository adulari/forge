# Automatic issue reports

When Forge itself breaks (a panic, an internal error that ends a turn, a render crash in the desktop
or phone app) it can tell the maintainers, so the bug gets fixed without anyone having to write it
up. A report describes the bug, never your work: no prompts, no model replies, no file contents or
code, no diffs, no tool arguments or results, no command lines, no environment variables, no keys or
tokens, no emails, hostnames, usernames or absolute paths.

It is on by default and takes one setting to change.

## Modes

| Mode | What happens |
|---|---|
| `auto` (default) | A scrubbed report is sent in the background. The TUI shows a quiet `reported` line; the apps show a small toast. |
| `ask` | Forge shows a card with the exact report and three buttons: **Send**, **Don't send**, **Always send** (switches to `auto`). |
| `off` | Nothing is collected, queued or sent. Anything already waiting on disk is deleted at the next start. |

Set it in any of these places; they all write the same setting:

- `config.toml`: `[reporting] mode = "auto" | "ask" | "off"`
- TUI `/config` (Privacy, "Error reports") or `forge report --mode off`
- Desktop and phone apps: Settings, "Error reports"
- Onboarding (the onboarding step calls `forge_reporting::set_mode`)
- `FORGE_NO_REPORTS=1` in the environment forces `off` whatever the config says. It also covers `forge serve`.

## What is reported, and what is not

Forge reports **bugs**, not conditions that are normal for a tool talking to the internet:

| Reported | Never reported |
|---|---|
| Panics (with a normalized backtrace), render panics, async task deaths | Rate limits, quota and billing errors |
| Internal invariant violations that end a turn or session | Authentication failures, missing keys |
| Provider replies Forge could not parse (protocol errors) | Network problems, timeouts, being offline, provider outages (5xx) |
| Store (SQLite) errors that are not disk-full / locked / read-only | A turn you interrupted, a command you cancelled |
| Failed compactions caused by an internal error | Config errors, a missing file, a stale host path, a database newer than the binary |
| Desktop/phone: React error boundaries, unhandled promise rejections, uncaught errors, bug-shaped Tauri command failures, desktop panics | A closed stdout (`forge run \| head`) |

The classification is explicit and tested: `crates/forge-reporting/src/classify.rs` (text rules),
`crates/forge-core/src/error_report.rs` (typed rules for core errors) and
`mobile/src/lib/reportingCore.ts` (the same rules for the apps).

## What a report contains

Every report is one closed JSON object (wire format v1). The only free text is a message of at most
300 characters, passed through the scrubber described below.

```json
{
  "v": 1,
  "source": "auto",
  "install_id": "random 32 hex, created on first use",
  "fingerprint": "16 hex: hash of kind + site + top frames, never message text",
  "error_kind": "panic",
  "site": "forge-core/src/lib.rs",
  "location": "forge-core/src/lib.rs:2514",
  "message": "scrubbed, content-free, <= 300 chars",
  "frames": ["forge_core::session::run_turn"],
  "context": {
    "version": "2.23.6", "git_sha": "86f7d9cdadb1", "surface": "tui",
    "os": "linux", "arch": "x86_64", "install_method": "binary",
    "provider": "inferhub", "model": "inferhub::cb/deepseek-v4.1-flash", "temper": "smith",
    "flags": []
  },
  "events": [{"k": "turn_start", "ago": 41}, {"k": "tool_call", "ms": 12, "ago": 3}],
  "occurred_at": 1793051903
}
```

- **Surface** is one of `tui`, `headless`, `daemon`, `cli`, `desktop`, `ios`, `android`, `web`.
- **Events** are a ring buffer of the last 40 harness events: a short kind (`turn_start`,
  `model_request`, `tool_call`, `compaction`) and a duration in milliseconds. Never content, never
  tool names or arguments.
- **Install id** is random, not derived from your machine, and exists so the relay can rate-limit
  and count distinct installs. The relay stores only a hash of it. It is separate from (and not
  linked to) anonymous usage statistics, which use no identifier at all.
- Provider and model are ids, never keys.

### How text is scrubbed

Everything passes through `crates/forge-reporting/src/scrub.rs` before it is written to disk, and
the relay scrubs it again before it reaches GitHub.

1. Known secrets on the machine (provider keys, tokens) are removed with the same redactor Forge
   uses for transcripts (`crates/forge-core/src/redact.rs`), then by shape (`sk-...`, `ghp_...`,
   AWS and Google keys, JWTs, `Bearer ...`, private keys, `password=...`, long opaque tokens).
2. URLs keep only their host; private and internal hosts, IP addresses and your own hostname become
   placeholders. Emails become `<email>`. Your username is removed.
3. The home directory becomes `~`; every other absolute path keeps only its basename.
4. Quoted strings, backtick spans and `{...}` blobs are dropped from messages, because that is where
   prompts, replies, tool arguments and file contents appear inside error text.
5. Text is flattened and truncated.

The tests in `crates/forge-reporting/src/scrub/tests.rs` and `crates/forge-relay/src/reports_tests.rs`
feed crafted home paths, API keys, prompts, emails and hostnames through the pipeline and assert none
survive.

## See exactly what is sent

- `forge report --show-last`: the most recent report, byte for byte as it would be sent.
- `forge report --sent`: what has been sent from this machine (log in the data dir:
  `<data dir>/reports/sent.jsonl`, last 50).
- `forge report --pending` and `--send-pending`: reports waiting for a decision in `ask` mode.
- Desktop and phone apps: Settings, Error reports, "What gets sent".

## Report a problem yourself

`/bug <what went wrong>` in the TUI, `forge report --message "..."` on the command line, and "Report
a problem" in the apps send the same scrubbed context plus your own description. A preview is shown
first and nothing is sent until you confirm. The apps can additionally attach the text of the last
error, with its own preview and toggle. Because a manual report is something you wrote on purpose,
its description (scrubbed for secrets, paths and hostnames, but otherwise as typed) becomes the
body of a public GitHub issue labelled `user-report`; do not paste anything you would not post
publicly.

## Offline, slow and rate-limited networks

Reports are written to `<data dir>/reports/queue/` first (at most 20, oldest dropped) and sent by one
background thread with a 5 second timeout; a session never waits on the network. A panic gets up to
two extra seconds to deliver before the process exits. If the relay answers 429 or 503 the client
stops sending until `Retry-After` has passed (default one hour, at most 24); a network failure pauses
sending for five minutes. A problem already reported from an install is not sent again for 24 hours,
and one run sends at most five reports.

## Where reports go

`POST https://forge.adulari.dev/relay/v1/reports` (override with `FORGE_REPORTING_URL`). The relay
(`crates/forge-relay`) validates and rate-limits (per install, per IP, globally), deduplicates by
fingerprint, keeps counts per version and OS in a small SQLite database, and opens or updates **one**
GitHub issue per distinct problem in the Forge repository, labelled `auto-report` (`user-report` for
manual reports). Later occurrences update the counts in the issue body; a new comment is added only
when a new version is first affected; a closed issue is reopened if the problem recurs on a newer
version than the one it was closed in. Deployment: `crates/forge-relay/README.md`.

Debug builds (`cargo run`, `cargo test`) never contact the public relay unless `FORGE_REPORTING_URL`
is set explicitly.

## For embedders: the API

```rust
use forge_reporting::{mode, set_mode, ReportingMode};

let current: ReportingMode = mode();              // honours FORGE_NO_REPORTS
set_mode(ReportingMode::Ask)?;                    // writes [reporting] mode in config.toml
```

`ReportingMode` is a serde enum (`"auto" | "ask" | "off"`, defined in `forge_config::reporting` and
re-exported). `forge report --selftest panic|internal --message "..."` raises a real panic or a
classified internal error to verify a deployment end to end.
