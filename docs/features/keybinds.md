# Feature: configurable TUI and app keybindings

> Key names are spelled one way everywhere (`Ctrl+K`, `Alt+M`, `Shift+Tab`, `Ctrl+Shift+C`, `PgUp`):
> in Help, in hints, in lists and in the keybinding editor. See [tui-interaction.md](tui-interaction.md).

> **Status: shipped.** Every TUI action key is remappable via `[keybinds.binds]` in
> `config.toml`. The desktop/web app has its own searchable Settings → Keyboard shortcuts
> editor with live capture, conflict warnings, per-action reset, and reset-all. The two
> surfaces are intentionally separate because the host TUI and client app have different
> action lifecycles and persistence boundaries.

## 1. Problem (JTBD)

> When I drive Forge in the terminal, I want the keyboard shortcuts to match my muscle memory and
> not collide with my terminal/multiplexer — so I can rebind any action without recompiling.

## 2. Config surface

`[keybinds.binds]` maps an **action name** to a **key combo**. A combo is a `key` plus modifier
flags. `key` is a single character (`"c"`, `"k"`, `","`) or a named key: `up`, `down`, `left`,
`right`, `enter`, `esc`, `backspace`, `delete`, `pageup`, `pagedown`, `home`, `end`, `tab`,
`f1`–`f12`.

```toml
[keybinds.binds.toggle_reasoning]
key = "x"
ctrl = true
alt = false
shift = false
```

Overrides **deep-merge** over the built-in defaults — writing one bind never unbinds the others.

### Default binds

| Action | Default | Notes |
|---|---|---|
| `interrupt` | Ctrl-C | Stop current turn (in an overlay: close it). Idle with text: clear the input and save it to prompt history (↑ recalls it). Idle with empty input: press twice within 2 s to quit |
| — | Esc | Stop current turn (idle: press twice to open the rewind picker; never quits). With messages queued it then sends the first one now ([steering.md](steering.md)) |
| — | ↑ | While a turn runs, with an empty prompt and messages queued: pull the newest queued message back into the prompt to edit it |
| — | Ctrl-Home | Full-screen: jump to the most recent user message above the view; again = the one before (the "↑ Previous message" pill at the top does the same on click; it and the "Jump to bottom" pill only show while scrolled up) |
| — | ↑/↓ · 1–9 · Space · Enter · ←/→ · o · n · Esc | Inside an `ask_user` form: move · pick · toggle · confirm · switch question · other / note · dismiss ([docs](ask-user-question.md)) |
| `command_palette` | / | Open slash-command palette |
| `skip_model` | Ctrl-Shift-K | Mid-turn: abort so you can retry on another model |
| `command_center` | Ctrl-K | Open the command center |
| `tier_up` | Ctrl-↑ | Pin the next turn to the next higher mesh tier (mid-turn: abort and retry) |
| `tier_down` | Ctrl-↓ | Pin the next turn to the next lower mesh tier (mid-turn: abort and retry) |
| `toggle_reasoning` | Ctrl-R | Show/hide reasoning blocks inline |
| `toggle_tool_card` | Ctrl-T | Expand/collapse the most recent tool call (click a card for any other). With tool groups folding: opens the newest group, then its newest call, then folds it |
| `toggle_tool_groups` | Alt-O | Full-screen: open every tool group (new ones too), or fold them all ([tool groups](tui-tool-cards.md)) |
| `prev_tool_group` | Alt-↑ | Full-screen: focus the previous tool group; then ↑/↓ move, Enter or Space toggle, Esc lets go |
| `next_tool_group` | Alt-↓ | Full-screen: focus the next tool group (same keys as above) |
| `undo` | Alt-Z | Undo last file write (`/undo`). Was Ctrl-Z until v2.27, which is now the terminal's suspend chord |
| `suspend` | Ctrl-Z | Suspend Forge to the shell; `fg` returns with the screen restored (Unix) |
| `verbosity_cycle` | Alt-V | Cycle the transcript between quiet (reasoning hidden, tool runs folded, passing hints left out), default and everything (reasoning shown, every tool run open) |
| `compact` | Alt-L | Compact/summarize conversation (`/compact`). Was Ctrl-L until v2.21: a config that still binds it to Ctrl-L is moved to Alt-L on load while `redraw` keeps Ctrl-L |
| `redraw` | Ctrl-L | Repaint the screen, as in a shell or Claude Code (session and transcript untouched) |
| `model_picker` | Alt-M | Open model picker overlay |
| `effort_cycle` | Alt-E | Cycle effort low→medium→high→xhigh→unset |
| `temper_cycle` | Alt-T | Cycle temper/mode |
| `keybind_config` | Ctrl-, | Open this configurator |
| `new_session` | Ctrl-N | Start a fresh session (`/new`) |
| `copy_last` | Ctrl-Shift-C | Copy last response to clipboard (`/copy`) |
| `scroll_up` | PageUp | Scroll transcript up |
| `scroll_down` | PageDown | Scroll transcript down |
| `help` | F1 | Show keybind reference overlay |
| `checkpoint` | Ctrl-S | Save session checkpoint (`/checkpoint`) |
| `reload` | Alt-R | Reload keybinds, the tier override and the model catalog (hooks, statusline, MCP, mesh and skills need a restart) |
| `voice` | Ctrl-V | Open the `/voice` recording overlay |
| `kill_line_forward` | Alt-K | Delete from the cursor to the end of the line |

