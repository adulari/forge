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
says `recent first`), then the registry order. Shortcuts (`F1`, `Alt+M`, `Ctrl+L`) are on the right;
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
the full shell command wrapped in a surface band, never truncated), then three numbered choices
with the legacy keys on the right:

| Choice | Keys |
|---|---|
| 1 Allow once | `y`, `Enter` (while it is focused) |
| 2 Always allow `<tool>` here (where the rule is saved) | `a` |
| 3 Deny | `n`, `Esc` |

Up/Down (or `j`/`k`, Tab) move the `❯` focus and Enter confirms the focused choice; it starts on
"Allow once", so Enter keeps its old meaning. Digits pick directly. Keys that are not a choice are
ignored: an arrow press used to deny the tool. The card above the block reads `awaiting approval`,
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
`Ctrl+Shift+C`, `PgUp`). At 120 columns and wider, commands and keys sit side by side.

## Tool groups and the navigation pills

In full-screen, a run of tool calls folds into one dim summary row (`Read 3 files, ran 2 shell
commands`), see [tui-tool-cards.md](tui-tool-cards.md) section 0a. Click it, or `Alt+↑`/`Alt+↓` to
focus a group and `Enter`/`Space` to toggle it; `Alt+O` opens or folds every group. The chords were
picked clear of Forge's own bindings and of Claude Code's defaults (`Ctrl+O`, `Ctrl+T`, `Alt+P`,
`Alt+T` are taken there), and all three are rebindable.

The "↑ Previous message · Ctrl+Home" pill (top right) and the "↓ Jump to bottom · Ctrl+End" pill
(bottom centre) show only while the transcript is scrolled up, and vanish the frame you are back at
the bottom. Both take a click. Below about 34 columns they shorten to the arrow and the chord.

## Compatibility

Every keybinding and slash command keeps its meaning. The one behaviour change is the permission
prompt ignoring non-choice keys. `App::permission_key` returns `None` for those, which the key loop
already treats as "prompt stays open".
