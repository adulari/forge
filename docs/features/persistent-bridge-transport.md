# Persistent bridge transport (P1)

The CLI bridge originally spawned **one CLI process per turn/re-drive** (`claude --print … --resume`),
paying a process spawn + session reload every time Forge drove the model again. The persistent
transport keeps **one long-lived process alive across turns** and writes each turn's delta to its
stdin, so re-drives within a turn reuse a warm process.

Implemented in `crates/forge-provider/src/cli_provider.rs` (`LiveSession`, `complete_persistent`).

## Embedding the harness

Harness mode starts a Forge `mcp-serve` child so the bridged CLI can call Forge's tools. By default,
`CliProvider` assumes the current executable is Forge itself. A different host executable must point
the provider at an actual Forge binary:

```rust
let provider = CliProvider::claude_code()
    .with_forge_binary("/path/to/forge");
```

`with_forge_binary` configures the harness process only; `with_binary` separately selects the
external `claude`, `codex`, or `agy` executable. The supplied Forge path must be executable and
support the `mcp-serve` command.

## claude — shipped (v0.4.63, #304)

claude Code exposes `--input-format stream-json` ("realtime streaming input", confirmed against
claude 2.1.195). Forge spawns:

```
claude -p --input-format stream-json --output-format stream-json --verbose \
  --include-partial-messages --tools "" --mcp-config <forge mcp-serve> \
  --strict-mcp-config --allowedTools mcp__forge \
  --append-system-prompt "<Forge harness policy>"
```

and keeps stdin open. Each turn:

1. Before the first harness turn, send the streaming `initialize` control request. It installs a
   bounded alias map (`Bash` → `mcp__forge__shell`, `Read` → `mcp__forge__read_file`, and the
   corresponding write/edit/search/web/notebook aliases), then poll `mcp_status` until `forge` is
   connected. This is the same control protocol used by Anthropic's Agent SDK.
2. Write one user line: `{"type":"user","message":{"role":"user","content":"<delta>"}}\n`.
   Harness policy is appended through Claude's system-prompt channel, not duplicated in this user
   payload.
3. Stream `stream_event.content_block_delta` text/thinking immediately. The later consolidated
   `assistant` block is deduplicated by block index; only a previously unseen suffix is emitted.
4. Read until the `{"type":"result"}` event (the turn boundary) — the process
   stays alive for the next turn.

**Defaults & safety.** On by default for claude; `FORGE_PERSISTENT_BRIDGE=0` (or
`CliProvider::with_persistent(false)`) opts out. The path falls back to the one-shot transport
whenever the live session can't be established *before any turn output ran* (spawn failure,
first-turn stdin-write failure, immediate exit with no tool executed, or a stall before any tool
ran). A stall carries `tool_ran` state: after a tool starts, Forge tears down the process and
returns a retryable failure but **never automatically replays the turn**, so an irreversible side
effect cannot execute twice.

**Authoritative discovery.** Forge enumerates Claude models with a non-billing `initialize`
control request rather than scraping `claude --help`. The sanitized record includes alias,
resolved model and effort/adaptive/auto/fast-mode capabilities; account, email, PID and local
configuration are discarded. `--help` parsing remains a fallback for older Claude versions.

**Respawn triggers.** Model change, transcript shrink (compaction), and a `FORGE_CHECKPOINT_SEQ`
change (a new user turn). Re-drives *within* a turn keep the same checkpoint seq and reuse the
process; a new user turn respawns so bridge-edit `/undo` snapshots stay turn-accurate.

**Proven.** Protocol fixtures cover initialization sanitization, the exact bounded alias map,
partial-message assembly/deduplication and system-prompt argv. Deterministic fake-CLI tests cover
MCP-before-prompt ordering, one-process reuse, safe pre-tool fallback, and the critical negative
case: a post-tool stall records exactly one process invocation. A live `--ignored` e2e against real
claude verifies context across two turns. Measured fixed overhead removed: **≈0.88s spawn→init per turn**
(4 samples). Honest scope: model inference dominates total turn time, so this is a real
per-re-drive latency saving that compounds with re-drive count, **not a headline multiplier**;
token cost is unchanged (both transports already send deltas — one-shot via `--resume`, persistent
via in-process context).

## codex — blocked upstream (investigated 2026-06-27, codex 0.141)

codex has **no usable persistent transport** today:

- `codex exec` reads instructions from stdin **once**, then exits — one-shot only.
- `codex exec-server --listen stdio` *is* a persistent JSON-RPC 2.0 endpoint and `initialize`
  works (returns a `sessionId`), but it is a **stub**: every turn method returns

  ```
  {"error":{"code":-32601,"message":"exec-server stub does not implement `thread/new` yet"}}
  ```

  The full protocol surface exists in the binary's strings (request methods `thread/start`,
  `thread/turn`, `turn/steer`, …; a ~40-event notification taxonomy `turn/started`, `turn/completed`,
  `item/agentMessage/delta`, `item/reasoning/textDelta`, `thread/tokenUsage/updated`, …; and an
  interactive approval flow `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`,
  `item/tool/requestUserInput`), but none of it is implemented in 0.141.

**Conclusion:** a persistent codex transport is not buildable now — it is blocked on upstream codex
implementing `exec-server`. When it lands, the integration is non-trivial (a JSON-RPC client driving
thread/turn lifecycle, the streaming event taxonomy, **and** the server→client approval protocol).
codex keeps its one-shot transport with `exec resume` (per-turn session reload, same context
continuity as claude's pre-persistent path).

## agy — not possible

antigravity (`agy` 1.0.12) has only `--print` (a single prompt, text output, then exit) — no
`--input-format`, no `--output-format stream-json`, no streaming mode to hold open. agy stays
one-shot.

## Status

| CLI | Persistent transport | Why |
| --- | --- | --- |
| claude | ✅ shipped (default on) | `--input-format stream-json`, proven |
| codex | ❌ blocked upstream | `exec-server` is an unimplemented stub in 0.141 |
| agy | ❌ not possible | no streaming-input mode exists |
