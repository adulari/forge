# Feature: TUI interaction (composer, lists, decision blocks, Help)

Every surface you type into or choose from in the Forge TUI follows one grammar: the Forgeline
tokens from [tui-theme.md](tui-theme.md), one list component, one decision block, one key model.
Fullscreen and `--inline` use the same components. Code lives in
`crates/forge-tui/src/app/render/` (`list_view.rs`, `popups.rs`, `permission.rs`, `question.rs`,
`input.rs`), `crates/forge-tui/src/help.rs` and `crates/forge-tui/src/app/state/interaction_ui.rs`.

## The composer

The only permanent frame. Its border colour is the mode colour (temper). The first row starts with
`❯`; continuation rows, explicit newlines and soft-wrapped text alike, hang at two cells under it.
The box grows up to the input row cap and then scrolls, saying `↑ N more` in its bottom border.

- Empty and idle it says `Message Forge…`; while a turn runs it says `queue a follow-up…`. A
  predicted next prompt shows as italic `try: <text>` and Tab accepts it.
- `/command` tokens are drawn in the brand colour, `@path` mentions in the info hue (`user`).
- `input_box_height` is the single source of its height, for layout and mouse geometry.

### Shell mode: `!command`

A line that starts with `!` runs in the shell instead of going to the model, as in Claude Code
(`!git status`, `! npm test`). The composer shows the `!` in the warning hue and the hints change to
`Enter run in shell`; the empty composer's placeholder says `! runs a shell command`. A line that
begins with a space (` !important…`) is an ordinary prompt, so a message can still start with `!`.

- The command goes through the session's own `shell` tool (`Session::run_user_shell`, in
  `crates/forge-core/src/user_shell.rs`): same workspace, sandbox, timeout and output budget as a
  model call, drawn as an ordinary `Run` card. It skips the approval prompt, since you typed it, but
  not the safety floor: privilege escalation (`sudo`, `su`, `pkexec`) and catastrophic deletes are
  refused with a card that says so and nothing runs.
- The run joins the conversation as a user message (`<bash-input>` / `<bash-stdout>`, the shape
  Claude Code uses) and the model responds to it. `[tui] respond_to_bash = false` only adds the run
  to the context; the next prompt's turn sees it.
- `/paths` and `@refs` in a shell line are shell text: they open neither the palette nor the file
  picker.
- A turn is running: the line goes back into the composer with a note, and Enter runs it once the
  turn ends (Esc stops the turn). A bare `!` explains the syntax.

### Editing keys

Readline keys work in the composer, and a Ctrl or Alt chord with no meaning is dropped instead of
typing its letter (Ctrl+X, Ctrl+B and Ctrl+Q used to insert `x`, `b`, `q`).

| Key | Action |
|---|---|
| `Ctrl+A` · `Ctrl+E` | Start / end of the line |
| `Ctrl+W` · `Ctrl+U` · `Ctrl+K` | Cut the word before the cursor / to the start / to the end of the line (`Alt+K` for the end of the line in the default keymap, where `Ctrl+K` is the command center) |
| `Ctrl+Y` · `Alt+Y` | Paste the last cut; right after it, `Alt+Y` swaps in the next older cut (a ring of 16) |
| `Ctrl+_` · `Alt+_` | Undo the last edit (a run of typing is one step; a cut, a `Ctrl+Y` and `Ctrl+G` are one step each) / redo it. Any new edit forgets what was undone. Redo is `Alt+_` because `Ctrl+Shift+_` is the same key as undo on a terminal that sends the raw `0x1F` byte |
| `Ctrl+P` · `Ctrl+N` | Previous / next prompt in history |
| `Ctrl+D` | Delete the character under the cursor; on an empty prompt press twice to quit |
| `Ctrl+G` | Edit the draft in `$VISUAL`, then `$EDITOR`, then `vi`; the saved text replaces the draft |

Consecutive cuts join one ring entry, so `Ctrl+W` twice then `Ctrl+Y` pastes both words back.
`Ctrl+K` is the command center and `Ctrl+N` starts a new session in the default keymap; the
`claude-code` preset ([keybinds.md](keybinds.md)) frees them for cutting and history. `Ctrl+_` is
`Ctrl+7` on terminals that send the raw `0x1F` byte, and `Ctrl+Shift+-` with the kitty protocol.
Code: `crates/forge-tui/src/driver/keymap.rs` (the key map), `crates/forge-tui/src/app/input_edit.rs`
(kill ring and undo), `crates/forge-tui/src/driver/external.rs` (`Ctrl+G`).

## One list component

`list_view.rs` draws every list: the slash palette, the command center, the `@file` picker, the
session, model, checkpoint, rewind, mode, MCP and memories pickers, the multi-select and
single-select prompts, the keybinding editor and Help. A list has a rounded frame (title left,
count or shortcut right), an optional filter row and group tabs, aligned rows, a `… N more` row
when it scrolls, and a detail line for the selected row.

- The selected row is a raised fill plus the `❯` caret plus bold, so it reads in 16-colour and
  `NO_COLOR` terminals where fills are not painted. Matched characters are brand bold.
- Descriptions clip with an ellipsis instead of wrapping; shortcuts and tags sit right-aligned.
- The same keys work everywhere: Up/Down, PgUp/PgDn and Home/End where the surface has them, type
  to filter, Enter to choose, Esc to close, Tab where it has a meaning.
- Text that arrives with typographic punctuation (`—`, `…`) is transliterated in the ASCII glyph
  tier, so nothing breaks `FORGE_GLYPHS=ascii`.

## Slash palette