**Alt chords on macOS.** Alt-O, Alt-↑/↓ and the other Alt binds reach Forge only when the terminal
sends Option as Meta (Escape+key). Without that, Option+O types `ø` and Option+↑ may move by
paragraph instead. Turn it on in Terminal.app (Settings → Profiles → Keyboard → "Use Option as Meta
key"), iTerm2 (Profiles → Keys → Left/Right Option key → "Esc+"), Ghostty (`macos-option-as-alt =
true`), kitty (`macos_option_as_alt yes`), WezTerm (`send_composed_key_when_left_alt_is_pressed =
false`) or Alacritty (`option_as_alt = "Both"`), or rebind the action to a Ctrl chord. A tool group
can always be opened with a click.

### Presets: `preset = "claude-code"`

Some Forge defaults sit on chords a Claude Code user presses by habit and that mean something else
there: `Ctrl+K` (delete to end of line), `Ctrl+N` / `Ctrl+P` (history), `Ctrl+R` (history search),
`Ctrl+Z` (suspend), `Ctrl+S` (stash) and `Alt+P` / `Alt+M` / `Alt+T` (model, mode, thinking).
`Ctrl+L` already redraws in Forge, as in Claude Code. Several of ours are destructive when pressed by
habit (`Ctrl+K` runs the command center). One line moves them all:

```toml
[keybinds]
preset = "claude-code"
```

| Action | Forge default | `claude-code` |
|---|---|---|
| `command_center` | Ctrl-K | F2 |
| `new_session` | Ctrl-N | Alt-N |
| `checkpoint` | Ctrl-S | Alt-S |
| `toggle_reasoning` | Ctrl-R | Alt-T |
| `model_picker` | Alt-M | Alt-P |
| `temper_cycle` | Alt-T | Alt-M |

Every other action keeps its Forge default. The freed chords fall through to the composer's
readline keys (`Ctrl+K` cuts to the end of the line, `Ctrl+N` / `Ctrl+P` walk the prompt history,
see [tui-interaction.md](tui-interaction.md)); `Ctrl+S` and `Ctrl+R` do nothing
instead of typing a letter, and `Ctrl+Z` suspends as in Claude Code. A bind you set yourself under `[keybinds.binds]` always wins over the
preset, and the configurator's reset (`r` / `R`) returns to the preset's chords. Hints, the welcome
screen and Help name the active chord (`F2  actions`).

`forge setup` asks once whether to turn the preset on (default yes when `~/.claude` exists) and
writes `[keybinds] preset`. `preset = "forge"` is the default.

### Text binds, unbinding and sequences

A bind may also be written as text: `undo = "alt+z"`, `help = "ctrl+shift+h"`, `redraw = "f5"`.
`"none"` removes a default bind, so a chord you never want (`Ctrl+Z`, say) types nothing and does
nothing; the keybinding editor does the same with `u` or Delete.

```toml
[keybinds.binds]
suspend = "none"
verbosity_cycle = "alt+v"
```

**Leader key.** Off by default. `leader = true` makes Ctrl+X the leader; any other chord works too
(`leader = "alt+space"`). Press it, and a which-key strip along the bottom lists the follow-up keys:

| After the leader | Action | After the leader | Action |
|---|---|---|---|
| `n` | new session | `s` | checkpoint |
| `m` | model picker | `u` | undo write |
| `e` | cycle effort | `y` | copy last reply |
| `t` | cycle temper | `r` | reload config |
| `c` | compact | `x` | stash prompt |
| `k` | command center | `z` | suspend |
| `h` | help | `v` | cycle verbosity |
| `,` | keybind editor | `l` | repaint |

Esc cancels, an unknown key is swallowed and ends the sequence, and the window closes after
`leader_timeout_ms` (default 2000). The leader takes the chord over from whatever action held it:
with the default leader, stash moves from Ctrl+X to Ctrl+X then `x`.

**Sequences.** `[keybinds.sequences]` maps an action to two or more space-separated steps;
`<leader>` stands for the leader key. The first step must be a function key or a Ctrl/Alt chord the prompt does not already use
(Ctrl+A/C/D/E/J/U/W/Y, Alt+B/F/Y), so typing, Enter and Esc are never swallowed; the leader
follows the same rule. A sequence for an action replaces that action's built-in
follow-up; `"none"` drops it.

```toml
[keybinds]
leader = true
leader_timeout_ms = 1500

[keybinds.sequences]
new_session = "<leader> o"
help = "ctrl+g h"
undo = "none"
```

Sequences that can never fire are reported once at startup.

### `[tui]` preferences

| Key | Default | Meaning |
|---|---|---|
| `scroll_speed` | 3 | Transcript lines per mouse-wheel notch (1-20) |
| `cursor` | auto | Prompt cursor: `block`, `bar` or `underline` |
| `show_tps` | true | Show the tokens-per-second rate in the status line |

These apply the next time `forge chat` starts.

## 3. Implementation

- **`forge-config`** (`keybinds.rs`) — `KeyCombo`, `KeybindsConfig` (`preset` plus a `BTreeMap<String, KeyCombo>`), a `Default`
  with the binds above (the shipped default map has 30), `KeybindsConfig::defaults_for(preset)`, and `write_keybind()` / `write_keybind_preset()` (rewrite one key under
  `[keybinds]`, preserving all other config keys). `Config` gains a `keybinds` field.
- **`forge-tui/keybinds.rs`** — `matches(combo, key_event)` compares a combo against a crossterm
  `KeyEvent`; `resolve_action()` returns the `KeyKind` for the first matching configured action.
  Shift is checked explicitly once Ctrl/Alt is held, so Ctrl-C (interrupt) and Ctrl-Shift-C
  (copy) don't collide. `key_event_to_combo()` captures a fresh binding in the configurator.
- **`forge-tui/keybind_configurator.rs`** — a fullscreen takeover (run via `Tui::run_fullscreen`)
  listing actions, current binding, and description; rebind (Enter → capture), reset one (r),
  reset all (R/Shift-R), save (s), discard (q/Esc). A key already bound to another action is refused.
- **`forge-tui/driver.rs`** — `poll_event()` calls `resolve_action()` **before** the static
  defaults, so configured binds win. The `Tui` carries the loaded `KeybindsConfig`.
- **`forge-cli/run.rs`** — the chat loop dispatches each new `KeyKind`. Global action keys
  (toggle reasoning, configurator, help, effort cycle, copy, reload, tier up/down, skip model)
  work in any state; the rest run idle, several by dispatching the equivalent slash command.

## 4. Notes / limits

- A configured plain-character bind with no modifiers will fire while typing — keep action binds
  modified (Ctrl/Alt) or use named keys.
- `tier_up`/`tier_down` pin an in-session routing tier. The first press moves from the currently
  displayed tier; repeated presses clamp at `trivial`/`complex`. While a turn is running, the
  active prompt is aborted and retried at the selected tier. The pin persists until another tier
  action or `/reload`, and an explicit per-turn model/tier hint still wins over it.
- Because `resolve_action` runs first, Ctrl-Shift-K maps to `skip_model` (not the old kill-line-forward); plain Ctrl-K is `command_center`
  and Ctrl-R to `toggle_reasoning` by default; remap them, or set `preset = "claude-code"` above, if you want the editing shortcut back.

## 5. Desktop/web app shortcuts

The app registry lives in `mobile/src/lib/shortcuts/bindings.ts`. It persists only non-default
overrides under the device-local `forge.appShortcuts.v1` key. All app call sites resolve a typed
action through `useAppShortcut`; changing a row therefore changes the real handler rather than
editing a disconnected copy of the host TUI configuration.

The editor supports Mod, Alt, Shift, letters, digits, function keys, navigation keys, and common
punctuation. Exact app conflicts and standard system/menu conflicts are shown before saving.
Conflicts remain saveable for advanced layouts. On macOS, editable native menu accelerators are
updated over a closed Tauri command so menu key equivalents and webview handlers stay aligned.
Standard Cut/Copy/Paste, Quit, window, approval, and checkpoint accelerators are not mutable.

Bare Y/N permission-decision shortcuts are deliberately outside the editable registry. They stay
session-scoped, do not fire while typing, and cannot be remapped into an unsafe global chord.
