# Unified workbench surfaces

Forge's expanded desktop and web shell uses one workbench state model for auxiliary
surfaces. The routed session remains the primary content; workbench surfaces attach to a
right or bottom lane without replacing navigation, session providers, split-session panes,
or the mobile tab layout.

## Invariants

- A surface definition owns its kind, title, placement, default size, and whether it needs a
  session. `mobile/src/components/workbench/model.ts` is the authoritative registry.
- Opening a surface selects it and reveals its lane. Opening another surface in the same lane
  retains the first as a selectable tab.
- Toggling the active surface hides the lane without discarding its tabs. Closing an individual
  surface removes only that resource and selects an adjacent tab.
- Right and bottom lanes are independent. A terminal can remain visible below a session while
  usage, git review, or a later inspector is visible at the right.
- A surface without a pinned `sessionId` follows the shell's active session. Resource-specific
  surfaces may pin a session and use `resourceId` to keep multiple files, previews, or terminals
  distinct.
- Compact and medium layouts keep their native navigation. Workbench shortcuts are inert when
  expanded shell chrome is unavailable.

## Extending the workbench

To add a surface:

1. Add its typed definition to `WORKBENCH_SURFACE_DEFINITIONS`.
2. Add its renderer to the dock host registry.
3. Call `useWorkbench().openSurface(...)` or `toggleSurface(...)` from any descendant control.
4. Add a shortcut or native desktop menu item only when the action is globally useful.
5. Test identity, tab retention, placement independence, and close fallback in the pure reducer.

Files/search/editor, browser preview, and review annotations extend this model. Multi-terminal
work should do the same rather than adding route-local visibility booleans or another fixed dock.

## Workspace files

The Files surface is session-scoped: the daemon derives its root from the session's worktree or
cwd, never from a client-provided absolute path.

- The browser lazily lists directories and offers ranked filename search or bounded text search.
- Selecting a file opens it as a retained workbench tab. Text files up to 1 MiB can be edited.
- Saves include the hash returned when the file was opened. A concurrent agent/editor change
  returns `409 Conflict`; Forge asks the user to reload instead of overwriting it.
- Canonical path checks reject absolute paths, `..`, `.git`, and symlinks escaping the session
  root. Search respects repository ignore rules, skips generated dependency/build directories,
  and stops at 20,000 files or 64 MiB of searchable text.
- Composer `@path` completion uses the same session-scoped search. Paths containing whitespace are
  inserted as `@{path with spaces}` and expanded relative to the correct session workspace.
- These filesystem endpoints are Direct-only today. Forge Anywhere intentionally does not bridge
  project files; the surface explains how to reconnect directly instead of retrying a disallowed
  route.
- Compact iOS/Android layouts expose Files as a session tab and keep file navigation/editor state
local to that route. Expanded desktop/web layouts open the same browser and editor as retained
workbench tabs.

## Branches and worktrees

The Git surface includes a session-scoped branch/worktree picker backed by the daemon:

- Local and remote refs are searchable and report current/default state, upstream, short OID, and
  the absolute worktree currently owning a local branch. `origin/HEAD` is not presented as if it
  were a checkout target.
- Create-and-switch and branch switching are available only for a clean, idle shared workspace.
  The daemon rechecks these preconditions when the action arrives; the UI's disabled explanation
  is informative, not the safety boundary.
- Forge-managed worktrees keep their generated `forge/subagent/<session>` branch until merge or
  discard. Branch mutation is intentionally blocked there because merge/discard owns that branch
  and its lifecycle.
- Forge's branch actions refuse to change a shared repository while any managed worktree exists or
  any session using that shared repository is busy. This avoids moving the base checkout under
  active work and keeps merge-back targeted at the workspace users saw when isolation started.
- A branch checked out by another Git worktree is visible but cannot be selected. Remote selection
  creates a local tracking branch only when no same-named local branch exists.
- Branch routes, like the rest of Git review, derive the repository exclusively from the addressed
  live session and are Direct-only until Forge Anywhere carries an explicit filesystem/Git bridge.
- Compact iOS/Android exposes the same Git dock under Review → Working tree; Turn remains available
  beside it for plan and per-turn diff artifacts. Expanded layouts can retain Git in the workbench.

## Diff review annotations

Working-tree, staged, turn, and fork diffs use the same review model:

- Paired deletion/addition lines receive bounded token-level intraline highlights. The algorithm is
  quadratic only below a strict token cap and falls back to a linear prefix/suffix comparison for
  generated or minified lines.
- Tapping a line starts an old/new-side range; tapping another line on that side extends it.
  Selection is explicit in split and unified modes and can be cleared without creating feedback.
- A review annotation records repository path, old/new range, staged/working-tree/turn/fork source,
  selected line context, exact patch fingerprint, and the operator's comment. Markers only restore
  against that fingerprint, so a later shifted diff cannot display feedback on the wrong line.
- Pending annotations live for the session, appear as removable composer chips, and are formatted
  as readable line-addressed context only after the next prompt is successfully sent or queued.
- The compact Review tab remains marked while annotations are pending. Turn and working-tree views
  both feed the same composer, so mobile and expanded desktop/web have one feedback contract.

## Desktop browser preview

The Browser preview surface is a session-pinned, retained right-lane tab in the Tauri desktop
app:

- Its address field accepts an HTTP(S) URL, hostname, localhost address, or bare development
  port. Bare ports resolve to loopback; public hostnames default to HTTPS. Native validation
  independently rejects every non-HTTP scheme.
- Back, forward, reload, external-browser opening, 50–200% zoom, fit/390/768 CSS-pixel viewport
  modes, and multiple retained preview tabs use the same workbench model as Files and Git.
- Arbitrary preview pages run in an isolated native child webview. The child label is outside the
  trusted main-window capability, normal navigation is restricted to HTTP(S), and Forge exposes no
  general page-to-app IPC or arbitrary-JavaScript command.
- Element picking injects only the bounded picker runtime. A pick attempts a custom navigation
  which native code intercepts and cancels; the main app receives the page URL/title, stable CSS
  selector, semantic attributes, visible text, and CSS-pixel bounds.
- Picked elements become removable, session-scoped composer chips. They are formatted into exact
  browser context and cleared only after the prompt successfully sends or queues, matching review
  annotation delivery semantics.
- Web/iOS/Android clients explain that embedded preview is desktop-only and never attempt a
  fragile cross-origin iframe. Remote-host port tunnelling, screenshots/recording, console/network
  capture, and agent-driven browser automation remain later explicit capabilities rather than
  being implied by this first isolated preview slice.
