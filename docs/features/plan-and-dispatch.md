# Feature: plan and dispatch — one prompt, split into parallel sessions

## 1. Why

Splitting a big task into parallel sessions used to mean creating each one by hand: writing a
prompt for every part, remembering which files each session should touch so they don't collide,
and starting them one at a time. Plan and dispatch does the split for you. Give the daemon one
prompt for a project; a **coordinator** session reads the project and proposes how to break the
request into independent work items; you review the split — approve all of it, approve a subset,
ask for changes, or cancel — and only then does anything start. Approved items run as ordinary
daemon sessions, each in its own git worktree by default, up to a limit you set, held back while
their dependencies are still running. You watch the whole thing from `forge board`: one card per
session, colour-grouped by dispatch, with a checklist while it is proposed and a progress view
once it runs.

## 2. A walkthrough from the board

Press `D` on the board to open the form. It asks for the request and four choices, each with a
sensible default so the fast path is "`D`, type, Enter":

```
╭─ Plan & dispatch ──────────────────────────────────────────────────╮
│ in forge   /home/floris/Documents/Repositories/Personal/AI/forge   │
│ ┌──────────────────────────────────────────────────────────────┐  │
│ │ Describe the work. Forge reads the project, proposes a split, │  │
│ │ and waits for your approval.                                  │  │
│ └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ▸ Worktrees      ● one per session   ○ shared directory           │
│    Each session gets its own branch; merge the results back.       │
│    Sessions may   edit files, ask before shell                     │
│    Run at once    ‹ 4 ›                                            │
│    At most        ‹ 8 › sessions                                   │
│                                                                     │
│  [ Enter  start ]  [ Esc  cancel ]                                 │
╰─ Enter start · Tab next · ←→ change · Esc cancel ──────────────────╯
```

Enter starts the dispatch: a coordinator session is created and selected, and its pane opens on
the Dispatch tab while the coordinator reads the project (`planning`). Once it calls
`dispatch_sessions`, the tab shows the proposed split as a dependency-aware checklist:

```
 proposed split · 3 sessions
     Mock split of the request into three parts.

 ▸[✓]  1  Notes file
        create a file notes.md
  [✓]  2  Tasks
        track the work
  [✓]  3  Summary                                            after 1, 2
        Reply with a one-line summary of the project.

  3 of 3 selected · 4 at once · worktrees on

 [ y  start 3 sessions ]  [ e  revise ]  [ n  cancel ]
```

`Space` (or a click on `[ ]`) ticks or unticks a row; unticking one also unticks anything that
depends on it, and ticking one also ticks whatever it depends on (§4). `Enter` unfolds a row's
full prompt. `y` approves the ticked items, `e` opens a composer for revision feedback, `n`
cancels. Once approved, the same tab becomes a progress view:

```
  ▰▰▰▱▱▱▱▱▱▱  1/3 done · 1 running · 1 waiting

 ▸✓  1  Notes file                                             finished
        merged back
  ⠹  2  Tasks                                                  running
        claude-cli::sonnet-5 · ▸ writing tasks.md · $0.02 · 4s
  ○  3  Summary                                          waiting for a slot
        waits for 1, 2

 [ A  merge all finished ]  [ n  cancel remaining ]
```

Each worker also shows up as its own card elsewhere on the board, tagged with a `◆ 2/3` chip in
the dispatch's colour; the coordinator's card shows the same colour and a `◆` marker. `z` zooms
the board down to just this dispatch's cards (`Esc` clears it); `w`/`X` merge or discard the
selected worktree session; `A` (or the button above) merges every `succeeded` item in index order,
one commit each (§7).
See `docs/features/project-board.md` § "Plan and dispatch" for the exact keys and card fields.

## 3. How the coordinator splits the work

The coordinator's first prompt (`forge_core::dispatch::coordinator_prompt`) tells it: read enough
of the project to split the work well (layout, the code the request touches, how it is built and
verified); make items independent, since parallel sessions that touch the same files conflict on
merge; prefer a few substantial items over many small ones, at most `max_items`; one item is a
correct answer when the work does not split cleanly; use `depends_on` only when an item needs
another item's result first; write each item's prompt as if for a fresh agent with no other
context — goal, files or areas, constraints, the exact verification command, and what "done"
means. It does not implement anything itself and is told not to edit files. It calls
`dispatch_sessions` once with a one-paragraph summary and the items, then ends its turn — the user
reviews the split before anything starts.

