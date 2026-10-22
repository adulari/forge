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

Both the bridge decoder and Forge's outer stream watchdog track tool start/finish state. An
identified in-flight MCP tool gets one additional idle window (2× total) so a legitimate build
near the shell tool's own timeout can return its result instead of racing the ordinary stream
silence cutoff. A stream with no active tool keeps the base cutoff, and a stuck tool remains
bounded; the extra window is not a general timeout increase.

**Authoritative discovery.** Forge enumerates Claude models with a non-billing `initialize`
control request rather than scraping `claude --help`. The sanitized record includes alias,
resolved model and effort/adaptive/auto/fast-mode capabilities; account, email, PID and local
configuration are discarded. `--help` parsing remains a fallback for older Claude versions.

**One process per conversation.** The provider keeps a small map of live sessions keyed by the
conversation owner (`CheckpointContext::session`; at most 4, least-recently-used idle one evicted).
A subagent or a second hosted session therefore never shares, or evicts, the main session's warm
process. Each slot has its own lock; a call that finds its owner's slot mid-turn does **not** queue
(queueing emits no events and would trip the caller's stall watchdog) but runs on the one-shot
path, without touching the shared resume state.

**Per-turn context travels in a turn file, not the process env.** The `forge mcp-serve` child is
spawned once, so its env goes stale. The parent sets `FORGE_BRIDGE_TURN_FILE=<tmp path>` at spawn
and, before every turn it writes to the process, atomically (tmp + rename) rewrites it with
`{"seq": <i64>, "mode": "<permission key>"}`. The child reads it per call: snapshot seq
(`forge_core::snapshot::current_seq`, falling back to `FORGE_CHECKPOINT_SEQ`) and the permission
mode used at each gate (`ForgeMcp::effective_mode`, falling back to the spawn-time mode; an
unparsable mode fails closed to `default`). A failed turn-file write refuses reuse and falls back
to one-shot. Consequently a new user turn, a new checkpoint seq or a temper switch no longer
respawns anything: the same claude process, prompt cache and structured history serve every turn.

**Respawn triggers.** Model change, effort change, history epoch change (`/rewind`, `/uncompact`),
transcript shrink (compaction), a workspace change (the session moved into or out of a git worktree,
[session-worktrees.md](session-worktrees.md); the process and its `mcp-serve` child run in the root
they were spawned in), a dead process, or the idle reaper. The reaper tears a parked
process down after 20 minutes idle (`FORGE_BRIDGE_IDLE_TTL_SECS` overrides).

**Prewarm.** A first turn would otherwise start `claude`, which starts `forge mcp-serve`, and wait
for the tool server to report connected before the prompt can be written (1.0-1.5 s of "nothing
happens"). A new conversation pinned to exactly one `claude-cli::` model starts that process when
the TUI starts (`Session::prewarm_pinned_bridge` -> `Provider::prewarm` -> `CliProvider::prewarm_live`)
and parks it in the conversation's slot marked `unused`: handshaken, holding no history. The first
turn takes it over like a fresh spawn (full transcript), provided the model, effort, epoch and
workspace match; otherwise it is torn down and the turn spawns as before. The warm-up holds the slot
and marks the owner as warming, so a prompt typed before it finishes waits for it rather than being
turned away as `Busy` or spawning a second process. Conversations with history are not warmed (they
resume claude's session). The idle reaper covers a warmed process nobody uses.

**Teardown is bounded by exit, not by a fixed sleep.** SIGTERM goes to the process group, then Forge
polls for the group to be gone and sends SIGKILL after the 2 s grace at the latest. Interrupted
turns used to pay the whole grace before the replacement process could spawn.

**Respawn restores claude's own history.** claude's `session_id` is captured from the stream
(`system/init`, `result`) into the live session and the shared resume state. A respawn for the
same owner/model/epoch passes `--resume <id>` and sends only `render_resume_delta` of the messages
claude has not seen, instead of the flattened transcript (which on big sessions was a 1M+ token
text blob with a cold cache). If the resumed process dies or stalls before any output and before
any tool ran, the resume state is cleared and Forge respawns fresh with the full transcript.

**Context size comes from claude, not from Forge's transcript.** The bridge runs the whole tool loop
inside claude, so every file read and build log lives in claude's own window while Forge's transcript
only records the replies. In a measured 40-turn session claude's window held ~650k tokens per request
while Forge estimated 18k, so auto-compaction never fired. The live decoder now reads each `assistant`
stream line's own `usage` (uncached + cache read + cache creation = that request's prompt size; the
`result` usage is a per-turn SUM and is not used), `Provider::context_fill` exposes the latest value
per conversation owner, and the session uses `max(transcript estimate, bridge fill)` for both the
gauge and the auto-compaction trigger. A compaction bumps the history epoch, so the process is
respawned from the compacted transcript.

**Nothing new to send.** When a re-drive carries no new user/system message, a reused process is told
to continue instead of being sent the whole transcript again (which it took for a fresh request and
answered with "nothing new is requested"). Repeated end-of-turn nudges (the review pass, the "prove
it" completion gate) are no longer deduplicated out of the request: a system message that directly
follows an assistant message and is answered by the model is a reaction, not standing guidance.

**Watchdogs.** The outer stream-idle guard (default 180s) used to undercut the bridge's own 300s
window, so a silent claude (long thinking, auto-compaction) was killed and, with it, the process
group of any running tool. `Provider::stream_idle_hint` lets a CLI bridge raise the outer budget to
its inner window plus a minute; every well-formed typed claude stream line (`system`, pings,
`input_json_delta`, ...) now counts as activity. The stderr drain keeps reading past its 16 KiB
capture cap (discarding the excess) so a long-lived child never sees EPIPE.

**Cost.** `result.total_cost_usd` is deliberately ignored: the bridge is subscription-billed, so
turns are $0 against Forge's USD budget (FR-5); token counts are still recorded.

**Proven.** Protocol fixtures cover initialization sanitization, the exact bounded alias map,
partial-message assembly/deduplication and system-prompt argv. Deterministic fake-CLI tests cover
MCP-before-prompt ordering, one-process reuse, safe pre-tool fallback, and the critical negative
case: a post-tool stall records exactly one process invocation. A live `--ignored` e2e against real
claude verifies context across two turns. Measured fixed overhead removed: **≈0.88s spawn→init per turn**
(4 samples). Honest scope: model inference dominates total turn time, so this is a real
per-re-drive latency saving that compounds with re-drive count, **not a headline multiplier**;
token cost is unchanged (both transports already send deltas — one-shot via `--resume`, persistent
via in-process context).

**Startup and per-request overhead (2026-10).** Measured against `claude -p` on the same model
with a local fake Anthropic API (debug build):

- `forge mcp-serve` answered claude's MCP handshake after ~0.94 s: ~0.55 s re-resolving every
  provider key over the secret store, ~0.3 s building subagent support (provider, router, a parent
  session row). claude hands its MCP servers its whole environment, so the parent now passes the
  *names* of the keys it resolved (`FORGE_BRIDGE_PROVIDER_KEYS`, claude only; codex curates its MCP
  environment) and the child adopts them, registering them so its shell tool still strips them from
  commands. Subagent support is built on the first `spawn_agents`/`send_to_agent` call. Handshake:
  ~0.1 s. claude spawn → first request: 1.9 s → 1.5 s.
- Every claude process Forge drives gets `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` (unless set): Forge owns
  memory, and claude's auto-memory instructions (~2.3k chars per request) pointed at a directory its
  disabled Write tool cannot reach, plus the project's `MEMORY.md` index (up to 25 KB).
- When no project instruction file exists, Forge falls back to `~/.claude/CLAUDE.md`, which the
  bridged claude also loads as user memory from the mirrored config dir. That copy is no longer sent
  a second time (`lean_context::claude_md_covers`).
- The quota probe (`bridge_stats::probe_claude_limits`) ran `claude --print .` with the user's whole
  setup: every `~/.claude.json` MCP server, the SessionStart hooks, CLAUDE.md, the skill listing and
  all tool schemas — a 107 KB request (~27k Haiku tokens) and ~1 GB of short-lived processes per
  probe. It now runs in the bridge home with `--strict-mcp-config --tools "" --no-session-persistence`
  and `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`: a 17 KB request.
- Recap, next-prompt suggestion and memory capture each routed a side-call model on the turn task
  after `Done`. With a real catalog that held the spinner ("Finishing") and the next queued prompt
  for ~1 s per turn in release (10.8 s in a debug build). On an interactive surface they now choose
  their model once, on a detached task (`post_turn_calls.rs`); under a single-model pin the choice
  skips catalog ranking entirely.
- `--model claude-cli::<unknown>` exits 1 before spawning: in stream-json mode claude silently runs
  its default model for an unknown `--model`.
- Bridged write prompts carry the call's arguments and diff, so the prompt names the file and shows
  the change, as a native write does.

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
