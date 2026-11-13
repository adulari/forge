# IDE mode

A review workbench for what agents changed in a project: a queue of changed files ordered by what
needs a human first, a diff you can act on hunk by hunk, a light editor, the agent you are
reviewing, and a bottom panel with checks, background shells and a terminal. It exists on the
desktop and mobile app (`mobile/src/components/ide/`) and in the terminal (below). Both read the
same daemon routes, so they never disagree about what is changed or reviewed.

## In the terminal

| | |
|---|---|
| `forge ide [--cwd DIR]` | Open IDE mode on a project. Starts the background daemon if none runs. |
| `F2` in chat or the board | Swap to IDE mode and back. The chat stays alive underneath, so drafts and scroll survive. Under the claude-code keymap preset it is `F3`. |
| `/ide` | The same, from the command line of the chat. |

Layout follows the terminal width. At 160 columns or more: rail (30) | editor | agent panel (44) and
an optional bottom panel. From 110 to 159 the agent panel opens as a drawer (`Tab`). Narrower than
that shows one pane at a time with a tab bar. The mouse selects rows and tabs. `?` lists every key.

### Reviewing

`j/k` move between changed files, `n/p` between hunks. `a` accepts (stages) a hunk, `x` reverts it
(asks first), `u` undoes the last revert, `r` marks the file reviewed and moves on. A reviewed mark
belongs to the file's content hash: when an agent edits the file again the mark clears. `Y`/`N`
answer the agent's pending permission from anywhere; `m` messages the agent.

### Risk-sorted review

The queue opens as "9 look here · 31 routine" (hunk counts) and lists the **Look here** files first,
each with a one-line reason ("deletes a test", "touches permission code", "new migration",
"removes error handling", "touches unsafe code"). **Routine** files follow: lockfile churn,
generated files, snapshots, whitespace-only hunks and pure import reorders. `A` (the **Accept
routine** button in the app) stages exactly the routine hunks of a shared tree, one hunk request
each, and `u` (Undo in the toast) unstages them again. A worktree agent has no index to stage into,
so its fully routine files are marked reviewed instead. The diff view shows each hunk's verdict on
its header line.

The verdict is computed on the daemon, with no model call, where the diff is parsed
(`crates/forge-cli/src/serve_git/risk.rs`), and rides additively on `GET /api/git/diff` and the
session diff (`risk`, `reason` per hunk) and on `GET /api/projects/changes` (`risk`, `risk_reason`,
`look_hunks`, `routine_hunks` per file), so the terminal and the app always agree. It is
conservative: a hunk is routine only when a rule proves it, a hand-written file's routine hunk never
drops a code line it does not put back, a diff the daemon cut short is never routine, and
indentation only counts as whitespace outside Python, YAML, Makefiles and Markdown. A shared-tree
file whose changes are all staged is listed as routine "staged" with no hunks left to count.

### The editor

`e` opens the file in a light built-in editor (a diff otherwise). Typing, `Enter` (keeps the
indent), `Tab`/`Shift+Tab` indent, arrows, `Home`/`End`, `PgUp`/`PgDn`, `Ctrl+Home`/`Ctrl+End`,
`Ctrl+Z`/`Ctrl+Y` undo and redo. Only the lines on screen are highlighted (syntect).

`Ctrl+S` saves with `PUT /api/workspace/file` and the hash the file was read with. If the file
changed meanwhile the status line says **agent changed this file: r reload / o overwrite**. When
an agent edits the file while the buffer is clean, the buffer reloads by itself. `Esc` and `F2` ask
before discarding unsaved edits, and other files do not open over them. `E` hands the file to
`$VISUAL` / `$EDITOR` (the terminal is returned to you, then the result is saved back with the same
hash check).

### Why-trace

`w` draws a gutter from `GET /api/blame`: a bar beside every line an agent turn wrote, one bar per
tool call. `W` (`Ctrl+T` in the editor) opens the card for the hunk or line under the cursor:
agent, turn, model, the prompt that started the turn and the tool call. `Enter` jumps to that turn
in the agent's timeline.

### Review draft

`c` comments on the hunk under the cursor. `Enter` adds the comment to the draft (the agent panel
shows "3 comments"), `Alt+Enter` sends just that one. `S` or `Ctrl+Enter` sends the whole draft to
the agent as one prompt, the same text the desktop sends. `d` drops the last comment. `b` asks the
agent, as a `/btw` side question that does not interrupt its turn, why the hunk exists.

### Bottom panel

`t` toggles it, `[` / `]` change tab.

- **Checks.** The project's `[[actions]]` with their last result. `Enter` runs one, `o` reads its
  output, `x` stops it. In the output the `file:line` references are links: `j/k` pick one, `Enter`
  opens the file in the editor at that line. A failing check marks the changed files its output
  names, which moves them up the queue. `f` sends the failing output to the agent to fix.
- **Shells.** The agent's background shells (`shell{background:true}`); `Enter` follows a log live.
- **Terminal.** A real shell on the daemon over `WS /ws/terminal`, so it works against a remote
  daemon too, and it survives leaving IDE mode. Keys go straight to the shell (`t` included);
  `Esc Esc` leaves, `Alt+Left/Right` switch tab.

### Files and Git

The Files tab (`3`) is the project tree with changed files highlighted; `Enter` expands a folder or
opens a file (its diff when it changed). The Git tab (`4`) shows branch, staged and unstaged files,
the commit message and the **Ship row**: `g` writes a message for what is staged, `Z` (or the Ship
button) stages if nothing is staged, writes a message if the box is empty, commits and pushes if
there is a remote. The row spells out which steps it will take.

`C` in the agent panel continues the session in Claude Code: the daemon prepares the hand-off and
the terminal tab runs `claude --resume` in the session's folder.

### First visit

A one-line, three-step hint along the bottom explains the queue, the diff keys and sending a
review. It moves on as you do each step, `H` hides it, and it is remembered in
`tui-ide-state.json` in the Forge data directory. Reviewed marks live in `tui-ide-reviewed.json`.

### Performance

Only visible rows are drawn and highlighted; parsed diffs and highlighted windows are cached per
file, content hash and scroll position. With a 5,000-line file open a frame takes well under the
16 ms budget (`cargo test --release -p forge-agent-tui five_thousand -- --nocapture` prints the
numbers).
