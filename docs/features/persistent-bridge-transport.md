# Persistent bridge transport (P1)

The CLI bridge originally spawned **one CLI process per turn/re-drive** (`claude --print … --resume`),
paying a process spawn + session reload every time Forge drove the model again. The persistent
transport keeps **one long-lived process alive across turns** and writes each turn's delta to its
stdin, so re-drives within a turn reuse a warm process.

Implemented in `crates/forge-provider/src/cli_provider.rs` (`LiveSession`, `complete_persistent`).

## claude — shipped (v0.4.63, #304)

claude Code exposes `--input-format stream-json` ("realtime streaming input", confirmed against
claude 2.1.195). Forge spawns:

```
claude -p --input-format stream-json --output-format stream-json --verbose \
  --tools "" --mcp-config <forge mcp-serve> --strict-mcp-config --allowedTools mcp__forge
```

and keeps stdin open. Each turn:

1. Write one user line: `{"type":"user","message":{"role":"user","content":"<delta>"}}\n`.
2. Read the NDJSON stream until the `{"type":"result"}` event (the turn boundary) — the process
   stays alive for the next turn.

**Defaults & safety.** On by default for claude; `FORGE_PERSISTENT_BRIDGE=0` (or
`CliProvider::with_persistent(false)`) opts out. The path falls back to the one-shot transport
whenever the live session can't be established *before any turn output ran* (spawn failure,
first-turn stdin-write failure, immediate exit with no tool executed), so a tool can never
double-execute. Once a turn has started, errors propagate as retryable instead of re-running.

**Respawn triggers.** Model change, transcript shrink (compaction), and a `FORGE_CHECKPOINT_SEQ`
change (a new user turn). Re-drives *within* a turn keep the same checkpoint seq and reuse the
process; a new user turn respawns so bridge-edit `/undo` snapshots stay turn-accurate.

**Proven.** `persistent_transport_reuses_one_process_across_turns` (deterministic: a 2nd turn on the
same process answers "reply 2"; a fresh spawn says "reply 1"), `persistent_falls_back_to_one_shot_when_binary_is_missing`,
framing + classifier unit tests, and a live `--ignored` e2e against real claude (codeword recalled
across two turns on one process). Measured fixed overhead removed: **≈0.88s spawn→init per turn**
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
