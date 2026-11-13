# Feature: scouts — orientation in a throwaway context

> Status: **EXPERIMENTAL, default `off`** (opt in with `scouts = "tool"` or `"auto"`; the A/B was
> inconclusive). Code: `crates/forge-core/src/scouts.rs`,
> `crates/forge-config/src/scouts.rs` (the setting), the built-in `scout` agent in
> `crates/forge-config/src/agent_builtins.rs`. Tests: `crates/forge-core/src/tests/scouts.rs`.

## Why

Exploration is most of what a coding agent reads. In the owner's store, `read_file` (30M chars),
`search` (8M), shell `grep`/`rg`/`find`/`ls` (5.4M) and `cat`/`sed`/`head` (6.4M) are about 74% of
all tool-result characters, and the median landed change spends 37% of its steps before the first
edit. Every one of those steps pays the full main prompt (133k tokens on average), while a step in a
fresh small context starts at 15-20k. The explore agent was used 10 times in 33k tool calls.

## What ships

```toml
[harness]
scouts = "off"    # default. "tool" | "auto" to opt in; /config has it under Behaviour
```

- `tool`: the model is offered `scout {question}`. A scout is the built-in `scout`
  subagent: read-only (`read_file`, `list_dir`, `search`, `glob`, `lattice`, read-only `shell`),
  plan-mode permissions, cheap tier (`trivial`) unless a pin applies, fresh context, at most 12
  model steps. It returns a brief of at most about 400 words: the answer, then facts each ending in
  `path:start-end`, then what it looked for and did not find. The parent keeps only the brief,
  truncated at 12,000 characters.
- `off` (default): no `scout` tool.
- `auto`: the same tool, described as "call this first", and in addition Forge runs a scout itself
  before the model's first step when the opening prompt of a session reads like a coding task
  (60+ characters containing a coding word, not a slash command), the repository has at least 25
  files, subagents are enabled and the model is not a CLI bridge. The brief arrives as a context
  note ("a read-only scout already explored this repository..."). Later turns are not scouted.
- `off`: nothing is offered or run.

`scout` is an alias for a one-agent `spawn_agents` call (`scouts::rewrite` in
`resolve_model_call`), so routing, the depth-1 guard, permissions, persistence and the child rows
are the subagent machinery unchanged. A scout cannot start scouts. A user or project agent file
named `scout` replaces the built-in.

## What the owner sees

- Scrollback: `scout: 14 reads -> 1.8k brief (saved ~90k)`. The saving is an estimate: scout tool
  calls times (the parent's transcript plus 8k of fixed prompt, minus an 18k scout context). It is
  prompt tokens the parent did not have to re-read, not dollars; most of them would have been cache
  hits.
- The scout appears as a child on every surface that shows subagents (TUI panel, desktop, phone),
  with its own model and cost.
- `forge turns` lists "turns that ran scouts" with the count; `forge turns --json` has
  `scout_turns`; the stored turn detail carries `"scouts": N`.

## A/B and the default

Endurance kit (`~/.cache/forge-ops/ccbench/endure`), `inferhub::cb/deepseek-v4.1-flash`, isolated
HOME/XDG/FORGE_DB, one release binary per round, `[harness] scouts` set by an arm file. Input tokens
are parent plus scout. Median over attempts.

| round | arm | tasks | n | solved | model calls | input tokens | wall |
|---|---|---|---|---|---|---|---|
| 1 | off | E1 E4 E9 E13 | 8 | 8/8 | 21.0 | 601k | void |
| 1 | auto, scout uncapped | E1 E4 E9 E13 | 8 | 8/8 | 34.0 | 717k | void |
| 2 | off | E1 E4 E13 | 12 | 11/12 | 17.5 | 425k | 106 s |
| 2 | auto, scout capped at 12 steps | E1 E4 E13 | 12 | 12/12 | 21.5 | 369k | 127 s |

- Round 1 wall times are void: InferHub answered 503 `no_capacity` for about 25 minutes in the
  middle of it. Its lesson is the cap: an uncapped scout took 40 model steps and 1.0M input tokens
  on one E1 run and 51 steps and 2.5M on one E9 run, several times what the parent spent. The
  scout now stops at 12 steps and is asked for at most 8 tool calls.
- Round 2 (cap on, no outage): scouts cut median input tokens by 13% (mean 4%), but cost 23% more
  model calls and 20% more wall time, and solved no fewer tasks (the one `off` miss was an
  `early_partial_report` on E4, unrelated to scouting). E13 is the
  "find the fix location from a bug report" task (66-file seeded repo, `tasks/E13-hunt` in the
  kit); there parent plus scout used 114k input tokens on average against 146k with scouts off
  (-22%, four runs each), and the scout used 7 and 12 model steps in the first two runs.
- The kit's contexts are small (max prompt 16k-42k), so the case the feature is for, a main prompt
  of about 133k paying for every exploration step, is not reproduced; the mechanical tasks were
  controls. n is small and DeepSeek's step counts vary by 2x between seeds. The A/B is
  inconclusive, so scouts are experimental and the default is **`off`** (it was `tool` while the
  feature was on trial). Re-run on long-context tasks before turning `tool` or `auto` on by default.

## Limits, stated plainly

- The brief is only as good as a cheap model's reading. The tool text tells the parent to read the
  cited ranges before editing them; the pre-scout note says the same.
- The "model's first steps are reads with no edit yet" trigger from the design is not implemented:
  it would have meant a hook in the model loop. `auto` decides from the prompt alone.
- A scout run costs a model call chain of its own. It pays off when the parent would have read
  more than a handful of files; on a task that names its one file it is waste, which is why
  `auto` skips small repositories and later turns.
- When a scout hits its step cap without an answer, the parent gets the subagent error text, not a
  brief, and falls back to reading itself.
