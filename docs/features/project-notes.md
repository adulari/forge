# Project notes

A scratchpad and a to-do list per project, on every surface.

## What it is

- One markdown **scratchpad** and a **checklist** per project, stored in the Forge store (`project_note`, `project_todo`, migration 50).
- The key is the project's checkout. A session in `.forge/worktrees/<name>` shares the notes of the repo it was branched from.
- Any open to-do can **start a thread**: a session in the project whose first prompt is the item. The row remembers the session and shows an "open thread" button from then on.

## Surfaces

| Surface | How |
|---|---|
| Desktop and web | Notes dock: View > Panels > Notes, the Layout menu, or the command palette ("Toggle Notes"). Follows the focused session's project; chips switch project. |
| Phone | Settings > Notes. Same panel, project chips along the top. |
| Terminal | `/notes` overlay. Keys: arrows move, Space ticks, `a` adds, `e` rewords, `d` deletes, Enter starts a thread, `p` edits the scratchpad, Esc closes. |
| Shell and agent | `forge notes`, `forge notes add "..."`, `forge notes done 2`, `forge notes reopen 2`, `forge notes rm 2`, `forge notes pad ["line"]`. Output is plain text with numbered items, so the agent reads it with a normal shell call. |

## Daemon routes

All under the daemon token. Reads need the `view` scope, writes the `prompt` scope.

- `GET /api/notes?project=` returns `{project, body, updated_at, todos[]}`.
- `PUT /api/notes` saves the scratchpad: `{project, body, expected_updated_at?}`. When `expected_updated_at` is stale and the text differs, the answer is `409` with `{code: "conflict", body, updated_at}` and nothing is written.
- `POST /api/notes/todos` adds an item; `.../todos/{id}/update`, `.../link`, `.../delete` change it; `POST /api/notes/clear-done` drops the ticked ones.

Every write bumps a counter that open `/ws/fleet` sockets turn into a `{"kind": "notes_changed"}` frame, and the app refetches. Edits made from the terminal write the store directly, so the app picks them up on its 20 second poll or on focus.

## Limits

Scratchpad 200 KB, a to-do 2000 characters, 500 to-dos per project.

## Not covered

Over Forge Anywhere the app has no notes route yet (the bridge allowlist lives in the hosted service), so the Notes surface says it needs a direct connection to the host, like pull requests and Actions.