**How items are phrased, and why it matters.** Each worker session judges whether a turn has to
change files from the turn's own prompt (`crates/forge-core/src/turn_contract.rs`). So the
coordinator is told to open an item that changes the project with the instruction itself — a verb
such as Add, Implement, Fix, Refactor, Update, Remove, Rename, Create, Write or Change — which makes
the worker's turn demand a real diff, and to end an item that only investigates, verifies or
reports with the sentence "Do not edit files.", which makes that turn read-only.

## 4. Approval

`POST /api/dispatches/{id}/approve` takes an optional `selected` list of 1-based item numbers
(omitted = every item). `forge_core::dispatch::resolve_selection` resolves it against the
dependency graph:

- Unselected items become `skipped`.
- A selected item whose dependency was **not** selected can never run — it becomes `cancelled`,
  transitively (an item depending on a dropped item is dropped too).
- Everything else still runnable becomes `queued`, and the scheduler (§5) runs immediately.

On the board, unticking an item cascades to whatever depends on it and ticking one cascades to
whatever it depends on, so the checklist can never represent a selection `resolve_selection` would
reject. The daemon still validates independently — a CLI or API caller is not bound by the board's
UI.

**Revise** (`POST /api/dispatches/{id}/revise`, only while `proposed`) sends the coordinator's
feedback and moves the dispatch back to `planning`; the coordinator calls `dispatch_sessions`
again with a revised plan, replacing the previous proposal.

**Cancel** (`POST /api/dispatches/{id}/cancel`, refused once the dispatch is `done`/`cancelled`)
moves every non-terminal, not-yet-running item to `cancelled` and the dispatch itself to
`cancelled`. Items already running keep running to completion — cancel stops new starts, not work
in flight; merge or discard them afterwards as usual.

## 5. Scheduling

`max_running` (1-8, default 4) caps how many of this dispatch's sessions run at once; the rest
wait as `queued`. `max_items` (1-12, default 8) caps how many items a coordinator may propose.
`forge_core::dispatch::schedule` decides, each pass, which queued items can start and which can
never start:

- An item is ready once every item it `depends_on` reached `succeeded` or `merged`.
- An item whose dependency reached any **other** terminal state (`failed`, `stopped`, `cancelled`,
  `skipped`, `discarded`) can never run: it becomes `cancelled`, and this cascades to whatever
  depends on it in turn.
- Among the items that are ready, as many start as fit under `max_running`; the schedule loops
  until nothing more changes (a failed start frees its slot and may cancel dependents).

The pass reruns after every approval, every worker turn that finishes, and every merge/discard —
`crates/forge-cli/src/serve_dispatch.rs`'s `advance_locked`, always under that dispatch's own lock
so two passes can never start the same item twice.

## 6. What the coordinator is told, and when

Every message to the coordinator is a `[dispatch]`-prefixed follow-up, queued through the normal
fleet message queue (so it survives a coordinator that is busy or not live) and delivered once the
coordinator goes idle:

- **On approval** — which items started now, which are waiting (for a slot, or naming what they
  wait on), and which were not selected or could not start.
- **On revise** — the user's feedback, verbatim, with an instruction to call `dispatch_sessions`
  again.
- **On cancel** — that the user cancelled, and how many sessions already running keep going.
- **Each time an item finishes** — its title, session id, outcome (`succeeded` /
  `stopped without finishing (<reason>)`), how many are still running/waiting, which items were
  just cancelled because of it, and its final reply (the trailing run of assistant transcript
  rows), truncated to 2000 characters. The coordinator is told it can message a still-running
  session with `message_session` if this report affects it.
- **When every item reaches a terminal state** — one message listing every item's final status,
  asking the coordinator to summarize what happened, note any overlap or conflict, and say what
  order to merge in. It is told not to edit files itself.

A coordinator's undelivered `[dispatch]` messages are capped at 8 (the fleet queue's general
per-sender cap); a message that would exceed it is folded into the newest pending one instead of
being dropped, trimming the older text's *beginning* to fit under the 16 KB fleet-message limit.

## 7. Worktrees, merge and discard

With `worktree: true` (the default) each item session runs in its own `.forge/worktrees/<id>`
branch, exactly like any other daemon session started with `worktree: true` — items can never
stomp each other's working tree. With `worktree: false` every item runs in the dispatch's own
`cwd`; the coordinator is told to keep items non-overlapping since nothing here prevents a
collision.

