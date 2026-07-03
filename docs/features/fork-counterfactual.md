# `forge fork` / `forge tree` — counterfactual session branching

> **Status: shipped.** Branch any past session before turn N, hold every earlier turn verbatim,
> and re-ask that one prompt — optionally on a different model. Then read the diff. "What would
> model X have done at turn 12" stops being a shower thought and becomes a command.

## What it does

```bash
forge fork 758e4d71 --turn 2 --rerun                     # re-ask turn 2, mesh-routed, diff after
forge fork 758e4d71 --turn 2 --model groq::llama-3.3-70b --rerun
forge fork 758e4d71                                      # fork at the last turn, continue by hand
forge tree                                               # fork lineage: sources + branches
forge replay <a> <b>                                     # compare any pair, turn by turn
```

`--rerun` runs the forked turn immediately (spawning `forge run --resume <fork>` under your
normal permission mode) and prints the counterfactual card: the replay summary diff plus the
aligned per-turn diff. The shared prefix is identical by construction, so **the diff IS the
effect of the change**.

Without `--rerun` the fork is just created; continue it interactively (`forge chat --resume`) or
with a pinned model.

## How it relates to `forge replay --rerun`

| | history before turn N | turn N | what it answers |
|---|---|---|---|
| `replay --rerun` | re-executed fresh | re-executed | "would today's mesh solve this whole session the same way?" |
| `fork --turn N` | **held verbatim** | re-executed (optionally pinned) | "with everything up to here EXACTLY as it was, what changes if I swap the model / rephrase?" |

A fork changes one variable; a rerun changes them all.

## Design

- `Store::fork_session(src, at_seq)` (migration 0006: `session.forked_from` +
  `session.forked_at_seq`, idempotent `add_column_if_missing`): one immediate transaction
  creates the new top-level session and copies the *active* message prefix `seq < at_seq` —
  compaction-soft-deleted rows are not resurrected, and the re-asked prompt itself is not
  copied (the fork's next turn supplies it, which is the point).
- Turn addressing: `--turn N` = the Nth user message (1-based); default = last.
- `forge tree` renders only fork families (a session with no fork relation is noise here —
  `forge sessions` is the flat list), labeled by first prompt.
- Forks are ordinary sessions: resumable, replayable, diffable, blame-able. Subagent children
  stay excluded from the tree.

## Limitations

- **Conversation-state only.** Files are not rewound to the fork point — the forked turn runs
  against the working tree as it is now. Use `/checkpoint` rewind first when the filesystem
  state matters to the comparison.
- The fork's transcript prefix is verbatim even if the original later compacted; a fork of a
  compacted session copies the compacted view's active rows.

## Verified

- Store test: fork copies exactly the prefix, links `forked_from`/`forked_at_seq`, leaves the
  source untouched.
- Live e2e (mock, scratch repo, isolated DB): two-turn session → `fork --turn 2 --rerun --mock`
  held turn 1, re-ran turn 2, printed the counterfactual card; `forge tree` showed the lineage.