A framed popup attached to the composer, at most eight list rows plus a detail line that shows the
selected command's usage. An empty query lists the commands you ran most recently first (the title
says `recent first`), then the registry order. Shortcuts (`F1`, `Alt+M`, `Alt+L`) are on the right;
skills carry a `skill` tag and file commands a `command` tag. Enter runs, Tab completes into the
composer, Esc closes.

## Command center

`Ctrl+K`. A modal over the dimmed chat, never over the composer. Group tabs All, Start, Session,
Model, Tools, Skills and Settings; Tab cycles them. Rows are grouped under UPPERCASE headers while
the search field is empty. Every registered command is in the All list. Enter stages the command in
the composer so arguments stay reviewable.

## `@file` picker

Opens on an `@` token. An empty query browses the top level with directories first (`src/`), a
query ending in `/` browses that directory, and any other query is a fuzzy search over every file in
the workspace (basename prefix, basename substring, path substring, then subsequence). Files git
reports as changed carry a `modified` mark, directories show their file count. Enter inserts a file
as `@path ` and closes; on a directory, Enter and Tab descend into it.

## Decision blocks

While a tool permission or a question waits, a block replaces the composer, with the status line
still below it. A heavy rail (`┃`) runs down the left; the bottom border carries the key hints.

**Permission** (warn rail). The title names the exact action in words (`Allow Write to create
mock-note.txt?`), a subject row says what changes (`new file · +1 line · ~/proj/mock-note.txt`, or
the full shell command wrapped in a surface band, never truncated), then four numbered choices
with the keys on the right:

| Choice | Keys |
|---|---|
| 1 Allow once | `y`, `Enter` (while it is focused) |
| 2 Always allow `<tool>` here (where the rule is saved) | `a` |
| 3 Deny | `n`, `Esc` |
| 4 Allow `<tool>` for this session (not saved) | `s` |

"This session" is last so `3` still denies. It adds the same scoped rule as `a` (a shell call is
scoped to the verb you were shown) to the running session and writes nothing to a config file; an
MCP tool's `a` is session-only too. A shell call that cannot be scoped is allowed once.

Up/Down (or `j`/`k`) move the `❯` focus and Enter confirms the focused choice; it starts on
"Allow once", so Enter keeps its old meaning. Digits pick directly. Keys that are not a choice are
ignored: an arrow press used to deny the tool.

**Notes.** Tab opens a one-line `note` field under the choices. While it is open, letters and
digits are text, the arrows still move the focus, Enter answers with the focused choice plus the
note, and Esc or Tab closes the field without answering (Ctrl+C still denies). The note reaches the
model with the tool result: on a deny it is the reason (`permission denied by the user (declined at
the prompt). The user said: <note>`), on an allow it follows the output (`[the user approved this
call and added: <note>]`). On a bridged CLI model (`claude`, `codex`) the note rides the relay's
refusal, so a deny note reaches that model too; an allow note has nowhere to go there. A blank note
is no note. The headless prompt takes the same: `y`, `s`, `a` or anything else, then the note on the
same line. The card above the block reads `awaiting approval`,
and Ctrl+T expands its diff. The block fits the 14-row inline region.

**Question** (info rail, never the warn colour). The question text, a tab strip when there are
several, numbered options (`✓`/`○` marks on multi-select), the `Other…` row with an inline field,
and `n` for a note. Typing a printable character on the list starts an `Other…` answer; a digit
past the last option opens it. There is no second input box.

## Help

`/help` opens Commands, `F1` and `?` (on an empty composer) open Keys, `/keys` opens Keys too.
One takeover with Commands, Keys and Mouse sections (Tab cycles them) and a search field: typing
filters. One row per command; the usage of the selected row is on the detail line; Enter on a
command inserts it into the composer. The Keys table is generated from the same registry the
keybinding editor edits plus the fixed keys, in one spelling (`Ctrl+K`, `Alt+M`, `Shift+Tab`,
`Ctrl+Shift+C`, `PgUp`). At 180 columns and wider, commands and keys sit side by side; below that
Help is one list that Tab switches between sections, because two half-width panes clipped most
command descriptions with an ellipsis.

## Tool groups and the navigation pills

In full-screen, a run of tool calls folds into one dim summary row (`Read 3 files, ran 2 shell
commands`), see [tui-tool-cards.md](tui-tool-cards.md) section 0a. Click it, or `Alt+↑`/`Alt+↓` to
focus a group and `Enter`/`Space` to toggle it; `Alt+O` opens or folds every group. The chords were
picked clear of Forge's own bindings and of Claude Code's defaults (`Ctrl+O`, `Ctrl+T`, `Alt+P`,
`Alt+T` are taken there), and all three are rebindable.

The "↑ Previous message · Ctrl+Home" pill (top right) and the "↓ Jump to bottom · Ctrl+End" pill
(bottom centre) show only while the transcript is scrolled up, and vanish the frame you are back at
the bottom. Both take a click. The top pill quotes the start of the message it goes to
(`↑ fix the login redirect… · Ctrl+Home`), the way Claude Code's sticky header shows the previous
prompt; the quote takes at most 36 cells and never more than half the row, and the pill falls back
to "Previous message" when the row is too narrow for a useful quote or the message has no text.
Below about 34 columns both pills shorten to the arrow and the chord.

The newest folded group ends its summary with a faint `click or Alt+O to expand`, so the fold is
discoverable without the help screen. On macOS the Alt chords need the terminal's "Option as Meta"
setting ([keybinds.md](keybinds.md)).

## Compatibility

Every keybinding and slash command keeps its meaning. The one behaviour change is the permission
prompt ignoring non-choice keys. `App::permission_key` returns `None` for those, which the key loop
already treats as "prompt stays open".