**One worker** (`w` on the board) uses the session routes: `POST /api/sessions/{id}/merge` stops
the session, snapshots the worktree, and 3-way-merges its branch back; on success the item becomes
`merged`. The result is left **staged, not committed**, for the user to review. The route refuses a
base repo with uncommitted tracked changes (409 with `dirty_files`), because a conflict restores
the base with `git reset --hard HEAD`, which is only safe from a clean tree.
`POST /api/sessions/{id}/discard` stops the session and drops its worktree and branch without
merging; the item becomes `discarded`. A merge **conflict** leaves the item's status exactly as it
was (still `succeeded`, say) and the 409 response with `conflicts` is returned — dispatch state
never silently marks a conflicted merge as done.

**Every finished worker** (`A` on the board, `forge dispatch merge`) uses
`POST /api/dispatches/{id}/merge`, under the dispatch's lock. Merging workers one after another
through the session route would fail: the first merge leaves the base dirty, and the second is
refused. So the batch route **commits each clean merge** on the base repo's current branch before
starting the next. Every step starts from a clean HEAD, a conflict discards only its own partial
apply, and the items merged before it are already in commits. The route:

1. Refuses up front with 409 + `dirty_files` if the base has uncommitted tracked changes. That
   includes a staged single-session merge from `w`: commit it first.
2. Takes each `succeeded` item with a session in ascending index order. It merges the item exactly
   as the session route does, marks it `merged`, and commits the staged result:

   ```
   Merge dispatch item <n>: <title>

   From Forge dispatch <id8> — <first line of the request, ≤ 72 chars>. Session <session id8>, branch <branch>.
   ```

   The commit uses the repo's own git identity and hooks. A branch that adds nothing gets no
   commit (`commit: null`).
3. Stops at the first conflict or error. The conflicted session is respawned and keeps its
   worktree; its item stays `succeeded`. A **commit** that fails (no git identity, a hook that
   refuses) also stops the run: that item is merged and left staged, and the reason carries git's
   message.

The response is 200 even for a partial run, since it is a report (see §10).

**Workers are not nudged into editing.** An ordinary worktree session is armed, for its whole
life, to push back with "implement the fix now" whenever a turn ends without a diff. A dispatched
worker is not: its turns are the coordinator's item prompt and the coordinator's follow-up
messages, and an item that verifies or reports — or a message that only informs — must not be
re-driven into inventing changes. The protection still applies per turn: an item phrased as a change
instruction demands a diff. A worker resumed after a daemon restart or a merge-conflict respawn is
recognised from the store and keeps this behaviour.

## 8. The CLI — `forge dispatch`

A thin client of the daemon's own routes, sharing `forge attach`'s discovery and auth (loopback +
the persisted daemon token). It never touches the store directly, so the board, `/dispatch`, and a
CLI-bridge coordinator all see the same state.

```
forge dispatch start "<request>"   [--cwd <dir>] [--no-worktree]
                                    [--mode default|accept-edits|bypass]
                                    [--parallel N] [--max-items N] [--model <id>]
forge dispatch list
forge dispatch show <id>            # id or unique prefix
forge dispatch approve <id> [--only 1,3]
forge dispatch revise <id> "<what to change>"
forge dispatch cancel <id>
forge dispatch merge <id>           # every finished item, one commit each
```

- `--cwd` — project directory (default: the current directory).
- `--no-worktree` — run every item in `--cwd` itself instead of a worktree per item; required
  outside a git repository.
- `--mode` — permission mode of the **item** sessions (default `accept-edits`); the coordinator
  itself always runs in `default` mode regardless of this flag, so any edit it attempts anyway
  reaches the user as a question.
- `--parallel` — `max_running` (1-8, default 4).
- `--max-items` — `max_items` (1-12, default 8).
- `--model` — pins the coordinator's model only; item sessions route as usual.
- `--only 1,3` on `approve` — approve just those item numbers; an item that depends on one left
  out does not start either (§4).
- `merge` — calls `POST /api/dispatches/{id}/merge` (§7) and prints one line per merged item with
  its commit's short sha (or `nothing to commit`), then the stop reason, conflicted files and the
  items not merged yet, if the run stopped.
- `--url`/`--token` (global on `forge dispatch`) — daemon base URL / token, same defaults as
  `forge attach`.

