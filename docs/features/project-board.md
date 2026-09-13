# Feature: The project board — `forge board`

## 1. Why a board instead of a chat

A chat window shows you one session. The moment you are running more than one — a fix in one
worktree, a refactor in another, an overnight `/goal` run, a duel — "is everything okay" stops
being answerable from any single terminal. You end up doing what the fleet always used to
require: `forge sessions`, `journalctl`, attaching to five tmux panes in turn, or grepping the
sqlite store to work out which session actually needs you.

`forge board` is a full-screen, live overview of every session on a project, laid out like a
kanban board — **Needs you · Working · Ready · Done** — so the question "what needs me" has one
answer instead of five terminals. Every card says which agent is doing what: model, current
task, last line, cost, context fill, and the signals that used to require digging — a permission
prompt sitting unanswered, a turn gone quiet, a model repeating its own opening sentence, a pin
that routes to a model Forge has already benched.

That last one is the board's motivating incident. On 2026-09-09, session `07ca114e` was pinned
to `meta::muse-spark-1.3-contributor` after that route started returning HTTP-success replies
with no text, no tool call, and zero tokens billed. Twelve consecutive turns died on it while
`forge models` had been printing that exact id as `benched` the whole time — nothing in the
turn's own error ("model returned an empty response … stopping the turn") named the pin or the
bench (`crates/forge-core/src/model_health_notice.rs`). The same session's stall guard
(`crates/forge-core/src/stall_guard.rs`) exists because a different live session spent 23 steps
opening every reply with the same sentence while quietly reading a different file each time —
structurally a stall, but invisible to a chat transcript scrolling past at normal speed. And a
task whose context a compaction had eaten kept re-driving a session for days
(`docs/features/stalled-tasks.md`) before anyone noticed the task itself was the problem, not the
model. All three are now signals a card shows without anyone going looking.

## 2. What it looks like

```
 FORGE BOARD   ● live    forge ▾ (project)    14 sessions · 2 need you · 2 working    $6.14 spent
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
  Needs you (2)                 Working (2)                   Ready (2)                    Done (30)
  ╭──────────────────────────╮  ╭──────────────────────────╮  ╭──────────────────────────╮  ╭──────────────────────────╮
  │ ● fix-retry-backoff      │  │ ⠹ migrate-schema-v33     │  │ ○ tui-header-cleanup     │  │ ✓ add-push-notifs        │
  │   meta::muse-spark-1.3   │  │   claude-cli::sonnet-5   │  │   groq::llama-3.3-70b    │  │   opencode::luna         │
  │   ▸ Re-pin the model     │  │   ▸ Backfill user_id     │  │   ▸ (no task open)       │  │   14 msgs · $0.42        │
  │   "…stopping the turn."  │  │   "3 rows migrated…"     │  │   "PR #1361 opened."     │  │   finished 2h ago        │
  │   ▰▰▰▰▰▰▰▱▱▱ 71% ctx     │  │   ▰▰▱▱▱▱▱▱▱▱ 23% ctx     │  │   ▰▱▱▱▱▱▱▱▱▱  4% ctx     │  │                          │
  │   ⚠ pinned model benched │  │   ⚠ quiet for 3m         │  │                          │  │                          │
  ╰──────────────────────────╯  ╰──────────────────────────╯  ╰──────────────────────────╯  ╰──────────────────────────╯
  ╭──────────────────────────╮                                 ╭──────────────────────────╮  ╭──────────────────────────╮
  │ ● board-render-tests     │                                 │ ○ quickstart-rewrite     │  │ ✗ dead-pin-repro         │
  │   waiting on a question  │                                 │   opencode::terra        │  │   meta::muse-spark-1.2   │
  │   ▸ pick option 1-3      │                                 │   ▸ (no task open)       │  │   stopped: no output     │
  │   "Which naming: A/B/C?" │                                 │   "docs/quickstart.md…"  │  │   47 msgs · $0.91        │
  ╰──────────────────────────╯                                 │   ▰▱▱▱▱▱▱▱▱▱  6% ctx     │  ╰──────────────────────────╯
                                                                 ╰──────────────────────────╯
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
 ↑↓ move  ←→ column  Enter open  a attach  p prompt  s steer  y/n allow  1-9 answer  i interrupt  m model  ? help  q quit
```

No emoji, rounded card corners, a braille spinner (`⠋⠙⠹⠸⠼⠴⠦⠧`, the same frames `app.rs`'s
`SPINNER` already animates the statusline with) on the one row genuinely streaming, and a
`▰▱` gauge for context fill. Everything on a card is a real field off `Card` (§4) — nothing here
is decorative.

