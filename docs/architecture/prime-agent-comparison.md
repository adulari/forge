# Prime Agent vs Forge — feature comparison and port plan

Date: 2026-08-06. Source studied: `PrimeIntellect-ai/prime-agent` @ `c5991bc` (MIT, fork of
Mario Zechner's pi-mono). Evidence: prime-agent `packages/coding-agent/docs/` +
`src/core/refinement/refinement.ts`, `src/core/rlm-runtime` docs, `prime-agent-runtime/src/rlm/`.
Forge evidence: file:line refs below, verified against source this session (not runtime-measured
unless stated).

Goal: identify everything prime-agent has that Forge lacks, judge which gaps matter, and rank
port work. License is MIT — recreating designs and lifting code are both permitted with
attribution.

## What prime-agent is

A TypeScript coding/research agent built around two bets:

1. **RLM runtime** — the model gets ONE tool: a persistent IPython kernel. File ops, shell,
   skills, and subagent spawning all happen as Python code. Python state (variables, parsed
   data, handles) survives across turns and compaction; kernels can snapshot to
   `kernel-state.dill` for session revival.
2. **Continual Harness** — `/refine` runs an LLM review over the current trajectory and emits
   small create/update/delete edits to durable supplemental state: prompt notes, memories,
   reusable skill descriptions, and subagent specs. History is journaled with before/after
   snapshots for rollback; an auto-refine gate fires on turn intervals and at compaction. The
   base system prompt is immutable. This is the "self-improving" part; the actual RL training
   loop lives outside the harness (opt-in `/traces` trajectory upload feeds Prime's platform).

Everything else (daemon workers, attach/detach, goals, autonomous mode, compaction, session
tree) is infrastructure around those two bets.

## Feature-by-feature

Status legend: **HAVE** (equivalent or better), **PARTIAL** (same idea, missing pieces),
**MISSING** (no equivalent), **SKIP** (deliberately not porting).

| # | Prime-agent feature | Forge status | Verdict |
|---|---|---|---|
| 1 | Continual Harness + `/refine` + auto-refine gate + rollback | **MISSING** | Port — highest value |
| 2 | Autonomous quality gates (`--autonomous-gate` shell cmds, retries, timeouts, unchanged-workspace skip, token/turn/time budgets) | **PARTIAL** | Port gaps |
| 3 | Heartbeats re-entering a live session (user `/heartbeat` + agent-created `rlm_heartbeat`) | **MISSING** | Port |
| 4 | Agent-to-agent messaging (peer/sibling/broadcast, steer vs follow_up delivery, CLI `send`) | **PARTIAL** | Port gaps |
| 5 | Retained, addressable, async subagents (admission handles, registry survives restart, usage attribution) | **PARTIAL** | Port gaps |
| 6 | Persistent goals with token budget + `goal.complete()` | **PARTIAL** | Port budget |
| 7 | Persistent IPython kernel as sole tool (programmatic tool calling, state across turns, dill snapshots) | **MISSING** | RFC first — architectural bet, conflicts with ADR-0008 posture if done naively |
| 8 | Python-backed executable skills (skill = importable package, typed callable) | **MISSING** | Depends on #7 decision |
| 9 | Opt-in trajectory/trace upload for RL training | **MISSING** | Skip as platform feature; note local trajectory-export analog |
| 10 | Daemon-backed sessions, attach/detach, supervisor, doctor | **HAVE** | — |
| 11 | Auto + manual compaction, kernel state surviving it | **HAVE** (compaction) | kernel part n/a |
| 12 | Session tree, `/fork`, `/clone`, branch summarization | **PARTIAL** — Forge `fork` is a counterfactual re-run CLI (`args.rs:472`), not in-TUI tree navigation | Low priority |
| 13 | Steering vs follow-up queued messages | **HAVE-mostly** — prime's user "steering" also delivers between assistant turns (usage.md), same as Forge's queue drain; the true gap is the a2a `steer` delivery mode, covered in #4 | Fold into #4 |
| 14 | `/btw` side questions (out-of-session Q&A) | **MISSING** | Small UX port |
| 15 | `/export` HTML, `/share` gist | **MISSING** (Forge has `forge replay` TUI) | Small port |
| 16 | TS extension API, npm/git packages, themes | **SKIP** | Forge = Rust single binary (ADR-0002); hooks+MCP+skills cover this |
| 17 | ACP mode (Zed) | **MISSING** | Optional, low priority |
| 18 | Model routing, multi-provider, OAuth subscriptions | **HAVE** (mesh is stronger: ranking, budget pressure, health, runtime failover, rationale; prime's only fallback is a startup `modelFallbackMessage` at session create, sdk.md) | — |
| 19 | MCP integrations | **HAVE** (forge-mcp client + serve) | — |
| 20 | Markdown skills (Agent Skills format), skill-creator | **HAVE** (forge-skills + builtin skills) | — |
| 21 | Memories (durable facts) | **HAVE** (auto-memory: `forge-store/src/memory.rs` — keyword+salience+recency recall, dedup) | Recall side arguably better; write side weaker (no reviewed refinement) |
| 22 | Worktree isolation, permission broker, Lattice code index, Assay review, workflows, remote/mobile surfaces, Anywhere relay, voice, bench harness | Forge-only | Forge advantages prime-agent lacks |

## Evidence per verdict

**1. Continual Harness — MISSING.** Prime: `refinement.ts` defines
`HarnessEntry {kind: prompt|memory|skill|subagent}`, `RefinementProposal` (CUD edits, JSON-only
model output), refinement history JSONL, rollback from before/after snapshots, local
(session-artifact) vs global (`~/.prime/agent/harness/`) scope, and an `AutoRefineReview` gate
(`turn_interval` | `compact` triggers). Forge: auto-memory only (`memory.rs`) — no supplemental
prompt notes, no learned skill descriptions, no learned subagent specs, no trajectory-review
edit loop, no rollback ledger. The mesh's "refinement" strings (`forge-mesh/src/context.rs:12`)
are routing context, unrelated.

**2. Autonomous gates — PARTIAL.** Forge `/loop`/`/goal`
(`run/autonomous.rs`): sentinel completion (`LOOP_COMPLETE`), task-plan progress + stall
detection (`GOAL_NO_PROGRESS_MAX 6`), iteration caps (200), auto-compact at 0.80 fill. Bridge
turns have an objective verification gate (per the v0.4.1 release, not re-verified this
session), and `quality_gates.rs` runs an Assay critic
over the turn diff (warn/block). Missing vs prime: user-defined shell gate commands with
per-gate retries and timeouts, don't-rerun-unchanged-workspace optimization, and token/wall-clock
budgets for autonomous runs (no `token_budget` in the goal driver — verified by grep).

**3. Heartbeats — MISSING.** Forge `forge schedule`
(`schedule.rs:1`) fires **fresh `forge run` invocations via OS timers** — headless analog of
cron. Nothing re-enters a *live* session periodically, and the agent cannot create its own
recurring check-ins. Prime has user `/heartbeat`, agent-created `rlm_heartbeat` (multiple, with
steer/follow-up delivery), plus session-targeted schedules with claimed ticks and coalesced
misses.

**4. A2A messaging — PARTIAL.** Forge has `send_to_agent` parent↔child within a session tree
(`mcp_serve/subagents.rs`). No peer/sibling messaging between fleet sessions, no broadcast, no
`forge send <session> <msg>` CLI (grep of `args.rs` found none), no steer-into-active-turn
delivery. The remote protocol can inject prompts into a daemon session (that is how
Helm `prompt_dev_session` works), so the transport exists — the missing part is the addressed
message surface + delivery modes + sender identity/limits.

**5. Subagents — PARTIAL.** Forge subagents are parallel headless children inside one turn:
Ask→Deny permission posture (`subagent/execution.rs`), blocking fan-out that "returns all
results, labeled" to the spawning turn (`subagent/requests.rs:32`). Prime children are full daemon sessions: `rlm()` returns an
admission handle immediately, results arrive later as messages, completed children stay
addressable across kernel restart/compaction/parent restore, and child usage is folded into the
parent turn with a persisted `child_usage_attributed` entry. Forge lacks async admission
handles, retained addressable children, and restart-surviving child registry.

**6. Goals — PARTIAL.** Forge `/goal` is arguably stronger on completion inference (task-plan
based, stall detection); prime adds an explicit token budget and elapsed-time accounting per
goal. Small gap.

**7/8. IPython kernel + Python skills — MISSING, decide via RFC.** This is prime's core
architecture, not a feature to bolt on. Arguments for a Forge analog (persistent scripting
tool with durable state): genuine wins on data-heavy/research tasks (parse once, reuse across
turns; program the tool sequence instead of round-tripping each call through the model).
Arguments against naive port: Forge's discrete tool surface is what makes the permission broker
(ADR-0008) meaningful — a kernel that does file IO + shell + spawning from inside Python
bypasses per-side-effect gating unless the kernel brokers host requests the way prime does
(typed `host.request` comms). A Forge version would need side-effect brokering from inside the
scripting environment. That is an ADR + RFC, not a card.

**9. Traces/RL upload — SKIP (platform).** Prime's `/traces` uploads trajectories to Prime
Intellect for training (`agent-traces.ts`, opt-in). The self-improvement users actually feel is
#1, which is in-harness and portable. A local analog (export sessions as training-ready
trajectory files) is cheap later; Forge already persists full transcripts + usage in the Store.

**Forge-only advantages (for the "which is better" question):** Model Mesh (deterministic
classification, ranking, budget pressure, health, failover — prime routes by explicit model
selection only, and its `rlm()` spawn *fails* rather than failing over), permission broker +
temper modes (prime has no built-in permission broker — default posture is trust-and-warn;
extensions *can* implement approval policies per `extensions/types.ts`), Lattice, Assay,
workflows, five client surfaces + Anywhere, voice, worktree isolation, hooks, benchmarking.
On safety and provider robustness Forge is clearly ahead; on session self-improvement and
long-running session ergonomics prime is ahead. That is the gap to close.

## Ranked port list

1. **Continual Harness** (`forge refine` + auto-refine): supplemental prompt notes / skill
   descriptions / subagent specs as store-backed entries with scope local|global, LLM
   trajectory review emitting CUD edits, journaled history + rollback, injection alongside
   auto-memory recall. Extends the Store (ADR-0005) and context pipeline; no ADR conflict.
2. **Autonomous gates + budgets**: `--gate <cmd>` (repeatable) with retries/timeout,
   unchanged-workspace skip, token + wall-clock budgets for `/loop`/`/goal` and headless runs.
3. **Heartbeats**: user `/heartbeat` + agent-creatable heartbeats on daemon sessions
   (automation_store already has schedules; add session-targeted delivery + claimed/coalesced
   ticks).
4. **Fleet messaging**: `forge send`, peer/sibling addressing, steer vs follow_up delivery
   modes, sender identity + queue limits over the existing remote protocol.
5. **Retained async subagents**: admission handles, results-as-messages, child registry that
   survives restart, usage attribution rows.
6. **Goal budgets**: token/time budget on `/goal` (fold into 2 if convenient).
7. **Steer delivery for queued messages** (mid-turn injection option).
8. **RFC: persistent scripting environment** (the kernel question) — write
   `docs/rfcs/persistent-scripting-tool.md` before any code; decide broker model.
9. Small UX: `/btw` side questions; HTML session export.

Not porting: TS extensions/packages/themes (16), ACP (17, revisit on demand), platform trace
upload (9).