`forge dispatch show` prints the request, the proposed or running split, and — while `proposed` —
the exact next commands (`approve`/`approve --only`/`revise`/`cancel`) for that dispatch id.

**`/dispatch <request>`** in chat starts a dispatch for the session's own working directory and
prints one line: the dispatch id and a pointer to `forge board` to watch and approve it. It never
returns an error to the transcript — an unreachable daemon or a validation failure both come back
as one explanatory note.

## 9. Direct vs. CLI-bridge coordinators

A coordinator hosted directly by `forge serve` calls the daemon's store in-process
(`DaemonSessionDispatch`, `crates/forge-cli/src/cli/commands/run/driver/daemon_dispatch.rs`) —
but it calls exactly the same `record_proposal` function `POST /api/dispatches/{id}/proposal`
uses, so the two paths cannot validate a plan differently.

A coordinator whose model runs on a CLI bridge (`claude-cli::…`, `codex::…`) executes its tools in
a separate `forge mcp-serve` process (`crates/forge-cli/src/mcp_serve/dispatch.rs`). That process
has no presenter to show the user anything — a blocking question inside the tool call could never
reach them. This is why approval is never a question asked from inside `dispatch_sessions`: the
tool call only records the proposal (over HTTP, on the daemon's own route) and returns
immediately; the model is told to end its turn and wait for a `[dispatch]` message. Recording and
ending the turn works identically whether the coordinator is direct or bridged, survives the board
being closed, and survives a daemon restart (the proposal lives in the store, not in memory).

The `dispatch_sessions` tool is advertised to a bridge session only while it coordinates a
dispatch still open to a proposal (`planning` or `proposed`), discovered by asking the daemon
which dispatch this session id coordinates. An ordinary bridge session or a dispatched worker
never sees the tool, so nothing but a designated coordinator can start a dispatch.

## 10. HTTP API

All routes live under `/<daemon-token>/`; a wrong token is a 404; errors are `{"error": "..."}`.

| Route | Body | Result |
|---|---|---|
| `POST /api/dispatch` | `{prompt, cwd?, worktree?, mode?, max_running?, max_items?, model?}` | 200 `{dispatch_id, coordinator_session_id, title}`; 400 on validation; 503 while draining |
| `GET /api/dispatches?limit=N` | — | 200 `[DispatchJson]`, most recently updated first (default 20, clamp 1-100) |
| `GET /api/dispatches/{id}` | — | 200 `DispatchJson`; 404 |
| `POST /api/dispatches/{id}/proposal` | `{summary, items: [{title, prompt, depends_on?}]}` | 200 `DispatchJson`; 400; 409 unless `planning`/`proposed` |
| `POST /api/dispatches/{id}/approve` | `{selected?: [int]}` (omitted = all) | 200 `DispatchJson`; 400 on an invalid selection; 409 unless `proposed` |
| `POST /api/dispatches/{id}/revise` | `{feedback}` | 200 `DispatchJson`; 400 if empty; 409 unless `proposed` |
| `POST /api/dispatches/{id}/cancel` | — | 200 `DispatchJson`; 409 if already `done`/`cancelled` |
| `POST /api/dispatches/{id}/merge` | — | 200 `MergeReport` (also for a partial run); 400 if `worktree` is false; 404; 409 if no item is `succeeded` with a session, or with `dirty_files` if the base repo has uncommitted tracked changes |

`MergeReport` (§7):

```
{ "merged": [ { "index", "title", "commit": sha|null } ],
  "stopped_at": { "index", "title", "reason", "conflicts": [file] } | null,
  "remaining": [index],            // succeeded items after the stop, not attempted
  "base_branch": string|null,      // the branch the commits landed on
  "dispatch": DispatchJson }
```

`prompt` is required, non-empty after trim, capped at 16 KB. `cwd` defaults to the daemon's
default cwd and is canonicalized; with `worktree: true` (the default) it must be a git repository
or the route 400s naming the fix. `mode` (worker permission mode) accepts `default`,
`accept-edits` (default), or `bypass`. `max_running` clamps to 1-8 (default 4), `max_items` to
1-12 (default 8).

`DispatchJson`:

```
{ "id", "coordinator_session_id", "coordinator_title", "cwd", "prompt", "summary", "status",
  "worktree": bool, "permission_mode": string|null, "max_running", "max_items",
  "created_at", "updated_at",
  "items": [ { "index" (1-based), "title", "prompt", "depends_on": [int], "status",
               "session_id": string|null, "outcome": string|null,
               "started_at": int|null, "finished_at": int|null } ] }
```

Additive fields elsewhere: `GET /api/sessions` rows gain `dispatch_id`, `dispatch_role`
(`"coordinator"|"worker"|null`), `dispatch_index` (1-based, workers only). `WS /ws/fleet` emits
`fleet_changed` on every dispatch state change. A session's `WS /ws?session=` snapshot gains
`turns_finished: u64`, incremented once per finished turn — how the supervisor (§5, §6) notices a
worker's turn ended even when it is shorter than a poll interval.

## 11. The store

Migration 35 (`crates/forge-store/src/migrations/m0035_dispatch.rs`) adds two tables:

- **`dispatch`** — one row per coordinator: `id`, `coordinator_session_id`, `cwd`, `prompt`,
  `summary`, `status` (`planning|proposed|running|done|cancelled`), `worktree`, `permission_mode`,
  `max_running`, `max_items`, `created_at`, `updated_at`.
- **`dispatch_item`** — one row per proposed item, keyed by `(dispatch_id, idx)`: `title`,
  `prompt`, `depends_on` (JSON array of 1-based indices), `status`
  (`proposed|skipped|queued|running|succeeded|failed|stopped|cancelled|merged|discarded`),
  `session_id`, `outcome`, `started_at`, `finished_at`.

`replace_dispatch_proposal` refuses to change the items once the dispatch has moved past
`planning`/`proposed` — an approved split is frozen. Both tables outlive the board closing and the
daemon restarting; `Store::active_dispatches()` is what the daemon reloads on startup (§12).

## 12. Limits and known gaps

- **A daemon restart can lose track of a turn that finished right before it.** The supervisor's
  per-item turn counter (`turns_finished`) is in-memory only; on restart every driver's counter
  starts again at 0. If a worker's turn had already finished in the moments just before the
  restart, the resumed driver comes back idle at counter 0, and nothing increments it again until
  the session is prompted anew — the item can be left showing `running` indefinitely.
- **Finished dispatches are only re-watched while among the 50 most recently updated
  dispatches** (`RECENT_DISPATCHES` in `crates/forge-cli/src/serve_dispatch/supervisor.rs`). A
  worker from an older, already-`done` dispatch that the user prompts again will not have that
  turn reflected back into the item's status once it ages out of that window.
- **`GET /api/sessions` does two store lookups per row** — `dispatch_for_coordinator` then, on a
  miss, `dispatch_item_for_session` — to fill in the additive dispatch fields for every session,
  every call.
- A coordinator that a daemon restart resumes while its dispatch is still `planning`/`proposed`/
  `running` is re-wired as coordinator automatically; one that finished coordinating is not
  (there is no dispatch left to revise or approve).
- `max_items` hard-caps at 12 and `max_running` at 8 regardless of what a request asks for — each
  item is a whole agent session, so these are deliberate ceilings, not tuning knobs.
- An item's forwarded final reply to the coordinator is truncated to 2000 characters
  (`REPORT_MAX_CHARS`); the full reply is still on the item's own session transcript.

## 13. Testing offline

`forge serve --local --mock` runs the daemon against the offline deterministic mock provider — no
API keys, no network. Point it at an isolated store so it never touches your real session history:

```sh
export FORGE_DB=/tmp/dispatch-test.db
forge serve --local --mock &
forge dispatch start "mock:dispatch split the work"
forge dispatch list
forge dispatch show <id>
forge dispatch approve <id>
forge dispatch show <id>      # watch items move to succeeded/merged
```

The mock provider keys on the literal text `mock:dispatch` in the latest user message: a
coordinator turn that sees it (and has the `dispatch_sessions` tool) proposes a fixed three-item
split — `Notes file` (`mock:write create a file notes.md`), `Tasks` (`mock:tasks track the work`),
and `Summary` (depends on both, replies with a one-line project summary) — then, once it sees the
tool's own result, replies "The split is ready for your review." and stops. Item prompts also key
on `mock:` prefixes (`mock:write`, `mock:tasks`) so each worker's mock turn does something
observable without a real model. `forge board --mock` is not a thing; run the board against the
same `FORGE_DB`/daemon instead — `forge board` (it talks to whatever daemon `forge attach`'s
discovery finds).
