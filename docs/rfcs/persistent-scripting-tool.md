# RFC: Persistent scripting environment as a Forge tool

Status: DRAFT — needs Floris's review before any implementation.
Date: 2026-08-06. Author: Forge. Context: docs/architecture/prime-agent-comparison.md items 7/8.

## Problem

Prime-agent's core architectural bet is a persistent IPython kernel as the model's ONLY tool:
file ops, shell, skills, and subagent spawning all happen as Python code; interpreter state
(variables, parsed data, handles) survives across turns and compaction, and can snapshot to
disk for session revival. This buys two real capabilities Forge lacks:

1. **Durable working state.** Parse a 200MB log once, keep the dataframe, query it across
   twenty turns. Forge re-reads and re-greps; each tool result burns context tokens.
2. **Programmatic composition.** One code cell filters 500 files and calls a helper on 30 of
   them. Forge round-trips each step through the model at token + latency cost.

The naive port is unacceptable: code executing inside a kernel does file IO, network, and
process spawning invisibly, which guts the permission broker (ADR-0008) — the property that
distinguishes Forge from prime-agent (which is trust-and-warn).

## Options

**A. Don't build it.** Forge's discrete tools + Lattice retrieval + subagent fan-out cover most
coding-agent work. Cost: the data-heavy/research niche stays weaker; the comparison doc's two
capabilities remain prime-agent advantages.

**B. Opt-in scripting tool with brokered side effects (recommended).** A `script` tool backed by
a persistent interpreter per session, off by default, enabled per session (`/script on`) or
config. Two isolation properties:

- **In-process effects are sandboxed by construction.** The interpreter is a Forge-owned
  subprocess started with no ambient credentials, cwd-jailed openings via a preopened-dir model
  (WASI-style if we choose a wasm runtime; or an OS-sandboxed child otherwise — decision point
  below), and no direct network.
- **All real side effects exit through typed host requests.** Inside the environment, `forge.*`
  builtins (read_file, write_file, shell, spawn_agent, …) serialize the request back to the
  session core, which routes it through the SAME permission broker as ordinary tool calls
  (ADR-0008 intact — prime's host.request pattern, hardened). A blocked request raises inside
  the script; the script can continue or abort.

Interpreter choice is the main open question: embedded scripting language in the single binary
(keeps ADR-0002; candidates: a wasm-sandboxed Python build, or an embeddable language the model
writes well — realistically Python is the only one models write fluently enough to be worth it)
vs an external `python3` the way prime bootstraps a venv (violates single-binary delivery
unless treated like other optional externals, e.g. git). Needs a spike measuring: model
fluency, startup latency, snapshot/restore feasibility, and sandbox strength per candidate.

**C. Kernel-as-only-tool (prime's full design).** Rejected: discards Forge's typed tool surface,
permission classifier granularity, per-tool telemetry, and the bridge protocol; forces every
model in the mesh (including weak free-tier models) to write correct Python for every file
edit. Prime can afford this with frontier models; the mesh cannot.

## Recommendation

B, gated on the interpreter spike. Success criteria for building past the spike: (1) a scripted
multi-file analysis measurably beats the discrete-tool baseline on tokens or wall-clock for a
representative task set; (2) every side-effect path demonstrably crosses the broker (prove the
failure path: a denied write must fail inside the script); (3) session-scoped state survives
compaction and daemon detach/reattach; (4) single-binary delivery preserved or the external
dependency is optional-with-graceful-absence.

## Non-goals

Python-backed skills (comparison item 8) until the interpreter exists; replacing any existing
tool; trace upload (platform feature, skipped in the comparison).
