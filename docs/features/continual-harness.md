# Feature: Continual Harness (`/refine`)

> **Status: shipped.** A port of prime-agent's `/refine`, adapted to Forge: the agent looks back at
> its own trajectory and proposes small, evidence-backed durable edits to how *it* operates on
> future turns — a supplemental prompt note, a reusable skill, or a subagent delegation spec. Unlike
> prime-agent, Forge delegates durable-*fact* memory to its own existing auto-memory system, so the
> harness never proposes `memory`-kind entries — only `prompt` / `skill` / `subagent`.

## 1. Problem (JTBD)
> When Forge makes the same mistake twice, or I have to give it the same instruction across
> sessions, I want it to notice and write that lesson down somewhere it will actually see it next
> time — without me hand-editing a prompt file, and without it silently rewriting its own base
> system prompt.

Auto-memory already solves this for durable *facts* ("the API key lives in `.env.local`"). The
Continual Harness solves it for durable *behavior*: conventions, pitfalls, house-style rules, and
reusable procedures.

## 2. What it is

`Session::refine()` (forge-core, `refinement.rs`) takes a bounded slice of the recent transcript
plus the harness entries and refinement history already in scope, sends them to a **trivial-tier**
model with a fixed system prompt, and asks it to propose a small batch of `create` / `update` /
`delete` edits against one of three entry kinds:

- **prompt** — a supplemental note added *alongside* the base system prompt (a convention, a
  pitfall, a house-style rule).
- **skill** — a reusable procedure: when to use it, the steps, how to verify it worked.
- **subagent** — a reusable delegation spec for `spawn_agents`.

Every edit must cite concrete evidence from the conversation (a mistake made and corrected, a
repeated pattern, an instruction given twice); the model is told to propose few precise edits, or
none at all, rather than a sweeping rewrite. Edits are journaled as one `HarnessRefinement` batch
(summary, rationale, expected outcome, and the full before/after snapshot of every entry touched),
so every batch is inspectable and reversible.

**Invariant: the base system prompt is never modified.** Harness entries are injected as an
additional, clearly-labeled context block (`harness_context_block`, `context_pipeline.rs`) — never
concatenated into or replacing `FORGE_SYSTEM`. A `memory`-kind edit is rejected outright: Forge's
auto-memory system already owns durable facts/preferences/decisions.

## 3. The `/refine` command

```
/refine [instructions]              propose + apply a refinement batch now, targeting this session
/refine --global [instructions]     same, but targeting the global scope instead of this session
/refine rollback <id>               invert a past refinement batch (id or a unique id prefix)
/refine status                      list harness entries in scope + recent refinement history
```

`instructions` is optional free text steering what the model should focus on (e.g.
`/refine be stricter about tool-call retries`). `/refine` and `/refine --global` make a model call
and run as a background task exactly like `/compact` (spinner ticks, gated while a turn is in
flight); `status` and `rollback` are pure store reads/writes and run inline. A successful refinement
reports its own summary and applied/rejected edit count via the same `PresenterEvent::Warning`
convention `/compact` uses.

## 4. Scope semantics

Every harness entry lives in exactly one scope: `session:<id>`, `project:<workspace root>`, or
`global`. A refinement's *target* scope — where its `create`/`update`/`delete` edits land — is
`session:<id>` by default, or `global` with `--global`. There is currently no `/refine` form that
targets `project:` directly (it's still injected/read, just not a manual-refine target yet).

Context injection and the `/refine status` overview both read the **scope chain**, most-specific
first: `session:<id>` → `project:<workspace root>` → `global`. Entries from a scope other than the
current target are still shown to the model as read-only reference context during a refinement pass
— never edited by that pass; the store enforces this boundary independently (an `update`/`delete`
naming an id outside the target scope is recorded as a rejected edit, not applied).

## 5. Auto-refine modes (`harness.auto_refine`)

| Mode | Behavior |
|---|---|
| `off` (default) | Refinement only runs when explicitly requested via `/refine`. |
| `compact` | A refinement pass auto-fires after every `/compact`. |
| `turns` | A refinement pass auto-fires every `harness.auto_refine_turns` completed turns. |

The turns-based gate (`should_auto_refine_turns`) is a plain counter: `auto_refine_turns == 0` never
fires (treated as disabled, not "every turn"), and any manual `/refine` resets the counter so an
auto-trigger never immediately re-fires on the next eligible turn. Auto-refine is best-effort — like
the other post-turn side calls (recap, suggestion, memory) — and never fails the turn it rides on.

## 6. Rollback

`/refine rollback <id>` inverts every applied edit in a past `HarnessRefinement` from its journaled
before/after snapshot — never from the entry's current live state — and journals the inversion
itself as a fresh refinement with `trigger = "rollback"`. A `create` is undone by deleting; an
`update` is undone by restoring the exact prior title/content/version; a `delete` is undone by
recreating the entry verbatim at its original id. `id` accepts either the full id or a unique prefix
against this session's recent refinements; an ambiguous or unknown prefix is reported rather than
guessed at.

## 7. Config (`[harness]`)

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Master toggle — gates both context injection and the ability to apply edits. |
| `auto_refine` | `"off"` | `"off"` \| `"compact"` \| `"turns"` — see §5. |
| `auto_refine_turns` | `20` | Turn interval for `auto_refine = "turns"`. |
| `max_context_entries` | `12` | Max harness entries injected into a turn's context. |
| `max_entry_chars` | `2000` | Max characters per injected harness entry; longer entries are truncated. |

`/refine status` is deliberately unfiltered by `harness.enabled` — it shows what's stored even while
injection/refinement is switched off.

## 8. Design

`Session::refine()` mirrors `Session::compact()`'s model-call shape end to end: the same
never-hard-fail-on-a-cheap-model candidate chain (top trivial candidates, then the routed model +
its own fallbacks, then the session's guaranteed-reachable model as a backstop), and the same
transcript-fitting helper (`fit_compaction_payload`) so a long session's harness prompt still fits
whichever candidate's context window. Dispatch: `CommandAction::Refine(RefineAction::Run { .. })` →
`DispatchOutcome::RunRefine` → `spawn_refine` (background task, busy/done machinery), gated while a
turn is in flight — same path as `/compact` → `RunCompact` → `spawn_compact`. `status` and
`rollback` are handled inline in `dispatch_command`, the same way `/memories` and `/uncompact` are.

## 9. Definition of done
- [x] `Session::refine()` proposes + applies one batch of harness edits from the trajectory.
- [x] Three entry kinds (`prompt`/`skill`/`subagent`); `memory`-kind edits rejected with an
      explanation.
- [x] Journaled, reversible batches (`HarnessRefinement`); `refine_rollback` inverts exactly.
- [x] Session/project/global scope chain; per-scope write isolation enforced by the store.
- [x] `/refine [--global] [instructions] | rollback <id> | status` — TUI command + palette entry.
- [x] Auto-refine gates for `compact` and `turns` modes; manual refine resets the turns counter.
- [x] Base system prompt never modified — harness entries injected as a separate labeled block.
- [x] Unit tests (edit validation, rollback inversion, scope isolation, id-prefix resolution);
      `cargo fmt` + `clippy` clean.
