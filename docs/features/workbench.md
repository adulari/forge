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

Files/search/editor, browser preview, review annotations, and multi-terminal work should extend
this model rather than adding route-local visibility booleans or another fixed dock.
