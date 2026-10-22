# Feature: expandable tool cards in the chat transcript

> **Forgeline update.** Cards now use one grammar in both display modes, sit on the turn rail,
> carry their own diff, and fold repeated reads. Section 0 is current; sections 1-5 describe the
> mechanics (expansion, hit-testing, full output), which are unchanged except where section 0
> says otherwise.
>
> **Tool groups update.** In full-screen, the cards of a run of calls fold into one summary row
> (section 0a). An open group shows the cards exactly as below.

## 0. Forgeline card grammar

```
 ▏ ✓ Write  mock-note.txt                                       new file · +1 · 4ms ▸
 ▏   ┊ 1 + deterministic mock output
 ▏ ✓ Edit   src/upload.rs                                              +18 −3 · 0.2s ▸
 ▏   ┊ ┄┄ @@ -38,7 +38,9 @@ ┄┄
 ▏   ┊ 38   let req = build(chunk);
 ▏   ┊ 39 − let r = send(req)?;
 ▏   ┊ 39 + let r = retry(3, || send(req.clone()))?;
 ▏   ┊ … 14 more lines · Ctrl+T expand
 ▏ ✗ Run    cargo test -p upload                                      exit 101 · 41s ▸
 ▏   ┊ test retry::gives_up_after_three ... FAILED
 ▏ ◆ Read   4 files                                                          0.2s ▸
```

- **Status glyph, verb, target, outcome, duration.** The glyph is a braille spinner while the call
  runs (animated at draw time over the baked row), `◇` while its permission prompt is open (the
  right-hand text reads `awaiting approval`), `✓` / `✗` when it ends and `◆` for a folded group.
  Verbs are humanised: `read_file` Read, `write_file` Write, `edit_file` Edit, `shell` Run,
  `grep` Search, `glob` Find, `web_fetch` Fetch, `web_search` Web, `spawn_agents` Agents; an unknown
  tool shows its own name. The target is decoded from the call's JSON (`headline`); raw JSON is
  never shown, in either mode. The card measures its own duration, so a trailing ` in 132ms` in a
  tool's summary is dropped.
- **Diffs live in the card.** `PresenterEvent::Diff` attaches to the running call (it arrives after
  `ToolStart`, before the permission gate). The collapsed card shows the stat (`+18 −3`, `new file
  · +1`) and up to eight rows - a line-number gutter, a coloured sigil, a full-row tint where the
  terminal paints backgrounds and a brighter tint on the changed words of a rewritten line - then
  `… N more lines · Ctrl+T expand`. The separate `✎ <absolute path>` header is gone; the card shows
  the path relative to the call's `cwd`. `render::diff_to_plain` stays ANSI-free unified text for
  remote and export.
- **Failures explain themselves.** A failed collapsed card prints the last three non-blank lines of
  its output under the header.
- **Reads fold.** A read that directly follows a finished read joins it: `◆ Read 4 files`. Expanding
  lists each file with its outcome and time. Any other row in between ends the run.
- **Inline prints once.** `--inline` cannot rewrite scrollback, so a call is held back while it runs
  (the composer border shows `Running Write mock-note.txt`) and the finished card, with its short
  diff, is printed once when the result lands. It has no triangle and no `Ctrl+T` hint, because it
  cannot open. Full-screen keeps clicking and `Ctrl+T`.
- **Remote.** A running row ends in `running`, which `remote_transcript_text` strips (the finished
  row arrives as its own line); the rail and the disclosure triangle never reach the phone.

The same machinery folds long **user turns** (`UserFold`): a canned command prompt (`/init`)
becomes one labelled row, a paste longer than six rows keeps its first six and `… N more lines ·
Ctrl+T show` (full-screen only: inline and the daemon driver show a generic message in full, because
the scrollback scrolls natively and a phone reads the same rows). The remote transcript gets the
hidden rows after the visible ones and never the fold hint. `Ctrl+T` opens the newest card that has a drawer, a folded turn included, so a tool
card that is newer always wins.

## 0a. Tool groups (full-screen)

