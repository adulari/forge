# Code archaeology: subagent requests and headless tool execution

## Boundaries

Two owners split out of `subagent.rs`, at the two ends of a delegated run:

- `subagent/requests.rs` — what the parent asked for and what it resolves to. The advertised
  `spawn_agents` / `send_to_agent` specs, request parsing and clamping, child addressing, agent
  type resolution (with the read-only default tool set), the write-capability check that decides
  whether a child needs its own worktree, and the argument rewriting that keeps a worktree child's
  paths inside its own root.
- `subagent/execution.rs` — running one tool call for a child with nobody to ask.

`subagent.rs` keeps the middle: routing a child, the model↔tool loop, nested spawns, lifecycle
events, and orchestration.

## The rules that define each owner

Requests are *untrusted model output*: the agent count, the agent type, and the addressed child
are all validated before a child session exists, which is why parsing/clamping/resolution sit
together rather than being spread over the run.

Execution is *headless by construction*: a child runs in parallel with its siblings and has no
interactive surface, so an `Ask` decision resolves to Deny rather than blocking on a prompt that
can never be answered, and no presenter events are emitted. The safety denylist still applies.
That contract differs from the parent's `invoke_tool`, so it gets its own owner.

## Interface

`subagent.rs` re-exports the request surface (including `SEND_TO_AGENT_TOOL`, which
`tool_dispatch` matches on) and `sum_usage`, so every external caller — `forge-core`'s session,
`tool_dispatch`, the CLI bridge, and the worktree isolation integration test — is unchanged.

## Characterization

The crate's subagent tests and the full `forge-core` lib suite (507 tests) pass unchanged,
including the worktree isolation behaviour that depends on `is_write_capable` and the read-only
default tool set.
