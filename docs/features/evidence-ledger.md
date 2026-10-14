# Evidence ledger

Proof that a session's work ran, recorded as it happens and rendered into `/pr` without the model
in the loop.

## What gets recorded

Everything lands in `<workspace>/.forge/evidence/<session-id>/`:

- **Screenshots.** `browser` and `device` `screenshot` actions write `0001-browser.png`,
  `0002-device.png`, … Names are monotonic and claimed with `create_new`, so a later shot never
  overwrites an earlier one, even when a CLI bridge child and the parent write into the same
  session. An optional `caption` argument says what the shot shows. Browser shots also record the
  page URL with its query string, fragment and any `user:pass@` removed, since those carry tokens.
- **Test proof.** A `shell` call that the completion policy (`completion::classify_tool`) classifies
  as a test, lint, typecheck or build check, and whose result header reports `exit 0`, is saved as
  `NNNN-shell.txt`: the command, the exit status and the last 40 lines of output (from the uncut
  output when the model's copy was truncated). Failing checks, non-check commands and commands whose
  status could be masked (`;`, `||`, background `&`) are never recorded.

Each capture appends one line to `manifest.jsonl`:

```json
{"ts":"2026-10-14T10:00:00Z","kind":"screenshot","file":"0001-browser.png","caption":"empty cart","url":"http://localhost:3000/cart","sha256":"…","source":"browser"}
{"ts":"2026-10-14T10:01:00Z","kind":"test_output","file":"0002-shell.txt","sha256":"…","source":"shell","command":"cargo test -p shop","exit_code":0}
```

`sha256` is the hash of the stored file. A torn last line (a crash mid-append) is skipped, not
fatal. The first capture writes `.forge/evidence/.gitignore` containing `*`, so evidence never
shows up in `git status` and the user's own `.gitignore` is untouched.

Outside any session (a bare tool call), screenshots go to a per-process temp file with a counter
in the name, as before but without overwriting.

## How tools know the session

The direct path's dispatcher runs every registry tool inside
`forge_tools::scope_session(session_id, workspace_root, …)`, so captures land under the same
workspace root `/pr` reads from, never the daemon's cwd. A CLI bridge's `forge mcp-serve` child has no such
scope; it reads the parent session id from `FORGE_CHECKPOINT_SESSION`, which the parent already
sets on the child (`bridge_mcp_env`), and its workspace from the cwd it entered
(`FORGE_BRIDGE_WORKSPACE`). The bridge records test proof through the same
`forge_core::evidence_capture` function as the direct path.

Subagent (`spawn_agents`) tool calls run outside this scope and are not recorded; their
screenshots fall back to temp files.

## In `/pr`

`/pr` appends two deterministic sections and tells the model to paste them verbatim:

```markdown
---
### Evidence (captured, not reviewed)
Checks that passed during the session:
- `cargo test -p shop` — exit 0 at 2026-10-14T10:01:00Z (`.forge/evidence/<id>/0002-shell.txt`, sha256 `fedcba987654`)
Screenshots:
- `.forge/evidence/<id>/0001-browser.png` (browser, http://localhost:3000/cart, sha256 `0123456789ab`) — empty cart

### Provenance
…
```

The section is omitted when the session captured nothing. A PR body is public, so the values of
leading `NAME=value` env assignments in a recorded command are shown as `NAME=***` (the local
manifest keeps the raw command), and the whole section then goes through the same secret redaction
as `forge replay` and `/export` ([redaction.md](redaction.md)), so a key passed as a flag or a
`user:pass@` URL becomes `[redacted]`. It says "captured", never "verified":
nobody, the model included, has looked at the images. Images are referenced by local path; publishing
them with the PR (an orphan branch) is planned work.

## Code

- `crates/forge-tools/src/evidence.rs`: the ledger, the screenshot sink and `pr_evidence_block`.
- `crates/forge-core/src/evidence_capture.rs`: test proof from verification shell calls.
- `crates/forge-cli/src/cli/commands/run/pr_body.rs`: the `/pr` body sections.