```
 ▏ I'll get my bearings in the project first.
 ▏ ▸ Listed 1 directory, searched for 2 patterns, read 1 file, ran 2 shell commands  1 failed · 3.3s
 ▏   ✗ Run    echo 'probe: missing fixture' >&2; exit 3                               exit 3 · 18ms
 ▏ The checks pass apart from one probe. Writing the summary now.
 ▏ ⠧ Created 1 file (+13), edited 1 file (+1 −1)
 ▏   ⠧ Run    wc -l mock-summary.md                                                         running
 ▏ Done.
 ▏ ▸ Read src/big.rs, ran cargo check  click or Alt+O to expand                             1.2s
```

A **group** is a maximal run of tool calls inside one turn with no other row between them: a card
joins the open group when the group's last card is still the newest row of the log as the call
starts. Assistant prose ends a group, and so does anything else that lands in the log (a notice, a
shell diagnosis, a new turn). Subagent and task calls are calls like any other and get their own
words.

- **Collapsed** (the default), a group is one dim row: a `▸`, the summary, and on the right the
  failures and the total duration. The summary counts the calls that succeeded by category in the
  order they first appeared, in plain English with plurals: `Read 3 files, searched for 2 patterns,
  ran 4 shell commands`, `Listed 1 directory, ran 2 shell commands`, `Edited 2 files (+41 −7)`.
  Files, directories, patterns and pages are counted by what was touched, not by call: three
  `read_file` slices of `src/big.rs` and three edits to it are `Read 1 file, edited 1 file (+12 −3)`
  (`./src/a.rs` and `src/a.rs` are one file; a read card that folded several reads counts its
  distinct paths). Shell commands, scripts, agents and other tools count calls. Created, edited and
  deleted files are separate categories, each with its own `(+a −d)`, so a deletion is never folded
  into "edited". A group that touched one thing names it: `Ran cargo test -p upload`,
  `Read Cargo.toml`, `Read src/big.rs` (however many slices), `Created notes.md (+2)`.
  Categories: read, listed, searched for (grep, glob, search), ran shell commands, ran scripts
  (`run_code`), created / edited / deleted files, fetched pages, ran web searches, ran agents
  (`spawn_agents` counts its agents), updated the task list, and `called <server|tool> N times` for
  MCP and anything else.
- **Failures stay visible, and are never counted as done.** A failed call is left out of the
  sentence and adds nothing to the `(+a −d)`: a failed `edit_file` next to a no-op write reads
  `Edited notes.md` with `1 failed` beside it, not `Edited 2 files`. The row says `N failed` in the
  error colour on the right, which is the part a narrow terminal never cuts (a focused row keeps it
  too), and each failed call (the newest three) stays listed under the summary as its one-line card
  header. A group where nothing succeeded says what it tried: `Tried to edit sample.rs`,
  `Tried to run 2 shell commands`. Why: a failed call is the one thing a reader needs without
  asking (the agent may have moved on without fixing it), and its header costs one row. A
  destructive call is not hidden either: a deletion is its own category, and the permission
  prompt, which is where a destructive shell command is decided, is never folded.
- **How to open it.** The newest group, while folded and finished, ends its summary with a faint
  `click or Alt+O to expand` (the chord is whatever `toggle_tool_groups` is bound to), the way
  Claude Code's collapsed line says `(ctrl+o to expand)`. Older rows stay quiet, and the hint is
  dropped rather than cut when the row has no room for it.
- **Narrow terminals.** A summary that does not fit its row switches to counts before anything is
  cut: `1 dir, 2 searches, 1 file read, 2 commands` instead of the sentence, and below 80 columns the
  duration gives up its cells. Card headers inside an open group keep their tool and target: a
  right-hand preview too long for the row is cut before the left side drops below half the row
  (`copy_mark::fit_right`).
- **Running.** While any call in the group runs, the summary row shows the spinner instead of the
  `▸` and lists what has finished; the call in flight hangs under it with its own spinner (the
  first call of a group shows alone until it finishes). A call waiting for permission also shows
  its diff preview under it, because that is what the decision is about. When the last call ends,
  the group folds to its single row.
- **Open.** A click anywhere on a collapsed group, or on an open group's `▾` header, toggles it.
  The cards of an open group hang two cells under the header and are today's cards: clicking one
  still opens its arguments and output. The toggled row stays on the same screen row, so the
  content under the pointer does not jump; following resumes only if that leaves the view at the
  bottom.