## 3. Columns

Column membership is a pure function of a session's `Health` (`live_card` in
`crates/forge-tui/src/board/model.rs`):

| Column | Condition |
|---|---|
| **Needs you** | `Health::Waiting` (a permission prompt or question is pending), `Health::Stalled` (busy and at least one `SignalLevel::Danger` signal is firing), or `Health::Failed` (idle, not waiting, and the last turn's outcome was `"failed"`). |
| **Working** | `Health::Busy` — a turn is currently running and none of the above apply. |
| **Ready** | `Health::Idle` — idle and healthy: finished its last turn cleanly, or has never started one. |
| **Done** | A persisted, not-running session (`past_card`) — resumable, not part of the live fleet. |

Within a column, cards sort by severity first (waiting beats stalled beats failed beats busy
beats idle; a danger signal outranks a warn signal at the same health) and by most-recent
activity as the tiebreak (`severity` in `state.rs`). Needs-you-first is deliberate: it is the
same ordering `docs/features/remote-control.md` §2e gives the fleet dashboard's phone view, so
the "what needs me" answer does not change surface to surface.

## 4. Cards — every field and where it comes from

A live card merges the fleet row (`GET /api/sessions`, always present) with the session's latest
WebSocket snapshot (`LiveSnapshot`, present once its `WS /ws?session=<id>` has delivered a
frame — until then the fleet row's own fields are the fallback). A past card (`Done`) comes only
from `GET /api/sessions/past`.

| Field | Source |
|---|---|
| `title` / `display_title()` | Snapshot title, else the fleet row's; empty → `session <8-char id>`. |
| `cwd` / `project()` | Fleet row `cwd`; `project()` is its last path component. |
| `worktree` | Fleet row `worktree` (`.forge/worktrees/<id>` when the session runs in one). |
| `model` / `tier` | Snapshot `model`/`tier` when live, else the fleet row's model; rendered via `model_short` (`provider::model` → `model`; a bare bridge id → the provider name). |
| `column` / `health` / `signals` | Derived — see §3 and §5. |
| `current_task` | The snapshot's `in_progress` task, else its first `pending` one. |
| `tasks_done` / `tasks_total` | Counts over the snapshot's task list. |
| `last_line` / `streaming` | The streaming edge's trailing ~160 chars while a reply is in flight, else the newest non-`system`, non-empty transcript row. |
| `cost_usd` | Snapshot `cost_usd`, else the fleet row's; formatted by `fmt_cost` (`$0`, `$0.0042`, `$1.23`, `$12.3`). |
| `context_pct` | `tokens * 100 / limit`, capped at 100; `None` when the limit is unknown (no fabricated denominator, matching the statusline's own gauge). |
| `last_activity` / `created_at` | Fleet row timestamps; `fmt_age` renders them coarsely (`43s`, `12m`, `3h 5m`, `2d`). |
| `subagents` | Snapshot subagent list (id, agent, task, model, phase, last line, done, ok, cost). |
| `queued` | Count of prompts queued behind a busy turn. |
| `busy` / `waiting` | Snapshot values when live, else the fleet row's; `waiting` is true when a permission prompt or a question is pending. |
| `read_only` | The fleet row has no input path at all — a terminal session too old to run the control channel, or `[remote] interactive_local_sessions = false`. |
| `terminal` | The session runs in a terminal, not hosted by this daemon — archive/mode are unavailable for it (remote-control.md §2e). |
| `past` / `archived` / `message_count` | Set only on a `Done` card, from the past row. |

## 5. Signals

Every signal in `live_signals` (`model.rs`) is something that used to require reading the
journal, the sqlite store, or a tmux pane by hand. Signals are sorted most-severe-first and
deduplicated; a card's strongest signal is what drives its `Stalled` classification.

| Level | Text | Trigger | What to do |
|---|---|---|---|
| Danger | `waiting on a permission` / `…question` / `…decision` | A permission prompt or question is pending. | Open the card; `y`/`n` or `1-9` or `e` to answer. |
| Danger | `silent for <age>` | Busy, not waiting, no activity for ≥ `STALL_AFTER_SECS` (600s / 10m). | Check the Live tab; `i` to interrupt if it looks stuck. |
| Warn | `quiet for <age>` | Busy, not waiting, no activity for ≥ `QUIET_AFTER_SECS` (180s / 3m) but under 600s. | Worth a glance, not yet urgent. |
| Danger | `repeating the same opening ×n` | The last ≥ `REPEAT_OPENING_ROWS` (3) assistant rows open with the same first sentence. | The stall guard (`stall_guard.rs`) already nudges then halts the turn; if it persists, interrupt and re-prompt with the missing context. |
| Danger | `pinned model is benched` | A recent system row mentions both "benched" and "pinned". | Press `m` to re-pin (empty clears the pin) or clear it from the CLI; see `docs/features/mesh-routing.md` §9, "A pin whose model is dead". |
| Danger | `model returned empty responses` | A recent system row mentions "empty response". | Usually the same dead-pin situation as above. |
| Danger | `stall guard fired` | A recent system row mentions "same sentence" or "stopping to avoid a loop". | The harness already ended the turn; read the Live tab and re-prompt with what it needs. |
| Warn | `a task has stalled` | A recent system row mentions "not moved for" or "stalled task". | See `docs/features/stalled-tasks.md`; open Tasks to see which one. |
| Warn | `stopped: <reason>` | Idle, not waiting, last turn's outcome was `"failed"`. `stop_reason_words` renders `StopReason` (`no output`, `hit the step cap`, `budget exhausted`, `interrupted`, …). | Open Overview to see the reason, then re-prompt. |
| Warn | `n recent tool failures` | ≥ 3 of the last 12 tool rows failed. | Open the Tools tab. |
| Info | `n queued` | Prompts are queued behind the current turn. | Informational — they deliver at the next turn boundary. |
| Info | `workflow running` | A `/workflow` script is active. | Open Overview or Live for its phases/log. |
| Info | `plan awaiting approval` | A `/plan` proposal is pending and the session is waiting. | It is a question underneath — answer it like any other (`1` = Build it, `2` = Cancel, free text = revise). |
| Warn | `context <pct>% full` | Context fill ≥ `CONTEXT_WARN_PCT` (80%). | Consider `/compact` or a fresh session soon. |
| Info | `read-only (no input path)` | The fleet row has no input path. | View-only card; nothing here can be sent to it. |
| Info | `runs in a terminal` | The session runs in a terminal rather than under this daemon (only shown when not also read-only). | Archive and mode changes are unavailable — no driver task owns its lifecycle. |

## 6. The detail pane

Opening a card (`Enter`/`o`, or clicking it twice) shows five tabs (`DetailTab`):

- **Overview** — title, cwd/worktree, model/tier, permission mode, temper, effort, cost, the
  context gauge, the last turn's outcome/stop reason, the plan card and workflow card when
  present, and the subagent list.
- **Live** (`Tail`) — the transcript tail, following the newest line until you scroll up (`F`
  jumps back to following). `t` toggles whether tool rows are shown alongside assistant/user
  rows.
- **Tasks** — the task list (title, status, assignee) and the `tasks_done`/`tasks_total`
  progress the card summarizes.
- **Changes** — git status for the session's worktree (`GET /api/git/status`: branch, base
  branch, staged/unstaged/untracked counts) plus the structured diff card the snapshot carries
  (`Snapshot.diff`, reused byte-for-byte from the remote page — see remote-control.md §2e):
  the one pending proposed change while a permission prompt is armed, otherwise the latest
  landed turn's diff.
- **Tools** — recent tool calls from `GET /api/history?include_tools=1` (call/result pairs,
  ok/failed).

When a permission prompt or question is pending, the pane opens with an attention block above
the tabs: the prompt or question text, its options (label + description) when it is a question,
and the answer affordances (`y`/`n`, `1`-`9`, or `e` for free text). Every answer echoes the
`prompt_seq` the board last saw for that session (`answer_permission`/`answer_option`, and the
composer's `Answer { seq }` mode) — exactly the seq-check `docs/features/remote-control.md`'s
`POST /api/answer` uses, so a keypress aimed at a prompt that has already been superseded by a
fresh one can never mis-fire.

## 7. Actions

| Action | Key | What it sends |
|---|---|---|
| Attach | `a` | `BoardAction::Attach(id)` — leave the board, run `forge attach <id>` on the session (the daemon's single-writer terminal client; a second `forge chat` on the same session would be a second writer), return to the board when it exits. |
| Prompt | `p` | Composer → `RemoteInput` `{"kind":"prompt","text":…}` over the session's WS. |
| Steer | `s` | `{"kind":"steer","text":…}` — delivered at the next turn boundary, ahead of the queue. |
| Allow / deny | `y` / `n` | `{"kind":"allow","yes":…,"seq":…}`. |
| Answer an option | `1`-`9` | `{"kind":"answer","text":"<n>","seq":…}`. |
| Answer free text | `e` | Composer → `{"kind":"answer","text":…,"seq":…}`. |
| Re-pin the model | `m` | Composer prefilled with the current model; submits as a plain prompt, `/model <id>` (or bare `/model` to clear the pin) — the board has no dedicated re-pin route, it drives the same slash command a chat session would type. |
| Cycle the mode | `M` | `BoardAction::SetMode(id, next)` → `POST /api/sessions/{id}/mode`, cycling `default → accept-edits → bypass → plan`. |
| Interrupt | `i` | `BoardAction::Interrupt(id)` → `POST /api/sessions/{id}/interrupt`. |
| Archive | `x` | Confirms first, then `BoardAction::Archive(id)` → `POST /api/sessions/{id}/archive`. Refused for a past or `terminal` card. |
| Resume | `r` | `BoardAction::Resume(id)` → `POST /api/sessions {resume:id, cwd}`. Only on a `Done` card. |
| New session | `N` / `W` | Composer collects the first prompt, then `BoardAction::NewSession {cwd, worktree, prompt}` → `POST /api/sessions {cwd, worktree, title?}`, prompt sent once it streams. `W` starts it in a fresh worktree. |
| Copy session id | `c` | `BoardAction::Copy(id)` — OSC 52 / system clipboard, the host's choice. |

Prompt/steer/model/answer/new-session all route through the same one-line composer
(`ComposerMode`); the label above it names which of these it is.

## 8. Keys

Generated from `HELP` in `crates/forge-tui/src/board/keys.rs` — the same table the in-app `?`
overlay and the footer keybar read, so none of the three can drift from what the keys actually
do.

| Key | Does |
|---|---|
| `↑↓ j k` | Move within the column |
| `←→ h l Tab` | Switch column |
| `Enter o` | Open the card (again: focus the pane) |
| `Esc` | Back — pane focus → board → close pane |
| `a` | Attach: drop into the session in this terminal |
| `p` | Send a prompt (queued while busy) |
| `s` | Steer: jump the queue at the next turn boundary |
| `y / n` | Allow / deny the pending permission |
| `1-9` | Pick an option of the pending question |
| `e` | Answer the pending question in free text |
| `i` | Interrupt the running turn |
| `m` | Re-pin the model (`/model`), empty clears |
| `M` | Cycle the mode: default → accept-edits → bypass → plan |
| `x` | Archive (asks first) |
| `r` | Resume a Done session |
| `N / W` | New session here / in a fresh worktree |
| `f` | Cycle the project filter |
| `/` | Filter cards by text |
| `[ ]` | Switch the pane's section |
| `t` | Show or hide tool rows in Live |
| `F` | Follow the live tail again |
| `c` | Copy the session id |
| `R` | Refresh now |
| `?` | This help |
| `q  Ctrl+C` | Quit — sessions keep running |

Not shown in the overlay but implemented: `Home`/`g` and `End`/`G` jump to the first/last card
in the column (or the top/bottom of the detail pane); `PageUp`/`PageDown` move 5 cards or scroll
10 lines. `Ctrl+C` always quits regardless of focus.

**Focus rules.** The composer, the confirm dialog, the help overlay, and the filter box each
take every key while open. Otherwise the board and the detail pane share the action keys
(`a p s i m M x r c y n 1-9`) and differ only in what `↑`/`↓`/`PgUp`/`PgDn` move — cards in a
column on the board, the scrollback of the open tab in the pane.

**Mouse.** Click a card to select it; click the already-selected card to open its pane. Click a
column header to move the cursor there (selecting its first card if nothing is selected in it).
Click a detail tab to switch to it and focus the pane; click the close control to close the
pane. Click the project-filter chip to cycle it, same as `f`. Click an action button to perform
it. The wheel scrolls the detail pane when it is open and hovered or focused, otherwise it moves
the selection within the column under the pointer.

## 9. Responsive layout

The board is designed for four columns side by side, but narrows gracefully:

| Width | Layout |
|---|---|
| ≥ 140 cols | Four columns, detail pane opens as a side split. |
| ≥ 110 cols | Four narrower columns, detail pane still splits (less headroom per card). |
| < 110 cols | One column at a time with a tab strip across the top (`←`/`→`/`Tab` switches which); the detail pane takes the full screen when open instead of splitting. |

`column_page` (`state.rs`) tracks which column is showing in the narrow layout and follows the
cursor column automatically.

## 10. How it works

**Daemon surface.** Everything the board uses is surface `forge attach` and the remote-control
PWA already use — no new endpoint:

- `GET /api/sessions` — the live fleet.
- `GET /api/sessions/past` — persisted sessions for the Done column (`PAST_LIMIT` 30 shown, the
  daemon can serve up to 200).
- `WS /ws?session=<id>` — one socket per watched live session, feeding `LiveSnapshot` frames.
- `WS /ws/fleet` — revision invalidations that trigger a fleet refetch.
- `GET /api/git/status` / `GET /api/history?include_tools=1` — fetched once per card the first
  time it is opened (`detail_actions_for`), and again on `R`.
- `POST /api/sessions/{id}/interrupt|archive|mode`, `POST /api/sessions` (new and resume) — the
  mutating routes behind the action keys in §7.

**Pure/adapter split (ADR-0004).** `crates/forge-tui/src/board/*` is the renderer-independent
half: it folds `BoardEvent`s in, turns key/mouse input into `BoardAction`s, and draws itself into
a ratatui frame — it never touches the network or the terminal directly. `forge-cli`'s board
host is the adapter: it polls/streams the daemon over the surface above, turns responses into
`BoardEvent`s, and carries out `BoardAction`s as HTTP calls or WS sends. The same split that lets
`forge-tui` serve both the interactive terminal and a headless renderer is what will let the
mobile and desktop apps render this same board later without re-deriving any of §3-§5's logic.

**Live update path.** A `WS /ws/fleet` invalidation triggers a fresh `GET /api/sessions` (and
`/api/sessions/past`) rather than the board maintaining its own delta logic. Each watched live
row (`watch_ids()` — anything with an input path that has not closed) gets its own
`WS /ws?session=<id>`, and every frame becomes one `BoardEvent::Snapshot`, dropped if its
`revision` is older than what the board already has. A `BoardEvent::Tick` roughly every 100 ms
advances spinner frames, signal ages (`fmt_age` is time-relative — "quiet for 4m" needs
recomputing even with no news), entrance/finish flash highlights, and toast expiry.

**What it does not do.** No new tracking and no new schema — the board is a read-only lens over
data the daemon already computes for the fleet dashboard, `forge attach`, and `forge sessions`.
Cards derive everything from rows already served; nothing the board does is persisted except
ordinary session mutations (prompt, mode, archive, …) through the same routes any other surface
uses. A `terminal: true` row cannot be archived or mode-changed — there is no driver task here
that owns its lifecycle, the same limitation `docs/features/remote-control.md` §2e documents for
that flag. The Done column only ever shows the most recent `PAST_LIMIT` (30) sessions, not the
full history.

## 11. Testing

- **`crates/forge-tui/src/board/tests.rs`** — TestBackend-free unit tests over the pure state
  machine: column/health classification (`live_card`), every signal in `live_signals` including
  the threshold constants, selection/navigation, the composer's submit-to-`BoardAction` mapping,
  and confirm/archive/mode-cycle behaviour. No terminal, no daemon.
- **Host tests (`forge-cli`)** — the network adapter's tests run the same `BoardEvent`/
  `BoardAction` contract against a mock daemon (the pattern the remote-control and fleet-message
  tests already use), so a route or wire-shape change is caught without a live server.
- **Manual check:**
  ```bash
  forge serve --local              # daemon on loopback
  forge run "..." --tui &          # or: forge chat, in another terminal
  forge board                      # or: forge board --all
  ```
  Confirm the new session appears in the right column within one fleet poll, that opening it
  fetches git status and history exactly once, and that a permission prompt raised by the
  `forge run` session shows up as `waiting` with working `y`/`n`. For headless verification of
  the rendered frame, drive it the way the other full-screen TUI docs do — `tmux` +
  `capture-pane` against a `forge board` session (see `docs/features/tui-rich-rendering.md` for
  the pattern this project already uses for TUI screenshots in CI-safe form).

## 12. Limits and open questions

- Signals are heuristics over a bounded transcript window (the last ~40 system rows, the last 12
  tool rows) — a real problem outside that window will not raise a signal until something inside
  it does.
- The "repeating the same opening" and "pinned model benched" signals key off substring matches
  on system-row text; a future rewording of those messages elsewhere in the harness needs a
  matching update here or the signal silently stops firing.
- A `terminal: true` row is visible and even promptable (it already runs the same protocol
  through the presence-heartbeat proxy — remote-control.md §2e), but cannot be archived or have
  its mode changed, because no daemon-owned driver exists for it to act on.
- The Done column caps at the 30 most recent past sessions; older ones exist and are resumable
  by id via `forge sessions`/`forge attach`, just not listed on the board.
- Re-pinning the model is a prompt-injected `/model` command, not a typed field — a session whose
  `/model` command has been renamed or removed by a future change would silently stop working
  from the board without a compile-time signal.
