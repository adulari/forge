# Code archaeology: one-shot CLI execution

## Summary

One-shot execution has a distinct lifecycle from interactive chat: it resolves a command or skill once, chooses stream-json handling or the session presenter's TUI/headless mode, executes one turn, and bounds interruption without creating resumable interactive state. That cohesive boundary is now owned separately from the long-running chat loop.

## History and invariants

- `2a4fbb7c` established the one-shot `run` entry point on the shared session builder.
- `8035de26` added first-run provider setup and the bounded plain-output heartbeat.
- `2818184f` added the `stream-json` early-return path so stdout remains a clean NDJSON event stream.
- `2810935f` added system guidance and task-tier propagation to the one-shot turn.
- `8b3073f9` added one-shot slash expansion so catalog commands and skills reach the model as resolved prompt/guidance rather than guessed prose.
- `4d2007b2` kept output-format behavior explicit at this boundary while preserving the established presenter paths.

## Boundary

`one_shot.rs` owns command/skill expansion and the complete one-turn execution policy. It preserves:

- project command trust checks;
- literal `//` escaping and unknown slash passthrough;
- command guidance and task-tier propagation;
- clean NDJSON output for `stream-json`;
- TUI final-frame retention;
- plain-output heartbeat and Ctrl-C partial-output retention.

Session construction remains owned by `session.rs`, including selection of the TUI or headless presenter. Provider routing, persistence, workspace rooting, Anywhere handoff checks, quota state, MCP, Lattice, and LSP setup therefore remain shared with interactive and daemon sessions.

## Interface as test surface

`one_shot_slash_passthrough_and_escape` characterizes plain prompts, literal slash escaping, and unknown/absolute-path passthrough through the extracted resolver. Existing CLI and core tests retain coverage of output presenters, session construction, routing, and turn execution.

## Leave alone

- `stream-json` stdout must contain only NDJSON events; heartbeat output remains disabled there.
- Unknown slash tokens and absolute paths pass through verbatim.
- Project-scoped catalog entries remain default-deny without explicit trust.
- Ctrl-C keeps already-streamed output rather than hard-killing the process.
- One-shot execution must not duplicate session construction or provider-routing policy.

## Questions still open

The parent `run.rs` remains oversized because it still owns the interactive TUI lifecycle and several independently evolving modal/event policies. Further reductions require history-backed boundaries with direct characterization rather than additional entry-point-only cuts.