- **Keys.** `Alt+↑` / `Alt+↓` (`prev_tool_group` / `next_tool_group`) put the keyboard focus on a
  group, starting from the ones on screen, and scroll it into view; the focused row is raised and
  its right side reads `Enter expand · ↑↓ move · Esc done`. While a group has focus, `↑`/`↓` move,
  `Enter` or `Space` toggle and `Esc` lets go; any other key lets go and is handled as usual, and a
  permission prompt or question takes the keys back. `Alt+O` (`toggle_tool_groups`) opens every
  group, including groups that arrive later, or folds them all; the row at the top of the view
  stays put. `Ctrl+T` on a folded group opens it, then opens its newest call, then folds the group
  again.
- **State.** Which groups are open is kept by group id (the id of the group's first card), so it
  survives scrolling, resizing and the log growing. `/clear` and a session switch forget it.
- **Off switch.** `[tui] fold_tool_groups = false` (also in `/config`, applied immediately) shows
  every call as its own card, exactly as before.
- **Inline mode** does not fold: native scrollback cannot reopen a row, so a folded group there
  would hide its calls for good.

**How it works.** `main_log` keeps every card exactly as it renders unfolded; nothing about the
log, the remote transcript or the activity viewer changes. The fold happens where the log is
wrapped for the transcript (`ensure_wrapped_main` -> `App::wrap_folded`, `app/group_flow.rs`):
each group's line range is replaced by its summary rows (collapsed) or prefixed with the header
(open). The wrap is memoised on the log revision, the width and a fold key (fold enabled, a
revision bumped by every toggle, and whether a permission prompt is open), so folding rides the
pass over the log the wrap already makes on every change. That pass runs on every committed
streamed line, so nothing in it may cost more than the group's rows:

- Each group's `Summary` is memoised in `FoldUi::summary`, keyed on the group id and checked against
  its cards' `(id, status, folded reads, has diff)`. A rewrap rebuilds only the group whose cards
  changed (in practice the running one), and groups that left the log are dropped from the memo.
- A card's `(+a −d)` is counted once, when its diff attaches (`ToolCard::set_diff` ->
  `diff_counts`). The diff holds both whole files, so counting it again per rewrap was a full-file
  diff per edit per streamed line: 40 edits to a 4000-line file cost 92 ms per rewrap in release
  before this, against 1.6 ms unfolded. Card headers read the same cached counts.

