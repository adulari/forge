# Feature: expandable tool cards in the chat transcript

> **Forgeline update.** Cards now use one grammar in both display modes, sit on the turn rail,
> carry their own diff, and fold repeated reads. Section 0 is current; sections 1-5 describe the
> mechanics (expansion, hit-testing, full output), which are unchanged except where section 0
> says otherwise.

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
Ctrl+T show`. `Ctrl+T` opens the newest card that has a drawer, a folded turn included, so a tool
card that is newer always wins.

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
  is the path for terminals without mouse reporting.
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
