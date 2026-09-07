# Feature: expandable tool cards in the chat transcript

> One transcript row per tool call, click (or `Ctrl+T`) to open it. Touches
> `forge-types` (`PresenterEvent::ToolResult.detail`), `forge-core` (`tool_detail`), `forge-cli`
> (`LiveEvent.detail`, the mouse/keyboard seam) and `forge-tui`
> (`app/tool_cards.rs`, the card lifecycle in `app.rs`).

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

`PresenterEvent::ToolResult` gained `detail: Option<String>` — a bounded slice of the raw result
(`forge_core::tool_detail`: 200 lines / 8000 characters, whichever comes first, with an explicit
truncation marker). Carrying the whole result instead would pin every megabyte of shell output in
the UI's line ring for the life of the session; carrying nothing is what the old rendering did.
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

## 4. Tests

`crates/forge-tui/src/app/tool_cards.rs` covers the rendering (one row collapsed, full arguments
and output when expanded, no triangle when there is nothing behind it, wrapping inside the row
width, cwd-relative paths, malformed args). `crates/forge-tui/src/app.rs` covers the lifecycle
(one row per finished call, click toggles in place, a later card stays clickable after an earlier
one grows, `Ctrl+T` without a mouse, same-frame completion, inline mode untouched, and a result
with no matching start still rendering). `forge-core` covers `tool_detail`'s bounds.