Only the rows that animate (a running summary's spinner, the running call) or have focus are
rebuilt at draw time, from a small sorted list of fold rows, and they reuse the memoised summary.
The summary text and rows are pure functions in `app/tool_groups.rs`.

> One transcript row per tool call, click (or `Ctrl+T`) to open it, and a full-screen viewer for
> everything it printed. Touches `forge-types` (`PresenterEvent::ToolResult.detail`,
> `PresenterEvent::ToolOutput`), `forge-tools` (`Tool::run_full`), `forge-store`
> (`Store::db_path`), `forge-core` (`tool_output.rs`), `forge-cli` (`LiveEvent`, the mouse/keyboard
> seam, `/output`) and `forge-tui` (`app/tool_cards.rs`, `app/output_view.rs`, the card lifecycle
> in `app.rs`).

## 1. Problem

A tool call printed **two** scrollback lines, emitted at different times:

```
  ↳ shell  {"command":"adb -s emulator-5554 shell dumpsys media_session 2>/dev/null | grep -i -A20 \"spot…
  ✓ shell  shell: exit 0 in 132ms
```

Three things were wrong with that. The call and its outcome are separate rows, so a reader pairs
them up by eye — and in a busy turn they are not even adjacent. The argument line is raw JSON,
escaped quotes and all, truncated exactly where the interesting tail lives. And the tool's actual
output never reached the screen at all: the presenter only ever received `summarize()`'s first
line, so nothing in the UI could show what a command printed.

## 2. Shape

One row owns both halves, with the outcome pushed to the right margin:

```
  ▸ shell  adb -s emulator-5554 shell dumpsys media_session | grep -i -A20 spotify   ✓ exit 0 in 132ms
```

Opening it reveals the arguments — decoded, one field per row, the identifying argument first —
and the output:

```
  ▾ shell  adb -s emulator-5554 shell dumpsys media_session | grep -i -A20 spotify   ✓ exit 0 in 132ms
     ┆ command  adb -s emulator-5554 shell dumpsys media_session 2>/dev/null
     ┆          | grep -i -A20 "spotify-media-session"
     ┆ cwd      ~/Documents/Repositories/Personal/AI/forge
     ┆ output
     ┆ Session 0: SpotifySessionService
     ┆ click or Ctrl+T to collapse
```

- **Click** anywhere on a card row to toggle it; the click is consumed, so no stray selection is
  left behind. **`Ctrl+T`** toggles the most recent card (rebindable as `toggle_tool_card`), which
  is the path for terminals without mouse reporting. With tool groups folding, `Ctrl+T` opens the
  newest group first (section 0a).
- A call with no arguments and no output renders `·` instead of `▸`: the affordance never claims
  a drawer that is empty.
- A running call shows `◍ running` until its result lands.
- `path` is shown relative to the call's own `cwd` when it sits under it, else with `$HOME`
  folded to `~`.

## 3. How it works

`PresenterEvent::ToolResult` gained `detail: Option<String>` — a bounded preview of the raw result
(`forge_core::tool_output::tool_detail`: the first 120 and last 60 lines within 8,000 characters,
the middle folded into a `… N lines hidden · full output: /output` marker). It keeps both ends
because a failing command prints its cause last: the first version kept only the head, which
dropped exactly the part a person opens a failing call to read. Carrying the whole result instead
would pin every megabyte of shell output in the UI's line ring for the life of the session;
carrying nothing is what the old rendering did. Section 4 is where the rest goes.
`LiveEvent::ToolResult` carries the same field (`#[serde(default)]`, so an older daemon's frames
still deserialize), which is what makes cards work in `forge attach` and daemon-hosted sessions.

In the TUI, a card's rendered lines live in `App::main_log` like any other scrollback, so
wrapping, scrolling, selection and copy keep working unchanged. Each card records the line range
it occupies; toggling re-renders it and splices the new lines over the old range, shifting the
cards after it by the change in height. Two positions have to be maintained carefully:

- **The log ring trims from the front** (`MAIN_LOG_MAX`). Every card's recorded start shifts, and
  a card whose own lines were trimmed is dropped rather than left pointing at someone else's rows.
- **A fast tool starts and finishes inside one frame**, so its card is still queued in `flush`
  when the result arrives and has no `main_log` position yet. The result then rewrites the QUEUED
  lines; splicing at the unset position inserted a duplicate row at the very top of the transcript
  (found in a live `tui-drive` run, regression-tested by
  `a_tool_that_finishes_inside_one_frame_updates_its_own_row`).

Click hit-testing needs the wrapped row a click lands on mapped back to a source line, so
`transcript::wrap_lines_indexed` returns that map and the wrap cache keeps it. A card row is
rendered two cells narrower than the area: the transcript reserves one column, and `wrap_lines`
breaks AT the wrap width, so a row filling it exactly would emit a second, empty row — which would
both look wrong and desynchronise hit-testing from the card's line range.

**Inline mode keeps the old two-line rendering.** `--inline` prints into the terminal's native
scrollback, which cannot be rewritten after the fact, so a card there would be a control that
never responds.

## 4. Full output

The preview is bounded on purpose, so a card also offers everything the call printed:

```
  ▾ shell  cargo test --workspace                     ✖ exit 101 in 41s · ⤢ 5,012 lines
     ┆ command  cargo test --workspace
     ┆ output
     ┆ running 812 tests
     ┆ … 4,830 lines hidden · full output: /output
     ┆ test result: FAILED. 811 passed; 1 failed
     ┆ ⤢ view full output  5,012 lines · 212 KB · click here or /output
     ┆ click or Ctrl+T to collapse
```

Clicking the `⤢` row, or `/output` for the latest call, opens the **full-output viewer**: a
full-screen takeover in the chat's own terminal (like the activity viewer, so a running turn keeps
streaming underneath) that pages the complete output with line numbers.

