# Task list on cheap models, November 2026

Claude Code now turns its own task list off for newer models. Should Forge keep its task list
(`update_tasks`, the pinned task block and the task nudges) for the cheap models the owner runs?

**Result: keep it, on every tier, with the defaults unchanged.**

- DeepSeek V4.1 Flash almost never uses the task list. Offering it or removing it made no measurable
  difference to whether long tasks finished, even across compactions.
- The models that do use it (Muse, Kimi, GPT-5.x) are the ones whose open lists now stop "done with work
  left" turns.
- Nothing measured here argues for removing it, and Claude Code published no measurement that would.

## What Claude Code changed (primary sources)

| Version (npm date) | Change |
|---|---|
| 0.2.93 (2025-04-30) | Todo list introduced |
| 2.1.16 (2026-01-22) | `TodoWrite` replaced by the Tasks system (`TaskCreate`/`Get`/`Update`/`List`). `CLAUDE_CODE_ENABLE_TASKS=0` keeps the old one |
| 2.1.233 (2026-08-14) | Task tools no longer offered on Opus 4.8, Sonnet 5, Fable 5, Mythos 5 and newer |
| 2.1.268 (2026-09-10) | Turned into an allowlist: offered only on Claude 3.x, Opus 4.0-4.7, Sonnet 4.0-4.6 and Haiku 4.5. Model IDs Claude Code does not recognise (DeepSeek through a gateway, for example) get none |

Sources:
- the [Claude Code CHANGELOG](https://raw.githubusercontent.com/anthropics/claude-code/main/CHANGELOG.md)
- the [tools reference](https://code.claude.com/docs/en/tools-reference.md#task-tool-availability)
- npm registry dates

`CLAUDE_CODE_ENABLE_TODO_TOOLS=1` turns the tools back on.

What this does and does not say:

- **The tools were not removed.** They became model-conditional.
- **The stated reason is not "hurts performance".** Anthropic says newer Claude models track multi-step
  work without a checklist, and that the tool definitions and reminders cost context. No official source
  says the tools hurt performance. That claim traces back to a Claude Code engineer's post about Opus 4.5
  (Feb 2026), which has no numbers.
- **No measurement either way.** Anthropic has published none. Haiku 4.5, the cheapest Claude model,
  keeps the tools.
- **Unrecognised models get none by fallback.** It is a default, not a finding about those models.
- **Confounded with a prompt rewrite.** The change shipped together with a much shorter system prompt:
  27k characters against about 6k in captured requests (`claude -p` against a stub server; no model was
  called).
- **The closest published ablation tested something else.** arXiv 2609.20804 (Sep 2026) found that an
  explicit planning step helps weaker models and mostly saves cost on stronger ones. That was a planning
  step, not a todo tool.

## What Forge does

- **The tool.** `update_tasks` is offered on every turn and profile. `FORGE_SYSTEM` asks for a short plan
  on non-trivial work.
- **Pinned list and reminders.**
  - An open list is pinned into every request.
  - A `[task list]` note is added at the start of a user turn while tasks are open.
  - A stale task is escalated after 3 unchanged turns.
- **Open-task re-drive.** A reply that leaves tasks open is re-driven, with a bounded budget. Before this
  branch, only `guarded` did this. `minimal`, the default for `inferhub::*`, skipped it, and an open list
  even blocked the mid-intent nudge. Now it runs on every profile.
- **New switches** (`[harness]`, used for this A/B; see [harness-profile.md](../features/harness-profile.md)):
  - `task_tool = false` removes the tool, its prompt line and every task nudge.
  - `task_nudges = false` keeps the tool but drops the nudges.

## How often the owner's models use it

Real use, from read-only counts of the owner's store since 2026-09-01: `update_tasks` calls as a share of
all tool calls.

| Model | Tool calls | `update_tasks` | Share |
|---|---|---|---|
| `inferhub::cb/deepseek-v4.1-flash` | 1,425 | 9 | 0.6% |
| `deepseek::deepseek-v4-pro` | 259 | 0 | 0% |
| `refuseless::glm-5.3-flash` | 1,758 | 24 | 1.4% |
| `bedrock::kimi-k3` / `kimi::k3-256k` | 4,857 | 175 | 2.2-5.3% |
| `meta`/`opencode` Muse Spark 1.3 | 21,615 | 691 | 2.4-3.5% |
| `codex-oauth::gpt-5.x` | 1,645 | 72 | 2.4-14.6% |

## The A/B

The suite is the long-run endurance suite ([endurance-2026-11.md](endurance-2026-11.md)). Run on
`inferhub::cb/deepseek-v4.1-flash`, fixed build, one prompt per attempt, never a "continue".

Normal context (the owner's compaction point; no attempt compacted). Tasks E1, E4 and E9-E12, two
attempts each:

| Arm | Solved | Median model calls | Median tokens | Median wall | `update_tasks` calls (all attempts) |
|---|---|---|---|---|---|
| tool + nudges | 12/12 | 31 | 0.97M | 176 s | 4 |
| tool, no nudges | 12/12 | 39 | 1.57M | 277 s | 5 |
| no tool | 11/12 (1 edited a protected test) | 29.5 | 1.04M | 202 s | 0 |

Forced compactions (`[compact_cap] inferhub = 40000`). Tasks E9-E12, three attempts each:

| Arm | Solved | Attempts that compacted | Median model calls | Median tokens | Median wall | `update_tasks` calls |
|---|---|---|---|---|---|---|
| tool + nudges | 12/12 | 9/12 | 49 | 1.38M | 361 s | 0 |
| no tool | 12/12 | 8/12 | 48 | 1.27M | 311 s | 0 |

How to read these tables:

- **The spread is noise.** The token and wall-time differences between arms run in both directions, and
  the arm with the task tool barely used it. Since the tool was not used, the differences cannot come
  from the tool. They are attempt-to-attempt noise at this sample size.
- **The one failure.** The single failure (no tool, E4) finished all 61 units of the rename but edited a
  protected test file (`tests/test_smoke.py`), which the verifier rejects. It is a rule break, not an early
  stop, and one case does not show a pattern.

## Decision per tier

| Tier | Models | Decision | Why |
|---|---|---|---|
| Cheap, rarely lists | DeepSeek V4.1 Flash / v4-pro | Keep (defaults) | Uses it in under 1% of calls. Offering it cost nothing measurable and removing it gained nothing. When it is used, the open list now re-drives a premature stop. |
| Cheap, lists | GLM 5.3, Kimi K3, Muse Spark | Keep (defaults) | 1.4-5.3% of calls. These are the sessions where open tasks anchor the new re-drive. Not run live: project rules limit live tests to DeepSeek Flash. |
| Frontier | GPT-5.x via Codex, Claude via the bridges | Keep | Claude Code's own gate has no published numbers. Bridged Claude and Codex keep their own loops. |

Not decided here, because it needs the owner's OK to run live on other models: whether Kimi, GLM or
Muse finish more long tasks with the list than without. To run it, use the same suite and arms with
`CCBENCH_FORGE` set to the fixed build and the model swapped.