| Key | Does |
|---|---|
| `↑` `↓`, `j` `k`, wheel | scroll a line |
| `PgUp` `PgDn`, `u` `d`, space | scroll a page |
| `g` `G`, `Home` `End` | top / end |
| `/` then Enter | search; smartcase, so a capital letter makes it case-sensitive |
| `n` `N` | next / previous match |
| `w` | toggle wrapping; unwrapped, `←` `→` scroll sideways |
| `y` | copy the whole output |
| `o` | open the kept file in `$PAGER` (default `less -R`) |
| `Esc` `q` | close |

A successful call opens at the top and a failed one at the end, where its cause is. Inline mode has
no cards and no room for the viewer, so `/output` there hands the latest kept file to `$PAGER`.

**Where the full output comes from.** The model's copy is itself cut: the shell tool captures up to
1 MiB per stream but gives the model 64 KB, head and tail. `Tool::run_full` returns that copy plus
the uncut text when a cut happened (`ToolRun { model, full }`); every other tool keeps the default,
whose model copy is already the whole output. After a call's `ToolResult`, core writes the uncut
text (or the result itself, when it is longer than the preview) to
`<store dir>/tool-output/<session>/<stamp>-<seq>-<call id>.log` and emits
`PresenterEvent::ToolOutput { name, output }`, which the TUI attaches to the card that result just
closed.

- The file sits beside the session store (`Store::db_path`), so a test's `FORGE_DB` store keeps it
  out of the real data directory and an in-memory store writes nothing at all.
- Providers reuse call ids (some send `call_0` on every turn), so the file name carries a timestamp
  and a counter; the id alone would let a later call overwrite an earlier card's output.
- Kept outputs older than seven days are removed the first time a process keeps anything.
- `LiveEvent::ToolOutput` carries the reference to `forge attach` and daemon-hosted sessions. The
  path is only readable on the daemon's machine: a card whose file cannot be read opens its preview
  instead, labelled as such. A client too old to know the frame fails to decode it and skips it.
- A preview that stands in for more output than was kept is shown in the viewer as **preview
  only**, never as though it were the whole output.

## 5. Tests

`crates/forge-tui/src/app/tool_groups_tests.rs` covers the summary (aggregation in order of first
appearance, plurals, combined diff stats, deletions kept apart, single-call wording, distinct files
for chunked reads and repeated edits, failed calls left out of the sentence and the stat, the
"Tried to" wording, the cached stat, the short form and the hint on narrow rows, the failure count
surviving a narrow row and focus), `crates/forge-tui/src/app/group_flow_tests.rs` the fold
(membership, prose splitting groups, click and keyboard toggles keeping the row in place,
expansion surviving resize and scroll, the running and awaiting-permission rows, failures under the
summary, the off switch, Alt+O, Ctrl+T's cycle, a rewrap and a spinner frame rebuilding no settled
summary, the hint on the newest group only, card headers at 60 columns), and `crates/forge-cli/tests/tui_fold_e2e.rs` drives a real
full-screen `forge chat --mock` over a PTY, clicks a summary with an SGR mouse report and reads the
screen back (`--ignored`).
`crates/forge-tui/src/app/tool_cards.rs` covers the rendering (one row collapsed, full arguments
and output when expanded, no triangle when there is nothing behind it, wrapping inside the row
width, cwd-relative paths, malformed args, the view-full-output row and its line count, the diff
in the card, the failure tail, the read group, verbs and durations, folded user turns). `crates/forge-tui/src/app.rs`
covers the lifecycle (one row per finished call, click toggles in place, a later card stays
clickable after an earlier one grows, `Ctrl+T` without a mouse, same-frame completion, inline mode
untouched, and a result with no matching start still rendering). `crates/forge-tui/src/app/output_view.rs`
covers the viewer (opening position by outcome, reaching the last line of a long output, search and
match cycling, Esc cancelling a search before the viewer, copy and pager actions, sideways scroll,
preview-only labelling). `crates/forge-core/src/tool_output.rs` covers the preview's head-and-tail
bounds, one enormous line, reused call ids never overwriting a kept file, and path safety.
