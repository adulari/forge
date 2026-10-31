# Changelog

All notable changes to Forge are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Forge adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **The long-session endurance test no longer fails on a busy CI runner.** Its wall-clock ceiling, meant to catch pathological slowness, rose from 90 s to 240 s after healthy runs took 108 s on the shared self-hosted runner.

### Added

- **`spawn_agents` children can return schema-checked data instead of prose, behind a new `[harness] subagent_output_schema` flag.** With the flag on, the parent can attach `output_schema` (a JSON Schema subset with root type object: `type`, `properties`, `required`, `additionalProperties`, `items`, `enum`) to an agent or to the whole call. The child gets a `submit_result` tool built from that schema; an invalid submission goes back with path-precise errors (`$.files[2].line: expected integer`) for up to 2 retries, and a child that hits its step limit or the progress guard gets one last `submit_result`-only call, which comes back as a `partial` result. A batch that asked for a schema returns one JSON envelope with a `status` (`valid`, `partial`, `invalid`, `text` or `error`) and the `data` for each agent. An unsupported schema is refused before any child starts. Default off, and with the flag off nothing changes: the tool spec, the child prompt and the combined output are byte-identical.
- **`/anywhere` opens a real Forge Anywhere menu in the terminal, with account sync in it.** One overlay shows your account, the connector and this device, the five sync categories (keys, skills, settings, MCP servers, instructions) with an on/off switch each, item and pending counts, last sync, conflicts and skipped items with their reasons, a Sync now action, a Sign in again list for logins that do not sync (Codex, xAI, MCP OAuth, run through the existing flows), and the enrolled devices. Arrows, Tab, Space/Enter and Esc drive it, it refreshes while open, and it spells out the not signed in, loading and error states with a next step. Turning keys on asks first and explains that they are sealed with a second vault key and that OAuth tokens never sync; turning any category off leaves local data in place; the web Recovery Center stays one labelled row away. The statusline now shows the sync badge on its own while any category is on (`/statusline toggle sync` hides it).

## [2.26.0] - 2026-10-30

### Added

- **You can now delete a session permanently from the CLI and the TUI, matching the apps.** `forge sessions --delete <id>` shows the title, turns, subagent sessions and worktree, then asks you to type the short id (`--yes` skips it); `/delete` in a chat does the same for the current session, Ctrl+W in the sessions picker for any other, and Del on a board card. Every surface runs one set of checks and says exactly why it refused (open in a terminal, running, owns a worktree, frozen by a handoff) and what to do. The delete removes the whole subagent tree and every row and snapshot folder keyed to it, tombstones it on synced devices, purges its journalled copies, and runs with `secure_delete` plus a WAL truncate so the text does not linger in the database file; `--compact-db` also vacuums when nothing else holds the store. Deleting a session open in a terminal from the app now names the host and says to run /delete there. See docs/features/session-delete.md.
- **Forge Anywhere account sync carries your setup to your other machines.** Opt in per device and per category (`[anywhere.account_sync]` keys, skills, settings, mcp, instructions; all off by default) with `forge anywhere sync enable <category>`, `/config`, the onboarding Anywhere step or `GET`/`PUT /api/anywhere/sync`. API keys and static MCP secrets are sealed twice and never leave through the Anywhere bridge's toggle; rotating logins (Codex, xAI, MCP OAuth) never sync, other devices just offer to sign in. Stdio MCP servers apply only where their command is installed, conflicts keep the newer edit, a statusline `anywhere_sync` widget shows health, and revoking a device re-seals keys under the new epoch with a rotate-at-the-provider checklist. Anywhere's data keys and device keys now live in the secret store instead of `state.json`, and continual-harness rows no longer jam the sync outbox.
- **Four opt-in `[harness]` flags make the loop harder to stop early, for A/B benchmarking.** `completion_contract` appends a short critical block to the end of the system prompt; `stop_retries` continues an announced-but-undone reply up to 3 times and retries empty or text-only tool-call replies 3 times before benching (a single empty reply never benches a model); `thinking_loop_breaker` tells a model that keeps restating its plan to act; `repeat_interrupt` names a call repeated 5 times with the same result and moves the progress ledger's cycle stop two repeats later. All default off, and each wins over `harness_profile = "minimal"`.
- **Sync across machines: a Sync page for carrying keys, skills, settings, MCP servers and instructions between your computers over Forge Anywhere.** Every category has its own switch, off until you turn it on, with item counts, last sync, pending changes and expandable conflicts and skipped items, plus Sync now, copyable sign-in commands for logins that do not sync, and hints for tools this machine is missing. Keys & tokens can only be switched on the computer itself, so over Anywhere and on phones that row is locked. The page is reachable from the Anywhere hub and Settings (and the settings search), and the desktop onboarding Anywhere step offers the same switches. The desktop title bar (Tauri and web) also gets a compact sync status pill next to the server chip, with a popover per category, Sync now and a link to the Sync page; phones show the status as a dot and short text on the Sync rows. The offline history view now skips account-sync records so they are never rendered.
- **Skills install from a git marketplace, sync to Claude and Codex, and count their uses.** The Skills page gets a Marketplace sheet (source chips, search, an Official-only switch, per-skill Install with an error toast, and an add-a-source form), an installed-packs list with Remove, a precedence note (a project skill beats a user skill of the same name), a Sync action that links your skills into `.agents/skills` and `.claude/skills` so bridged agents see them, and a per-skill task form that copies the `/skill name task` line. The daemon serves it from the new `serve_skills.rs` (`/api/skills/{state,available,install,remove,marketplaces,sync,invoke}`; project scope resolves through a session's repository), `forge plugin install` and the UI share one install path that also accepts a single-skill marketplace entry, and every `/skill`, `forge run` and model `use_skill` load bumps a count in `skill-usage.json` shown as `uses` on each row.
- **MCP servers can be inspected and controlled from the app, and synced to Claude Code and Codex.** `GET /api/mcp` now reports each server's scope, status, last error and tool count, with `POST /api/mcp/reload` and `/api/mcp/probe` to connect on demand, `PUT`/`DELETE /api/mcp` to edit or remove a server, and a Built-in group (browser, web tools, self-MCP). Tapping a server lists its tools and resources and lets you switch individual tools off (`disabled_tools` in `mcp.toml`), which hides them from the model and refuses calls to them. New sessions get a chip row to leave servers out of that session only (`mcp_off`), and `forge mcp sync --to claude,codex [--dry-run]` writes your enabled servers into `~/.claude.json` and `~/.codex/config.toml`, referencing tokens by environment variable name and never by value.
- **PR Watch walks away from a pull request for you.** Arm it per session from the PR sheet (Off, Fix, or Fix + merge with squash, merge, rebase or GitHub auto-merge) and the daemon polls `gh pr view` every 30 seconds. When a check fails, a reviewer requests changes or a new comment lands, it sends one fix turn into the session per distinct issue (head commit plus failing checks plus comment ids), and when everything is green in merge mode it merges bound to the seen head commit and marks the session done. A watched session runs in the Auto permission profile so a plain `git push` is pre-approved while a force push is refused and stops the watch. Anything it cannot resolve (closed PR, merge failure, four fix turns without green, missing `gh`) is recorded as `blocked_reason` instead of spending more turns. Watches persist across daemon restarts, exempt the session from idle unload, show as a badge on the Fleet card, a glyph in the sidebar and a PR Watch list in Schedules (`GET/POST /api/pr/watch`, `GET /api/pr/watches`).
- **Fork sheet can try a thread on another model, and messages can rewind.** `POST /api/sessions/{id}/fork` now takes optional `model`, `effort`, `temper`, `prompt` and `title`, so the child pins its own model, effort and access and starts with a first prompt while the original keeps running. The Fork sheet gains a model picker, effort and access rows, a first-prompt box and Cancel / Fork / Move (Move forks, then archives the source). User messages get a "Rewind to here" action that opens the restore-scope step and puts the prompt back in the composer, disabled with the reason while a turn is running and with a warning when another live session shares the worktree. The terminal's `/fork` accepts `--model <id>`.
- **Pairing a device on your LAN now pins the daemon's certificate, so a phone or the desktop app no longer has to trust whatever answers on that address.** `forge serve --lan` keeps its self-signed certificate across restarts (saved beside the token, 0600) and the QR and `forge serve pair` link become `https://host:7420/#token=...&fp=sha256:...`: the token and the leaf fingerprint ride in the URL fragment, and the old `/<token>` path form still parses. The desktop app dials the daemon itself through a pinned loopback proxy that accepts exactly that leaf, and reports "certificate changed, re-pair" if it ever differs. The daemon advertises `_forge._tcp` over mDNS (TXT `id` and `fp`, never the token), the desktop Connect screen lists those hosts under Nearby, `forge serve --tailscale` publishes the daemon through `tailscale serve` and puts the trusted MagicDNS address in the QR, and `[remote] public_url` points links at a reverse proxy you manage; Remote Access reports the Tailscale state (not installed, logged out, stopped, HTTPS off, ready). The Connect screen now has a "Scan pairing code" button, an "Other ways" sheet (paste with a clipboard button, photo of the code on web, enter by hand), a confirm card showing the sanitised host with a plain-http warning and "Connect anyway", camera-busy and no-camera states, a 10 fps web decoder, an Install button or iOS Add-to-Home-Screen hint on web, and it strips the token from the address bar. iOS and Android pinning needs a native module and ships with a later native build.
- **Schedules are usable by people and leave a trustworthy run history.** The Schedules panel gets a cadence builder (Hourly, Daily, Weekdays, Weekly, One-time, Custom) that reads back as words ("Weekdays at 3:00 AM  ·  Next run Jul 30, 3:00 AM"), an optional time zone, All/Active/Paused filters and search, Edit, Run now, a last-result line on every row that opens the session it created, a Results inbox with read and unread, and template starters (Daily brief, Dependency audit, Weekly changelog) in the empty state. Every OS timer now launches `forge schedule tick`, which writes a `schedule_run` row (running, ok, failed, interrupted when the process died, skipped when the last tick was still going), enforces a per-schedule runtime cap, retries with backoff, honours a missed-run policy, and can pause the schedule after a failure, after N runs, or once the run judges a "stop when" condition true. `forge schedule add` gains `--tz`, `--max-runtime`, `--max-runs`, `--retries`, `--stop-on-error`, `--stop-when` and `--skip-missed`, and `forge schedule run` and `forge schedule history` are new. The daemon adds `/api/schedules/{id}/update`, `/run`, and `/api/schedule-runs`. Time zones need systemd, so on macOS and Windows a zoned schedule is refused with a clear error rather than firing at the wrong hour. Timers created by older versions keep running `forge run` directly and appear in history after they are paused and resumed once.
- **Each paired device gets its own token, scopes and a Remote Access page, so you can revoke one phone without touching the rest.** `forge serve pair [--ttl 10m] [--preset viewer] [--label NAME]` prints a fresh single-use link (and QR) that must be opened within its TTL; `forge serve clients list|revoke ID` shows every device with its last connection and cuts one off at once, live WebSockets included, while the owner token and every other device keep working. Tokens are stored as SHA-256 hashes in `serve-clients.json` (0600). Six scopes (view, prompt, approve, terminal, workspace-write, manage-providers) are enforced in the REST and WebSocket paths, and a view-only device gets a typed `scope_denied` refusal plus a "This connection can view but not change" state in the app. Settings gains a Remote access page with Online/Off status, a rotating pairing QR, Local/LAN/Tailscale/Public address tabs with copy buttons, and the devices table with Revoke. Every mutation, approval and refusal is appended to `remote-audit.jsonl`, and short-lived single-use WebSocket tickets keep the bearer out of WebSocket URLs.
- **Sessions have a lifecycle: star them, mark them done, restore them from the archive, and let the daemon tidy up.** Starred sessions float to the top of the fleet and sidebar and are never auto-archived; Done sessions sink into a collapsed, counted section. The daemon gains `POST /api/sessions/{id}/star`, `/done` and `/restore` (un-archive without resuming), and the session lists carry `starred` and `done_at`. A new `[sessions]` config section sweeps every five minutes: `idle_unload_minutes` (default 60) drops an idle hosted session from memory while its history stays in the store, and `auto_archive_days` (default 3, `0` disables) archives stale sessions that are not starred and own no worktree. The session sheet and sidebar menu gain Star, Mark done, Restore from archive and Delete forever, and the TUI gains `/star`, `/done`, `/archived [restore <id>]` and `/export --json`.
- **The model picker groups by vendor, shows how each model is reached, and remembers favourites.** Each row carries a Subscription bridge, OAuth or API chip, favourites and your recent picks sit above the vendor groups, and a star and a hide button on every row in the picker and on Settings, Models & mesh write `[mesh] favorites` and `[mesh] disabled` through a new `PATCH /api/models` (the model list also reports both lists, and hidden models can be restored from Models & mesh). A pinned model that is benched or unknown shows a banner above the composer offering the nearest healthy replacement, same vendor first.
- **Keyboard shortcuts cover sessions, the composer and the sidebar, and one `keybindings.json` now feeds the app and the TUI.** New chords: next/previous session (Mod+Shift+]/[), Ctrl+Tab through the most recent sessions, close session (Mod+W), app settings (Mod+,), Toggle Preview (Mod+Shift+B), composer-only effort/mode/model/plan (Alt+E, Alt+Shift+T, Alt+M, Alt+P) and sidebar-row archive/rename. Entries carry a focus scope (`composerFocus`, `sidebarFocus`, `hasSession`), so scopes that cannot overlap may share a chord without a conflict warning. The daemon serves the file at `/api/keybindings`; the app syncs its overrides through it, the TUI layers its `tui` section over `[keybinds.binds]`, and bindings from a newer build are kept intact. Settings lists the locally handled chords as read-only rows.
- **Pull requests and GitHub Actions are first-class in the app, not agent-only.** The daemon gains `/api/pr/*` (status, list, detail, diff, create, comment, merge) and `/api/actions/*` (workflows, runs, run detail, dispatch inputs, dispatch, rerun, cancel, delete) over the host's `gh`, reporting a missing or signed-out `gh` as data instead of an error. The session header shows a PR chip whose colour follows the checks and which polls while they run; it opens a Summary / Changes / Checks view with a hide-whitespace toggle and a per-commit scope, comments, and a merge bound to the head commit the reviewer saw (a newer push is refused) that pulls the base branch afterwards when it is checked out and clean. The git dock's commit menu gains "Push & create PR", a global Pull requests screen lists every project's PRs, and a GitHub Actions dock in the workbench's right lane pins workflows, runs `workflow_dispatch` with its inputs, re-runs, cancels and deletes runs and shows job and step status. GitLab (`glab`), a PR-mode setting and the AI review guide are not part of this change.
- **The browser tool gains a `batch` action, and `eval` and cookie reads are now opt-in.** `batch` takes a `steps` array of ordinary browser calls, runs them in order, stops at the first failure and returns one compact report plus a single final page snapshot. `eval`, `replay` and `cookies` now return a refusal that names `[browser] allow_eval` or `[browser] allow_cookie_access`, both default false, until the user enables them.

### Fixed

- **Settings pages share one frame, and the Settings rail no longer turns white in dark mode.** The navigator's scene wrapper painted the light react-navigation default background above the app's dark surface, so the transparent Settings rail showed as a white slab that hid the title and "Forge Anywhere" label; the app now sets the navigation theme from its tokens and the rail and shell own their own backgrounds. Every Settings page (including all Anywhere pages, which now keep the rail) uses one header with the same title style, one 760px reading column (1040px for two-column pages) and the Settings tab highlight, and the rail shows Anywhere's status as a dot instead of truncating its label. The Anywhere page lists hosts, devices and sync/billing as full-width rows, names which device is this one, shows last seen and enrolled dates so identical "Forge on iPhone" entries can be told apart, flags enrollments inactive for 30 days with a Review link to the existing revoke flow, and sets the privacy note in readable sans text.
- **Account sync records no longer depend on which key epochs a device holds.** A machine enrolled after a key rotation only ever holds the current epoch, so it derived different record ids from older machines and every item was duplicated with no last-writer-wins between the copies. Identity is now the item named inside the payload: a device reuses the id the account already uses, duplicate ids collapse onto the smallest one (only the device holding the newest copy tombstones the rest), and change detection uses a persisted local key so a rotation never makes unchanged items look edited.
- **File names in chat open the file they mean.** Clicking `SOLVER.md` used to look only at the workspace root and fail with "workspace path does not exist" when the file sat in a subfolder or outside the workspace. The daemon now finds a workspace file by its path suffix (`SOLVER.md` opens `notes/SOLVER.md`), turns an absolute path inside the workspace into a relative one, and opens a file outside the workspace read-only when this session's own messages or tool calls name that absolute path. Any other path is still refused, with a message that says why. The editor saves to the resolved path and shows outside files as read only.
- **Pasting the code Claude shows now finishes the sign-in.** Forge typed the code and Enter into `claude setup-token` in one write, and Claude Code's input box reads a multi-character write as pasted text, so Enter never submitted it and setup sat on "Connecting Claude Code". Enter is now sent on its own after a short pause.
- **Connecting Claude Code from `forge setup` works over SSH.** The sign-in link is one clickable link even when it wraps, and `c` copies it to your clipboard (OSC 52, so it reaches your own machine through SSH); `o` is hidden when there is no browser to open. Paste the code Claude shows and it lands in the code field, which now sits under the link so the QR code can no longer push it off the screen. The QR code shrinks to fit the window (lower error correction, no padding rows, and six modules per cell on terminals that draw Unicode sextants) and `v` gives it the whole page. Claude's own output keeps its spaces, a redrawn prompt shows once, and the sign-in page no longer re-encodes the QR on every frame (idle CPU drops from about 28% to 8%).
- **Resuming a session no longer opens the setup screen.** `forge chat --resume <id>` and the session picker skip the once-per-release setup run, which sat in front of the session being resumed after every update; a fresh `forge` or `forge setup` still opens it, and the Claude reconnect card still appears when needed.
- **The iOS and Android app start again.** A keyboard listener added with the new session shortcuts called `window.addEventListener` on React Native, where `window` exists without DOM events, so every launch failed with "undefined is not a function" (auto-report #1763). The listener, and the overlay Escape stack's, now run only where DOM events exist.
- **A retried prompt can no longer be queued twice.** The mobile app now stamps each prompt with a `client_msg_id` when you compose it and re-sends that same id on every retry and offline-queue flush; the daemon remembers the last 256 ids per session and drops a repeat instead of enqueueing it again, so a lost acknowledgement cannot duplicate a prompt. The field is optional: older apps send none and older daemons ignore it.

### Security

- **A pairing link no longer puts the token in a path, a query or a log line.** The new link keeps the token and certificate pin in the URL fragment, which browsers never send to a server, and the web Connect screen removes it from the address bar and history as soon as it is read.
- **The daemon now rejects requests whose `Host` header is not an address, a local or tunnel name, or listed in `[remote] allowed_hosts`, and `SECURITY.md` carries the remote-access threat table.** This closes DNS rebinding against a token path; `X-Forwarded-For` is believed for the audit log only from loopback or `[remote] trusted_proxies`. The legacy control page, which embeds the owner token, is owner-only.
- **The mobile npm audit gate waives the two `decode-uri-component` denial-of-service advisories until Expo SDK 58.** The only fix is expo-router 58, which needs SDK 58, so every mobile PR was blocked. The waiver lapses automatically once expo-router 58 is installed.

## [2.25.2] - 2026-10-29

## [2.25.1] - 2026-10-29

### Added

- **Settings search finds any setting, and the theme is yours to change.** The Settings rail (and the top of Settings on a phone) has a search field over every page and every configuration row, so typing "sleep" lands on Keep this computer awake, scrolled into view and highlighted; the command palette's Settings chip lists the same rows. Configuration groups fields into Notifications, Terminal, Threads, Git, Worktrees, Audio and Archived sections when the daemon has settings for them. Appearance gains six colour presets (Midnight, Graphite, Forest, Nord, Solarized, Rose) that keep your light or dark choice and persist, a WCAG contrast readout that warns on low-contrast pairs, and import and export of a theme as JSON (32 KiB cap, saved as `custom:<slug>`).
- **`/theme` switches the TUI accent palette.** `/theme` lists forge, ocean, grove and rose; `/theme ocean` applies it live on truecolor terminals and saves it as `tui.palette`. `[tui.colors]` overrides single roles (`brand = "#FF8A30"`) on top of the palette, and Settings, Configuration lists the palette setting.
- **The terminal gains /fork, /stash, /skills and /timeline, and `forge chat --prompt`.** `/fork [message]` branches the live session at its latest message into a child, continues there and sends the message as the child's first turn, leaving the parent untouched. Ctrl+X (the `stash` keybind) parks the half-written draft and, on an empty composer, brings the newest one back; `/stash [push|pop|list|clear]` does the same by command. `/skills` lists the skills the model can load with scope and description, `/timeline` opens the past-message picker (the `/rewind` picker), `@` file completion ranks files with uncommitted edits first among equal matches, and `forge chat --prompt TEXT` sends a first message from a launcher.

## [2.25.0] - 2026-10-29

### Added

- **Find works in the terminal.** Mod+F with the terminal focused (or Find in terminal from the session menu) searches the retained scrollback, highlights every match, scrolls to the active one as you step with Enter and Shift+Enter, and Esc closes the bar and returns the keyboard to the shell.
- **Notifications are quiet, targeted and click-through.** Settings, Alerts switches Session finished, Needs attention and Failed on or off, chooses between a toast plus system notification ("Always") or system notifications only while Forge is in the background, and adds a Sound chime. A session shown in any split pane no longer raises a toast; the toast lasts 6 seconds with an Open button, and Open or a clicked notification goes to the session with the composer focused (the desktop shell can only bring the window forward, as the OS reports no click). Failed turns now alert, and a repeat of the same alert inside 15 seconds is dropped. **Hide notification text** makes every notification say only that a session needs you: it withdraws the sealed-alert key on iOS so the daemon sends the generic alert, and makes the web service worker show generic text. Android gets two channels, Needs attention (high priority, heads-up) and Status (low), replacing the single "Forge sessions" one, and an installed iPhone web app asks once, on launch, whether to enable notifications. Notification titles are cut to 80 characters and bodies to 240 with escape sequences removed, including the daemon's `push_notification` desktop channel.
- **The terminal notifications gain a sound, per-event switches and a first-run invitation.** `[notifications]` accepts `sound = false | true | "/path/to/file"` (the system sound, or a file played with `afplay`, `paplay` or `aplay`) and `on_done`, `on_input` and `on_error` flags; a turn that ends in an error now notifies whatever its duration. With notifications off, the first turn of 30 seconds or more adds one line offering to turn them on, once per installation.
- **Replies show tables, images and math, and file paths are clickable.** Typeset `$E=mc^2$` and `$$ … $$` blocks render on the desktop and web apps (KaTeX loads as its own chunk the first time a reply has math; phones show the TeX source), `![alt](docs/shot.png)` shows the project image inline, and a path like `src/a.ts:12` in a reply, a prompt or an `@src/a.ts` mention becomes a chip with a file-type colour that opens the Files dock at that line. An image path opens the new image viewer, which also serves attachments and tool screenshots: zoom with buttons, keys or ctrl-wheel, step through a set, Copy and Save. The daemon serves them through `GET /api/workspace/image` (project or evidence images by path, confined to the session) and `GET /api/evidence` plus `/api/evidence/image` (the session's captured screenshots, thumbnails downsampled with at most two decodes at once).
- **A Gallery dock per thread lists the screenshots its browser and device tools took.** Toggle it from the Layout menu or the command palette: each tile shows the thumbnail, caption, tool and age, updates while a tool run is capturing, and opens the viewer with arrow-key stepping. A browser or device screenshot call also shows its picture under the tool row. The terminal UI names an image's source after its alt text (`[image: the cart] (docs/cart.png)`) instead of dropping the path.
- **One command palette with category chips, fuzzy search and files.** ⌘K and ⌘P now open the same palette (⌘P starts on Sessions); All, Sessions, Commands, Files and Settings chips filter it, Ctrl+Tab cycles them, and each section in the All view shows five rows with an "N more" row that drills in. Typing an abbreviation finds the entry ("tgr" finds "Toggle Git review"), ranked by match, then sessions that need you, then recency (commands you ran lately come first). Files lists workspace file names for the open session under "File names in <project> (branch)" and opens the pick in the Files dock, with "Select a project to search files" and "Type at least two characters" when there is nothing to search yet. Slash commands and skills are Commands rows, Settings pages are Settings rows, and online Forge Anywhere hosts are searched alongside this one with a host label on their hits.
- **Palette keyboard and screen reader support.** PageUp and PageDown move by eight rows, the search field is an ARIA combobox that announces the active option, the result count is a live region, and Enter during an IME composition no longer selects a row.
- **A message hit in search opens its session scrolled to the matching message.** The transcript pages in older history until the row is loaded, then centres it. A failed or unavailable message search shows a Retry row instead of an empty list.
- **Escape closes only the topmost overlay.** Sheets, dialogs, menus, the quick composer and the palette register with one capture-phase stack, so Esc over a palette that sits on a sheet closes the palette and the next Esc closes the sheet. An IME composition and the terminal keep their own Escape.
- **A goal can be paused, resumed, reworded and cleared, and the app shows the plan and the spend above the composer.** `/goal pause` stops after the turn in flight without starting another or paying for the evaluator, `/goal resume` continues at the same turn with its token and time budget, `/goal edit <objective>` rewords a running or paused goal, `/goal clear` drops it, and `/goal status` (or a bare `/goal`) prints where it stands; these run while a turn is in flight. The goal is now a state the host reports (`active`, `paused`, `budget limit`, `complete`, `failed`, `cancelled`, as `goal` on the snapshot), shown as a chip on the terminal composer border (`2/5 · 12.5k/50k tok`, `paused · 2/5`) and in the app's goal banner with the turn, token budget bar, elapsed time and the Pause, Resume, Edit, Stop and Clear buttons that state allows. A Plan dock sits on the composer with `Plan 2/4`, done rows dimmed, a pulsing dot on the task in progress and hollow circles for the rest, fed live from `update_tasks`. A Spend dock lists the session's tokens, billed spend and list-price estimate per model and per subagent (`GET /api/sessions/{id}/spend`, totals equal `/api/usage?session=`). Each dock folds, moves up or down, hides for the chat and remembers that on the device, and Settings, Appearance, Thread docks puts the plan in the side panel instead on wide windows.
- **A finished turn closes with what it took: "Worked for 23s · 4.1k tokens · $0.04".** The desktop, web and phone chat print it as a faint line under each completed turn, summed from the usage the daemon now attaches to every model reply in `GET /api/history` (`input_tokens`, `output_tokens`, `cost_usd`, absent when a reply was not billed), so it survives a reload and covers turns from before this release. The newest turn's line waits until the session is idle.
- **An answered question stays in the transcript as a decision record.** The chat shows the header chip and the question, the chosen label in bold and its description under it (a form shows one block per question, with any note), read back from the stored `ask_user` call and answer. The terminal prints the same two faint rows under the turn instead of a loud, bold block, with the picked option's description beside the answer.
- **Edit rows say how much they changed.** History edit-call rows carry the lines added and removed (`adds`, `dels`), counted by the daemon from the call's full arguments, so an edit row in the apps shows `+12 −3`, a call that took two seconds or more shows its duration, and a folded group's summary reads "Edited src/a.ts (+8 −2)". Repeated edits to one file inside an opened group fold into one row, "Edit src/a.ts ×3 +12 −4", that lists the individual edits when tapped.
- **Each subagent a call started gets its own row under the call.** A `spawn_agents` or workflow row lists every child with its status, task, model chip and cost, opens the child's transcript on tap, and stays visible when its tool group is folded.
- **The Git review dock does the whole commit flow: generate the message, commit, push, pull, fetch and discard.** A sparkle in the message well asks a trivial-tier model for a conventional-commit message from the staged diff (`POST /api/git/commit-message`; the diffstat, recent subjects and a bounded patch go in, nothing is committed) in the language set by `[git] commit_language` (default English, overridable per repository in `.forge/config.toml`). The commit button is split: it runs "Commit" or "Commit & push", whichever you used last, the caret picks the other, and Ctrl/Cmd+Enter in the message well runs it. Above the file list, Fetch, Pull and Push show the branch's ahead/behind counts against its upstream (`POST /api/git/{fetch,pull,push}`): push never forces and sets the upstream on a new branch, pull is fast-forward-only with a one-tap "pull with a merge instead" when git refuses, and none of them can prompt for a password or hang past 120 seconds. Unstaged and untracked rows get a discard action, with a confirmation, that restores the file to the index or deletes the untracked file (`POST /api/git/discard`). Pull and discard refuse while the session is mid-turn; over Forge Anywhere the dock stays read-only.
- **Merge conflicts are listed, with a button that hands them to Forge.** Paths with unresolved conflicts get their own CONFLICTS group (they no longer show up as staged or unstaged), the status names the unfinished merge or rebase, and "Resolve with Forge" starts a turn on the session that resolves each file, stages the result and leaves the commit to you.
- **The Git diff is one scroll of collapsible file cards with syntax colour.** Every changed file is a card (staged, unstaged and untracked each get theirs) that loads its diff only while open, so a hundred-file change costs a hundred headers; picking a file in the list opens and scrolls to its card, and the toolbar collapses or expands them all. Lines are coloured for Rust, TypeScript/JavaScript, Python, Go, shell and JSON with the chat's tokenizer, a hunk header widens the context by 20 lines (`GET /api/git/diff?context=N`), "hide whitespace" drops whitespace-only changes (`ignore_whitespace=true`), a diff the daemon capped says how many lines it left out, the staged/unstaged/untracked groups collapse, and each row carries a language-coloured file tag. Review comments stay attached when you widen the context around them.
- **The Files dock can create, rename and delete.** New file and New folder in the header (slashes create nested folders), rename and delete on every row, delete behind a confirmation (`POST /api/workspace/{create,rename,delete}`). They share the editor's confinement and also refuse a symlinked directory that leads outside the session root; create and rename never overwrite, and a folder that is itself a git repository is not deleted.
- **The sidebar and the Fleet tell you provider, project, state and change size at a glance.** Every session row carries a monochrome mark for the provider that answers it (Claude, OpenAI/Codex, Gemini, OpenCode, Grok, Kimi, Qwen, Ollama, OpenRouter, Bedrock), the same mark sits on the composer's model chip, the model picker, Settings > Providers and inside each usage ring, whose tooltip now reads like `Claude 5h 6% · resets in 4h 1m`; the TUI statusline and the board cards get a one-cell coloured mark from the same table. `GET /api/sessions` rows gain `provider`, `parent_id` (the session a fork came from), `diff_added`/`diff_removed` (working-tree lines against HEAD) and `branch`, measured in the background so the listing never waits on `git`. Forks nest under their parent, a session whose last turn failed shows a red dot (previously rendered nowhere), each project shows a `+N` change total, and the collapsed icon rail tints each thread working, needs you, done or failed and gains a Browser button. On the phone Fleet each card shows its project and a branch chip with the dirty-line count, a project chip row narrows the list, an Offline hosts section explains hosts that are missing, and a draft dot marks unsent text.
- **The desktop sidebar is organised the way you work.** Project groups fold (remembered across reloads), a project shows a New thread button on hover or focus, rows rename in place (double-click or F2), and the right-click menu forks, opens a worktree thread, archives and deletes, saying why an action is unavailable instead of hiding it. A sort toggle switches between last updated and newest first, the sidebar drags between 200 and 360 px (double-click the edge resets it) and remembers its width, and the footer gains Schedules, What's New with an unread dot, and a phone-pairing shortcut. Workspaces group projects: the switcher at the top of the sidebar narrows the sidebar, the Inbox and Schedules to one workspace, and each project's menu adds it to a workspace.
- **The app shows the follow-up queue above the composer and lets you edit it.** Every prompt sent while a turn runs stays listed with its position, and each row can be pulled back into the composer to edit, moved earlier or later, run now (it stops the turn and starts that prompt, like `Esc` in the terminal) or removed. A prompt held on the device while the connection is down gets the same rows. The moves are the new `queue_move` and `queue_send_now` remote inputs, checked against the row you saw so an edit against a queue that has since shifted is refused, and a move reorders what the running turn will absorb to match. Removing a queued prompt also drops its "sent" bubble, which used to linger and then raise a "message didn't confirm" toast.
- **`[chat] follow_up = "steer" | "queue"` sets what Enter does while a turn runs, in the terminal and the app.** `steer` (the default) is today's behaviour: the prompt is queued and also handed to the running turn at its next step. `queue` only queues it, so a long turn stays on its original task. In the app `Cmd/Ctrl+Enter` does the opposite of the setting, a small `Enter: steer` toggle sits under the composer while busy, and the setting is also `Enter while busy` in Settings, Configuration. The app sends the new `queue` remote input for it, and an older daemon that does not know it is never sent it.
- **`ArrowUp` and `ArrowDown` recall earlier prompts in the desktop and web composer, with the terminal's rules.** `ArrowUp` in an empty composer, or with the caret at the very start, brings back the newest prompt of the session, then older ones; consecutive repeats are collapsed, `ArrowDown` steps forward and past the newest restores what you had typed, and any edit ends browsing. A caret in the middle of a draft or on a lower row keeps moving the caret.
- **Attachments are part of the draft, and files can be dropped onto the composer.** A finished upload survives a reload or a switch to another session along with the text, instead of vanishing. Dropping files on the desktop or web composer attaches them (the desktop window no longer swallows OS file drops), and attachments show as tiles with a thumbnail for images and a page tile for PDFs, in the composer and in the sent message.
- **`Ctrl/Cmd+Z` and `Ctrl+Shift+Z` undo and redo what the composer inserted itself.** A voice transcript, an `@file` mention, a command chip or an accepted suggestion can be taken back, which the browser's own undo could not do for a value set from code. In the terminal prompt `Alt+_` redoes what `Ctrl+_` undid.
- **The composer names the model that will answer, shows the mode, and knows the side-question commands.** The empty placeholder reads "Ask deepseek-v4.1-flash anything about this workspace" (just "message..." before a model is known), a mode chip beside the effort chip opens the daemon's own `/mode` picker and shrinks to an icon below 420 px, `/btw` and `/side` complete in the command list, and a side question no longer leaves a "sent" bubble that no history row would ever claim. A message that invoked a skill carries a small `skill` badge.
- **The quick composer opens from any app.** The desktop app registers the Quick Composer chord (Alt+Space unless you rebind it in Keyboard shortcuts) with the operating system, raises the window and drops you straight into the composer, even from the tray with the window hidden; a chord pressed while Forge is still starting is held and delivered once the app can show it. When the system refuses the chord (another app owns it, or a Wayland session has no global shortcuts) the shortcuts row says so in place instead of failing silently, and `forge-desktop --quick-composer` does the same thing for a compositor binding.
- **Launching Forge twice gives you one window.** A second launch focuses the running app instead of opening another daemon probe, and forwards `forge://session/<id>`, `forge://new`, `forge://inbox` and `forge://fleet` links to it. A new Desktop page in Settings adds Launch at login and Start minimized (a login launch then stays in the tray), a switch for background notifications (on by default, only while the window is unfocused) and a usage refresh interval (default 5 min, 2 to 60).
- **A crash now says what crashed and what to do.** The crash screen is labelled startup, render, uncaught error, unhandled rejection or renderer loop, offers Reload and Copy diagnostics (a scrubbed summary with no tokens, URLs or paths) plus Check for updates and Install update on desktop, and warns when reloading has not helped. If the app has not become interactive after 15 seconds it asks whether to keep waiting, restart or look for an update. On Linux a dead WebKit renderer is reloaded automatically, and stopped after three crashes in a minute so a crash loop ends on the crash screen instead of a blank window.
- **The tray icon follows the system theme.** It is a monochrome Forge mark in dark or light ink and switches when the theme does (a template image on macOS), and its session list now puts waiting and running threads first and then the most recently touched.
- **A release can carry a headline and summary chips, and the app shows them.** Two optional comment lines under a release heading (`<!-- headline: … -->`, `<!-- chips: A | B | C -->`) become `headline` and `chips` in `GET /api/changelog`; What's New renders them as a hero block above the newest release's notes, and the post-update sheet is now titled "What's new in vX" with the same block. `docs/architecture/versioning.md` explains which version number is which, the OTA rules and the Android minSdk policy, and `mobile/PERF_RULES.md` lists the rendering and network rules the app follows.
- **Digits 1-9 pick an answer row in a question card on the desktop and web apps.** The first nine rows show their number, and tapping a selected single-choice row again clears it.
- **The usage dock shows where a quota is heading.** Under each window's reset line a pace dot reads "runs out 1h 30m" or "≈75% by reset", worked out from the burn rate so far, for a window the daemon flags or one that would be empty before it resets.
- **A laptop daemon stays awake while it works.** `[power] keep_awake = "off" | "while_working" | "while_remote" | "always"` (default `while_working`) holds an OS sleep inhibitor (`systemd-inhibit` on Linux, `caffeinate` on macOS, `SetThreadExecutionState` on Windows) while a turn is running and for 30 seconds after, or while an app is connected for `while_remote`. Settings has a "Keep this computer awake" switch that says the screen can still lock, and Diagnostics shows whether the inhibitor is held or why it could not be. The Linux guard command ends with the daemon, so a killed daemon cannot leave the machine unable to sleep.
- **The session store is copied before every migration and checked at start.** Opening a store that needs a schema migration first copies it to `forge.db.pre-v<N>.bak` (SQLite online backup, the two newest kept), so a build with the wrong schema can no longer take the only copy of your history. `forge serve` runs `PRAGMA quick_check`; a damaged store is renamed `forge.db.corrupt-<time>`, the newest readable backup is put in its place and Diagnostics reports it for a week.
- **A store that will not open gets a recovery screen instead of a dead daemon.** `forge serve` stays up on its port and answers every route with `503` and `code: "store_recovery"`; the desktop, web and phone apps then show a screen with the error, a Restore button per backup, "Run migration again" and, on the desktop, "Open data folder". The store a restore replaces is kept as `forge.db.replaced-<time>`, and the daemon carries on starting once the store opens.
- **`forge backup` copies the session store, lists the copies and restores one.** `forge backup` writes a consistent snapshot (default `backups/` beside the store, `--out` to choose), works on a store this build refuses, `forge backup list` shows every copy, and `forge backup restore <file>` replaces the store while the daemon is stopped, keeping the old file.
- **`forge update --rollback` puts back the build the last update replaced.** An update now keeps the old binary beside the new one as `forge.prev` (one extra binary of disk); rolling back swaps the two, and rolling back again undoes it. Package-manager installs are pointed at their own tool.
- **A restarting host says so.** On shutdown the daemon lets running turns finish, then closes every session, fleet and terminal socket with WebSocket code 1001 ("host restarting"). The apps keep retrying every second, show "host restarting" in the session's status line and a banner instead of "unreachable", and toast it once.
- **`[host] max_active_sessions`, `max_terminals` and `max_helpers` cap what one daemon runs.** A session over the limit is refused with `429` and `code: "host_cap"`, a terminal over the limit closes its socket with code `4429`, and both reach the apps as a toast; helpers over the limit wait for a free slot instead of failing the turn. Every limit is off (`0`) by default.
- **A second `forge serve` on the same config directory stops and names the first.** The daemon takes an advisory lock on `serve.lock`, so the refusal reads "another forge serve is already running for <dir> (pid 4242)" and a crashed daemon never leaves a stale lock behind.
- **`forge service logs` shows the daemon's output.** `-n` sets the line count and `-f` follows; it reads the user journal on Linux and `serve.log` in the data directory on macOS.
- **Forge runs as a container.** `packaging/docker/Dockerfile` installs the released binary (checksum verified) into a Debian slim image that runs `forge serve --lan --port 7420` as an unprivileged user under `tini`, with a health check and everything Forge keeps under `/home/forge`. `docs/features/host-durability.md` covers it with the rest of this host work.
- **On Windows, a shell's children die with the daemon.** Shell tool and background-job children are placed in a job object created with `KILL_ON_JOB_CLOSE`, so a crashed `forge serve` no longer leaves dev servers and build tools running.
- **A terminal that exits on its own cleans up after itself.** A shell that ends with code 0 closes its tab and selects the neighbour; any other exit keeps the tab, its output and a Restart button, labelled `exited (3)` with the code. The daemon now records each shell's exit code (`exit_code` on the terminal list and on the `status` frame) and drains the shell's last output before it marks the terminal exited, so `echo bye; exit 3` no longer loses `bye`.
- **Links in terminal output are clickable.** `http(s)` URLs open in the Preview dock in the desktop app (the system browser on web and phones), `src/a.ts:5` style paths open the Files dock at that line (the Files screen on a phone), and OSC 8 hyperlinks a program prints work the same way. A bare file name in `ls` output is left alone, and dragging across text to select it never counts as a click.
- **Ctrl+C and Cmd+C copy a selection in the terminal, and a second press within half a second interrupts.** With nothing selected Ctrl+C is the interrupt as before and Ctrl+Shift+C copies.
- **A background terminal tab shows when it has printed.** A dot marks output you have not seen, a `live` badge shows a tab that is printing right now, and an exited tab shows its code. The daemon's terminal list carries an output counter (`sequence`) for this.
- **A session's first terminal starts when the session is created, and its terminal tabs come back.** The dock opens onto a shell that is already running, and reopening the app or switching sessions restores that session's tabs from the daemon, keeping each tab's history. `[terminal] prewarm = false` turns the early start off; a host with `[host] max_terminals` set never starts one early.
- **`[terminal] shell`, `shell_args` and `agent_shell` choose the shells.** `shell` and `shell_args` set what a new terminal tab runs (default: `$SHELL`, or `%COMSPEC%` on Windows). `agent_shell = "auto" | "sh" | "bash" | "pwsh" | "powershell" | "cmd"` sets the shell the agent's `shell` tool uses; the PowerShell choices run with `-NoProfile -NonInteractive`.
- **Appearance has a Terminal section: text size, scroll speed and a close prompt.** Text size runs 11 to 16, scroll speed 0.5× to 3× on the desktop and web apps, and closing a terminal tab whose shell is still running asks first (turn it off in the same place).
- **Ctrl+F or Cmd+F finds text in whatever you are looking at: the conversation, the git diff or the open file.** One find bar (query, hit count, match case, previous and next, Esc to close) opens on the surface you last clicked in, and the transcript search also asks the host for matches in pages you never scrolled to (`GET /api/history?q=`), so a word that only appears 400 messages back is found and jumped to. Stepping to a hit loads the window around it with one request (`?around=<seq>`) instead of paging through everything between, and "Latest" brings you back; a host that predates either parameter falls back to paging. On a phone, Find is in the session menu. The terminal registers with the same bar once its scrollback is searchable.
- **A sticky bar over the transcript quotes the prompt the visible turn answers, and a prompt rail outlines the whole conversation.** Press the bar to scroll back to that prompt. On wide windows with a mouse, thin ticks down the right edge mark every user prompt of the session, including ones that were never paged in; hover reads the prompt, a click jumps there. The rail hides under two prompts and on touch screens.
- **`Ctrl+F` finds text in the full-screen terminal transcript.** A bar on the last row takes the query as you type (smart case), `Enter` and the arrow keys step between matching rows, every occurrence on screen is underlined and the current row is filled, and `Esc` closes it where you are. `/` is left to slash commands.
- **The app says when history is incomplete, and why a turn failed.** Reconnecting after the host could no longer replay what you missed, or a session whose stored record has holes (`GET /api/history?integrity=1`), shows a dismissible "History incomplete" dock above the composer; a failed turn shows a dock with a title (Rate limited, Network error, Sign-in problem and so on), the one-line cause and the host's full message folded behind it (`thread_error` on the snapshot).
- **The session header shows the socket's state.** A chip reads Live, Reconnecting, Paused in background, Offline or Ended (a dot alone on a phone while live).

### Changed

- Release builds compile fewer crates: unused dependencies are gone, syntect, qrcode and rcgen drop features Forge never calls, and tokio's test clock is only built for tests. A CI-style release build is about 14% faster.
- **A tool group opens by itself while a call in it is running and folds once the run is done.** Your own tap still wins: a group you closed stays closed.
- **Shell rows are titled by what they do.** The `bash -lc '…'` wrapper and a leading `cd dir &&` are dropped, and `sed -n 'a,bp' file` reads "View file a-b". File paths in tool rows keep their end (`…/deep/dir/name.rs`) instead of losing the file name.
- **A very long reply stops re-laying out all of its Markdown on every frame while it streams.** Past 16,000 characters only the newest part keeps Markdown layout, the older part shows as plain text, and the whole reply renders as Markdown once the turn settles.
- **The mobile app's Expo SDK 57 packages are at the patch versions Expo now expects.** Twelve patch bumps (`expo` 57.0.31 and eleven `expo-*` modules), applied by `package.json` and a lock regenerated in place; `expo-doctor` in PR CI had started failing on a clean `main`, which made every mobile pull request unmergeable.
- **A narrowed desktop window keeps the desktop layout.** Between 640 and 1023 points wide the app now shows the sidebar as the icon rail, with the docks and split panes intact, instead of falling back to the phone tab bar; the rail's expand button opens the full sidebar over the content until you navigate. Phone widths and the web build are unchanged.
- **Account-wide usage refreshes every five minutes instead of every 30 seconds.** The interval is the new Usage setting on the Desktop page; a running session's own usage still updates live.
- **A dropped connection retries with jitter and looks for a moved daemon.** Reconnect delays are spread by 25% either way so a restarted daemon is not hit by every session at once, and every third failed attempt makes the desktop app re-detect a local daemon that came back on a new address. `forge run --help` now says what a run does with a permission prompt when stdin is not a terminal: it denies the call.
- **The generated systemd unit is hardened.** `forge service install` now writes `NoNewPrivileges=true`, `ProtectSystem=full`, `LockPersonality=true` and `RestrictSUIDSGID=true`: nothing an agent runs can gain privileges through setuid, and `/usr`, `/boot` and `/etc` are read-only to it, while your home directory and projects stay writable. A drop-in loosens it for a workflow that needs `sudo`. Re-run `forge service install` to pick it up.
- **The terminal measures its real glyph width and settles resizes.** The column count comes from a measured character width instead of a 0.6em estimate, so it stays right after a font or zoom change. Resizes wait for a drag to settle once the scrollback is large, and a terminal reattached after a reload gets a one-column nudge so a program that repaints in place redraws at the right size.
- **The protocol-mismatch banner says the pairing is kept.** It names both versions and which side to update, so an out-of-date host or app no longer reads like a reason to pair again.

### Fixed

- **The macOS desktop app compiles again.** The tray now follows the system theme through `Window::app_handle`, which needs `tauri::Manager`, but that trait was imported only on Linux and Windows, so both macOS bundles of v2.25.1 failed to build.
- **Desktop releases build again.** The new desktop plugins moved the Rust `tauri` crate to 2.12 while `@tauri-apps/api` stayed on 2.11, and `tauri build` refuses that split, so every desktop bundle of v2.25.0 failed. Both are on 2.12 now, and PR CI checks that they share a minor.
- **Forge Anywhere no longer drops the relay link in a loop over one stale envelope.** A host that received an envelope whose sequence it had already accepted closed the whole relay connection; the relay redelivered the same envelope after the reconnect, so the host went down again every minute and phones and desktops showed it as offline. The envelope is now discarded and logged with its sender and sequence.
- **Resizing the desktop window from desktop to phone width no longer throws a render error.** The Fleet tabs swapped a stack navigator for a tab navigator when the window crossed the breakpoint, handing the tab view a state with no `preloadedRouteKeys`; one tab navigator now serves both widths and the tab bar is simply hidden on wide windows.
- **File names with underscores are no longer italicised in replies.** `src/my_file_name.ts` read as `my` + italic `file` + `name.ts`; an underscore inside a word now stays literal, as in standard Markdown.
- **A long edit call no longer loses the name of its file in the chat.** The stored call arguments are capped at 200 characters and were written alphabetically, so a big `new`/`old` pair pushed `path` past the cap and the row showed no file; identifying keys (`path`, `command`, `pattern`, …) now come first, and an `ask_user` form is kept whole.
- **A denied microphone says "Microphone access is off" and offers Open settings.** The phone apps open the system settings, the web and desktop apps say where to allow the microphone, and the message stays until dismissed instead of vanishing after a moment.
- **A phone browser's keyboard no longer covers the field in a sheet.** The sheet now rides up by the part of the screen the on-screen keyboard takes, read from the browser's visual viewport.
- **The server list survives a kill mid-save, and turning push off while offline is retried.** The list is written to a staging key first and promoted afterwards, so a half-finished save is finished on the next launch instead of reading as no servers, and an unsubscribe the daemon could not be told about is replayed on a later open.
- **The fleet socket noticed a closed app only on the next change or ping.** It never read from the socket, so a vanished client lingered for up to 25 seconds; it now drops as soon as the client goes.
- **Typing the same character twice in a row in the desktop and web terminal no longer drops the second one.** `ll` and `//` reached the shell as `l` and `/`.
- **Switching terminal tabs no longer leaves the status line on "disconnected".** A socket the dock had already replaced could report its close after the new one connected, and its late output could land in the new terminal's scrollback.

### Security

- **Git remotes and clone URLs the daemon accepts are validated.** `ext::`, `fd::` and other `<helper>::` URLs, `file:` URLs, bare local paths, and anything beginning with `-` or containing control characters are rejected; only an existing remote name, an `https`, `http`, `ssh` or `git` URL, or scp-style `user@host:path` passes, and the daemon's network git commands run with `GIT_ALLOW_PROTOCOL=https:http:ssh:git:file` so a URL already in a repository's config cannot reach `ext`. The validator is shared for anything else that takes a remote.

## [2.24.1] - 2026-10-28

### Added

- **Every subagent a session ever started is reachable from the apps, not just the current batch.** The Agents tab lists the live batch first, then every earlier child grouped by turn, newest first, and a `spawn_agents` or workflow row in the chat gets a chip per child it started that opens it. The host serves the list at `GET /api/sessions/{id}/agents` (also bridged through Forge Anywhere); chat rows are matched to children by the time window the call spanned, so rows from before this release link too. A host without the route keeps showing the live batch only. Back from a subagent now returns to its parent even in a window narrowed from the desktop layout.
- **A subagent opens as its own session, rendered exactly like the main chat.** In the full-screen TUI press `Ctrl+O`, pick an agent with `↑↓` and press `Enter` (in a workflow's view, `Enter` on an agent does the same): its conversation, tool rows, markdown and scrolling come from the same transcript renderer as your session, under a `main chat › explore · model · running` breadcrumb with its task. A running child updates as it saves each message, a finished one reads from the store, and that includes children from a session you resumed. `Esc` returns to the main chat exactly where you left it (scroll, panels and queue untouched), `↑↓`/`PgUp`/`PgDn`/`Home`/`End` or the wheel scroll the child, and `Ctrl+T` expands its tool calls. The agents panel and the `Agents` line in the transcript now say how to open one. New `forge_core::stored_replay_items` reads any session's persisted conversation as replay items (`crates/forge-tui/src/app/child_view.rs`, `crates/forge-cli/src/cli/commands/run/child_view.rs`, `crates/forge-core/src/replay.rs`).
- **A subagent opens as its own session in the desktop, web and phone apps.** The Agents tab and the running-agents pills above the composer now open each child as a nested, read-only session that renders with the same chat as any other: messages, tool rows and groups, markdown and scrolling, updating while the child works. The header reads `parent session > explore` and takes you back, the composer is replaced by a note that the child works for its parent, and the child never appears in the session list. It rides the existing `/ws?session=<id>` and `/api/history` routes (so it works over Forge Anywhere unchanged), and a host without the new `session_id` on each agent row keeps the cards as plain, non-clickable rows.
- **Releases no longer need a version-bump PR, and the release pipeline builds entirely on GitHub-hosted runners.** `scripts/release.sh vX.Y.Z` tags main and dispatches `release.yml`, which stamps the version into `Cargo.toml`, `Cargo.lock`, `mobile/src-tauri/tauri.conf.json` and the changelog inside the build; the package-manifest PR then carries main up to the released version. All five CLI targets (x86_64 and aarch64 Linux inside the pinned glibc 2.31 container, macOS, Windows) build in parallel without waiting for a self-hosted runner, the upgrade/reconnect/rollback gate now runs inside the release instead of on every push to main, and a `dry_run` dispatch rehearses the whole release without publishing. Details in `docs/ci/releasing.md`.

### Fixed

- **pulldown-cmark 0.13.5 fixes RUSTSEC-2026-0414** (exponential event growth with `ENABLE_WIKILINKS`, high); the lockfile moves off 0.13.4.

- **The shell tool refuses a fixed `sleep` of 30 seconds or more and points at polling.** A model waiting on a remote job ran `sleep 540; ssh host check` and sat for minutes whether or not the job had finished. A standalone `sleep N` (also `10m`, `1m 30s`) at the top level of a command, or in a `commands` batch, is now refused before anything runs, with the check part of the command echoed back as a `poll_until_exit_zero` example; sleeps inside `while`/`until`/`for`/`if` bodies, groups, subshells and `&` background jobs, and anything under 30 seconds, still run. The tool description, the base prompt and the bridge preamble now say to poll for the condition, never sleep (`crates/forge-tools/src/shell/wait.rs`).
- **The Git review dock works for any session the chat can show, and says what is wrong.** The dock looked a session up only among the ones the daemon had loaded, so a saved session that was idle or left over from before a restart answered "no such session" and the app showed "pairing invalid, re-scan" with a Retry button that could never work; the status, diff, branch and index routes now fall back to the session's saved worktree or folder (a detached-HEAD worktree included). The app also shows the daemon's own message for a missing session instead of the pairing error, which is now kept for a wrong token only, and the dock shows a plain "not known to this server" or "not a git repository" state.

- **Subagents can write.** `spawn_agents` children were limited to `read_file`, `list_dir` and `search` whatever their type, and any call that needed approval was silently denied, so asking agents to write files failed under the default temper. An agent type with no `tools:` field now inherits every tool, as in Claude Code (imported agents included); a request with no or an unknown `agent` runs the new built-in `general-purpose` type (all tools); `explore` and `plan` stay read-only; and the listing the model sees says which types are `all tools` or `read-only`. A child still never exceeds the parent's temper (`plan` blocks its writes, the safety denylist applies). When a child's call needs approval, the question now appears on your own prompt labelled with the child, one at a time (also through the claude/codex bridge); with no one to ask (`forge run`, detached children) the call is refused with a message saying so rather than that the agent lacks the tool. Children edit your tree in place unless worktree isolation is on, in which case their changes are merged back (`crates/forge-core/src/subagent/requests.rs`, `execution.rs`, `child_ask.rs`, `crates/forge-config/src/agent_builtins.rs`, `crates/forge-cli/src/mcp_serve/permission_relay.rs`).

- **A subagent's session takes your messages, and `Ctrl+O` then `Enter` always opens it.** Enter on the main-chat row used to open the old line pager (and `→` from there switched to the agents), so the nested session never appeared. The agents panel now has one path: Enter on an agent opens its session, rendered with the same `you` block, message gutters, tool rows and markdown as the main chat, under a `main chat › agent` breadcrumb with the chat's own composer and status line. The composer names who you are talking to (`Message explore…`); what you send runs as a follow-up turn of that agent's own session and its reply streams in and is persisted there. `←`/`→` (with an empty composer) cycle through the main chat and every agent, each keeping its scroll, and `Esc` returns to the chat where you left it. A message sent while the chat or the agent is still working stays in the composer with a note instead of starting a second loop.

- **The desktop performance fixture can set its window title.** `perf-fixture` called `setTitle`, which the main window's capability did not allow, so it raised an unhandled rejection (caught by the new bug reporting as issue #1713); `core:window:allow-set-title` is now granted (`mobile/src-tauri/capabilities/default.json`).

- **The hosted benchmark data loads from forge.adulari.dev.** The site's proxy to the `benchmarks-data` release answered 502 because GitHub's redirect headers outgrew nginx's default header buffer; both proxy blocks now use 32k buffers, so clients get the mirror from the site instead of falling back to the GitHub URL (`crates/forge-relay/deploy/forge.adulari.dev.nginx`).

- **The desktop and web composer grows with your message again.** It measured the right height for wrapped text, but the textarea's `flex: 1` (a 0% basis inside a column) overrode that height, so the box stayed one row tall and older lines scrolled out of view. The web textarea now sizes from its measured height, up to six lines and then scrolling (`mobile/src/components/chat/Composer.tsx`).

## [2.24.0] - 2026-10-27

### Added

- **Forge now reports its own bugs, automatically and without your content.** A panic, an internal error that ends a turn, a provider reply Forge cannot parse, a failed compaction, a store error, and in the desktop and phone apps a React error boundary, an unhandled rejection or a desktop panic, is classified (bugs are reported; rate limits, bad keys, network trouble, interrupts and config errors are not), scrubbed (no prompts, replies, code, tool arguments, paths, hostnames, emails or keys; home becomes `~`, other paths keep only the basename, URLs keep only a public host, quoted strings are dropped), fingerprinted by error kind and normalized in-crate frames rather than message text, queued on disk (bounded, offline-safe) and sent in the background with short timeouts to a new `POST /v1/reports` endpoint on `forge-relay`. The relay validates, rate-limits per install, per IP and globally, deduplicates by fingerprint, keeps counts per version and OS in SQLite, and opens or updates one GitHub issue per problem (`auto-report`), commenting only when a new version is first affected and reopening a closed issue that recurs on a newer version. On by default with `[reporting] mode = "auto" | "ask" | "off"` (`ask` shows a card with the exact report: Send, Don't send, Always send), toggles in `/config`, Settings and `forge report --mode`, and `FORGE_NO_REPORTS=1` forcing it off. `forge report --show-last` prints the last report exactly as sent, `--sent` lists what left the machine, `/bug` (TUI), `forge report --message` and "Report a problem" (apps) send your own description with a preview first, and a 429/503 from the relay pauses sending. New crate `forge-reporting` (`mode()`, `set_mode()`, serde `ReportingMode`) is the API the onboarding step uses. See `docs/features/issue-reporting.md` (`crates/forge-reporting`, `crates/forge-core/src/error_report.rs`, `crates/forge-cli/src/reporting.rs`, `crates/forge-relay/src/reports.rs`, `mobile/src/lib/reporting.ts`).

- **Forge updates itself on launch.** New `[update] auto = true` (off with `auto = false` or `FORGE_NO_AUTO_UPDATE=1`; `[update] check = false` / `FORGE_NO_UPDATE_CHECK` still disable everything). On an interactive launch only (a terminal on stdin and stdout, not CI; never pipes, headless or `mcp-serve`): a standalone install with a newer release in its cache prints `Updating Forge 2.23.7 → 2.23.8…`, downloads, verifies that the new build runs, swaps it in and re-executes with the same arguments, all within a 20 s budget (`FORGE_AUTO_UPDATE_BUDGET_SECS`); on a timeout or failure it launches the current version and tries again next time. With nothing cached yet the session stages the download in the background (`staged-update/` in the data dir) so launch stays instant, and the next launch swaps it in. The first session after an update opens with `Updated Forge X → Y · what's new: …`. Homebrew and Scoop installs upgrade in the background during the session and say it applies next launch; AUR, cargo and system installs, which need root or a build, keep the update card (`crates/forge-cli/src/update_auto.rs`).
- **The daemon updates itself when idle.** `forge serve` checks on start and every few hours; a standalone install with a newer release replaces its binary only while no session is mid-turn, then restarts the service (a foreground `forge serve` keeps running until restarted by hand). `GET /api/update` reports `auto_update` so a client knows not to trigger it (`crates/forge-cli/src/serve_update.rs`).
- **Forge Desktop updates itself on launch.** A build that can replace itself (a mounted AppImage; the updater plugin's silent install on macOS and Windows, untested here) holds the main window, checks first with a 4 s timeout, and when a release exists shows a small `Updating Forge…` window, installs within 20 s and relaunches before the main UI appears; any failure or timeout opens the app as before and retries next launch. During a session it checks every 6 hours and installs in the background without asking, leaving `Updated to X - restart to apply` as a courtesy. When the local Forge CLI/daemon is outdated and can update itself, the app triggers `POST /api/update` itself once the daemon is idle (loopback only, once per version) unless the daemon reports `auto_update`. deb/rpm/AUR installs keep the banner. `FORGE_NO_AUTO_UPDATE=1` / `FORGE_DESKTOP_AUTO_UPDATE=0` turn the install off (the banner asks), `FORGE_NO_UPDATE_CHECK=1` turns the check off (`mobile/src-tauri/src/auto_update.rs`, `mobile/src/lib/updateBanner.ts`).
- **The desktop app sets Forge up for you.** On a computer with no `forge` command, the app downloads the platform build from the latest release, checks it against the release `checksums.txt`, installs it (`~/.local/bin`, or `%LOCALAPPDATA%\Programs\forge`), installs and starts the background service and connects, with progress in the window; then it walks the same shared setup as the terminal in a new `/onboarding` screen: a step rail beside a content card (a progress bar and one column on a narrow window), each step opening with what was found, one primary button and the rest under details. **Set up everything recommended** is the welcome's action, with a live checklist that pauses on a sign-in until you finish it. Claude Code, Codex and xAI sign-ins run inside the daemon and show the sign-in page, a device code and a QR code (a new dependency-free encoder, `mobile/src/lib/qr.ts`), so no token ever reaches the app. Existing installs see it once on the first launch of this version, and a desktop install whose Claude Code has no Forge token gets a **Reconnect Claude Code** banner that runs the same sign-in. The daemon serves it at `/api/onboarding` (plan, state and detected machine), `/steps/{id}`, `/actions`, `/jobs/{id}` (long poll) with input and cancel, loopback only and absent from the Forge Anywhere allowlist; no response carries a credential. Provider rows for Claude Code, Codex and Antigravity now report their real login state (`ready`, `missing`, `unverified`) instead of always `unverified`, and `SetupProgress` became the design system `Stepper` (`crates/forge-cli/src/serve_onboarding.rs`, `mobile/src/app/onboarding/`, `mobile/src/components/onboarding/`, `mobile/src-tauri/src/onboarding.rs`, `docs/features/onboarding.md`).
- **The full-screen TUI offers updates instead of printing a line.** When a newer release exists, a card sits above the composer with the version, the first lines of its release notes and the release link, and three keys: `u` Update now, `l` Later, `s` Skip this version (`crates/forge-tui/src/update_card.rs`, `crates/forge-cli/src/cli/commands/run/update_flow.rs`). A standalone install replaces itself in the background with progress on the card (no download bar over the TUI); a Homebrew, Arch (`forge-bin`), Scoop or cargo install shows its exact command and runs it with the terminal handed to the package manager (`c` copies it). Afterwards the card says `Updated to X · restart Forge to use it`; `r` re-executes Forge on the new build and resumes the session, and `v` restarts the background service when a service is running and no session it hosts is mid-turn. The card never appears over a running turn, and its letters stay in the composer while it holds a draft. Plain chat gets the same skip (`Update now? [y/N/s]`).
- **Update checks run in the background every 6 hours.** On start and every 6 hours while running, never blocking, with the network asked at most once per 6 hours across launches (was once a day). What was found and the skipped version live in `update-state.json` in the data dir, not in config; "Later" lasts one process, so the next launch offers again from the cached release without waiting on the network. `[update] check = false` and `FORGE_NO_UPDATE_CHECK=1` still turn it all off. `FORGE_UPDATE_FEED_URL` points the lookup and the download at another releases API, and `FORGE_UPDATE_TARGET` points the self-update at another file (tests).
- **The daemon reports and applies its own update.** `GET /api/update` returns the running version, the newest release, whether it can update itself and the exact command when a package manager owns it; `POST /api/update` (loopback only: refused when `Host` is not loopback or a tunnel's forwarding headers are present) replaces a standalone binary and then restarts the service once no session is mid-turn. `GET /api/identity` gains `update_available` so phones can show a daemon-outdated notice (`crates/forge-cli/src/serve_update.rs`).
- **Forge Desktop shows update banners with real actions.** Instead of a `View` link, the banner offers Install (an install that can replace itself) or Open release page, `Forge (CLI) X is available on this computer` with Update (the daemon's self-update), and `Updated to X, restart to apply` with Restart. Skip this version is remembered; dismissing a banner hides it for a day. An AppImage that installs in the background no longer restarts under you: it says so and waits for Restart. `FORGE_DESKTOP_AUTO_UPDATE=0` still turns the background install off, and the banner's Install button asks instead (`mobile/src/lib/updateBanner.ts`, `mobile/src/components/UpdateBannerHost.tsx`, `mobile/src-tauri/src/auto_update.rs`).
- **Benchmark-driven ranking now works without an Artificial Analysis key.** Forge reads a published copy of the scores (`https://forge.adulari.dev/data/benchmarks.json`, then the rolling `benchmarks-data` GitHub release), refreshed every three hours by the new `benchmarks-mirror` workflow, which transforms the feed in Rust (`forge benchmarks --export-mirror`), validates it and publishes it. Precedence is your own key, then the mirror (ETag, 6 h TTL, an unrated catalog model re-checks at most hourly), then a snapshot compiled into the binary, then the name heuristic; the cache records which source it came from. Set `mesh.benchmark_mirror = false` or `FORGE_BENCHMARK_MIRROR=0` to keep Forge off the network for this. `forge benchmarks`, the `/mesh` overlay, the Models screen and the README now credit "Benchmark data: Artificial Analysis" (`crates/forge-cli/src/benchmarks.rs`, `crates/forge-cli/src/benchmarks/`, `.github/workflows/benchmarks-mirror.yml`, `scripts/refresh-benchmarks-snapshot.sh`, ADR-0011).
- **The desktop workbench is visible, and it does more.** The split panes, docks and shortcuts the Machined designs describe existed but were reachable only by keyboard, and some were broken. A **Layout** button in the title bar (and the web top bar) now lists every pane, panel and preset with its shortcut, the same entries are in the command palette and the macOS View menu, and a one-time tip points at them. Up to four sessions share a window as two or three columns or a 2x2 grid (each at least 400px wide; on a narrower window the extra panes stay mounted but hidden, with a `+N hidden` note). Open a session into a pane by dragging it from the sidebar (drop on a pane, or on the New pane zone), right-click > Open in split, or Alt/Shift-click; drag a pane header onto another pane to swap; maximize, close and cycle focus with Ctrl+Shift+M, Ctrl+Shift+D and Ctrl+[ / Ctrl+]. Focus follows a click, and the docks follow the focused pane's session. New right-edge docks show a session's **Tasks**, **Agents** and **Plan** (Ctrl+Shift+T / A / L) beside the existing Files, Git review, Browser preview and Usage; docks stack as tabs that close individually and reorder by drag. Presets (Focus, Side by side, Grid 2 x 2, Review) and Reset layout set pane count, docks and sidebar in one step. Every seam is draggable with a visible hover state, resets on double-click and nudges with the arrow keys: between panes, on the left of the right dock (240-760px) and above the terminal. Layout, dock tabs and every size persist across launches (`forge.workbench.v2`, migrating the old `forge.splitPanes` entry). Phones and narrow windows are unchanged.
- **The desktop app's interface updates over the air.** A merge to main that changes the UI now reaches installed desktop apps within about half an hour, without a release: CI publishes a signed web bundle to the rolling `desktop-ota` prerelease (never "Latest", so the Tauri updater is untouched), and the app checks every 30 minutes, verifies it against the updater public key, unpacks it beside its app data and serves it on the next launch or from a "Reload to update" banner, on the same origin so your data survives. A bundle is only applied to a binary with the same runtime version, a hash of everything native the UI depends on, so a change to the Rust shell, its plugins or its capabilities still needs a full release. A bundle that never reports healthy is rolled back to the previous one (or the built-in UI) and not retried. Settings, Diagnostics & updates shows the active bundle and has Check now and Reset to built-in; `FORGE_DESKTOP_OTA=0` turns it off (`mobile/src-tauri/src/ota.rs`, `.github/workflows/desktop-ota.yml`, `docs/features/desktop-ota.md`).
- **Guided onboarding opens on first launch, once for everyone.** Bare `forge`, `forge chat`, `forge run` and `forge nl` in a terminal open a paged setup the first time a version with onboarding runs, including on machines that already work, where detection pre-fills what is there (working models, the service, imports) and "Looks good, finish" is one key. A machine with no working model reopens it every launch until one is connected; a working one can say "not now" twice, then only gets a one-line hint. `forge setup` (alias `forge init`) always opens the full flow, `FORGE_NO_SETUP=1` and non-terminal launches never do. **Set up everything recommended** on the welcome runs every decision-free action in sequence with a live checklist: reconnect Claude Code, safe defaults, imports, the background service (this computer only) and error reports. Steps cover models (Claude Code, Codex, Antigravity, ChatGPT and SuperGrok plans, free API tiers first, a local Ollama model sized to the machine, each with a live check), safety and cost with an example each, web search (keyless, local SearXNG container or a Brave key), benchmarks, bringing over Claude Code, Codex and Cursor settings, the service, Anywhere, the desktop app, notifications and theme, and error reports. Progress is `onboarding.json`, shared with the desktop app. The engine is the new `forge-onboarding` crate (state, detection under a 1.5 s budget, step catalog, serializable action jobs) so the desktop app can run the same flows through the daemon. Replaces the old `forge init` wizard and its `[Y/n]` prompts. See `docs/features/onboarding.md`.
- **Claude Code, Codex and xAI sign in with one action.** Forge runs the CLI's own login in a pseudo-terminal, shows the sign-in page URL and a QR code (so it can be finished on a phone over SSH), takes a code back through a masked field when the page shows one, and carries on by itself. For Claude Code it captures the token `claude setup-token` issues, stores it as `claude_oauth_token`, and checks it with a one-line bridge round trip; the token is never printed or logged, and a masked paste field appears only if it cannot be read. An install whose Claude Code has its own login but no Forge token is offered "Reconnect Claude Code", also as a small card at session start with Enter to connect, `l` for later and `n` to stop asking.
- **The installers hand off to setup.** `install.sh` and `install.ps1` run `forge setup` after installing when a terminal is available (`FORGE_NO_SETUP=1` skips it), `install-desktop.sh` and `.ps1` open the app when they finish, and the Homebrew formula, Scoop manifest and AUR package say to run `forge`. The AUR `url` now points at `Adulari/forge`.

### Changed

- **The claude bridge no longer shares your interactive `claude` login, and needs a token.** The bridged claude ran with its own config dir that symlinked your `~/.claude/.credentials.json`, so Claude's refresh lock did not cover it: when the 8-hour access token expired the bridged claude refreshed and rotated the single-use refresh token on its own (replacing the link with a file the next spawn's rebuild discarded), which invalidated your main login about once a day. The bridge home no longer links `.credentials.json` or `backups/` (an older Forge's link or copy is removed on the next start; your real file is never touched), and claude's own `.claude.json` there is kept across spawns instead of being recreated each time (that is what left an 84-byte backup in `~/.claude/backups` every spawn). The bridge now authenticates with `CLAUDE_CODE_OAUTH_TOKEN`; without a token or an `ANTHROPIC_API_KEY` it is treated as not configured: routing skips `claude-cli` quietly without benching it, a pinned claude model fails with the setup hint, and a session prints the hint once at startup. **To keep using the claude bridge:** run `claude setup-token`, then `forge auth claude_oauth_token` and paste the token (or export `CLAUDE_CODE_OAUTH_TOKEN` for `forge serve`), restart `forge serve`, and run `claude /login` once if your main login was already invalidated (`crates/forge-provider/src/claude_bridge_auth.rs`, `claude_bridge_home.rs`, `docs/features/persistent-bridge-transport.md`).

### Fixed

- **Background job notices are one quiet row in the app chat, and tool rows stop showing raw paths and wrapper names.** A job's exit used to print as a monospace line that wrapped over three lines (`⚙ job 3212062 ✓ exited 0 · exec /home/me/bin/agentlive-exec /mnt/…`). It is now a row like a tool call: a status glyph, `Job`, the program it ran (`agentlive-exec`) and `exit 2` / `killed` only when it did not succeed; tapping opens the full command (wrapped, monospace, `~` for the home folder), the job id, the log path with a copy button and the output tail. Home folders fold to `~` and long paths keep their filename across tool and job rows. A `run_code` script of one tool reads as that tool (`Run sleep 75; cat ~/.local/share/…`, `Write 2 files`, `Call list_tasks`) instead of `Script <command>` or `write · 2 calls`, and the internal `tool_call` wrapper is never named: it resolves to the tool it reached, so a group header no longer says `called tool_call 1 time`.
- **Forge never replaces a build you compiled yourself.** The new automatic updates treated any `forge` outside Homebrew, Scoop or `~/.cargo/bin` as a standalone release install, so a source build (for example `~/.local/bin/forge` symlinked into `target/release`) would have been overwritten by the next release, and a daemon running one would have restarted itself into whatever `cargo build` last produced. Builds without the release marker (`FORGE_DISTRIBUTION`, set only by the release workflow) are now a separate `source` install: no update card, no automatic update, no daemon self-update or restart, and `forge update` prints how to rebuild (`crates/forge-cli/src/update.rs`).
- **Switching back to a recent session in Forge Desktop is a visibility flip.** On a wide window the last three sessions stay mounted but hidden (`<Activity>`: state, DOM, scroll and drafts kept, sockets closed while hidden and resumed from the last revision on show), a rail click makes the session active in the same frame and the URL follows from an idle callback, and hovering a row fetches its transcript and mounts it hidden before the click. A session opened for the first time paints its header and tab strip first and mounts the transcript in the next frame. Measured off-screen (60 Hz Xvfb, median of 3 interleaved rounds, ms to DOM-final): recent session 74 -> 15, new session 57 -> 51 (header on screen after 35, was 57), hovered new session 56 -> 44. Memory with five sessions visited: RSS 1055 -> 1099 MiB, DOM nodes 1063 -> 2796. Phones and narrow windows keep stacked screens.
- **Desktop tab switches no longer wait on the router, and first visits are display toggles.** Switching Chat/Tasks/Agents/Review/Files/Terminal/Replay on web/desktop is now a local state change that paints at once; the URL follows from an idle callback (deep links, reload and Back still see the segment). Once a session has painted, the remaining segments mount hidden in idle slices and the Review and Files tabs' first fetch is prefetched, so a first visit costs no more than a later one. Measured off-screen (60 Hz Xvfb, median of 3 interleaved rounds, ms to DOM-final): first visit Tasks 20 -> 14, Agents 28 -> 16, Review 69 -> 25, Files 44 -> 18, Replay 28 -> 16; later switches 13-22 -> 13-18. Hovering a session in the rail now starts its history fetch before the click.
- **The desktop chat scrolls with the wheel and trackpad, and its scrollbar points the right way.** On web the transcript was an `inverted` list, which react-native-web draws with `scaleY(-1)` on the scroll container, flipping the scrollbar and fighting wheel and trackpad scrolling. The web transcript is now an upright list pinned to the newest row, with the same contract for the screen above it (offset from the newest end, "near the oldest row" loads the next page, jump to latest, previous/next response); an older page landing at the top no longer moves what you are reading. Native keeps the inverted list.
- **A turn that made several calls to one tool no longer shows phantom rows.** Parallel `run_code` calls arrive as all the call rows and then all the result rows; results were paired only against the newest call, so three calls rendered as five rows (three with no description, two of them argument-less orphans) under a header counting five. Results now pair first-in first-out, so each call keeps its own result, the header count matches the rows, and every row names what the script did.
- **Markdown tables in chat honour column alignment and scroll sideways.** `:---`, `:---:` and `---:` align their column; columns size to their text instead of being crushed to equal slivers, and a table wider than the pane scrolls horizontally.

- **Split panes no longer crush their content.** A pane could be dragged to 25% of the window, which left the session header, banner and composer clipped at about 300px; panes now keep a 400px minimum and a right dock can no longer widen past what the content column needs. The right dock had a fixed width, the terminal's height and the split ratio were forgotten on every launch, and the View > Notes menu item on macOS did nothing (removed).
- **An interrupted compaction no longer leaves "auto-compacting" on screen for good.** Interrupting a turn (Esc, or a queued prompt taking over) while it compacted aborted the task before `CompactionFinished` was sent, and nothing else cleared the band, so it stayed at 95% through every later turn, its timer restarting with each one, while the session went on running tools. Any turn end now clears it (`crates/forge-tui/src/app.rs`).

## [2.23.6] - 2026-10-25

### Changed

- **The mobile session screen is calmer and fits a phone.** Untitled sessions are named after their first prompt instead of `session db89c45e`; the meta line drops the host/transport words and the model's provider prefix on phones; the tab row fades at its cut edge and keeps the active tab in view. Chat prose is larger on phones (15/23); your own messages are a quiet filled block instead of a bordered card with an accent rail and a `YOU` label. Tool calls are one quiet line naming what they did (`Run cargo test`, `Read src/app.rs`, `Script make`), with `failed` marked once; a script's details show its commands and decoded source instead of escaped JSON, with the raw arguments one tap away. Scripts are counted by the calls they made (`Ran 9 shell commands`, `Ran 10 tool calls in 2 scripts`), as in the TUI. The composer is about half as tall: attach buttons share the model/effort row and the command chips appear only while you type. The response stepper no longer floats over the chat on phones.

### Fixed

- **Forge Desktop on Linux animates at the monitor's refresh rate on NVIDIA.** WebKitGTK paces `requestAnimationFrame` with `drmWaitVBlank`; NVIDIA's proprietary driver answers it with `EOPNOTSUPP`, so WebKit fell back to a fixed 60 fps timer and every frame took 16.0 ms, even on a 144 Hz monitor. The desktop executable now defines and exports its own `drmWaitVBlank`, which libwebkit's calls resolve to ahead of libdrm's. It passes calls the driver supports straight through (Mesa, AMD, Intel are unchanged) and only on `EOPNOTSUPP` waits for the next vblank itself, timed from the CRTC's mode (`mobile/src-tauri/src/vblank.rs`). On an RTX 4090 with a 144 Hz monitor under Hyprland this took animation from 60 to 143 fps, one frame per vblank (7.0 ms). `FORGE_VBLANK_DEBUG=1` prints the first calls to stderr.
- **The desktop updater no longer writes an AppImage over a non-AppImage install.** The updater plugin replaces `$APPIMAGE`, or the running executable when that is unset, so an AppImage extracted to a directory and started through its `AppRun` had its `usr/bin/forge-desktop` overwritten by the v2.23.3 AppImage. On Linux an update is now downloaded and installed only when Forge runs from a mounted AppImage file (`APPIMAGE` is an existing file and the executable lives under the mount point in `APPDIR`), both for the automatic update at launch and for Install and relaunch. .deb, rpm, AUR and unpacked installs say a new version is available and link to its release page instead (`mobile/src-tauri/src/auto_update.rs`, `mobile/src/lib/updater.ts`).
- **Mobile chat history stays on screen during long turns.** A live history refresh that fell more than a page behind (a tool-heavy turn writes several 15-row pages between refreshes, or the phone was away) dropped every loaded page, leaving only the running turn's tool group over a mostly empty screen; it now pages back to join what was loaded. The chat also keeps fetching older pages until it fills the screen, since one folded page left a large empty band and FlatList never asked for more.
- **Mobile streaming is smooth and keeps the whole reply.** Replies past 2 KB slid their start out of view because snapshots carry only the reply's tail; the daemon now sends `streaming_offset` and the app rebuilds the full text. The app commits at most one snapshot per frame, finished Markdown blocks no longer re-render (or re-highlight) per token, and over Forge Anywhere the connector paces session frames to 20 a second so the phone's per-frame decrypt no longer falls behind. Measured on a desktop build at phone width against a paced mock reply: median time between visible text updates 45 ms → 32 ms, p95 181 ms → 54 ms, worst 592 ms → 169 ms, text reflows that shrank the reply 149 → 16.
- **The running turn says what it is doing.** The large pull-to-refresh spinner beside "thinking…" (which floated over the composer) is replaced by one line with the turn's phase and elapsed time (`Thinking · 12s`, `Run cargo test · 4s`, `Reading the result · 2s`), from new additive `activity` and `turn_started_at` snapshot fields.
- **Opening a session is faster.** The warm-start cache persisted everything, including changelog responses and every loaded history page, and was re-serialized every second while a reply streamed; it now keeps only the newest history page of the six most recent sessions (652 KB → 50 KB in the test profile; session open after a reload 170 ms → 38 ms).

## [2.23.5] - 2026-10-25

### Fixed

- **Background (detached) subagents show in the agents panel.** `spawn_agents` with `detached: true` admitted its children and returned, but nothing reported them, so the TUI band and right rail and the app's Agents tab stayed empty while they ran. Each detached child now reports start (role, model), live progress and its outcome and cost through the session's background presenter sink, so it shows in the TUI and in the remote snapshot (desktop and mobile Agents tab) like a blocking spawn and stays listed after its turn ends. The store row is written before the panel hears the outcome, so `list_subagents` and the panel agree. `cancel_subagent` settles the row as failed, and the labeled result at the next turn boundary settles any row nothing else did without printing it twice (`crates/forge-core/src/subagent/detached.rs`, `detached_subagents.rs`). On CLI-bridge sessions `forge mcp-serve` writes detached children to the bridge sink too, files them under the bridged session so it delivers their results, and marks children still running when the bridge process exits as interrupted instead of leaving them `running`.

## [2.23.4] - 2026-10-25

### Added

- **The tray menu answers permission prompts.** When a session waits on a permission, the tray shows Allow and Deny for it, plus Review Decision… (questions and plan approvals get only Review Decision…). A click carries the server, session and prompt it was built for: a prompt that was already answered or replaced is never answered by an old click, the daemon re-checks the same prompt number, and a locked app opens instead of answering (`mobile/src-tauri/src/tray.rs`, `mobile/src/lib/trayDecisionCore.ts`).
- **The usage screen and usage panel show Claude and Codex plan limits.** Claude's 5-hour session, weekly all-models and weekly Fable-only limits, and Codex's 5-hour and weekly limits, each with percent used, reset time and how long ago it was seen; a limit not seen yet says so. The Fable-only weekly limit was parsed and then thrown away (`cli_stream.rs`), and the fallback mapping turned per-model limits into the all-models one. Per-model limits stay out of routing, so a full Fable limit does not block Haiku or Sonnet. `/api/usage` returns the last reading of every limit that has not reset and refreshes Claude and Codex limits in the background at most every 15 minutes.

### Changed

- **One server control on the fleet, switchable on the desktop.** The fleet showed two rows of server pills, and the filter row only ever matched the active server. Now there is one row: All (every server's sessions together, a daemon paired twice listed once) plus one pill per server with its status and waiting count; picking a server shows its sessions and makes it the target for new sessions. On the wide desktop layout the sidebar has the same pills and the title-bar server chip is a menu (All servers, each server, Manage servers…) that shows the real connection type and whether the server is reachable.
- **The full-screen TUI transcript is calmer to read, collapsed and opened (Alt+O).** A failed command appears once with its diagnosis and fix dimmed under it, instead of a second `shell failed <command>` row; scripts are counted by the calls they made (`Ran 8 shell commands`, not `Ran 3 scripts, ran 8 shell commands`); silence-guard `N steps since the last update` lines no longer split groups; `$HOME` shows as `~`; tool groups get a blank row from the prose around them; the composer border says `Running a command` instead of repeating it. Opened groups wrap long commands under the text column (at most 4 rows per call), keep the exit status right-aligned and never cut it (`runnin`), and show a `run_code` script as a parent with its calls nested under it instead of its raw JavaScript (`crates/forge-tui/src/app/group_rows.rs`, `group_body.rs`, `card_text.rs`).
- **Shortcut hints follow the platform and your bindings.** Linux and Windows hints said `⌘N`, `CtrlN` or "Command U"; every hint now formats the current binding (`Ctrl+N`, `Ctrl+K`).

### Fixed

- **Codex OAuth usage is subscription usage, not API spend.** Subagent and assay calls priced codex-oauth calls at Codex's API rates, so they showed as dollars; they now bill a subscription at $0, and older stored rows show as Included. Codex limit readings no longer vanish from the screen five minutes after they were taken.
- **The right sidebar (usage panel) can be opened.** Its only visible opener appeared after a plan limit had been seen, so most setups had no button at all; the sidebar footer and the collapsed rail now always have one, and the command palette has Toggle usage panel. The fleet's `$X today` added up the lifetime cost of the listed sessions; it now shows API spend since local midnight.
- **The "Installed Forge app" sheet no longer opens on every launch.** It compared versions before the desktop version had loaded, saw the client version first and reported an update.
- **Session ages survive a daemon restart.** Restored sessions took the restart time as their creation and last-activity time, and any snapshot change re-stamped activity, so every session showed the same age and they aged together. Both times now come from the store and only real session events count as activity.
- **Answering No to a question no longer forces an implementation pass.** After an `ask_user` answer the empty-diff nudge re-drove the turn with "You have not modified any files. Implement the fix now"; a turn that asked the user now ends on the answer.
- **Chat fixes found driving a live session:** a successful tool call whose output mentioned "error" or "failed" showed as failed; a streamed sentence and a follow-up queued mid-turn were each painted twice; the jump-to-latest button was hidden once a session had more than one response; Markdown tables rendered as raw pipes; ordered lists past 9 pushed the dot onto its own line; diff tints stopped at the end of the text; git review showed one clipped file row; the permission card pushed the composer off a short window, read "13S", and a shell permission never named the command (the daemon now sends it).
- **Desktop shell fixes:** the desktop Inbox listed nothing; Notices said "back to Inbox" from the sidebar; sidebar times froze; footer buttons did not show the current screen; the collapsed rail lacked New session, Inbox, Notices and History; the title-bar host name overlapped the server chip on narrow windows; the sidebar usage rings carry their percent instead of looking like spinners; Workflows was unreachable; Session tree and the Anywhere hub lacked the Settings rail; biometric unlock and haptics toggles that do nothing on desktop are hidden; a provider's status dot contradicted its expired badge; the Connect screen showed two Anywhere prompts; tray "Check for Updates…" opened a page without an updater; the About panel never showed the daemon version.

## [2.23.3] - 2026-10-24

### Added

- **The app folds tool activity into one dim row per run of calls, like the full-screen TUI.** In a session's chat on iOS, Android, web and desktop, tool calls the assistant makes without writing anything in between collapse into a summary such as `Read 3 files, searched for 2 patterns, ran 4 shell commands` or, when the run touched one thing, `Ran cargo test -p upload` or `Read src/big.rs`. Files, directories, patterns and pages are counted once however many calls touched them, and bridged `mcp__forge__<tool>` calls count as the tool. A failed call is never counted as done: it is left out of the sentence, the row says `1 failed` on the right, the failed call stays listed under the row, and a run where nothing succeeded says `Tried to edit sample.rs`. While a call runs, the row shows a spinner and the call in flight stays under it; permission prompts are unaffected, because they sit above the composer. Tap or click a row (or focus it and press Enter or Space) to open the per-call cards in place; open rows stay open while the list scrolls and grows. Settings > Appearance > Fold tool calls turns it off. History rows carry no diff or timing, so the app's rows have no `(+a −d)` or duration and a written file reads as edited rather than created. Each group's summary is reused until its calls change, so streamed tokens and history refetches do not rebuild settled groups (`mobile/src/lib/toolGroups.ts`, `mobile/src/components/chat/ToolGroupRow.tsx`).

### Fixed

- **Settings on a wide desktop window has a way back.** Settings and its sub-pages hide the sidebar and, at desktop width, there is no tab bar, so nothing led back to the session you came from. The Settings rail now starts with a Back link that returns to the last screen outside Settings (the session or the fleet), from any settings page (`mobile/src/app/(tabs)/settings.tsx`, `mobile/src/lib/nav.ts`).
- **Forge Desktop signs in to Forge Anywhere with the account the Forge CLI on the same computer already uses.** A signed-out desktop app used to ask for a new GitHub sign-in even when `forge anywhere` was set up in the terminal. At launch it now creates its own device request and has the local CLI approve it (`forge anywhere approve --expect-signing-key`), which succeeds only when the relay returns the key the app generated. Each stays a separate device, with its own keys. Signing the desktop app out stops this until the next manual sign-in. The CLI must be 2.23.3 or newer; with an older CLI the app shows the normal sign-in.

## [2.23.2] - 2026-10-24

### Changed

- **The desktop app installs updates on its own.** At launch it checks for a newer signed release, installs it and restarts, without waiting for the Install button. A release whose UI could not be clicked (2.23.0) left users with no way to update, because the button lived in that UI; the check now runs natively (`mobile/src-tauri/src/auto_update.rs`). Linux does this for the AppImage only, since a .deb install would need a root password; .deb users keep the button. Set `FORGE_DESKTOP_AUTO_UPDATE=0` to turn it off.

### Fixed

- **The desktop title bar moves the window again, and its minimize, maximize and close buttons work.** Windows and Linux draw their own title bar, but the app never had permission to drag or control the window, so every call was refused. Clicks also never reached the bar: an empty toast container sat over it, because `pointerEvents: "box-none"` inside an inline style becomes invalid CSS on the web and the box caught every click (the notice banner had the same fault). The bar now counts clicks anywhere on it, and an open sheet leaves a drag strip over it.
- **The "Installed Forge app" sheet shows the notes for the version you installed.** It used to show the server's newest release, so a 2.23.1 app on a 2.21.0 server listed 2.21.0's changes under a 2.23.1 heading. When the server has no notes for the version yet, the sheet says so and links to the published release notes. Entries render their formatting instead of raw `**` and backticks, show the summary line with the detail behind a tap, and the sheet has side padding again. The What's New panel renders formatting too.

## [2.23.1] - 2026-10-24

### Fixed

- **The desktop app opens to a working window again on Windows and Linux.** Since 2.13.6 it showed the title bar and tabs over an empty grey pane and ignored clicks, while the same bundle worked in a browser. Two configuration faults in `mobile/src-tauri` caused it. First, Tauri adds nonces to the page's `style-src` at build time, and a nonce makes the WebView ignore `'unsafe-inline'`. That refused the three `<style>` tags Reanimated injects, so every entering animation stayed at its hidden first frame, and the invisible "Installed Forge app" sheet sat over the window catching clicks. `style-src` is now excluded from that rewrite (`dangerousDisableAssetCspModification`). Second, the HTTP scope `http://*/*` follows URLPattern rules, where a missing port means the default port only. Every request to a `forge serve` daemon on 7420 failed with `url not allowed on the configured scope`, so the fleet never loaded. The scope now also allows `http://*:*/*` and `https://*:*/*`. Two tests in `security_config.rs` fail on the old configuration.

## [2.23.0] - 2026-10-24

### Security

- **Dependency advisory RUSTSEC-2026-0387 is cleared.** `crypto-bigint` 0.7.6 (0.7.5 had timing variability in operations meant to be constant-time; reached through `p256`/`ecdsa`).

### Changed

- **Each fact on the Forge screens is shown once, and jargon that carried no information is gone.** The bridge notice is one short row (`CLI bridge: uses your local login; provider terms may limit third-party use (/help)`; it was two wrapped rows and the full text stays in `/help`). A finished turn's tokens are in its footer only, so the status bar shows them while the turn runs and not after. The welcome's key row lists `Ctrl+J newline` and the model picker instead of repeating the five keys the composer border already shows. The status bar no longer draws the routing tier, `⟨auto⟩`, `mesh auto`, `ctx` before any number exists, or `untracked`; a pinned model's answer label no longer says `your pick`. Help is one full-width list below 180 columns (two half-width panes clipped most descriptions) and the longest command descriptions are shorter. After an approved edit the folded group names the file (`Edited calc.py (+1 −1)` instead of `edited 1 file`).
- **An unknown model is stated once at launch and once when you try to use it, and Ollama is named only if you set it up.** `forge chat --model nonexistent-model-xyz` printed the launch warning, a `Model unavailable` composer title and a failed turn, and the warning said `Ollama isn't running` to people who never installed it. Now the launch warning says `no provider knows it` (adding `and Ollama isn't running` only with `[local] model`, `[local] autostart` or `OLLAMA_HOST`), and Enter on a prompt while the whole pin is known uncallable keeps the text in the composer and prints `not sent: model 'x' isn't available. Run /model to pick another.` instead of failing a turn. An id merely missing from the catalog still sends.

### Fixed

- **An idle chat holds about 65 MB instead of 185.** The tokenizer vocabulary (45 MB, built at launch for a first turn nobody had asked for yet) is now built at the first input event, a session that is opened and left idle never holds it, and a first prompt types slower than the build takes (first text of a one-line question: 1.43 s before, 1.32 s after). The model-listing sweep that a fresh store or a stale catalog runs parsed several MB of provider JSON into trees and freed it all, but the allocator kept about 65 MB of those pages for the life of the session; the sweep now returns them. The extra store pools above are gone too. Resident size 15 s after launch, three runs per arm: 237, 184 and 234 MB before, 66, 65 and 68 MB after. On the Claude bridge the Forge terminal process went from 176-180 MB to 61-63 MB, and the three-process tree from 454 MB to 336 MB (`claude` itself is 232 MB of that and `forge mcp-serve` 39 MB, both unchanged).
- **`forge chat` is ready for input in about 125 ms instead of 450.** On a clean profile with the keyring unlocked the process spent 270-385 ms before the composer accepted text, most of it the same work done more than once: a scan of every provider's key through the Secret Service on each start although a config existed (205-235 ms), the session store opened, migrated and pooled five times (15-60 ms and 7-10 MB each), the keyring lookups queued behind the terminal setup, and a probe of the daemon that built an HTTP client in front of the first frame. A saved config now settles the first-run check with one `stat`, the key lookups start at dispatch and overlap the terminal and the store, a chat process opens its store once and every later open reuses that pool (`Store::open_shared`; the daemon keeps independent handles), pools open two connections instead of eight and add the rest on demand, and the daemon probe runs beside the first frame. Measured on `inferhub::cb/deepseek-v4.1-flash`, nine starts per arm interleaved on a loaded machine: median 449 ms (284-514) to 125 ms (89-154) from launch to a usable composer, first paint 330 ms to 61 ms; a resumed session (`--continue`, three runs) 383 ms to 148 ms. `RUST_LOG=forge::startup=debug` now prints each step from `main` on, with the resident set.
- **A poll that waits on a background job stops when the job ends.** `shell` poll mode (`poll_until_exit_zero`) re-ran its status command until its budget ran out even after the job it was waiting on had died: on the parity benchmark a release build was killed 130 s in and the model's three `grep -q "build complete" <job log>` polls kept waiting another 4 minutes before it looked at the job. A poll now watches the running background jobs whose log path or pid its command names (or the only running job when it names none) and, as soon as one ends, returns "stopped waiting" with how it ended (exit code, or killed by a signal) and the last lines of its log (`crates/forge-tools/src/shell/poll.rs`).
- **A script call shows what it does instead of its source, and a bridged model reads as its plain name.** On the native loop a `run_code` call that wrapped `tools.edit_file(...)` printed the raw JavaScript as its row (`run_code await tools.edit_file({path: 'calc.py', old: ...`) under the rows of the calls it made; the row is now `Script  edit` (the verbs of the tools the script calls), and the calls keep their own rows. The status bar no longer prints the `claude-cli::` / `codex-cli::` / `agy-cli::` bridge id in front of the model (`haiku`, `gpt-6-luna`) or the word `subscription` after it; `/status` and `/usage` still say how a model is paid for. The `1% of 1M` context gauge on `claude-cli::haiku` and `claude-cli::sonnet` is correct, not a wrong denominator: the Claude CLI reports `contextWindow` 1000000 for `claude-haiku-5-5` and `claude-sonnet-5-5`.

## [2.22.0] - 2026-10-24

### Security
- **Dependency advisories RUSTSEC-2026-0385 and -0386 are cleared and a yanked crate replaced.** `tokio` 1.54.1 in every lockfile (`AsyncFd::new` and related constructors violated I/O safety), `ctutils` 0.4.5 (non-constant-time mask generation and `u128` comparisons in 0.4.2, reached through `aes-gcm` and `crypto-bigint`) and `clipboard-win` 5.4.3 (5.4.1 was yanked); the desktop app's lockfile also takes `rustls-webpki` 0.103.16 and `webpki-roots` / `webpki-root-certs` 1.0.11.
- **Dependency advisories RUSTSEC-2026-0381, -0382 and -0383 are cleared.** `rustls-webpki` 0.103.16 and `webpki-roots` / `webpki-root-certs` 1.0.10 stop trusting the distrusted emSign intermediates that `cargo deny` and `cargo audit` now report for 0.103.15 / 1.0.8 / 1.0.9, which was failing the security checks of every pull request.

### Added
- **`RUST_LOG=forge::turn=debug` now shows where a model-loop step spends its time.** Besides the phases before the first token it prints each step's `request`, `response complete`, `tools start` and `tools done`, the failed-command explanation (`shell diagnose: start` / `done`) and the recap, suggestion and memory capture after the answer (`post-turn side calls: start` / `done`), so a turn splits into the model's time and Forge's. Five DeepSeek tasks on the native loop read: model 74.0%, tools 11.1%, work after the answer 9.0% (4.8 to 8.2 s a task), shell diagnosis 0.5%, request building, guards and the gaps between steps 0.4%, the last step's model time 5.0%; Forge's own work between a tool result and the next request is under 10 ms in every step (`docs/features/turn-latency.md`).
- **Quitting the chat prints how to resume it.** `Resume this session: forge chat --resume <id>` (with the session's name when it has one, cut to the terminal width) is printed after the terminal is restored, once the session has any messages. `forge chat --plain` prints it too when stdout is a terminal.
- **A bridged turn is supervised from inside `forge mcp-serve`.** A `claude-cli` or `codex-cli` model runs its own tool loop in a subprocess, so the parent saw one response per run and none of the calls. The same progress ledger now runs call by call where every call lands (`crates/forge-core/src/bridge_progress.rs`, `crates/forge-cli/src/mcp_serve/progress.rs`), with the narration and idle lanes off: a nudge is put in front of the result that earned it, and a stop replaces that result and every later one with "Forge stopped this turn... Make no more tool calls; reply now with a short summary". The stop is also reported to the parent through the sink (`StreamEvent::Stall`), which ends the turn as `stalled` with the model's summary instead of re-driving the plan, and a model that keeps calling tools 30 s after the stop has its process group killed.
- **`mesh.budget.hard_stop` accepts `"auto"`, the new default.** `true` and `false` still work. `auto` stops an unattended turn at the per-turn USD cap, `mesh.max_turn_input_tokens` and `mesh.bridge_turn_token_cap`, and only warns an attended one, whose person sees the spend and can interrupt; the session, day, week and month caps are enforced either way. Set `hard_stop = true` to stop every turn as before (`crates/forge-config/src/hard_stop.rs`, `docs/features/fix-budget-cap.md`).
- **A turn that is making progress is never stopped by a step count; a turn that is not is nudged once and stopped cleanly.** `mesh.max_steps` (it was 100) and `mesh.max_steps_unattended` (it was 400) now default to `0`, meaning none, so the attended "send `continue`" pause and the unattended `reached soft step cap 100 at step 100; unattended turn continuing toward hard cap 400` warning no longer happen. A per-turn progress ledger (`crates/forge-core/src/progress_ledger.rs`, `progress_report.rs`, `progress_supervisor.rs`) judges each tool step by what it produced: a new file write, a closed task, a failing check passing for the first time that failure was fixed, the working tree reaching a state it was never in this turn (so silent `sed -i` or generator-script work counts, and a file that flips between known states does not) or a steer is strong progress and resets everything; a wait (poll mode, a running `shell_job`, a command that sleeps or watches first, a call cut by its time limit) and the first step after a harness gate are neutral; a repeated call with an identical result, consecutive failed or stale steps, the same error again, a check that fails with the same output after every edit, a restated opening, 24 steps of waiting, or (on headless turns) 200 steps with no edit, task or check or 96 of waiting are not. One nudge per episode names what was observed ("`shell` failed 4 times (command not found: x); the `test` check is still failing"), then the turn stops with the new stop reason `stalled` ("Stopped: no progress for 10 steps (last change: step 31, edit_file src/lib.rs). Your edits are kept. Send `continue`, or tell it what to try."). Headless stops (no person watching; `--mode bypass` at a terminal someone is looking at is not headless) also preserve the uncommitted work and print `Resume: forge run --continue`; a bare `continue` that repeats what the turn was stopped on and gets the same result is stopped again at once (the same call that now succeeds is not), and the next turn's model is told why it was stopped. `mesh.progress_guard = "warn"` reports where it would have stopped and lets the turn run on, `"off"` keeps only the nudges, and `mesh.idle_nudge_steps` / `mesh.idle_stop_steps` (60 / 200, `0` disables) set the idle bars. Explicit limits keep working and now count the whole turn across re-drive passes: `mesh.max_steps`, `--max-turns`, `--max-budget-usd`, goal and loop budgets. See `docs/features/progress-supervision.md`.
- **A stalled subagent keeps its work.** A child now has its own progress ledger: it is nudged once and stopped when it stops getting anywhere, and the parent receives its last words plus the reason with `ok = false` instead of an error that threw them away. A child's `maxTurns` is capped by `mesh.max_steps` only when that is set (`crates/forge-core/src/subagent.rs`).
- **`RUST_LOG=forge::turn=debug` traces the path from Enter to the first token.** One line per phase (turn spawned, routed, request dispatched, bridge spawn, tool server ready, prompt written, first line from `claude`, first stream event, first text delta); `docs/features/turn-latency.md` lists what each phase is and the rule it follows.
- **Claude Code skills and commands behave as written.** An imported `SKILL.md` or command now has its frontmatter consumed instead of dropped. `allowed-tools` become allow rules and `disallowed-tools` deny rules for the one turn the skill starts (a skill the model loads with `use_skill` grants for the rest of that turn), so a skill that pre-approves `Bash(gh pr view:*)` no longer prompts for it; a configured deny, the safety floor and `plan` mode still win, a configured `ask` of equal specificity still prompts, a bare disallowed tool leaves the tool list the model sees, and a project skill's `allowed-tools` count only in a trusted project. `disable-model-invocation` keeps a skill out of the model's `use_skill` listing, `user-invocable: false` keeps it out of the `/` palette and refuses `/name`, `argument-hint` shows in the palette, `paths` is listed beside the skill so the model loads it for matching files, `effort` sets that turn's reasoning rung, and `context: fork` + `agent` runs the invocation as one subagent so a large skill never enters the session's context. Templates gain `$ARGUMENTS[N]`, 0-based `$0`, `arguments:` names, shell-style quoted arguments, `${CLAUDE_SKILL_DIR}`, `${CLAUDE_SESSION_ID}`, `${CLAUDE_PROJECT_DIR}` and `${CLAUDE_EFFORT}`, and ``!`cmd` `` / ` ```! ` blocks inject a command's output before the model sees the text, only in a trusted project and only for commands a rule, the temper or the read-only allowlist permits. A skill invoked with a short argument (`/review zz`) was also classified by that argument alone, so its turn was trivial chat with no tools at all (1,262 input tokens, nothing to call); a turn that carries a skill's methodology now always gets the tool surface (13,795 tokens on `inferhub::glm-5.3-flash`, and the skill's `touch` ran). `forge run "/skill"` runs a forked skill with no argument (it ended silently before) and stays in this process instead of being handed to a daemon session that would drop the skill's text, grant and fork (`crates/forge-skills/src/invocation.rs`, `crates/forge-skills/src/template.rs`, `crates/forge-skills/src/inline_shell.rs`, `crates/forge-core/src/turn_grant.rs`, `crates/forge-core/src/skill_shell.rs`, `crates/forge-core/src/skill_fork.rs`, docs/features/command-skill-system.md).
- **Subagent definitions read Claude Code's fields, and `explore` and `plan` ship built in.** An agent file's `disallowedTools`, `permissionMode` (only ever stricter than the parent's temper), `maxTurns`, `skills` (loaded into the child's system prompt), `effort`, `isolation: worktree`, `mcpServers` and `color` are honoured or recorded, with YAML lists and comments accepted. `spawn_agents` can name Claude Code's `Explore` and `Plan` (case-insensitively) without any file: both are read-only (`plan` temper, search, read and read-only shell), and `explore` runs on the trivial tier; a file of the same name replaces one. The new `/agents` command lists every type with its source and settings (`crates/forge-config/src/agents.rs`, `crates/forge-config/src/agent_builtins.rs`, `crates/forge-core/src/subagent/requests.rs`, `crates/forge-cli/src/cli/commands/run/agents_cmd.rs`, docs/features/subagent-orchestration.md).
- **Instruction files load like Claude Code's: every file, parents included, with imports and rules.** Forge injected the first non-empty of a fixed list, so a repository `AGENTS.md` hid the user's global `~/.claude/CLAUDE.md`, a session in `repo/pkg/` never read `repo/AGENTS.md`, and `CLAUDE.local.md`, `@path` imports, `.claude/rules/` and nested `CLAUDE.md` files were ignored. Now the user-global files (`<config>/forge/AGENTS.md`, `~/.forge/AGENTS.md`, Claude's `CLAUDE.md` and `rules/`) load first, then for every directory from the filesystem root down to the workspace its `.forge/AGENTS.md`, `AGENTS.md`, `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md` and unconditional `.claude/rules/` / `.forge/rules/` files, each headed with its path when there are several (a world-writable parent such as `/tmp` is skipped). `@path` imports load up to five levels deep (a project file only from inside its repository, never a secret the built-in rules protect); HTML comments are dropped; a file reached twice is carried once. A `CLAUDE.md` in a subdirectory, and a rule whose `paths:` globs match, arrive with the first read or edit of a file under it, attached to that tool result so the pinned block and the prompt cache behind it stay put (again after a compaction; never from `node_modules`, `vendor` or `target`). The per-turn freshness check covers every path the load looked at, imports and rule directories included, in one batch of `stat`s off the async executor. The claude bridge is sent only the files a bridged claude does not read itself (`AGENTS.md`, `.forge/` files), still de-duplicated through `lean_context::claude_md_covers`. `--setting-sources` without `user` or `project` skips that layer (`crates/forge-core/src/instructions.rs`, docs/features/instruction-files.md).
- **Skills, commands and agent definitions resolve up to the repository top.** A session started in `repo/pkg/` did not see `repo/.forge/commands`, `.forge/skills`, `.forge/agents` or `.claude/agents`, and was offered `/init` (or an automatic setup turn) in an already set-up repository. Project resources now come from the working directory and each parent up to the nearest `.git`, the nearest directory winning a name collision; in a trusted working directory an untrusted parent contributes nothing. Project `config.toml` stays cwd-only (`crates/forge-config/src/project_dirs.rs`).
- **A permission prompt can allow for this session and carry a note to the model.** The prompt has a fourth choice, `s` (or `4`), that allows the call and every later one it would match until the session ends and writes nothing to `.forge/config.toml`; before, `a` was the only way to stop being asked and it always saved a rule. `Tab` opens a note field: what you type goes to the model with the tool result, as the reason on a deny (`The user said: use rg instead`) and after the output on an allow (`[the user approved this call and added: ...]`), so a refusal can say what to do instead of leaving the model to retry. A bridged `claude`/`codex` model gets a deny note through the permission relay; the headless prompt takes `y`, `s`, `a` or anything else followed by the note on the same line. `3` still denies and `a` still saves (`crates/forge-types/src/interaction.rs`, `crates/forge-core/src/tool_dispatch/runtime_allow.rs`, `crates/forge-tui/src/app/state/interaction_ui.rs`, docs/features/tui-interaction.md).
- **`/rewind` asks what to restore.** Choosing a message opens a second step: code and chat (the old single action, still first), chat only, code only, or summarize from there. Code only puts the files back and keeps the conversation, with a note telling the model which files went back so it reads them again instead of editing from memory; chat only leaves the files alone; summarize folds the message and everything after it into one summary through the `/compact` pipeline (typing in the menu is the focus, like `/compact <focus>`) and leaves the files as they are. Before, Enter restored chat, context and files together and there was no way to keep either (`crates/forge-core/src/session_rewind.rs`, `crates/forge-cli/src/cli/commands/run/rewind_menu.rs`, docs/features/rewind.md).
- **Bare `forge` and `forge chat "prompt"` start a session like `claude` does.** On a terminal with a configured provider, `forge` opens the chat instead of printing a summary and exiting (a pipe, CI, `TERM=dumb` or no provider still gets the short panel, exit 0). `forge chat "explain this repo"` sends the prompt as the first message and leaves the chat open (with `--plain` the prompt runs first, then stdin). `forge mcp add` now takes `claude mcp add` syntax unchanged: `forge mcp add --transport http sentry https://mcp.sentry.dev/mcp -H "Authorization: Bearer ..."`, short flags `-t`/`-H`, and a positional command without `--`; a URL with no `--transport` means `http` (Claude Code tries to spawn it as a command). `forge mcp list` (alias `ls`) lists servers, and `forge mcp add-json <name> '<json>'` adds one server from its `.mcp.json` entry, with any token going to the keyring (`crates/forge-cli/src/first_run.rs`, `crates/forge-cli/src/cli/commands/mcp/add.rs`).
- **New slash commands from Claude Code: `/context`, `/diff`, `/permissions`, `/add-dir`, `/doctor`, `/rename`, `/mcp reconnect`, `/review`, `/security-review`.** `/context` breaks the next request down by system prompt, tool definitions, project instructions, memory/goal/tasks and conversation, with a bar and percentages of the model's window. `/diff [path]` opens the uncommitted changes against `HEAD` (untracked files listed) in the searchable full-screen viewer with diff colours. `/permissions` lists every allow/ask/deny rule with where it came from and `/permissions allow|ask|deny Bash(git log:*) Read` adds session rules in Claude Code syntax. `/add-dir <path>` lets the file tools work in another directory for the session. `/doctor` runs the local `forge doctor` checks in the session. `/rename <title>` names the session. `/mcp reconnect [server]` re-establishes MCP connections. `/review [PR|branch|path]` and `/security-review` are built-in commands that a project or user command of the same name replaces. `/diff`, `/doctor` and `/mcp reconnect` run off the render loop, so the chat stays responsive (`crates/forge-cli/src/cli/commands/run/slash_commands.rs`, `crates/forge-core/src/context_report.rs`, `crates/forge-skills/src/builtin_review.rs`).
- **Claude Code parity benchmark baseline.** `docs/benchmarks/claude-code-parity-2026-10.md` records Forge 2.21.0 against Claude Code 2.1.308 on the same models (quality, speed, tokens, multi-turn, SWE-bench Verified), with the per-run data, SHA-256 commitments for the withheld graders, a standard-library renderer and an independent script that rechecks the report against the raw bench output, so later fix runs append to the same report. Documentation only.
- **`forge run -p` is a drop-in for `claude -p`.** In print mode stdout carries only the final answer, so `answer=$(forge run -p "...")` works; the session header, routing line, tool trace, recap and cost footer (7 of the 8 stdout lines before) go to stderr when it is a terminal or with `--verbose`. `--output-format stream-json` now has Claude Code's shapes: one `assistant` event per finished message instead of one per streamed word, a `system/init` with `cwd`, `model`, `permissionMode`, `tools`, `mcp_servers`, `slash_commands` and `skills`, and a `result` with `usage`, `total_cost_usd`, `duration_ms`, `duration_api_ms`, `num_turns` and `permission_denials`; `usage.input_tokens` (here and in `--output-format json`) now excludes cache reads, as Anthropic reports it, so a consumer adding `cache_read_input_tokens` no longer counts the cached prefix twice. New flags: `--include-partial-messages` (`stream_event` deltas), `--input-format stream-json` (one user message per stdin line, each a turn of the same session), `--system-prompt[-file]`, `--append-system-prompt-file`, `--add-dir`, `--effort`, `--max-budget-usd` (stops inside the turn, `error_max_budget_usd`), `--session-id`, `-n/--name` (`--resume <name>` finds it), `--no-session-persistence` (spend still counts toward `forge stats` and the caps), `--mcp-config`/`--strict-mcp-config` (Claude Code's `.mcp.json` shape), `--setting-sources`, `--fallback-model`, and the `auto`/`dontAsk` permission modes. The session-shaping flags are one `SessionFlags` struct that `forge chat` can share (`crates/forge-tui/src/print_presenter.rs`, `crates/forge-tui/src/stream_json.rs`, `crates/forge-cli/src/cli/args/session_flags.rs`, docs/features/claude-code-parity.md).
- **The full-screen transcript folds tool activity into one dim row per run of calls, like Claude Code.** Calls the assistant makes without writing anything in between collapse into a summary such as `Read 3 files, searched for 2 patterns, ran 4 shell commands`, `Edited 2 files (+41 −7)` or, when the group touched one thing, `Ran cargo test -p upload` or `Read src/big.rs`, so the prose between them is what stands out. Files, directories, patterns and pages are counted once however many calls touched them, so three slices of one file read `Read 1 file`. A failure is never hidden and never counted as done: it is left out of the sentence and the `(+a −d)`, the row says `1 failed` on the right, where a narrow terminal never cuts it, the failed call stays listed under it, and a group where nothing succeeded says `Tried to edit sample.rs`; a deletion is never merged into "edited". While a group runs, its row spins and the call in flight shows beneath it, and a call waiting for permission keeps its diff on screen. Click a summary to open it in place (the row stays under the pointer), or press Alt+↑/↓ to focus a group and Enter or Space to toggle it; Alt+O opens or folds every group, Ctrl+T opens the newest group, then its newest call, and the newest folded group ends with a faint `click or Alt+O to expand`. Open groups stay open across scrolling and resizing. On a narrow terminal a summary switches to counts (`1 dir, 2 searches, 2 commands`) before anything is cut, and card headers keep their tool and target ahead of the right-hand preview. Each group's summary is memoised on its cards and a card's diff stat is counted once when the diff attaches, so a rewrap on every streamed line rebuilds only the group that changed. `[tui] fold_tool_groups = false` restores one card per call; inline mode is unchanged, because native scrollback cannot reopen a row. `forge chat --mock` gains a `mock:tools` scenario that exercises it (`crates/forge-tui/src/app/group_flow.rs`, `crates/forge-tui/src/app/tool_groups.rs`, docs/features/tui-tool-cards.md).
- **`web_fetch` returns Markdown and can page through long pages.** Pages are rendered from their `<main>`/`<article>` region with headings, links (made absolute) and code blocks kept, and navigation, scripts and page chrome dropped; before, every page was flattened to one line, and the 10k-character window was often spent on the menu. Plain-text, JSON and source files come back verbatim instead of having `<T>` generics stripped as tags. A new `offset` argument reads past the window, fetched pages are cached for 15 minutes so paging does not refetch, and `max_chars` goes up to 100k (`crates/forge-tools/src/web/page.rs`).
- **More Claude Code hook events, `http` hooks, `if`, `async`, and hooks that run in parallel.** New events fire from their real seams with Claude Code's payloads and matcher values: `PostToolUseFailure` (a call that ran and failed, including a `shell` command that exited non-zero; a Claude Code `PostToolUse` hook no longer fires for it), `PostToolBatch` (once per step, before the next model request; a block ends the turn), `PermissionDenied` (Forge refused a call on its own; `retry: true` tells the model it may try again), `SubagentStart` (its `additionalContext` joins the child's task), `StopFailure` (a turn that ended on a provider error, typed `rate_limit`, `billing_error` …), `InstructionsLoaded`, `CwdChanged`, `WorktreeCreate` / `WorktreeRemove` (a hook replaces `git worktree` for the `worktree` tool, for SVN, Perforce or plain copies), `UserPromptExpansion` (a typed `/command`) and `PreModelSwitch` / `PostModelSwitch` (a hook can keep the current model on `/model` and the pickers). `type: "http"` POSTs the event JSON to a URL, with header variables expanded only from `allowedEnvVars`; `if` narrows a tool hook to calls matching a permission rule (`Bash(git *)`, `Edit(*.ts)`); `async: true` runs a command hook in the background and hands its answer to the next turn. All hooks one event matches now start together (two 1 s hooks take about 1 s instead of 2 s) and are applied in declaration order. `forge run` now fires `SessionStart`, `UserPromptExpansion`, `UserPromptSubmit` and `SessionEnd` like `claude -p`, and `SessionStart`'s `source` is `startup`, `resume`, `clear` (`/new`) or `compact` instead of always `startup` (`crates/forge-core/src/hooks_*.rs`, `crates/forge-config/src/hooks.rs`, docs/features/hooks.md).
- **`web_search` takes `allowed_domains` and `blocked_domains`.** Allowed domains are added to the query as `site:` operators and both lists filter the results (`crates/forge-tools/src/web.rs`).
- **`forge bench tasks` picks tasks, places workspaces and says why a task failed.** `--tasks` and `--skip` take names or globs (`*`, `?`) and reject a pattern that matches nothing; `--workdir` sets where attempt workspaces are created (default `TMPDIR`). Each result row gains `agent_secs`, `test_secs` and, for an unsolved attempt, a one-line `reason` taken from `test.sh`'s output, and the run ends with a table of every attempt (`crates/forge-cli/src/bench/suite_select.rs`, `suite_report.rs`, `docs/features/bench-tasks-and-export.md`).
- **`forge bench tasks --agent claude-code` records what Claude Code reports.** It reads `stream-json` output, so the metrics rows carry `model_calls` (`num_turns`), `tool_calls`, `tool_errors`, `duration_ms`, `ttft_ms` and cost, and a run killed at the timeout keeps its tool and turn counts. `--clean` runs it with `--setting-sources project --strict-mcp-config`, so your hooks, `CLAUDE.md` and MCP servers do not take part in a comparison.
- **`forge run --quiet` prints the answer and nothing else.** Not even the trace a terminal stderr gets under `-p`; errors still reach stderr. It implies `-p`, conflicts with `--verbose`, and is accepted and ignored with `--output-format json` and `stream-json`.
- **JSON results carry `usage.cache_creation_input_tokens` and `modelUsage`.** `--output-format json` and the `result` event of `stream-json` (and each `assistant` message's `usage`) report tokens written to the prompt cache apart, and `modelUsage` has Claude Code's per-model object (`inputTokens`, `outputTokens`, `cacheReadInputTokens`, `cacheCreationInputTokens`, `webSearchRequests`, `costUSD`) keyed by Forge's model id. `usage.input_tokens` now excludes the cache writes too, as Anthropic's does, so `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` is the real total; it only differs on routes that report cache writes (the claude bridge, Bedrock).
- **`--verbose` is a real switch with `--output-format stream-json`.** It adds Claude Code's `system/status` event (`{"status":"requesting"}`) before every model call; without it the stream is unchanged.

### Changed
- **Upgrade: project trust.** A repository's own `.forge/` hooks, MCP servers, allow rules and a more permissive `permission_mode` are ignored until you trust it. Interactive `forge chat` asks once; for headless, CI or daemon use run `forge trust [path]` or set `FORGE_TRUST_PROJECT=1` (see Added).
- **Upgrade: auto-edit asks before risky shell.** Ordinary commands still run, but shell that deletes, moves, copies or redirects outside the workspace, touches a raw disk, pipes into a shell or runs inline code that deletes files now prompts. Unattended runs that relied on auto-edit running everything will see prompts (see Security).
- **Upgrade: compaction summaries come from the session's own model by default** (`[mesh] compact_model = "session"`), which can spend paid credits or plan quota. Set `[mesh] compact_model = "auto"` for the previous free-model-first order.
- **Upgrade: session store schema 44 → 45.** The first 2.21.0 run migrates `forge.db`; older Forge versions refuse to open it afterwards. Back up `~/.local/share/forge/forge.db` first if you may roll back.
- **The native loop scales its final check to the project and stops once it passes.** The base prompt asked for "one final complete build/test/lint pass" whatever the repository size or the task said about building. On the large-repository suite (Forge, Twisted, dulwich and npm snapshots) DeepSeek through Forge spent about twice Claude Code's time after its first edit (1,652 s and 195 model calls against 516 s and 106 over 22 runs): whole-workspace `cargo check --tests` passes on a task that says a full build takes very long and other crates must be read, re-runs after a format fix, and new scratch experiments after the focused check had already passed. The prompt now says to cover only what the change touches when the task limits what to build or run or a whole-workspace build would take minutes, and to finish once verification passes instead of widening into new experiments (`crates/forge-core/src/lib.rs`, `FORGE_SYSTEM`). Paired A/B against main on `inferhub::cb/deepseek-v4.1-flash`, run side by side: suite c, 11 tasks x 2 seeds, 22/22 solved in both arms, agent wall 3,508 s to 2,825 s (-19%, median paired ratio x0.92, faster in 13 of 22), model calls 506 to 415, input tokens 14.6M to 10.8M, output 244k to 191k (reasoning 139k to 111k), time after the first edit 1,652 s to 1,165 s; suites a/b/d, 26 tasks x 1 seed: 23/26 solved against 22/26, agent wall unchanged (3,322 s against 3,317 s, median paired ratio x1.02), model calls 367 to 339, input tokens 8.3M to 6.5M. Against Claude Code on the same model (its suite c run of 2026-10-22, not side by side) the native loop goes from 1.69x to 1.36x its wall time, and input tokens from 1.43x to 1.06x. Single runs of one task vary by 2-3x, so per-task numbers are noise; the sums are what moved.
- **A message queued while a turn runs is now visible, editable and sendable.** Typing while Forge works cleared the composer and left a `1 queued` chip on the border and, once the answer ended, a `⏳ queued (1 pending) — sent at this turn's next step` row that stayed in the transcript for good; the queued text was nowhere, Up recalled a history copy of it (so editing and re-sending queued it twice: `2 queued` in the capture), and nothing said that Esc already sends it. In the TUI benchmark's queued-follow-up scenario Forge lost to Claude Code on every judge vote. The terminal now draws the queue in the live region above the composer, one dimmed line per message cut to the width (the newest three, the header counts them all): `2 queued · ↑ edit · Esc stop and send now`. **Up** on an empty composer pulls the newest queued message back to edit (its pasted images return as attachments) and takes it out of the queue and the steer inbox, so the running turn cannot still pick it up; **Esc** is what it always was, stop the turn and send the first queued message, and the band now says so (it stays quiet while a permission prompt or question owns the keyboard). The band is gone the moment the turn takes the message, so no row is left behind, and the duplicate border chip was removed. A headless daemon mirrored to a phone or `forge attach` draws no band, so it still prints the note. After the turn takes it the echo reads `you · follow-up` instead of `you · steer`. Scenario captures (fake API, bridge arm): the queued text is on screen at the first poll with the band and was never on screen before (5 s wait); after Up the queue is empty (before: still `1 queued`, then `2 queued` once the edit was sent). `crates/forge-tui/src/app/render/queue_band.rs`, `crates/forge-cli/src/cli/commands/run/queue_images.rs`, `docs/features/steering.md`.
- **A permission prompt for a read-only command says why it exists in an untrusted project.** In the Ask temper `git status`, `git diff` and `git log` ask in a project you have not trusted (git runs what a repository's config names, so the auto-allow of #1630 is gated on `forge trust`) and nothing on the prompt, or in the startup notice, said so: the TUI benchmark's git probe raised a prompt for every git command in an untrusted project. When a shell call asks only for that reason (Ask temper, every statement read-only, no rule deciding it, `forge trust` the only missing piece) the panel adds one line above the choices, `Trust this project to skip prompts for read-only git: forge trust`, shortened in steps on a narrow terminal. It is advisory and is worked out from the command text and the trust store alone: it never starts git in the repository (the very exposure the trust gate keeps out) and changes nothing about what a call may do; a repository whose config names a helper still asks after `forge trust`. An edit, `touch x`, `git push` or `git -c core.pager=x log` get no line. `forge_core::permission::shell_trust_hint`, `docs/features/project-trust.md`.
- **The welcome and the composer no longer clip words on a narrow terminal.** At 60 columns the welcome ran off the edge mid-word (`/init writes guidance + agen`, `preview ro`) and the composer's placeholder cut `! runs a shell command`. Every welcome row now wraps at spaces with a hanging indent under its own text (a long example lands under `Try`, not under the margin), the `/init` nudge puts the state on one row and the action under it when it cannot fit on one, the `Keys` and `More` rows drop whole entries before they would clip, and the placeholder gives up the shell hint first. Tested at every width from 40 to 140 at 24 and 40 rows: no row is wider than the terminal and no word is one the full-width welcome does not contain. `crates/forge-tui/src/app/render/welcome.rs`, `crates/forge-tui/src/app/render/input.rs`.
- **Plain words on the main surfaces, and the CLI-bridge terms once per install.** The footer and the assistant label named the router's tiers (`trivial`, and `pinned` for a model you chose) next to the model; they read `simple` and `your pick` now, while `/status` and `/mesh` keep the router's names. The `CLI bridge: runs your local claude/codex login; provider terms may limit third-party use` notice, which printed in the transcript of every bridge session, is shown once per installation (a marker beside the store, `<data dir>/notices/cli-bridge-terms`, or `<FORGE_DB>.notices/` for a scratch store) and its text is kept under Notes in `/help`. In the resume scenario (two bridge sessions on one store) it printed twice before and once after. `crates/forge-config/src/install_notices.rs`, `crates/forge-core/src/lib.rs`, `crates/forge-tui/src/help.rs`.
- **Five loop detectors became one.** The identical-call doom loop, the oscillation guard, the failure-loop guard, `ToolFailureTracker` (which also refused the third identical call outright) and the structural stagnation tracker were replaced by the progress ledger's lanes under one latch (one nudge per episode, a stop only after it), and the silence guard's 40-step "summarize your findings" ask by the idle nudge. A token-ceiling or dollar-cap abort is now reported as `budget_exhausted` (it was a finished answer, exit 0). A harness note that repeats verbatim (`Hint`, `LoopGuard`, `Continue`, `Silence`) is dropped after the third copy in a burst and admitted again 25 steps later or after a compaction, so a diagnostic that persists is not lost.
- **A config that pins the old step defaults is reported.** `max_steps = 100` together with `max_steps_unattended = 400` are hard limits now; startup warns once so the two lines can be removed (`crates/forge-config/src/config_diagnostics.rs`).
- **Ask runs shell that provably only reads without a prompt, like Claude Code.** `git status`, `git diff`, `ls`, `cat calc.py`, `grep -rn x src`, `rg`, `wc -l` and pipelines or `&&`/`;` chains of such commands no longer prompt in the default temper; in the scripted s04 file-edit scenario Ask raised 2 to 3 prompts against Claude Code's 1. A call qualifies only when every statement is a short allow-listed binary written plainly, with no flag that writes, runs code or reaches the network, no redirect (beyond `2>&1` and `>/dev/null`), substitution, subshell, backgrounding, wrapper or path-named binary, and the paths it names or sweeps pass a secret guard: the `read_file` secret globs through `~`, wildcards and symlinks, a command that reads whole trees stays inside the workspace, and `git` and `file` run unprompted only in a project you trusted with `forge trust` (git never starts there otherwise), and `git` only in a repository whose own config runs nothing, with a `git` and a `sh` that `PATH` does not take from the checkout. The safety floor, deny and ask rules, hooks and the other tempers are unchanged, and `[permissions] auto_allow_read_only_shell = false` restores the prompt (`crates/forge-core/src/permission/ask_shell.rs`, `docs/features/temper-modes.md`, ADR-0008). The Read-only temper's classifier shares the per-binary rules and is tightened with them: `uniq - out` and `uniq *.txt` wrote their last operand, `git grep -nOsh x` ran `sh`, `>&1file` and `>&2` followed by a no-break space wrote a file, and abbreviated hostile options (`sort --out=x`) and `git log --show-signature` were accepted.
- **The bridge's completeness review runs only when it can find something.** After a bridged claude or codex turn that changed files, Forge re-drove the whole conversation once more to re-check every requirement. In the baseline parity run it ran in 71 of 78 bridged turns, cost 8 to 15% of bridged wall time, 22 to 31% of input tokens and 18 to 28% of model calls, and changed file content in about 2 of them. Two defects made it fire almost always: the "long request" test split sentences on every `.`, so `config/limits.conf` ended one, and it inspected `git diff HEAD`, which is empty once the model has committed its own work (44 of 48 haiku turns had; in 29 the model went to `git show` instead). It now runs when a check failed and has not passed since, the answer admits unfinished work (phrases such as "untested" or "not yet", not bare words like "skipped"), the request lists three or more items, a request of two listed items or five prose sentences was never checked after the last edit, or the change since the turn's starting commit spans three files or 60 lines. File names, decimals and code no longer split a sentence, and the review is told to run `git diff <turn-start-sha>`. Live on two small `forge bench tasks` runs with `claude-cli::haiku` (single runs, debug build against the release baseline): the review fired 2 of 2 times before and 0 of 2 after, with 18 → 15 model calls and 364k → 292k input tokens (`crates/forge-core/src/review_gate.rs`, `crates/forge-core/src/completeness.rs`, `crates/forge-core/src/completion.rs`).
- **`[git] commit_policy` has an `auto` default: `unit` interactively, `end` for headless one-shot runs.** `forge run`, `forge run -p` and `forge bench` no longer tell the model to commit each verified unit and no longer remind it about uncommitted files; it commits once when the task is complete. An explicit `unit`, `end` or `never` in the config still wins, and a `forge run` handed to the daemon stays interactive. `end` still makes one commit: on the two live tasks above the single commit call was unchanged, so the measured saving is the reminders and the extra commits of multi-unit tasks, not a call per task (`crates/forge-config/src/commit_policy.rs`, `crates/forge-core/src/commit_policy.rs`, `crates/forge-cli/src/cli/commands/run/one_shot.rs`, `crates/forge-cli/src/bench.rs`).
- **`/clear` starts a new conversation and `Ctrl+L` redraws, as in Claude Code.** `/clear` (also `/reset`) is now `/new`; clearing only the screen is `/cls`. `Ctrl+L` repaints the screen where it used to compact the conversation with a model call; compaction moved to `Alt+L` (`[keybinds.binds] compact`), and a config that still binds `compact` to `Ctrl+L` is moved on load. Rebind either in `forge keys` or `[keybinds.binds]` (docs/features/keybinds.md).
- **A turn's answer arrives sooner, and nothing optional runs before it.** A one-shot `forge run` printed its `result` only after a post-turn memory side call (0.4–3.5 s after the first token on `inferhub::cb/deepseek-v4.1-flash`); every surface now gets the result first, and a one-shot run makes no recap, suggestion or memory call at all (`mesh.auto_memory_one_shot = true` keeps the capture, after the result). Before the first token, a pinned turn no longer ranks the whole catalog for a fallback chain it then discards (~300 ms), the tokenizer vocabulary and Codex plan are loaded in the background during startup (~150 ms), and the session-start memory recall embeds nothing when there are no memories and otherwise overlaps routing. An embedding model that rejects a request (the retired `gemini::text-embedding-004` answers 404) is not asked again in that process. Measured on a pinned trivial turn: ~650 ms of work before the model request dropped to ~60–180 ms, and `result` now lands 20–120 ms after the first token (`crates/forge-core/src/lib.rs`, `crates/forge-mesh/src/chain.rs`, `crates/forge-core/src/pinned_context.rs`, `crates/forge-provider/src/embedder.rs`).
- **Unpinned routing waits less.** The OpenCode Go / Kimi usage polls no longer hold startup (~0.9 s measured, up to a 5 s timeout); the route uses the last stored reading and the poll lands for later routes. The tier classifier is skipped when the deterministic floor is already Complex (no verdict could change it), asks for low reasoning effort, and the model is selected once instead of twice (`crates/forge-core/src/llm_router.rs`, `crates/forge-cli/src/cli/commands/run/session.rs`).
- **`!command` runs a shell command from the prompt, like Claude Code.** A line that starts with `!` (`!git status`) no longer goes to the model as text: it runs through the session's own `shell` tool, shows as a `Run` card, joins the conversation as a user message and the model responds to the output (`[tui] respond_to_bash = false` only adds it to the context). It skips the approval prompt since you typed it, but not the safety floor: `sudo`, `su`, `pkexec` and catastrophic deletes are refused with a card that says so. The `!` is drawn in the warning hue, the hints read `Enter run in shell`, the empty prompt says `! runs a shell command`, and `/paths` or `@refs` in the line open neither the palette nor the file picker. A line that starts with a space is still an ordinary prompt, and a `!command` typed while a turn runs goes back into the prompt instead of being lost (`crates/forge-core/src/user_shell.rs`, `crates/forge-cli/src/cli/commands/run/bang.rs`, docs/features/tui-interaction.md).
- **Readline editing in the prompt: kill ring, undo, history chords and an external editor.** `Ctrl+K`, `Ctrl+U` and `Ctrl+W` now feed a kill ring that `Ctrl+Y` pastes and `Alt+Y` cycles (consecutive cuts join one entry); `Ctrl+_` undoes the last edit, a run of typing counting as one step; `Ctrl+P` and `Ctrl+N` walk the prompt history; `Ctrl+D` deletes forward, and on an empty prompt asks twice to quit (`Ctrl+D again to quit`); `Ctrl+G` opens the draft in `$VISUAL` or `$EDITOR` and takes back what you save. `Ctrl+K` is the command center and `Ctrl+N` a new session in the default keymap, so there `Alt+K` cuts to the end of the line; the Claude Code preset below frees them (`crates/forge-tui/src/app/input_edit.rs`, `crates/forge-tui/src/driver/keymap.rs`, `crates/forge-tui/src/driver/external.rs`).
- **`[keybinds] preset = "claude-code"` moves Forge's actions off Claude Code's chords.** `Ctrl+Z` undoes a file write and `Ctrl+L` compacts the conversation, which is not what a Claude Code user means by suspend and redraw. The preset moves the command center to `F2`, new session to `Alt+N`, undo to `Alt+Z`, checkpoint to `Alt+S`, compact to `Alt+L`, reasoning to `Alt+T`, the model picker to `Alt+P` and the mode cycle to `Alt+M`, so `Ctrl+K`, `Ctrl+N`, `Ctrl+P` and the rest keep their Claude Code meaning. A bind you set yourself still wins, the keybinding editor resets to the preset's chords, and hints, the welcome screen and Help name the active chord. `forge setup` asks once whether to turn it on, defaulting to yes when `~/.claude` exists (`crates/forge-config/src/keybinds.rs`, docs/features/keybinds.md).
- **An unknown `--model` id fails on the provider's first refusal.** When startup already warned that no catalog lists the pinned id, one fast rejection now ends the turn with the provider's error instead of waiting ~5 s for a second identical one (`crates/forge-core/src/model_request.rs`).
- **`forge run "task"` prints like `-p` whenever stdout is not a terminal.** A pipe or file got the whole line view (session header, MCP and routing lines, tool trace, cost footer: 5 of 6 stdout lines for `Reply with exactly: ok`), so `answer=$(forge run "...")` never held just the answer. It now holds the answer only, the trace goes to stderr with `--verbose` (or when stderr is a terminal), and the run stays in this process instead of being handed to a `forge serve` daemon, which could not return the answer. On a terminal the streamed line view is unchanged, and so is `/goal`.

### Fixed
- **The Codex bridge asks about the tool it really runs.** Codex calls Forge's deferred tools through `tool_call{name, arguments}`, and the permission prompt, the read-only/trust check, the duplicate-approval memory and the edit and verification ledgers all looked at the wrapper: the prompt read "runs a command on your machine" without the command, the same `touch` was asked twice, and an edit made through the wrapper counted as no edit. Every surface now unwraps the inner tool and its arguments (`forge_types::deferred_call`), `tool_search` lookups no longer show in the transcript, and the trust store's location (`XDG_CONFIG_HOME`, `FORGE_TRUST_PROJECT`) reaches the bridge's tool server, so read-only `git diff` no longer asks in a trusted project. The closing message of a Codex turn is now the messages after its last tool call, so the narration before a tool is no longer taken for an abandoned intention.
- **A completeness audit that searched from `run_code` is not sent to search again.** The identifier-migration audit counted only top-level `search`/`shell` calls, so a search the model batched in `run_code` (`tools.search(...)`, a grep through `tools.shell`) read as skipped and drew "You skipped the REQUIRED repository search": all 5 of those retries in the last `inferhub::cb/deepseek-v4.1-flash` parity run were false, 10-18 s each.
- **A task that names its checks gets only those checks.** The base prompt's "check only what your change touches" sent DeepSeek and Haiku to compile every crate they had edited on a task that says the other crates have to be handled by reading (100-180 s of `cargo check` per run, where Claude Code ran the one named check). When a task names the checks to use or says building is slow, the model now runs only those and confirms the rest by reading and searching.
- **The bridge's completeness review runs only when nothing checked the change, and a clean review answers in one line.** On the Claude Code parity suites the review ran on 69-100% of bridged tasks, added 10-15% of the wall time (on `claude-cli::sonnet` suites a/b/d, 139 s of the 118 s Forge was behind Claude Code), and changed no outcome once a check had passed after the last edit (5 edits after a review in 148 attempts and 3 in 60 SWE-bench Verified attempts, none deciding a result). A passing check after the last edit now ends the decision unless a check is still failing or the answer admits unfinished work; listed requirements and a large change earn the review only when nothing was checked. A review that finds nothing missing replies `No change needed.` instead of rewriting the requirement list nobody sees. Measured on `claude-cli::sonnet`, suites a/b/d, one attempt: 25 of 26 solved (Claude Code 2.1.308: 21 of 26), agent wall 823 s against Claude Code's 963 s (before: 1,081 s per attempt), input tokens 2.43M against 2.50M (before: 2.60M per attempt), reviews in 15 of 26 tasks (before: 36 of 52). `claude-cli::haiku`, suite c: 11 of 11, 513 s against Claude Code's 591 s (before: 719 s per attempt). See `docs/features/completeness-review.md`.
- **A bridged turn that edits files an earlier turn left uncommitted is no longer told it changed nothing.** The turn's progress marker was `git status --porcelain` plus `HEAD`, so editing a file that was already modified (or rewriting an untracked file) left it byte-identical. A bridged edit never reaches the direct-path edit counter, so in a multi-turn session on the claude bridge every later turn that touched the same files got "You have not modified any files. Implement the fix now" after its review, and spent a model round explaining that it had (3 of 7 turns per FB-sonnet multi-turn session). The marker is now the working-tree fingerprint the review already uses (HEAD, status, the diff against `HEAD` and each untracked file's content, test byproducts left out). It also feeds the tools-unavailable check and the migration audit's progress test.
- **A bridged turn's completeness review shows only that turn's change.** The review carries the change it reviews (`git status`, `git diff` against the turn's start commit and each new file), but in a multi-turn session with uncommitted earlier work that block held every earlier turn's files too, went past its 12 KB limit and fell back to telling the model to run `git diff`: one extra tool call and model round per review. The evidence is now limited to the paths this turn changed (the turn-start baseline the review gate already keeps), and a turn that changed nothing says so instead of sending the model to look.
- **A question about a file renamed into place no longer runs the identifier-migration audit.** The migration classifier matched "renam" anywhere in the prompt, so a read-only question about which temporary file dulwich writes before it is renamed into place drew the first-pass sweep guidance ("search for EACH old/deprecated identifier"), the completeness audit and its retry: two extra model rounds, about 40 s per run on `inferhub::cb/deepseek-v4.1-flash`, for a question with no old identifier in it. The atomic-write idiom and the rename call ("renamed into place", "renamed over", `os.rename`, `fs::rename`, `rename(2)`) are removed before the prompt is classified; "rename X to Y", "renamed flags" and the rest still classify (`crates/forge-core/src/completeness.rs`, see `docs/features/existing-test-guard.md`).
- **A failed shell command no longer waits for its explanation.** The shell error interceptor made its model call inside the tool call, so the failed command's result reached the model only after a trivial-tier completion: on an `inferhub::cb/deepseek-v4.1-flash` pin that was 2.7-3.1 s in 29 of 52 bench tasks, and most of those calls restated what the output already said (a test's assertion, "exit 1 is git grep's no matches status"). Now a failure whose output is the answer gets no call (a verification run that reported its verdict, a failed assertion, `grep`/`rg`/`git grep`/`diff`/`cmp`/`test` exiting 1); an unattended session whose side calls stay on its own pinned model (`forge run --model X`, `forge bench`) gets none either, since that model reads the failure itself on its next step; everything else runs on a detached task. The model is handed the diagnosis only when it is ready by the next request, the person sees it whenever it lands (on the TUI also after the turn is over), and the pattern table still answers common failures at once (`crates/forge-core/src/shell_diagnosis.rs`). FD on DeepSeek, a turn that runs one failing command, 10 runs each, interleaved: 7.5 s median against 5.4 s (paired median 2.2 s faster, faster in 10 of 10; the old build waited 2.4 s median on the diagnosis). On the 12 suite2 tasks where the interceptor fired most, one attempt each side by side: 11 of 12 solved on both builds (different misses, neither near a failed command), 10 inline waits totalling 26.7 s removed, no diagnosis call made. See `docs/features/shell-error-interceptor.md`.
- **The first request no longer waits for the pinned model's context-window lookup.** On a store that had never seen the pinned model (a first launch, every benchmark seed) the first request waited for the background window sweep, 0.6-1.2 s of the first answer in the TUI and in `forge run`. The wait exists so nothing is sized from the 32k floor that the real window would size differently, and a first request (system prompt, tools, one prompt) is under every limit the floor sets. The session now waits only when the context is past a quarter of the floor's compaction trigger or a request would not fit the floor's trim budget, and the step-start compaction check settles the lookup the same way (`crates/forge-core/src/window_lookup.rs`). `forge run --model inferhub::cb/deepseek-v4.1-flash` on a fresh store, 10 runs each, interleaved, two rounds: first token 2.9 s median against 2.0 s, and 2.7 s against 1.8 s (paired medians 0.73 s and 0.72 s faster, faster in 8 of 10 each).
- **A one-shot run on the Claude bridge starts `claude` while it routes.** `forge run --model claude-cli::haiku` spawned the bridged `claude` only when the first request was dispatched, after the turn's hooks and routing; it now prewarms it as soon as the session is built, as the TUI already did. Against a local fake Anthropic API (no quota), 10 runs each, interleaved: the first API request left 585 ms after start against 535 ms (paired median 38 ms earlier, earlier in 9 of 10), Claude Code 2.1.308 at 441 ms in the same rounds.
- **The clock line is to the minute.** The `[time]` line sits in the bridged `claude`'s first user message, ahead of the prompt, and with seconds in it no two first requests were ever byte-identical: every trivial `claude-cli::haiku` turn wrote its whole first message (2,266 tokens) to the prompt cache, where Claude Code reads its date-only prompt back. A repeated first prompt within the same minute now reads it (12,230 cached, 0 written, in 3 of 5 live runs; the old build wrote 2,266 in 5 of 5).
- **`forge run --resume` on a `claude-cli` model keeps what the model already read.** Each `forge run --resume` is a new process, and the claude bridge kept the claude session it drove only in memory, so the next prompt started a fresh claude session with the transcript rendered as text. Forge's transcript of a bridged turn holds the replies, not the tool calls that ran inside claude, so the model had lost every file it had read and every command it had run: in the multi-turn benchmark 57 of 76 `read_file` calls of `claude-cli::sonnet` re-read a file an earlier prompt had already read or written, and `claude -p --resume` (Claude Code's own continuation) needs none of them. The bridge now records the claude session of each conversation after every successful call (`<data dir>/bridge-sessions/`, removed when the next call starts, so a failed call or a process that dies mid-turn leaves nothing behind) and a later process `--resume`s it when the conversation, workspace and model match and the transcript still holds every message claude was given, unchanged and in order; anything else starts fresh as before. The same applies to a restarted daemon. Measured on multi-turn sessions S1 and S3, two seeds, base and fix side by side (4 session pairs, `claude-cli::sonnet`): tool calls per session median 30 to 20, wall time median 169 to 129 s (paired median -17.5 %), input tokens paired median -3.7 % with cache writes -62 %, every grader passed and every context probe scored 1.0 in both arms. One seed on `claude-cli::haiku`: 51 to 40 and 20 to 12 tool calls, wall -11 % and -35 %, input +25 % on the longer session (claude now keeps its tool results, as Claude Code does) and -25 % on the shorter. Single-turn tasks are unchanged (6 suite tasks: 5 of 6 solved in both arms). See `docs/features/persistent-bridge-transport.md`.
- **A bridged Codex task no longer reports its tokens twice.** `codex exec resume` (the completeness review, a guard or verification re-drive) puts the running total of the whole thread on `turn.completed`, and Forge recorded that as the turn's own usage, so each later resume counted every earlier model call again: a task whose thread sent 328,308 input tokens was stored as 266,085 + 328,308. On `codex-cli::gpt-6-luna` (26 tasks, two seeds) the store held 22.3M and 20.8M input tokens for threads whose Codex rollout files add up to 10.6M and 10.3M, so the bridge read as 5.1x the input of Codex CLI itself where it was 2.5x (43.2M recorded, 20.8M sent, against 8.5M). Each turn now records the difference from what its thread had already reported (`crates/forge-provider/src/cli_provider/codex_usage.rs`), so the token gauge, the spend ledger, `forge replay` and the benchmark metrics agree with what Codex sent; a total that goes down is taken as a new thread.
- **A model id nothing serves now fails at once, in one plain line.** `forge chat --model nonexistent-model-xyz` showed nothing at launch, waited 5 to 6 s (an unrecognised bare id is sent to the local Ollama server, and the refused connection took the pinned-outage backoff before a second identical failure ended the turn), then stacked a `pin_failover` warning, an unrelated 32k-context note and `provider unavailable: '... (adapter: OpenAI)'` under a border that said to send the message again. A provider's "no such model" answer in any of its wordings (DeepSeek, Mistral, Gemini, Ollama, Together, xAI, Azure as well as the OpenAI shape) is now permanent, and so is a bare id with no server behind it, so the pinned turn fails on the first call with `model 'x' isn't available (<why>). Run /model to pick another.`; the provider's text goes to `.forge/forge.log`. At launch the chat warns once for a pin nothing lists, with close matches from the same provider only (it used to offer the first five models of the whole catalog), and the composer border says `Model unavailable · Run /model to pick another`.
- **Esc on an answer that has just finished no longer prints "interrupted".** The loop cleared its busy flag one pass after the turn's last events were queued, so an Esc in that gap aborted a finished task, threw the queued events away (the answer's closing line and usage row) and printed `interrupted — stopped responding` under a complete answer. Esc now applies what the turn queued first and leaves a finished turn alone. A real interrupt says `Interrupted. Type what to do instead.` instead of wording that read like a hang.
- **A DeepSeek reply that writes its tool call as text no longer ends the turn with no answer.** `inferhub::cb/deepseek-v4.1-flash` answered a one-line question that only names a file ("what does `pkg/mod.py` do with X?") with its own DSML markup as the whole reply (`<｜｜DSML｜｜ calls>`, `<｜｜DSML｜｜ invoke name="read_file">`, a space after every namespace): 0 tool calls, no answer, and the raw markup stored as the answer, in 4 of 4 runs on two builds. Two defects stacked. The turn was classed trivial and the keyword check that decides whether a trivial turn is shown tools missed a prompt that only names a file, so the request carried no `tools` and the model, with no definitions to follow, improvised a call in the V4.1 dialect (`calls` as the wrapper, a space after the bars). And the DSML parser removed the namespace and then looked for `<invoke`, which the leading space hid, so the reply counted as a final answer and neither the recovery pass nor the retry guard fired. DSML now parses in all three dialects (V3.2 `function_calls`, V4 `tool_calls`, V4.1 `calls`), relaxed about bars and spacing and strict about structure: a block that is cut off, has a `string="false"` value that is not JSON, repeats a parameter or holds stray text runs nothing, and the model is asked twice to make a real call before the markup is cut from the answer. A prompt that names a source file (`a/b.py`, `src/lib.rs:12`, `Cargo.toml`) is now shown tools, and a model that writes a call as text, or whose call was recovered from text, gets the tools on its next request. Measured on `deepseek-v4.1-flash` over InferHub, two seeds each: the question that names a file went from 0 of 6 answered across three builds (every run ended on the raw markup) to 2 of 2, each with one `read_file`; a question that names no file, which still starts without tools, had its DSML call recovered into typed calls in 2 of 2 runs, got the tools for the next step and answered, where the previous build ended on the markup. Across the 67 earlier DeepSeek bench stores, 4 of 201 sessions (all of them trivial-tier turns that carried no tools) ended this way. See `docs/harness/text-tool-calls.md`.
- **A script pinned to one model no longer waits for model discovery on a fresh install.** `forge run -p --model inferhub::cb/deepseek-v4.1-flash` with no cached catalog (a new install, and every seed of a benchmark run, which starts with fresh config directories) held its first request for a bounded first-run discovery that asks every installed agent CLI what it can run: `agy models` started the six MCP servers of the Antigravity configuration plus its status line, `claude -p` started for a capability probe, then `claude --help` and `codex debug models` ran. The first call took 3.9 s (3.7-5.3 s, n=5) against 1.9 s warm, started 10 processes and peaked at 954-1110 MB across the tree. A session pinned to one model that names its provider (no `pin_failover`) routes nothing through the catalog, so it now neither waits for discovery nor starts a refresh it would not outlive: the same call takes 2.1 s (1.9-2.4 s, n=5), starts nothing and peaks at 176 MB. The remaining 0.4 s over a warm call is the context-window lookup for a store that has never seen the model. A pinned TUI still refreshes behind its first frame, and a pin set, `pin_failover` or a bare alias still read the catalog (`crates/forge-cli/src/cli/commands/run/catalog_startup.rs`).
- **Model discovery no longer starts your Antigravity MCP servers, and every probe yields to the turn.** `agy` has no flag that skips its MCP servers, hooks or status line, so a plain `agy models` started every server in `~/.gemini/antigravity/mcp_config.json` before it printed a list (1.07 GB for 1.8 s). It now runs with a scratch `HOME` that holds a link to its OAuth token and nothing else (490 MB, no MCP server, the same 13 models), and asks again with the real environment only when agy answers that it is signed out. The `claude` capability probe runs with `--strict-mcp-config` in an empty directory (it no longer lets claude index the project you launched Forge in) and beside `claude --help` instead of before it. Every probe runs at nice 10 in its own process group, which a timeout now kills whole instead of orphaning the servers the CLI started, and the catalog cache is written by rename, so a refresh cut short by the end of a one-shot run leaves the previous file instead of a truncated one.
- **A "Final pass" request no longer arms the keep-the-tests-green guard.** The guard read the noun "pass" as "the tests must pass", so in an unattended run (`forge run`, bench) a last prompt such as "Final pass: rerun everything and confirm each change is covered by a test" armed it for the rest of the session and added the "existing tests describe the current behavior" text to the turn. Where the model had widened an existing test import line in an earlier turn, the end-of-turn check then restored that line from the turn-start commit although the model had just put the original back, and sent it round again. Multi-turn suite, session S3 on `cb/deepseek-v4.1-flash`, five seeds each on the same base: that last prompt took 4 to 14 model calls and 93 to 475 k prompt tokens (median 11 calls, 332 k; three of five seeds restored and re-driven), against 3 to 6 calls and 62 to 176 k (median 4 calls, 100 k) with the fix, and the session median fell from 800 k to 551 k prompt tokens. A "pass" after an article, an ordinal or a quantity word ("final", "a quick", "one more", "second", "review") is a round of work and arms nothing; "the tests pass", "must pass", "it passes" and "keep passing" still do.
- **A bridged model is no longer taught a tool it cannot call.** The `run_code` guidance ("end with `return`", "`await tools.<tool_name>(...)`") rides with the tool and is meant for direct models, but the request builder sizes every request against the direct tool catalog, so the block was also sent on `claude-cli` and `codex-cli` turns. In Codex's code mode a script is not a function body: 13 of the 20 failed `exec` scripts in a `codex-cli::gpt-6-luna` run (26 tasks x 2 seeds) were `SyntaxError: Illegal return statement`, each one a wasted model call, and the block added about 0.8 KB to every request on both bridges. A request for a bridge model no longer carries it (`crates/forge-core/src/routing_policy.rs`); seven tasks run after the change have no failed script.
- **A harness nudge no longer counts as something the person asked for the tests.** The existing-tests guard reads the session's user-role messages, and the harness stores its own nudges as user messages. The push-back sent after a restore ("fix the production code rather than the tests") read as a request to change tests and lifted the protection it had just enforced for the rest of the session; a nudge that says tests must pass could arm the keep-it-green guard on a request that never mentioned it. Only what the person typed is read now.
- **The existing-tests guard no longer sends a model back to old behavior that its own correct change replaced.** It put back every edited existing test and told the model that "hidden evaluation uses the ORIGINAL tests", which Forge cannot know (SWE-bench's tests are the upstream ones, and they are updated when the issue changes behavior). On django-13821 the bridged haiku made the right edit (the error message and the test that asserts it), the guard restored the test, and the model then reverted its own fix to match the old one: 1 of 3 solved against 3 of 3 for the same model without the guard. On a task that loosens one assertion, "do not rewrite ... what they already contain" kept opus and haiku from touching it, and they reported a red test. Edits are now sorted. Put back, with the reason per file: a deleted test file, more assertions removed than added, an added skip/xfail/ignore marker, an assertion that cannot fail, changed lines with no code change behind them, and any change when you said to leave the tests alone. Passed: appended tests, a reorder, a re-indent. Kept: changed lines of an existing test when code outside the tests changed too; the model is asked once, with the changed lines quoted, whether the request changes what the test pins down, and is never told to revert. A request that asks for test changes ("update the tests") has only the hiding edits put back, in code-change runs too: a worktree build session used to arm the strictest level whatever you asked, and restored an appended test. No text the model sees claims how the work will be judged any more: the push-back, the guidance, the minimal-diff bias and the completeness audit prompts no longer mention hidden tests or evaluation. The skip and assertion patterns are shared with `bench integrity`, so the live guard and the scorer agree on what hides a failure. Replayed over the 44 existing-test edits in 26 recorded SWE-bench patches, the old guard restores 44 and the new one 0 (16 asked, 28 passed). FD-deepseek, two attempts: five suite2 tasks 10 of 10 (main on the two tasks it was run on: 2 of 4, both misses a docs file, not the guard), and django-13821 2 of 2 with the question answered and the edit kept; samples this small show that nothing regressed, not that the pass rate moved (`crates/forge-core/src/test_guard.rs`, `crates/forge-core/src/completeness.rs`, `crates/forge-cli/src/bench/integrity.rs`; see `docs/features/existing-test-guard.md`).
- **A chat keeps working after the binary it started from is replaced.** On Linux a replaced executable reads back as `<path> (deleted)`, and a `claude-cli` or `codex-cli` chat handed that path to its CLI as the Forge tool server, so every later turn failed with `Forge MCP startup failed` while the session stayed open. Self-launches now go through `/proc/<pid>/exe`, which also keeps the child on the same build as its parent; OS timers written by the daemon keep the installed path. The chat notes once that it is still running the pre-upgrade build.
- **A restarted daemon no longer resumes a session that a terminal has open.** The restore treated a heartbeat older than a minute as "gone", so a chat whose loop was briefly stalled got a second driver beside it and lost its presence for good. A session with a terminal claim (fresh heartbeat, or a quiet one whose loopback control channel still answers) stays with the terminal, and the daemon drops its fleet flag for it. A terminal that opens a session now clears that flag, claims the session before the session build, and puts its claim back if another process clears it.
- **A bridged Codex no longer spends a model call, and a seventh of its input, learning Forge's tool list.** Codex defers MCP tools behind `ALL_TOOLS`, and the bridge preamble told the model to dump every `mcp__forge__*` description before it began: 17.7 KB for the 17 tools, one model call in 50 of 52 baseline sessions (13 of 13 in both 13-task seeds, 2 of 13 now), then about 4.7k tokens in every later request (an estimated 15% of the arm's input tokens on `codex-cli::gpt-6-luna`). The tools are callable with no lookup (checked against Codex's Responses stub: `tools.mcp__forge__list_dir` runs in the first script), so the preamble now spells the arguments of the 12 core tools, tells the model to print `r.content...text` and not `JSON.stringify(r)` (which escaped every quote and newline of each result into the context), and to wait on a yielded cell with one `wait` of a large `yield_time_ms` rather than a model call every 30 s (`d04-long-build-await`: 27 calls and 440 to 645 s became 9 calls and 212 s). With the neighbouring bridge fixes, 13 tasks sent 2.99M input tokens in 138 model calls where the same 13 sent 4.91M and 5.59M in 187 and 204 (tool output into the context 0.31 MB against 0.59 and 0.63; failed `exec` scripts 1 against 9 and 5). A test checks every signature against the served tool schemas.
- **A failed `/resume`, archive toggle or observe no longer ends the chat.** The loop returned the error, so after an upgrade migrated the store the old build's `/resume` closed the terminal. The failure is shown in the transcript instead.
- **An empty session someone has open is no longer deleted by another Forge process.** The startup sweep removed sessions with no user message older than ten minutes without checking whether a terminal or the daemon held them.
- **A prohibition that names a path, a tree or a list of protected files no longer makes the turn read-only.** `Do not modify anything under logs/, ops/ or data/, nor README.md, the existing tests, ...` matched the bare phrase `do not modify anything` anywhere in the prompt, so a debugging request that ended in a list of protected files ran under the read-only contract: shell and every write denied, the model diagnosed the crash and ended with no edit (the a04 task of the Claude Code parity bench: 0 of 5 Forge runs against Claude Code's 5 of 5; the same on the Codex arms). The same blindness sat behind `without changing files under tests/`, `make no edits to ...`, `Don't change anything else`, `Don't edit it, it comes from finance`, a bare `Do not edit:` followed by a bullet list, a protected file named `code.py` and `Read-only files: data/, logs/`. A prohibition now reads as read-only only when it names no part of the tree: the object is `anything`, `files` or `any file` (or, when the prompt asks for no change, `code`, `the repo`, `it`) and nothing narrows it (`under`, `outside`, `other than`, `except`, `else`, `unrelated`, a named path, a list, a colon). True read-only requests are still detected, and more of them: `Don't touch anything`, `without editing any files`, `Do not make changes to the code`, `make no edits whatsoever`, `Do not edit any file in the repo`, `This is a read-only question`. Of 114 change-request phrasings 38 were read as read-only before and none are now; of 77 read-only phrasings 29 were missed before and none are now. Measured on `inferhub::cb/deepseek-v4.1-flash`: a04 goes from 0 of 2 solved (no edit made, read-only contract injected) to 2 of 2 with no contract; c08 stays 2 of 2; two explicit read-only question tasks are still answered with no edit, under the read-only contract, in 4 of 4 runs (`crates/forge-core/src/turn_contract/read_only.rs`, `docs/features/turn-contract.md`).
- **A mistyped `/command` during a turn no longer freezes the TUI.** An unrecognised `/name` typed while a turn ran awaited the session lock the turn holds, so the chat stopped drawing and reading keys (not even Esc worked) until the turn ended, and for good once the turn asked a question (`ask_user`): the turn waited for the UI, the UI for the lock. On an unattended host that read as a hang with the process alive at 0% CPU. A command or skill typed mid-turn now says `finish or Esc the current turn first` without touching the session, and a phone's image attach mid-turn waits for the lock off the render loop too. Reproduced with a slow streaming stub on `forge chat`: `/definitely-not-a-command` froze it (all 57 threads parked; the turn's worker blocked in `ask_form` waiting on the UI), `/model`, `/compact` and plain text did not.
- **A completeness review no longer spends a model call fetching the change it reviews.** The review nudge told the model to run `git status --short` and `git diff <start>` and to read each new file, so its first step in the review was a tool call, a full request of the whole context, to see what Forge could have handed over. Forge now runs the two commands (without test byproducts), reads the new files, and puts the output in the nudge as data to check (`crates/forge-core/src/review_evidence.rs`); a change over 12,000 bytes, or a new file over 6,000, keeps the old instruction. Through the Codex bridge (`codex-cli::gpt-6-luna`) a review turn took 2.4 and 3.3 model calls in two 26-task seeds (1.4 and 2.3 of them tool calls) and takes 1.4 (0.4 tool calls, 7 of 10 reviews with none) over 13 tasks; through the Claude bridge (`claude-cli::sonnet`, 4 tasks) 1 or 2 assistant events against 2 or 3. Which turns are reviewed does not change.
- **`/rename` works while a turn runs.** It was queued until the turn ended, so a long turn left the old title in `/sessions` and the apps. It now writes the title through the session's detached store handle, off the render loop, and says so at once.
- **A model that fills in every property of a tool no longer has its calls refused.** On `codex-oauth::gpt-6-luna` every direct `shell` call carried `commands: []` next to `command` and was rejected (``pass either `command` or `commands`, not both``): 117 calls in 52 benchmark runs, 23 of the 23 direct calls in a 12-run re-measurement, and the transcripts of the lost b01, b03 and d01 runs held rejected calls (the bridge's 334 `shell` calls never carried it). The Responses request left `strict` out, which that API reads as `true` (every property present); it now sends `"strict": false` like Codex and the other OpenAI paths (`crates/forge-provider/src/oauth_responses.rs`). Whatever a model still sends for an optional property it is not using (`commands: []`, `paths: []`, `path: ""`, `cwd: ""`, `timeout_secs: 0`, `null`) is now removed from the call before the permission rules, hooks and the tool see it, from the tool's own schema (`forge_tools::normalize_args`; direct loop, `tool_call` and `run_code` inner calls, subagents and `forge mcp-serve`; external MCP tools and the transcript are untouched). `shell`, `read_file` and `multi_edit` also read a blank `command`, `path` or per-edit `path`, an empty `commands` and a zero line number as not given, a one-entry `commands` repeating `command` as that one command, and `forge_types::shell_command_text` judges the batch when `command` is blank, so a blank string cannot hide a batch from a deny rule. Counts that cannot be 0 now say `"minimum": 1` in their schema (`head_limit: 0` used to return nothing). Six native-loop tasks (b01, b03, d01, a04, d06, a05), 2 seeds each, `codex-oauth::gpt-6-luna`: rejected `shell` calls 30 on the first Codex run, 23 on main, 0 after; empty placeholders across all tools 105, 65, 2; tool errors 54, 31, 21; solved 6, 7 and 8 of 12 (within noise: a05 s1 lost to the existing-tests guard and a04 to the read-only contract on every arm). See `docs/features/tool-argument-placeholders.md`.
- **`apply_patch` applies a Codex patch whose chunks follow one another.** A chunk of context lines with no `-`/`+` line was turned into an edit whose `old` and `new` were identical and refused the whole patch (15 of the 19 failed `apply_patch` calls in the first native-loop runs, and b01's retry loop), and a chunk such as a bare closing `}` was refused as ambiguous when it occurs dozens of times. Chunks are now applied as Codex reads them: in file order, each on the first whole-line match after the previous one, a `@@ <line>` header or a context-only chunk naming lines it must come after. The first chunk of a patch still has to match exactly once, a scope that is not in the file is ignored, and a patch of only context says so instead of claiming an identical edit (`crates/forge-tools/src/core_tools/codex_patch.rs`, `crates/forge-tools/src/core_tools/edits.rs`). The patch `gpt-6-luna` sent for b01, replayed on the task's original `src/cli.rs`, was refused before and applies after.
- **`forge bench tasks --agent codex` runs against Codex 0.163.** It passed `--full-auto`, which `codex exec` removed (`unexpected argument`), so every instance failed to start. The driver now asks the installed codex (`codex exec --help`): `--full-auto` while it exists, otherwise `--sandbox workspace-write -c approval_policy="never"`, the same sandboxed, never-ask mode; `--dangerously-bypass-approvals-and-sandbox` stays a choice for the person running the benchmark (`crates/forge-cli/src/bench/external_agent.rs`, `docs/benchmarks/swe-bench.md`).
- **A benchmark attempt no longer waits for the recap and the memory capture that `forge run` skips.** `forge bench tasks` and `bench swe` built their Forge session without marking it one-shot, so after the answer they made two more model calls (a one-line recap, then a memory extraction) before the attempt ended and its wall time was read, which `forge run` never does. On `inferhub::cb/deepseek-v4.1-flash` over 26 tasks x 2 seeds the answer was followed by a median 5.0 s (mean 5.6 s, 4.4% of all attempt wall time) of these calls, and a trace of six more tasks shows 4.8 to 8.2 s each, so the Forge-vs-Claude-Code wall comparison charged Forge for work the product does not do (FD median 69.9 s vs 59.6 s for Claude Code on the same model). The bench session now goes through the same `mark_one_shot` as `forge run` (`crates/forge-cli/src/bench.rs`); in a concurrent A/B over 15 tasks the baseline binary's sessions had a median 5 s (mean 6.1 s, max 14 s) between the final answer and the last stored row and the sessions with the change had none (median paired wall difference -5.4 s with one outlier task excluded).
- **The bridge's completeness review no longer counts what a test run leaves behind, and reads real check results.** Through the Codex CLI bridge (`codex-cli::gpt-6-luna`, 26 tasks x 2 seeds) Forge's end-of-turn `exec resume` turns (43 completeness reviews in 52 sessions, plus guard and verification re-drives) were 64 percent of the arm's input tokens. The gate was already shared with the Claude bridge; what it measured was wrong in four ways. A repository without a `.gitignore` lists `__pycache__/*.pyc` as untracked, so a two-line fix plus one pytest run read as a four-file change, and the review then showed the model the cache in `git status` and the model spent its review and the next verification turn deleting it (a01: 3 extra turns, 0.92M input tokens, 50 s for a two-line fix; 13 of 52 sessions spent tool calls on `__pycache__`/`.pytest_cache`). Untracked files a command wrote (a started server's `.state/`) counted as edited files (d05). Work an earlier turn left uncommitted counted again on every later turn. And the evidence was inverted: Forge's `shell` answers `shell: exit 101 ...` as an ordinary result, so codex marks the call `completed` and claude `is_error: false`, a failing `cargo test` was recorded as a passing check, `FailingCheck` could never fire, and a `commands` batch (`["cargo build", "cargo test"]`) was read as a script with an unchecked status and never counted as a check, on the direct loop too (b06 was reviewed as "unchecked"). Build and test byproducts (`__pycache__`, `.pytest_cache`, `.mypy_cache`, `.ruff_cache`, `.tox`, `node_modules`, `.gradle`, `.forge`, `*.pyc`, a top-level `target/`) are no longer changed files; untracked files count toward the size of a change only when a file tool wrote something this turn; a turn's size is measured against what the tree held when it began; a bridged `shell` call whose result is not `exit 0` is a failed call; each entry of a `commands` batch is classified on its own and the batch header says which one failed (a failing `git diff --check` after a passing test run is not a failed test); sweeping up `__pycache__` no longer stales a passing check. The signals and thresholds are unchanged. Same eight tasks, one run each, `origin/main` against this change: a01 0.92M to 0.14M input tokens and 50 s to 18 s (3 extra turns to 0), d05 0.40M to 0.14M and 38 s to 26 s (1 to 0), 4.46M to 3.67M input tokens and 17 to 13 model calls over the eight, 7 of 8 to 7 of 8 passing (b06's failure was the model naming the wrong commit before any review; its rerun passed); the final head passes a01, a05 and b06 again at 0.20M, 1.16M and 0.75M (`crates/forge-core/src/review_gate.rs`, `crates/forge-core/src/completion.rs`, `crates/forge-core/src/working_tree.rs`, `crates/forge-provider/src/cli_provider/cli_stream.rs`, `docs/features/completeness-review.md`).
- **A review or verification turn on a CLI bridge no longer replaces the answer to a question.** On the multi-turn session S1 the memory probe ("answer purely from our conversation, without opening any files or running any commands") was answered correctly, and then Forge saw the finished task list of the earlier turns, told the model "You reported every task Done. Before this turn can end, prove it", the model ran the test suite, and the turn's result was that run's line, "The requested verification is complete: `python3 -m unittest discover -v` passed all 19 tests." (probe 0.0; S2 0.67 the same way; Codex's own loop and the native loop 1.0). Three causes. The bridge loop re-verified an all-Done list without a tool call or change in the turn (the direct loop had that rule); the settle step called the tree changed whenever the re-drive had run the tests, because the fingerprint counted every `__pycache__/` file by name; and a re-drive that did change the tree replaced the answer with its own one-liner. The gate is now one rule for both loops (`Session::turn_did_work`), the tree fingerprint hashes the bytes of untracked work files and skips test-run byproducts, and a re-drive that changed the tree shows the original answer and then what it did (`answer_after_review`). S1 after: one model call for the probe, probe 1.0, the same grader score (`crates/forge-core/src/completeness.rs`, `crates/forge-core/src/model_loop.rs`, `docs/features/completeness-review.md`).
- **A request that says to update the tests is no longer told the tests are immutable.** The bench task a05 asks to "Update every caller, the tests, the README ... Don't touch `package.json`, `tsconfig.json` or `test/helpers.ts`, and keep the existing tests (updated for the new signature, not deleted or skipped)". The `test` in `test/helpers.ts` read as a ban on the tests, Forge injected "Existing tracked test files are immutable for this session", and through the Codex bridge the model obeyed, left `npm test` red and said so (0 of 2 seeds; Codex's own loop 2 of 2). A prohibition now names the tests only when the word `test`/`tests` is the object of the "don't edit/change/touch ..." clause (a path is not the tests, and the clause ends at ", and ..." or a parenthesis); a change verb pointed at the tests ("update every caller, the tests", "adjust the tests to match", "fix the failing test") lifts the guidance and the `AnyEdit` level; across a session the latest request that speaks to the tests decides. a05 on the Codex bridge before and after: fail, 0.94M input tokens, 85 s to pass, 0.54M, 51 s (final head: pass, 1.16M, 76 s); on the native loop (`codex-oauth::gpt-6-luna`) it still passes, without the contradicting message (`crates/forge-core/src/test_guard.rs`, `docs/features/existing-test-guard.md`).
- **A silent shell command no longer looks like new output every time it runs.** Its whole result is the header `shell: exit 0 in 42ms`, and the timing made each run a different line, so a loop of silent commands never read as stale (`crates/forge-core/src/loop_progress.rs`).
- **A fresh Claude bridge process starts about 40 ms sooner.** Before it writes the first prompt the bridge polls claude's MCP status until Forge's tool server is `connected`, and slept 50 ms between polls although the server answers `initialize` about 45 ms after it is spawned, so the poll that found it ready landed up to one interval late. The interval is now 10 ms: from `bridge: spawn claude` to `bridge: forge mcp ready` the median went from 351 to 298 ms and from 316 to 282 ms in two interleaved runs of 12 and 20 spawns per arm, with claude run without a login so no quota is used. The rest of that wait is claude's own start; `docs/features/turn-latency.md` now lists where the 165 ms before the spawn goes.
- **A long bridge plan is no longer cut off by a fixed re-drive count.** The budget of 8 continue re-drives now counts re-drives that close no task, so a plan that finishes a task per re-drive can run to the end (`crates/forge-core/src/model_loop.rs`).
- **Signing in to ChatGPT no longer ends on a 400 about a model you did not choose.** The check after `forge auth codex-oauth` asked for `gpt-5.4-mini`, which the backend refuses for ChatGPT accounts (`400 "The 'gpt-5.4-mini' model is not supported when using Codex with a ChatGPT account"`), so a login that had worked printed an error. The check now runs on a model taken from the catalog the backend returns for that account (a small tier when it has one, never a WebSocket-only id), and still reports a dead or non-entitled login as before (`crates/forge-provider/src/codex_oauth.rs`, `crates/forge-provider/src/codex_oauth/models.rs`). The success message no longer suggests a hard-coded model; `forge models` lists what your plan can run.
- **`codex-oauth` lists the models your ChatGPT plan can run, on a fresh install too.** The list was a bundled table plus whatever a Codex CLI on the same machine had advertised, so a machine without that CLI saw `gpt-5.4-mini` (refused for ChatGPT accounts) and none of the `gpt-6` family, and `forge run --model codex-oauth::gpt-6-luna` warned `unknown model`. Forge now asks the backend (`GET /backend-api/codex/models`, the catalog the Codex CLI caches in `~/.codex/models_cache.json`) and remembers the answer; if the backend cannot be reached it falls back to the last answer, then the Codex CLI's list when that CLI uses a ChatGPT login, then seeds that hold only models the backend listed for a ChatGPT account. A model OpenAI ships appears without a Forge release (`crates/forge-provider/src/codex_oauth/models.rs`, `docs/features/codex-oauth.md`).
- **The Codex CLI bridge no longer seeds `gpt-5.4-mini` and the retired `gpt-5.x` ids as its fallback models.** The table used when `codex debug models` fails and nothing is cached now mirrors the models the backend lists for a ChatGPT login; an API-key login still gets `gpt-5.4-mini` from the live probe, which is the primary source. The docs, quickstart and `scripts/bridge-e2e.sh` use `gpt-5.6-luna` instead of the model a ChatGPT login is refused (`crates/forge-provider/src/cli_provider.rs`).
- **A `run_code` script can update the task list.** `update_tasks` was on the script deny list, so a script that led with it, as `gpt-6-luna` did (`tools.update_tasks(...)` followed by its first reads), died on its first line with `` `update_tasks` cannot be called from run_code `` and discarded every call batched after it, wasting the step. The payload was never the problem: the same call succeeded as a normal tool. It only rewrites the model's checklist, so it now runs inside a script like any other tool and is audited as an inner call; the other session-control tools stay refused (`crates/forge-core/src/codemode/catalog.rs`, `docs/features/codemode.md`).
- **Opening the TUI in an indexed project no longer burns ~0.6 s of CPU and ~40 MB 1.5 s after the first frame.** The background Lattice update asked for each file's language, and the first such call compiled the tags queries of all 19 grammars on that thread (and again on every other thread that touched Lattice: the watcher, the read tool's outline), even when nothing needed parsing. A language is now detected from its extension alone and its query is compiled once per process, on the first file of that language that must be parsed (`crates/forge-index/src/extract.rs`). `forge lattice update` with nothing to parse went from 0.63 s / 79 MB to 0.04 s / 35 MB, `forge lattice status` from 0.72 s / 74 MB to 0.03 s / 34 MB, and the TUI harness's idle sample (the 2 s after ready, `inferhub::cb/deepseek-v4.1-flash`, `s01`) from 22-30% CPU / 209-219 MB to 6-10% / 179-185 MB. A cold index of this repository (1,588 files, 23,649 symbols) is unchanged in result and 5.7 s to 5.1 s of CPU. Steady state was already 0-1% CPU in one process.
- **A bridged edit or command now says what it is asking to do.** A claude-cli turn names Forge's tools `mcp__forge__<tool>`, and the TUI looked them up by the bare name, so the permission prompt read `Allow Edit of ?` and `Allow Shell to run this command?` with no file or command, the card showed the new text instead of the path, and the turn folded to `Called forge 4 times`. The namespace is dropped when the call arrives, so the prompt reads `Allow Edit of calc.py?` (absolute paths inside the session directory are shortened like a native call's) and shows the command, and the calls group as reads, edits and runs.
- **A one-line fix no longer costs a second and third approval in Ask mode.** Through the claude-cli bridge on a fresh permission config, `In calc.py the add function has a bug... Make only that change.` raised three prompts (the edit, the model's own check, `git add && git commit`), where Claude Code raised one; the baseline run's third prompt was the bridge's completeness review, which now runs only on evidence. `[git] commit_policy = "auto"` is `unit` only while committing is free for the user: when the permission broker would stop at the model's own `git add && git commit` (the Ask posture, no allow rule for them), it resolves to the new `request` policy, which tells the model not to commit unasked and silences the commit reminders; `/commit` and "commit this" still commit. An allow rule for both commands, "Allow shell for this session", or a posture that runs shell unasked keeps `unit`, an explicit `commit_policy` always wins, and the permission broker is unchanged. Live haiku runs on a fresh config: 3 prompts in both of two runs before, 1 prompt in all three after (the model also stopped running its own check, which is its choice, not the broker's).
- **A session pinned to a Claude bridge no longer makes its first prompt wait for the bridge to start.** The first turn started `claude`, which started `forge mcp-serve`, and waited for the tool server to connect before the prompt could be written: 1.0-1.5 s between Enter and the first request, against 0.06 s for Claude Code. A new conversation pinned to one `claude-cli::` model now starts that process when the TUI starts (`Provider::prewarm`, `CliProvider::prewarm_live`), and a prompt typed before it is ready waits for it instead of spawning a second one. On a stub API with 0.25 s of model latency the first answer text comes 0.39-0.44 s after Enter (was 1.2-1.9 s; Claude Code 0.41-0.55 s). Conversations with history are not warmed, since they resume claude's own session.
- **The first prompt after Esc no longer waits two seconds for the abandoned bridge process to die.** The teardown slept the whole 2 s kill grace after SIGTERM even when the process had exited at once; it now waits for the process group to be gone (up to the same grace) before SIGKILL. The next prompt after an interrupt reaches the API 2 s sooner.
- **The TUI is ready in about 0.3 s on a gateway model again.** `forge chat --model inferhub::cb/deepseek-v4.1-flash` on a store that had never learned the model's context window held startup for the window sweep (600-1100 ms, a network round of ~35 hosts); ready went from 240-420 ms to 840-1300 ms. The sweep now starts at session build and the first request waits for it right before sizing it (`pin_window::start_pinned_window_lookup`, `Session::settle_window_lookup`), so the pinned window is still known before the first request is sized and startup waits for nothing. Ready is 260-350 ms with a fresh store.
- **Starting the TUI on a pinned session no longer spends a model turn on a quota probe.** The Claude rate-limit probe (a real `claude` start plus a haiku turn, ~116 KB of request) ran at every TUI start with a stale reading, also for sessions pinned to another provider whose routing it cannot affect. It is skipped for pinned sessions and runs without hooks, CLAUDE.md or tool definitions otherwise (17 KB of request, 3.2 s to 2.0 s).
- **A pinned TUI no longer waits for model discovery on a machine's first launch.** With no cached catalog the session built inline discovery first, bounded at 12 s (0.5-0.8 s measured with no keys configured), before the first frame. A pinned, attended session routes nothing through the catalog and outlives the background refresh, so it now starts at once and the refresh lands behind it: ready in 0.39 s instead of 1.0-1.2 s on a fresh `HOME`. Unpinned and unattended sessions still wait, since their first route reads the catalog.
- **The completeness audit no longer crashes a turn when compaction runs inside it.** The audit remembered where the transcript ended before its model loop, then sliced from there afterwards; auto-compaction inside the loop shrank the transcript below that index and the process aborted with `range start index 16 out of range for slice of length 7`, losing the attempt (2 of 8 compaction-heavy `forge bench tasks` runs). Positions are now marks that compaction keeps valid, and a rewind invalidates them instead of pointing at other messages (`crates/forge-core/src/transcript_mark.rs`). `forge bench tasks` and `forge bench swe` also record a panicking task as a failed attempt instead of losing every attempt not yet written.
- **Sessions and bench tasks sharing a store no longer fail with `database is locked`.** SQLite's stock busy handler polls every 100 ms, which never lands in the gap between the transactions of a writer committing in a loop: in 4 of 5 trials it starved a second writer for its whole 5 s timeout, which surfaced as `building session: starting session: database is locked`, `running the agent turn: database is locked` and lattice `stale-root prune failed`. Connections now wait with millisecond polls for up to 30 s (5 of 5 trials got the lock in under 5 s), lattice root and stale-file prunes delete 16 files per transaction with a pause between them instead of one statement per root, PageRank batches pause the same way, and an index update whose workspace was deleted mid-walk stops instead of issuing one delete per remaining file (`crates/forge-store/src/busy.rs`, `crates/forge-store/src/lattice_store.rs`, `crates/forge-index/src/lib.rs`).
- **A pinned model's real context window is used from the first request.** A store that was never swept — every task of `forge bench tasks` gets its own `FORGE_DB` — knew no context windows, so a run pinned to `inferhub::cb/deepseek-v4.1-flash` (1,048,576 tokens) was sized from the 32,000-token floor: it auto-compacted at 25,600 tokens (13 messages → 7, three or four times per question on a 390k-line repository), clipped tool output and dropped older messages from the request. A session now looks up the window of any pinned model nothing knows yet before it starts (one bounded sweep, at most 6 s), and warns when it is still unknown. Measured on two 390k-line suite tasks with deepseek, each on a fresh store: compactions 3 → 0 and 4 → 0, no clipped tool output or trimmed messages, uncached input 175,548 → 95,402 and 196,268 → 45,123 tokens now that the prompt cache holds (90-93% of input cached), and the second task went from unsolved to solved. Total input on the first rose from 0.39M to 1.35M tokens because every request now carries the whole conversation, the same as Claude Code's 1.43M on that task (`crates/forge-cli/src/cli/commands/run/pin_window.rs`).
- **A pinned run keeps its side calls on the pinned model.** The shell-error diagnosis, recap, suggestion, memory capture, auto-mode classifier, `prompt`/`agent` hooks and goal evaluator of a session pinned with `--model` or `/model` went to the first usable trivial-tier model, so a run pinned to deepseek diagnosed a failed command on `groq::llama-3.1-8b-instant` and its token and cost totals were incomplete. They now use the pin, and a subscription pin (a CLI bridge, a ChatGPT plan) skips them, so neither the pin nor the plan is left; `mesh.pin_failover = true` restores the old choice (`crates/forge-core/src/btw_policy.rs`).
- **`warning: N model(s) benched` is no longer printed on every scripted run.** The startup notice is advice for a person, so it now shows only when someone is attending the session, or when a model the run is pinned to is the one benched (`crates/forge-cli/src/cli/commands/run/session.rs`).
- A change request that protects one file ("fix it, and don't edit the existing tests") is no longer treated as a read-only turn. `do not edit` / `don't edit` / `do not make changes` now only forbid writing when nothing narrower follows them; the scoped form had denied every write, so Forge diagnosed the bug and reported it lacked permission to fix it (3 of 26 benchmark tasks lost on DeepSeek).
- **A prohibition that names part of the tree no longer makes the turn read-only.** `do not edit` followed by a verb list ("do not edit, delete, or rename the existing tests") ended its clause at the first comma and read as a universal ban, and `files`, `code` or `any file` followed by a narrowing word ("do not edit files under tests/", "don't edit code outside src/parser", "don't edit files other than ANSWER.md") matched as the whole tree. Either one put the turn under the read-only contract: shell and every write were denied, the model diagnosed the bug and said it lacked permission, and on a Claude bridge the same contract text made a sonnet session decline to edit (seen on the dulwich bug in the large-repo benchmark, where the sentence was the one #1603 already fixed). 11 of 29 probed phrasings of this shape were misread; "in any way", "in place" and "yet" still leave a ban universal (`crates/forge-core/src/turn_contract.rs`).
- **"Keep the existing tests green" now protects the existing tests.** The existing-tests guard fired 0 times in 130 baseline bench runs while 13 Forge runs failed by rewriting an existing test. It was armed only by `bench swe` or by a prompt that forbade editing tests in one phrasing, it cut a sentence at the `.` of `./...` (so "keep `go test -race ./...` green" never matched), and it read `git status`, which is clean once the model has committed its own edit. A prompt that says the tests must keep passing or stay green, and does not ask for them to change, now arms it. It compares the tree with the commit the turn started on. It counts rewritten, reordered or removed test content but lets a test appended to a file, or a new test file, through. When the edit was already committed it puts the originals back and keeps the model's version as a patch in the git directory, instead of stashing nothing. The push-back no longer claims a hidden evaluation when a person asked (`crates/forge-core/src/test_guard.rs`).
- **A rename no longer forces an alias, or an edit to a frozen contract.** The identifier-migration prompts called any path that still reads the old name "a CONCRETE OMISSION" until it kept an alias, so a task that renamed `User` to `Account` in a repo whose `docs/COMPAT.md` froze an environment variable got a `BILLING_ACCOUNTS_FILE` alias and an edit to `COMPAT.md`. The alias rule now covers a deprecation only; a plain rename needs no alias unless the task asks for compatibility, and a name the repository declares frozen or public, and the file that declares it, are left alone. The bounded production-scope sweep is unchanged (`crates/forge-core/src/completeness.rs`).
- **The built-in deny rules refuse less than they meant to.** `.env.example`, `.env.sample`, `.env.template` and `.env.dist` can be read: the `.env.*` rules covered the committed template next to the real `.env`, in `read_file`, `list_dir`, `search`, `glob` and the shell verbs, and Auto mode no longer asks about them. One denied entry in a batched `read_file` withholds only itself and the rest of the batch is returned with a note naming what was withheld. `rm -rf /tmp/scratch` works: the `rm -rf /*` glob matched every absolute-path delete, and a multi-statement line containing `rm -rf /tmp/x` hit the literal `rm -rf /` fallback; deleting named paths under the system temp directory is allowed, while `/tmp` itself, `/tmp/*`, `..` escapes, expansions and symlinks out of it stay refused. A `path:START-END` entry in a batched read is now judged by its file, so `.env:1-5` no longer slips past the secret rules (`crates/forge-core/src/permission/narrowing.rs`).
- **`system/usage` is sent once per change in the running total, and an error result carries the usual fields.** `--output-format stream-json` repeated an identical `system/usage` event after every model call, and the `error_during_execution` result of a run that failed before its first turn had only `result` and `stop_reason`; it now also has `uuid`, `duration_ms`, `duration_api_ms`, `num_turns`, `total_cost_usd`, `usage`, `modelUsage` and `permission_denials`, zeroed, as Claude Code's does.
- **Forge no longer writes its state into your project.** Undo checkpoints, the evidence ledger and background-job records lived in `<workspace>/.forge/{checkpoints,evidence,jobs}/`, so anything that compares the tree, a `docker COPY`, `rsync` or `grep -r` saw them (a benchmark's integrity check failed a correct answer on exactly that: a08 lost 2 of 52 haiku passes). They now live in Forge's data directory, one folder per workspace (`~/.local/share/forge/workspaces/<name>-<hash>/`); `FORGE_STATE_DIR` overrides the root and a scratch `FORGE_DB` gets its own, so isolated runs never touch the real data directory. Checkpoints and evidence from earlier versions are moved across the first time the workspace is opened, so `/undo` and `/rewind` still reach them, and jobs recorded under `.forge/jobs/` are still listed and stopped. `.forge/` is left for what the project authors (`AGENTS.md`, `mcp.toml`, skills). `read_file` can still read the job logs the start message names. Folders of workspaces that are gone are removed after 30 idle days. See `docs/features/workspace-state.md`.
- **The existing-tests guard no longer leaves `refs/stash` in your repository.** It took a model's rewritten tests out of the tree with `git stash push`, which left `refs/stash` in 8 of 10 SWE-bench repos Forge worked in and would have handed you Forge's test edits at your next `git stash pop`. The edits are now saved as a patch in Forge's state folder and the originals are checked out from the turn-start commit; the push-back tells the model to `git apply` the patch. No ref, stash entry or `.git/` file is written. `cargo test` also no longer lets that guard run against the checkout the tests run in: one test did, and reverted whoever had uncommitted edits to a `tests/` file.
- **Editing a read-only file you own works, and a failed write says why.** A `chmod 444` file is the owner's to edit: Claude Code's `Edit` writes straight through, Forge's tools failed with a bare `Permission denied (os error 13)` and sent the model through `ls`, `stat` and `chmod` (d06: about 4 calls and 5 s, and it varied by run). `write_file`, `edit_file`, `multi_edit`, `append_file`, `apply_patch`, `notebook_edit` and `/undo` now lift the owner-read-only bit for the write and put the mode back. When the file is someone else's, on a read-only mount, or in a folder you cannot write, the error names the path, its mode and owner, and tells the model to report it rather than `chmod` or `sudo` around it. The shell-failure diagnosis no longer answers "permission denied" with "try `chmod +x <file>` or prefix with `sudo`", and the model-written diagnosis is told never to suggest `sudo`.
- **A bridged model is no longer sent to a `shell_job` tool it cannot see.** `shell` says a background job is managed with `shell_job`, but the bridge's lean tool list defers `shell_job`, so a haiku run called `mcp__forge__shell_job` and was told no such tool exists. On the lean surface the `shell` description and a started job's result now say to run it through `tool_call {"name": "shell_job", ...}`; the full surface is unchanged. The five "could not be parsed as JSON" failures in the same runs never reached Forge: claude rejected the model's own tool input (a raw tab in three `multi_edit` calls on Go and Makefile text, a raw newline in two `shell` `commands` calls). The `edits` and `commands` descriptions now open with a reminder to escape tabs and newlines; whether it lowers the rate is not measured (`crates/forge-cli/src/mcp_serve/deferred.rs`, `crates/forge-tools/src/shell.rs`, `crates/forge-tools/src/core_tools/multi_edit.rs`).
- **`forge run "task"` no longer hangs on a stdin pipe nobody closes.** A supervisor or script that started the run with an open pipe on stdin and never wrote to it blocked in `read_to_string` forever. With a prompt argument the run now waits 3 seconds for the first byte (or EOF), like `claude -p`, then goes ahead with a warning on stderr; data that has started flowing is still read to EOF, so `cat file | forge run "summarize"` is unchanged. With no prompt argument stdin is the only source, so the wait is 30 seconds and ends in the usual empty-prompt error. A terminal stdin is never read (`crates/forge-cli/src/cli/commands/run/one_shot/piped_stdin.rs`).
- **`--continue` reattaches the most recent session of the current directory.** `forge run --continue` and `forge chat --continue` took the newest session in the whole store, so a script in one project could pick up a conversation from another. They now match the directory a session was created in (or the git worktree it works in), like `claude -c`; with no session there the error names the directory and points at `--all-dirs` (the old behaviour), `forge sessions` and `--resume <id>` (`crates/forge-store/src/session_catalog_store.rs`, `crates/forge-cli/src/cli/commands/replay.rs`).
- **`forge auth <provider> --list` keeps stdout to the keys.** The "reading keys from: ..." line went to stdout, so `forge auth groq --list | wc -l` counted it; it is on stderr now (`crates/forge-cli/src/cli/commands/local/auth.rs`).
- **A notice no longer cuts a streaming answer in two.** Queueing a follow-up while the answer was still streaming closed the answer segment at that instant, so "queued (1 pending)" landed in the middle of a sentence and the rest of the answer started on a new row. A note raised by the surface now waits for the end of the segment (the next tool call, the end of the answer, or the next prompt) and prints on its own row right after it. Notices the turn itself raises between steps (warnings, failover, the completeness review) still end the segment (`crates/forge-tui/src/app.rs`, `crates/forge-tui/src/app/transcript_flow.rs`).
- **The "↑ Previous message" and "Jump to bottom" pills only show while the transcript is scrolled up.** The previous-message pill sat over the newest rows even at the bottom of the conversation. Both pills now disappear the moment you are back at the tail, keep their chords (Ctrl+Home, Ctrl+End), shorten to the arrow and chord on a narrow terminal, and still take a click. The top pill quotes the start of the message it jumps to (`↑ fix the login redirect… · Ctrl+Home`), the way Claude Code's sticky header shows the previous prompt (`crates/forge-tui/src/app/render/transcript.rs`).
- **After `/clear` or a session switch, a click could open a tool card from before the clear.** The cards kept their old positions, which then pointed at new rows, so opening one rewrote lines that belonged to something else. Cards whose rows were cleared are now dropped with them.
- **An unbound Ctrl chord no longer types its letter into the prompt.** Ctrl+X, Ctrl+B, Ctrl+Q, Ctrl+G and Ctrl+D inserted `x`, `b`, `q`, `g` and `d`, so a habitual `Ctrl+D` or `Ctrl+G` corrupted the draft. Ctrl and Alt chords with no meaning are now dropped, while AltGr text such as `@` and `€` still types (`crates/forge-tui/src/driver/keymap.rs`).
- **The claude-cli bridge starts faster, sends less, and the turn ends at its answer.** Measured against `claude -p` with the same model on a local fake API: claude spawn → first request went from 1.9 s to 1.5 s (the bridged `forge mcp-serve` re-resolved every provider key and built subagent support before answering claude's MCP handshake; it now adopts the key names its parent resolved and builds subagents on first use, 0.94 s → 0.1 s). Each request drops claude's auto-memory section (~2.3k chars, plus the project's `MEMORY.md` index) because Forge owns memory, and a `~/.claude/CLAUDE.md` used as Forge's fallback instructions is no longer sent a second time next to claude's own copy. Recap, suggestion and memory capture chose their model on the turn task after `Done`, holding "Finishing" and the next queued prompt about 1 s per turn; they now run on one detached task. The Claude quota probe no longer starts your `~/.claude.json` MCP servers or hooks or sends CLAUDE.md, skills and tool schemas: 107 KB → 17 KB per probe (`crates/forge-cli/src/mcp_serve.rs`, `crates/forge-core/src/post_turn_calls.rs`, `crates/forge-cli/src/bridge_stats.rs`).
- **`--model claude-cli::<typo>` fails at once with exit 1.** claude silently ran its default model for an unknown `--model` and exited 0; Forge now rejects a claude model it cannot explain before spawning and lists the valid ones (`crates/forge-cli/src/cli/commands/run/pin_check.rs`).
- **A bridged edit's permission prompt names the file and shows the diff.** It read "Allow Edit of ?" with no path, because the relayed question carried only the tool name; it now carries the arguments and, for writes, the diff a native write shows (`crates/forge-cli/src/mcp_serve/permission_relay.rs`).
- **The iPhone app no longer clears Forge's push alerts before you see them.** Every notice sync (app open, return to the app, a push arriving and a 30-second poll) removed every "Open Forge to view an update." alert from Notification Center, but the real-text notification that was meant to replace it is only raised while the app is in the background. A hosted alert is now cleared only after a real-text notification was actually raised for it, and the app's own lock-screen copy (shown while the app lock is on) is never cleared (`mobile/src/components/notices/NoticesWatcher.tsx`, `mobile/src/lib/notices/core.ts`).
- **A failed APNs send shows up in the daemon log.** Transport failures for alerts and Live Activity updates were logged at debug level and never reached the journal; they are now warnings, and a rejected send includes Apple's `reason` (for example `BadTopic`) next to the status. Device tokens are still never logged (`crates/forge-cli/src/apns.rs`).
- **A message from another agent session no longer takes away the recipient's write access.** A peer message whose sentence opened with "read-only" put the receiving turn under a no-writes contract, so a session in Full mode reported it had no permission to run shell or edit files. Peer messages already promise the recipient that a peer cannot change its permissions; the contract now ignores them (`crates/forge-core/src/turn_contract.rs`).
- **A call refused by a deny rule names the rule.** The refusal said only "not allowed in Full mode or is blocked by a deny rule", so a model whose one command read `~/.ssh/config` concluded shell was banned for the whole session. It now names the matching rule (for example `grep */.ssh/*`) and says the tool itself still works (`crates/forge-core/src/permission.rs`).
- **Web search uses the local SearXNG again.** The resolver that stops `web_fetch` from reaching private addresses also refused `localhost`, so a running SearXNG on `localhost:8888` (or `FORGE_SEARXNG_URL`) was silently skipped and every search fell through to DuckDuckGo, which throttles after a query or two, and then to keyless Bing. SearXNG now uses its own client (`crates/forge-tools/src/web/search.rs`).
- **`web_fetch` no longer follows a redirect to a different site.** Permission rules match the URL the model asked for, so following `golang.org` to `go.dev` read a host nobody approved. The redirect target is now reported and fetching it is a separate, gated call.
- **`scripts/searxng-setup.sh` binds SearXNG to 127.0.0.1 and writes a random `secret_key`.** It published port 8888 on every interface, which exposes an open search proxy on a server.
- **The Forge app no longer logs you out when one request is rejected.** Any 401 on an authenticated Anywhere request sent the app straight to "Reconnect with GitHub", which then needed a fresh device approval. A rejected access token (for example a phone clock running behind the service) is now renewed first, and the app only asks you to sign in again when the renewal itself is refused or the renewed token is rejected too (`mobile/src/lib/AnywhereProvider.tsx`).
- **Bridged `claude-cli` sessions record the tools and model calls the CLI made.** The whole agent loop runs inside one provider call, so metrics counted one model call and no tool calls however much it did, and `forge replay` and `bench export` showed assistant messages with no tool calls. The session now writes the streamed tool calls into the audit ledger and the CLI's model-call count onto the usage row (`usage.api_calls`, store migration 46).
- **`forge bench tasks --mock` no longer routes through the mesh.** The offline run pinned nothing, so it went to the real default model, printed the Claude CLI-bridge terms notice and spent calls on an LLM tier classifier. It now pins the mock provider. A turn that leaves the workspace untouched is recorded as outcome `no_change` instead of `ok`, so a run that did nothing no longer reads as an ordinary attempt. The staging of the workspace before `test.sh` (`git add -A`, so a bare `git diff` is empty) is now documented, and the patch is taken against the baseline commit, so an agent that commits its own work is not scored as an empty patch.

## [2.21.0] - 2026-10-18

### Security
- **A cloned repo could run commands and switch Forge to bypass with no prompt.** `.forge/config.toml` was merged unconditionally (so `permission_mode = "bypass"`, an allow-all rule and `[[hooks]]` applied), and `.forge/settings.json` hooks and `.forge/mcp.toml` stdio servers started at session start. They now wait for project trust (see Added).
- **Auto-edit, the default temper, ran every shell command, including `rm -fr ~`.** The docs said it asked before shell; the code allowed all of it. It now runs ordinary commands and asks for shell that deletes, moves, copies or redirects outside the workspace, touches a raw disk, pipes into a shell or runs inline code that deletes files (`auto-edit asks: …`); README, ADR-0008 and the temper docs now describe that (`crates/forge-core/src/permission/auto.rs`, docs/features/temper-modes.md).
- **The "unoverridable denylist" only matched the literal `rm -rf ~`.** `rm -fr ~`, `rm -r -f ~`, `/bin/rm -rf ~`, `rm -rf $HOME`, `rm -rf ~/*`, `cd ~ && rm -rf .`, `find ~ -delete`, `echo ~ | xargs rm -rf`, `X=~; rm -rf $X`, `eval 'rm -rf ~'`, `mv ~ x`, `rsync --delete … ~/` and inline `shutil.rmtree` of home all ran, even in bypass (one deleted the test home). A structural safety floor now parses flags and targets and refuses any recursive delete, move-away or wipe of home, `/` or a top-level system directory in every temper (`crates/forge-core/src/permission/catastrophic.rs`).
- **`sudo`, `su`, `doas`, `pkexec`, `run0` and `passwd` are never run from a tool call, in any temper.** They ran with no terminal in auto-edit and bypass, where they can only fail and each failure counts toward `pam_faillock`'s account lockout (screen unlock included). The call is refused and the model is told to hand you the exact command to run yourself.
- **The auto temper allowed writes outside the workspace through a symlink.** Its escape check was lexical, so `lnk/x` with `lnk -> ../outside` (or a link the model had just made) wrote outside without asking; symlinks, dangling ones included, are now resolved.
- **The claude/codex bridge let auto-edit write outside the workspace.** `mcp-serve` skipped the beyond-workspace ask the direct path applies; both now share it (`crates/forge-core/src/workspace_policy.rs`, `crates/forge-cli/src/mcp_serve.rs`).
- **The session store was world-readable.** `forge.db` and its `-wal`/`-shm` were created 0644 in a 0755 data dir; they are now created 0600 (existing ones are tightened on open) and the data dir is 0700 (`crates/forge-store/src/lib.rs`).

### Added
- **Forgeline transcript for the TUI.** Conversation, markdown, code, diffs, tool calls and notices now share one look in full-screen and inline. Each turn runs down a rail: a cool `▎` for you, an ember `▏` for Forge while it works (it cools once the turn settles; full-screen), one `✦ forge` label per turn with the routed model right-aligned, and a faint footer with elapsed, tokens and cost. Wrapped text hangs under its rail, prose stops at a readable width, headings, lists, blockquotes and links each have their own treatment, tables lose their grid (bold header, rule, numbers right-aligned), and code is a banded block with a language and `/copy` header and line numbers past five rows (a `│` rule where backgrounds cannot be painted). Tool calls are one row in both modes (`✓ Write  src/lib.rs  +18 −3 · 0.2s`) with a humanised verb, a decoded target and no raw JSON; an edit carries its diff in the card with line numbers, row tint and word-level emphasis; a failed call shows the last three lines of its output; consecutive reads fold into `◆ Read 4 files`; the card waiting on a permission prompt reads `◇ … awaiting approval`; inline prints each finished card once. Notices use one grammar (`✓ · ▲ ✗`, hanging indent, legacy `⚠ ✖ ⏹ ⚙ ◈ ※` glyphs re-glyphed with no forge-cli change), repeats collapse to `×N`, errors with a known fix get a `Next:` line, and the CLI-bridge notice prints once. Thinking is one faint `▸ Thought for 12s` line, in full-screen long pasted messages and `/init`-style command prompts fold (`Ctrl+T` opens them; elsewhere a message is shown in full), the recap only follows a long or tool-heavy turn and never restates the answer, and a re-answer after the completeness review no longer glues the answer to itself. Existing keybindings and commands are unchanged. See `docs/features/tui-rich-rendering.md`, `tui-tool-cards.md` and `tui-inline-scrollback.md`.
- **Forgeline foundation for the TUI.** The terminal UI now resolves one theme before its first draw: explicit colour tokens per tier (truecolor, 256, 16 colours, `NO_COLOR`), a light mode, an ASCII glyph set for the Linux console, `TERM=dumb` and non-UTF-8 or CJK locales, and reduced motion. New `[tui]` keys `theme`, `color`, `glyphs` and `reduce_motion` (and `FORGE_THEME`, `FORGE_COLOR`, `FORGE_GLYPHS`, `FORGE_REDUCE_MOTION`) override detection, and `forge config` shows them under Interface. A remapping backend brings syntect and external statusline colours down to the active tier, and no dark slab is painted behind overlays any more. The chrome is redesigned around it: a one-row header (`✦ forge`, repository and branch, remote-control dot, version from 120 columns) in fullscreen, a composer border that carries the live line (spinner verb, elapsed, tokens, rate, waiting-for-you, armed-key and stop-reason states), the mode, queue, goal and loop chips and the hints for the current state, and a one-row status line (model, tier, cost, tokens, throughput, context gauge, mesh effort) whose widgets drop in a fixed order with a `+N` marker. The separate turn-activity row, the input-box title and the status line's phase and hint text are gone; the counters they printed are listed by the new `/status` inspector (also in the palette, Ctrl+K and Help), which shows every fact the header and status line can drop. Startup errors print as `✗ message`, detail, `Next:` and honour `NO_COLOR`. See `docs/features/tui-theme.md` and `docs/features/tui-header-statusline.md` (`crates/forge-tui/src/theme/`, `crates/forge-tui/src/app/render/`).
- **Forgeline panels and onboarding.** The empty chat is now a welcome state instead of a printed logo: a half-block ember wordmark on tall terminals (`✦ forge` otherwise), the tagline, a single `/init` nudge only while the project is uninitialised (it clears once guidance exists, with no pushed warning in fullscreen), three things to try, the key row and, when a launcher supplies them, recent sessions; inline prints a compact welcome row. The tasks band shows `Tasks 1/3`, the task in progress and the running agents on one row; `Ctrl+O` opens the lists with per-task and per-agent elapsed times and cost, and on terminals 150 columns or wider the same content becomes a 38-column right rail (tasks, agents, changed files with +/−, context gauge) so the transcript keeps its height and its mouse selection geometry. Plan cards, subagent groups (`◆ Agents` with `├`/`└` branches) and workflow history lines use the notice grammar; the workflow view is a header row, phase checklist with nested agents, log and hint row; `/btw` is a titled card with the answer rendered as markdown, scrollable, with its provenance in the footer border; `/usage` aligns its numeric columns, prints costs as `$0`, `<$0.01` or two decimals, draws quota bars and shows explicit empty states; the mesh and voice overlays and the `/config` editor (two panes from 120 columns, `PgUp`/`PgDn` between sections, one value-widget grammar, help with the key path last) and `forge setup` wizard (step dots, recommended choices labelled) use the shared frame, caret and token grammar; the board drops its retired glyphs (`crates/forge-tui/src/app/render/{panels,welcome}.rs`, `app/render/overlays/`, `workflow_view/render.rs`, `config_editor/render.rs`, `init_wizard.rs`).
- **TUI interaction redesign: one list, one decision block, one Help.** The composer keeps the only permanent frame (hanging indent for continuation and wrapped rows, `↑ N more` when it scrolls, `queue a follow-up…` while a turn runs, `/command` in brand and `@path` in the info hue). The slash palette, command center, `@file` picker, every session, model, checkpoint, mode and memory picker, the multi-select prompts, the keybinding editor and Help now share one list component: rounded frame, `❯` caret with a raised row, matched characters in brand, right-aligned shortcuts and `skill`/`command` tags, a `… N more` row and a detail line with the selected command's usage. The palette lists recently used commands first, the command center has group tabs (`Tab` cycles All, Start, Session, Model, Tools, Skills, Settings), the `@file` picker browses directories (`Tab`/`Enter` descend), fuzzy-searches the workspace and marks files git reports as changed, and PgUp/PgDn/Home/End work in the lists that have a cursor. A tool permission and an `ask_user` question now replace the composer with a decision block: a `┃` rail (warn for permissions, the info hue for questions), the exact action as its title (`Allow Write to create mock-note.txt?`), the file or the full wrapped shell command, and numbered choices with the legacy keys on the right (`y`/Enter, `a`, `n`/Esc, plus 1-3 and Up/Down with Enter confirming the focused choice). Keys that are not a choice no longer deny the tool. A question's `Other…` field is inline: typing on the list starts it. Help is one takeover with Commands, Keys and Mouse sections and a search field, one row per command (usage on the detail line, Enter inserts it), the keys table generated from the keybinding registry in one spelling (`Ctrl+K`, `Alt+M`, `Shift+Tab`), and two columns from 120 cells (`crates/forge-tui/src/app/render/list_view.rs`, `permission.rs`, `popups.rs`, `help.rs`, `docs/features/tui-interaction.md`).
- **Archived sessions are reachable from the terminal too.** `forge sessions --archived` lists them, `--all` lists everything with an `[archived]` tag, `--unarchive <id>` brings one back, and `forge chat --resume <id>` un-archives what it resumes, matching the daemon and the app (`crates/forge-cli/src/cli/commands/replay.rs`).
- **`PermissionRequest` hooks and `prompt` / `agent` hook types (Claude Code parity).** A `PermissionRequest` hook fires when a tool call would ask you and can answer for you: `hookSpecificOutput.decision.behavior` `allow` (with an optional `updatedInput`, re-checked against the workspace, the safety floor and deny rules) or `deny` (with a `message` for the model and optional `interrupt`); exit 2 denies, no answer shows the prompt. It also fires in headless runs and on the claude/codex bridge. Hooks can now be `type: "prompt"` (the event JSON, via `$ARGUMENTS`, goes to the session's cheap side-call model or the hook's `model`) or `type: "agent"` (a read-only subagent with `read_file`/`search`/`glob`/`list_dir` in the workspace, up to 50 turns), answering `{"ok": true}` or `{"ok": false, "reason"}`; `ok: false` acts like a command hook exiting 2 on every event, in a Claude Code `settings.json` or as a native `[[hooks]]` entry. A `PostToolUse` block reason now reaches the model, as in Claude Code (`crates/forge-core/src/hooks_llm.rs`, `crates/forge-core/src/hooks.rs`, `crates/forge-core/src/tool_dispatch/permission_hooks.rs`, docs/features/hooks.md).
- **`forge trust` and project trust.** A cloned repository's `.forge/` no longer takes effect until you trust it: its `[[hooks]]`, `.forge/settings.json` hooks, `.forge/mcp.toml` servers, allow rules, a more permissive `permission_mode`, and every section that can run commands or send data elsewhere are ignored and named once (`note: this project is not trusted, so Forge ignored its hooks, 1 allow rule, …`), while its `ask`/`deny` rules, a `plan`/`default` temper and cosmetic settings still apply. An interactive `forge chat` asks once; `forge trust [path]`, `--list`, `--revoke` and `FORGE_TRUST_PROJECT=1` manage it, trust is stored in your config directory and covers linked worktrees of the trusted checkout, and "always allow" answers in an untrusted project are kept there too. `forge doctor` reports an untrusted project (`crates/forge-config/src/project_trust.rs`, `crates/forge-cli/src/cli/commands/trust.rs`, docs/features/project-trust.md).
- **A compacted session keeps its instructions, memories, task list and working files.** AGENTS.md, recalled memories, the goal, the approved plan and the task list used to be one-shot system messages that a compaction folded into the summary and nothing restored. They are now a pinned session-context block, rebuilt for every request right behind the system prompt and never stored, so it cannot be folded or doubled; a session stored before this still carries the old copies and is not sent them twice until a compaction folds them away. AGENTS.md is re-read when the file changes (one `stat` per turn), memories are recalled again after every compaction and on a resumed session's first turn (goal plus your latest request, top 6), and the task list is a snapshot taken at the start of each turn and after each compaction, so an `update_tasks` call no longer rewrites the provider's cached prefix. `update_tasks` now returns the full list (`- [status] title`) instead of counts, and the per-turn `[task list]` note keeps its instruction without quoting the list a second time. After a compaction the newest five of the last 20 files read or edited are re-read from disk and added as one `[files restored after compaction]` note (a file over 25k tokens, a binary, or anything past 40k tokens in total is skipped; a claude/codex bridge gets only the paths; a later compaction replaces the note instead of stacking another). The cache gets a breakpoint behind the stable prompt, and a resumed claude/codex bridge is sent the block once and again, as an update, only when it changed (`crates/forge-core/src/pinned_context.rs`, `recent_files.rs`, `file_restore.rs`, `crates/forge-provider/src/cache_hints.rs`, `crates/forge-provider/src/cli_provider/resume.rs`, docs/features/context-pipeline.md, docs/features/context-compaction.md).
- **Compaction keeps what the session knew.** The summary is now written by the session's own model (the `/model` pin, else the model the last turn ran on, then its fallbacks, then the free shortlist) at the session's effort, instead of the best free model first; `[mesh] compact_model = "auto"` restores the old order. The prompt follows Claude Code's structure: a hidden `<analysis>` pass that is stripped before storing, then primary request, key concepts, files and code with snippets, errors and fixes including your corrections, problem solving, pending tasks, current work and a next step quoting your latest instruction. After the summary Forge appends `## User messages (verbatim)`: your own messages from the folded stretch (newest whole, older capped at 2,000 characters, about 8k tokens in all, oldest dropped first and counted), which later compactions carry forward; the empty-reply nudge, auto-fix and stop-hook feedback and other harness-written rows are left out. The summarizer reads tool results as the store holds them, not the 1,500-character pruned heads, and a transcript larger than its window is summarized in parts with a rolling summary instead of losing the middle. `/compact <what to keep>` adds your instructions to that one summary. The reply is capped at 20,000 tokens, a call that runs past 300 s moves to the next model, and a summary is judged thin by its section structure instead of the English word "goal" (`crates/forge-core/src/compaction_run.rs`, `compaction_summary.rs`, `compaction_input.rs`, `compaction_chain.rs`, `crates/forge-store/src/compaction_source_store.rs`, `crates/forge-config/src/compact_model.rs`, docs/features/context-compaction.md).
- **`forge bench recall` measures what a session still knows after compaction and resume.** A scripted session in a throwaway repo, store and config plants constraints, a correction, decisions, file and function names, an error and its fix, pending tasks and numbers, compacts (exactly as `/compact` does), checkpoints the store, then asks one question per fact and scores each answer by regex, plus probes that check the model obeyed a planted rule when it wrote a file and a commit. It reports per-category and total recall for the live session and, with every probe on its own freshly resumed session, after a resume, with token and cost totals (`--json` for the full report, `--model`, `--turns-scale`, `--skip-resume`, `--keep-workdir`, `--mock` for the offline plumbing check). `--scenario hard` adds about 40 turns of large tool outputs under a low `[compact_cap]` so auto-compaction fires repeatedly (the report counts auto against manual compactions and names the summarizing model), facts that exist only in tool results, an `AGENTS.md` rule, an overridden instruction, a work-state probe scored against the real git history and a task-list probe. The scenarios are static tables, so a new fact is one entry. Baselines are in `docs/benchmarks/recall-baseline-2026-10.md`.
- **Text inside popups and viewers can be selected and copied with the mouse.** Dragging over the `/btw` card, mesh/usage/config overlays, pickers, the activity viewer, the full tool-output viewer or the workflow view used to select (invisible) transcript text beneath, or nothing, because mouse capture hides the terminal's own selection. A press outside the plain transcript now starts a screen-cell selection: the highlight is painted on the drawn frame, a drag that starts in a framed popup is clipped to its interior so the chat beside a centered card is not copied, decoration cells are skipped, and release copies. A click without a drag copies nothing and leaves no highlight; the selection clears when the overlay closes or a key is pressed (`crates/forge-tui/src/app/screen_select.rs`, `crates/forge-cli/src/cli/commands/run.rs`, docs/features/tui-rich-rendering.md).
- **Pasting a screenshot works over SSH, and files and paths attach as images.** `Ctrl+V` now means "paste image, else voice": it reads the clipboard through `wl-paste`, `xclip` (using the forwarded `DISPLAY`, refreshed from `tmux show-environment` after an ssh reconnect), `pngpaste`/`osascript` on macOS or PowerShell on Windows, with a 2 s timeout per backend so a dead forwarded display cannot hang the prompt, and falls back to `/voice` only when no image is found. An empty paste says why nothing was attached (no `DISPLAY`, `xclip` missing, text-only clipboard) instead of staying silent. Dropping or pasting one or more file paths (quoted, backslash-escaped, `file://`, `~`) attaches images as image blocks and text files as `@path`; a path under `/home/<user>` or `/Users/<user>` that does not exist on this machine says so. `@shot.png` attaches the image instead of reporting `(binary)`, and `/image` accepts quotes, `~` and `file://`. Image type comes from the file's magic bytes (png/jpeg/gif/webp/heic), images over 2000 px are downscaled (PNG for screenshots, JPEG 85 for photos), and HEIC/HEIF is converted through `heif-convert`, ImageMagick or `sips`. An image prompt typed while a turn runs is queued with its images and attached when it runs; the `//` escape attaches images too (`crates/forge-cli/src/clipboard_image.rs`, `crates/forge-cli/src/attach_paths.rs`, `crates/forge-cli/src/image_input.rs`, docs/features/image-input.md).
- **Images now reach the claude and codex bridges and the Responses OAuth providers, and a model that cannot see one says so.** The `claude-cli` bridge sends the latest image turn as stream-json image blocks, `codex-cli` passes it with `codex exec --image`, and `codex-oauth`/`xai-oauth` send `input_image` parts; before, all three dropped it silently while the mesh routed image turns to exactly those models. Vision capability now comes from OpenRouter and models.dev input modalities (cached next to context windows) before the name heuristic, which gained Kimi, GLM-V, DeepSeek-VL, Mistral Small/Medium/Large 3+, Qwen-VL and Gemma, and `mistral-large` is no longer assumed text-only. Routing keeps picking vision models for follow-ups while the image is still in the transcript, and the new `[mesh] vision_fallback` (`warn` default, `describe`, `switch`) replaces the silent fail-open when the selected or pinned model is text-only (`crates/forge-provider/src/cli_provider/images.rs`, `crates/forge-provider/src/oauth_responses.rs`, `crates/forge-mesh/src/vision.rs`, `crates/forge-core/src/vision_policy.rs`, docs/features/image-delivery.md, docs/features/mesh-routing.md).
- **Running sessions pick up new providers, keys and models without a restart.** A `forge chat` TUI or daemon-hosted session now notices a `[[providers.custom]]` block added to config.toml, a key stored or removed with `forge auth`, a catalog another process refreshed, and (hourly) models a provider newly lists. A background task checks four file timestamps every 30 seconds (about 12 microseconds, no network), rediscovers only when something changed, and swaps the picker/pin-check catalog at a turn boundary without touching a pinned model. `Ctrl-R` triggers it immediately, and a one-line `catalog refreshed: +50 modelgateway` appears only when the model set changed. Custom/Azure provider registries are now swappable at runtime and secret writes leave a change stamp (`crates/forge-cli/src/cli/commands/models/watch.rs`, `crates/forge-core/src/catalog_feed.rs`, `crates/forge-config/src/provider_registry.rs`, docs/features/mesh-routing.md).
- **ModelGateway works as a custom provider.** A `[[providers.custom]]` endpoint running new-api (ModelGateway, `https://www.modelgateway.co/v1`) is now priced live from its `<origin>/api/pricing` page at official price less the listed discount, which is what calibration calls were actually billed; `model_ratio` is ignored because it does not match the bill. `modelgateway::*` models get the same `minimal` harness profile as InferHub (`crates/forge-cli/src/context_windows/parsers.rs`, docs/features/mesh-routing.md).

### Changed
- **Upgrade: Project trust.** A repository's own `.forge/` hooks, MCP servers, allow rules and a more permissive `permission_mode` are ignored until you trust it. Interactive `forge chat` asks once; for headless, CI or daemon use run `forge trust [path]` or set `FORGE_TRUST_PROJECT=1` (see Added).
- **Upgrade: Auto-edit asks before risky shell.** Ordinary commands still run, but shell that deletes, moves, copies or redirects outside the workspace, touches a raw disk, pipes into a shell or runs inline code that deletes files now prompts. Unattended runs that relied on auto-edit running everything will see prompts (see Security).
- **Upgrade: Compaction summaries come from the session's own model by default** (`[mesh] compact_model = "session"`), which can spend paid credits or plan quota. Set `[mesh] compact_model = "auto"` for the previous free-model-first order.
- **Upgrade: Session store schema 44 → 45.** The first 2.21.0 run migrates `forge.db`; older Forge versions refuse to open it afterwards. Back up `~/.local/share/forge/forge.db` first if you may roll back.
- **`forge board` key hints move to the top row.** The contextual key hints sit right-aligned in the header row instead of the footer keybar (same keys, same order); on a narrow terminal the lowest-priority hints drop first and the title is never clipped. The footer keeps toasts only.
- **Forgeline: welcome examples are selectable, the board wears the theme.** On the empty-transcript welcome, `↓` from an empty composer focuses the three example prompts (`❯` caret and raised fill; `↑`/`↓` move, `Enter` copies the example into the composer without sending, `Esc` or any other key leaves). `forge board` cards drop their frames for the list/card grammar (caret and raised fill on the selected card, exceptions-only status pill on the title row), lane headers use the categorical colour tokens, status glyphs follow the glyph tier (ASCII twins), and the detail pane header carries a health pill.

### Fixed
- **Anywhere: a newly paired phone is no longer ignored until the host restarts.** A host's relay refreshed device keys with the access token from when it connected, which expires within minutes; on a relay up for hours every refresh was rejected, the error was dropped, and every request from a device enrolled after the connect was discarded as "unknown or revoked device" until `forge-serve` restarted. The refresh now renews the token from the state file, is rate-limited, and logs failures.
- **TUI polish pass.** Plan-card steps wrap at the live terminal width and hang under the step text instead of leaving a flush-left orphan on 80 columns. List overlays (palette, command center, Help) size the shortcut column from the rows on screen, so one long shortcut further down no longer clips every description; `/config` widens its label column to the longest visible label (`Anthropic (Claude API)` is no longer cut). The inline welcome row drops the tagline when it does not fit whole instead of cutting it mid-word. The plain `TERM=dumb` chat banner and hook-block notice fold to ASCII.
- **Config writes keep your comments; clearer config errors; doctor checks secrets; CLI pipe and flag fixes.** Every programmatic `config.toml` write (providers, Azure, keybinds, mode, subscriptions, statusline, `/config`, Anywhere) now edits with `toml_edit`, so comments and ordering survive, and a file that does not parse is refused with `file:line:column` instead of being overwritten; `.forge/mcp.toml` edits keep comments on untouched servers. A wrongly-typed config value reports its file, line and column, and unknown keys warn instead of vanishing. `forge doctor` now decrypts `secrets.enc` and fails on a corrupt store or mismatched key. A closed or full stdout no longer panics (`forge mcp tools | head` exits 141, `> /dev/full` prints one error line). `forge mcp tools` honours the tool allowlist, and an `mcp_call` prompts once. `forge run` accepts `-p`, `--verbose`, `--input-format text`, `--permission-mode`, `--dangerously-skip-permissions`, `--max-turns`, `--allowedTools` and `--disallowedTools`.
- **Dependency refresh.** Clears the open Dependabot advisories that have a fix (vitest and @vitest/mocker 4.1.12 in mobile; browserslist, nanoid and baseline-browser-mapping in the promo project), folds the safe cargo group (rmcp 3.5.1 with the `ServerConfig` rename, tokio, serde, clap, uuid, reqwest, tree-sitter 0.26.13 and others) and the GitHub Actions patch bumps. `braces` and `node-forge` have no published fix and stay waived; `decode-uri-component` needs the Expo SDK 58 `expo-router` major. The three new `zip` extract() advisories (RUSTSEC-2026-0367/0368/0369, fixed only in zip 9) are waived in cargo audit and cargo deny until Tauri and self_update move off zip 4/8.
- **Bedrock `bedrock::global.moonshotai.kimi-k3` no longer loops on a rate limit it cannot clear.** The pinned-model warning now says why (`rate limited (Bedrock: Too many tokens, please wait before trying again.) — retrying pinned model in 49s`) and the full error is logged. The vendored genai Bedrock adapter keeps the exception type (`throttlingException`, `serviceUnavailableException`, `validationException`, ...) from stream frames and `x-amzn-ErrorType`, so throttles, outages and over-long inputs are classified by type. The text fallback no longer reads `429` inside a request id, a byte or token count as a rate limit, nor any mention of `quota`. A repeated "Too many tokens" throttle (a single request over the account's tokens-per-minute quota, which waiting never fixes) compacts the context after the second occurrence and suggests requesting a higher quota; request-rate throttles keep plain backoff (`crates/forge-core/src/throttle_recovery.rs`, `crates/forge-provider/src/genai_provider/bedrock_error.rs`, `vendor/genai-0.6.5/src/adapter/adapters/bedrock/`).
- **Steer, follow-up, dispatch revise and restart persistence.** A reply a steer interrupted is now published to history, resume and the phone (it stayed provider-only, so the steer appeared straight after the prompt and the answer read as a reply to the wrong message). Fleet `follow_up` messages queue FIFO behind the running turn through a new `follow_up` remote input instead of being absorbed mid-turn like a steer, and a prompt no longer overtakes an image prompt or follow-up queued before it. A dispatch revised to `planning` whose coordinator ends its turn without proposing (or is gone) is handed back as `proposed` after a short quiet period instead of staying in `planning` forever. A daemon-hosted session's queued prompts are saved in the store (session schema 45) and restored on restart, and sessions that were never prompted are restored too. The built-in PWA no longer renders the newest turn twice after a reload: the live tail drops the lines the first history page already shows (`transcript_total`).
- **Mesh, credential and provider robustness, second pass.** The first run with no model catalog now discovers inline (bounded) instead of routing to seeds that may be disabled; `forge api` benches failing models, honours `max_tokens`/`max_completion_tokens` and refuses `n > 1`; concurrent `forge auth` runs and concurrent MCP/xAI OAuth refreshes no longer lose updates or spend the same rotating refresh token (xAI also stops calling a network blip a sign-in failure); synced usage rows keep their provider, model and timestamp; `vision_fallback = "switch"` now switches under a `--model` pin; headless `forge run` attaches `@image.png` mentions; codex plan turns are no longer priced in USD; a `/goal` stops cleanly when a spend cap refuses its turn; a tripped session cap skips the recap/memory side calls; the inline recap gives up after 20s; completeness review no longer fires on read-only turns outside git; a 400 "does not support this protocol" fails over; strict credit mode also governs `/btw`-style side calls under a pin set; custom/Azure endpoints reject credentials, bad ports and plain-http Azure, and the CLI enforces the daemon's env-var-name rule; the secret-store error text no longer says "keyring" for file errors; a Headroom proxy outage names the proxy.
- **Mesh, provider and credential edge cases.** A rejected API key (401) no longer reports "no provider configured"; Ollama's "does not support tools" and vLLM's auto-tool-choice errors fail over instead of killing the turn; a pin set now fails over within the set on a repeated fast 5xx; custom providers can no longer take the `ollama`, `claude-cli`, `codex-cli`, `agy-cli`, `codex-oauth` or `xai-oauth` namespaces; a corrupt `secret.key` is reported instead of silently regenerated (which orphaned every stored key); `forge doctor` and the Claude quota probe skip bridges listed in `[mesh] disabled`; `forge mesh --smoke` no longer calls an unroutable pick ready; the weekly cap appears in the budget-refusal message; `/btw` side calls follow the model that served a multi-model-pinned turn; the bridge `spawn_agents` tool is offered at the default `max_depth = 0`; codex bridge tool results show the result text; `forge auth` rejects conflicting `--remove/--replace/--list`; `/usage` no longer prints "sub" for an idle period; `CreditMode` descriptions no longer promise an output-token cap that was removed.
- **TUI polish, second pass.** Shift+Enter and Alt+Enter insert a newline instead of submitting; Alt+B/F, Alt+Left/Right and Alt+Backspace edit by word and unbound Alt chords no longer type their letter; the `@file` picker keeps Ctrl+U/W, Home/End and cursor movement; the mouse wheel scrolls the palette, command center and pickers; rebinding `toggle_reasoning` frees Ctrl+R; invalid key names in `[keybinds]` warn at startup; `/keys` lists the fixed keys; a huge paste collapses to a chip and is echoed in part; DIM text meets 4.5:1 contrast; the statusline's left cluster ends in an ellipsis rather than a mid-word cut; a `--model` pin shows on the statusline at startup; `/commit` and `/pr` check for a git repo (and changes) before spending a turn; `/model <id>` warns when the id is not in the catalog; `/output` explains that only long outputs are kept.
- **Headless, setup and security leftovers (ML-G03-G10).** SIGINT during `forge run` exits 130 with an error
  result in json/stream-json; a direct-path stop with unfinished tasks now exits non-zero as
  `tasks_unfinished`; stream-json carries snake_case `stop_reason` and `tool_use` ids matched by
  `tool_use_id`; flat `*.md` marketplace skills install as loadable `<name>/SKILL.md`;
  `plugin remove` of an unknown pack fails; `migrate` backs up a differing config.toml, writes the
  bundle 0600 and honours `FORGE_DB`; `skill from-session` skips disabled/keyless models; `send`
  explains a 404 as wrong token or old daemon; cancelling `forge setup` skips the local-LLM step and
  the wizard scrolls to the selected row; unreadable config disables telemetry; systemd units honour
  `XDG_CONFIG_HOME`; a catalog reload supersedes a refresh stuck retrying a down provider; broken
  pipes exit quietly, non-UTF-8 args no longer panic, no ANSI in piped errors or non-TTY heartbeats;
  `forge api` accepts any `Bearer` casing, rejects unknown roles and warns on open non-loopback
  binds; `assay --yes` combines with `--max-cost`; doctor lists custom providers and missing key
  env vars; a new `forge.db` is created 0600.
- **Reliability and perf batch (Lattice watcher).** The Lattice watcher no longer ignores edits when the project root itself sits under a dot or `build/` directory; `FORGE_MOCK_TOKEN_DELAY_US=0` is honoured (`crates/forge-index/src/watch.rs`).
- **Reliability batch 2 (attach, signals, store hygiene).** `forge attach` reconnects with backoff when the connection drops and exits non-zero if it cannot; the full-screen TUI restores the terminal on SIGTERM/SIGHUP; `forge run` treats SIGTERM like Ctrl-C and exits 130 on an interrupted turn instead of reporting success; a model bench written while the clock ran ahead is ignored once the clock is corrected, and benches expired for over a week are pruned by maintenance; a session stuck `agent_active` after `kill -9` stops being exempt from retention a week past the horizon; `forge sessions | head` no longer panics; exceeding `ulimit -f` is an ordinary write error instead of SIGXFSZ; a locked store no longer costs five seconds per configured context window; a corrupt `serve-state.json` no longer breaks `forge anywhere status`/`doctor`; an idle TUI wakes about 20 times a second instead of 60 (`crates/forge-cli/src/attach.rs`, `crates/forge-tui/src/driver.rs`, `crates/forge-store/src/model_health_store.rs`).
- **Session and context batch 2 (ML-G02).** A manual `/compact` or `/uncompact` refreshes the context gauge, and a resumed session seeds it from the store; a short transcript is summarized inside a `<conversation>` wrapper and must still carry the sectioned structure; resuming after a `kill -9` says so once; the `--resume` picker deletes its unused placeholder session; resumed fullscreen transcripts show tool cards instead of raw JSON; unanswered nudges for an abandoned task list carry across turns instead of restarting; `forge models` after disabling a provider rewrites the cache; headless tool denials say why and what to change; `tool_use` events carry the model's own args; UserPromptSubmit hooks get the session id on every submit path; the unit tests no longer read the developer's real `~/.claude/CLAUDE.md`.
- **Daemon and remote-client fixes (batch ML-G04).** A failed turn now reports `last_turn_outcome: failed` instead of success; `POST /api/sessions` with an unknown model returns 400, an idle `interrupt` returns `interrupted:false`, `fork` rejects an `at_seq` outside the session's history, `GET /api/history` returns 404 for an unknown session, workspace routes return 400 for a NUL byte, and git stage/unstage treat paths literally (no pathspec magic). A fleet `steer` is delivered ahead of queued follow-ups. `forge attach` errors no longer print the daemon token and explain the `--lan` HTTPS mismatch; `forge mcp` says when servers exist but none is enabled; `dispatch list/show` progress ignores skipped and cancelled items. The PWA labels final answers as forge (not "note"), hides Archive on terminal sessions, and keeps a push-enable error visible. docs/features/remote-control.md names protocol 10 (`crates/forge-cli/src/serve.rs`, `serve_git.rs`, `serve_workspace.rs`, `attach.rs`, `remote_assets/app.js`, `crates/forge-core/src/lib.rs`, `steer.rs`).
- **Anywhere and integrations fixes (batch ML-G05-G09).** `forge anywhere devices --revoke` accepts the 12-word V2 Recovery Kit (or a `.forge-recovery` path) and fails clearly without a terminal; deleting a synced session journals a tombstone; a read-only account is reported as such instead of an enrollment problem; status shows readable dates, no trial line for non-trial accounts, and honours logout; an unseen sender device refreshes the key set instead of dropping the relay; expired-certificate errors de-duplicate in the daemon log; hosts honour `[anywhere] host_name`; doctor advises about a service outage first; public replay links omit system rows; a handoff checks the workspace before stopping the source driver; web new-session no longer keeps a stale cached project path; the web credential store closes its IndexedDB connections. MCP: `logout` and `remove` clear the dynamically registered OAuth client; a connecting/failed/unauthorized server says so instead of "tool no longer exists"; top-level `call_timeout_secs`, `connect_timeout_secs` and `max_reconnect_attempts` in `.forge/mcp.toml` apply; `mcp import` keeps credential-bearing connection strings out of plaintext and warns on URL credentials; `mcp add` validates names and warns about plaintext secrets; `mcp_search_tools` hides denied tools and reports the total; `forge_set_mode` rejects unknown modes; TUI "always" on MCP tools says it lasts the session; a second `/image` keeps the first; `web_fetch` text extraction ignores `>` inside quoted attributes; `web_search` rejects an empty query; `forge api` rejects non-image data URIs; `forge doctor` reports missing language servers; voice model download progress is throttled when stderr is not a terminal; the vision-switch warning mentions a `--model` pin (`crates/forge-cli/src/anywhere/`, `crates/forge-store/src/session_catalog_store.rs`, `crates/forge-mcp/src/`, `crates/forge-config/src/`, `crates/forge-tools/src/web.rs`, `mobile/src/app/new-session.tsx`).
- **Hooks behave the same on the CLI bridge, and CC matchers are regexes.** A bridged built-in tool call now honours a `PreToolUse` argument rewrite and `inject` context (before only `block` worked), `PostToolUse` `inject` reaches the model, and bridge hook payloads carry `cwd` and `session_id` like the direct path. A Claude Code hook matcher with regex syntax (`.*`, `Rea.*`, `^Read$`, `mcp__.*`) now matches instead of silently never firing. `UserPromptSubmit` hooks receive the session id, a `PostToolUse` payload carries the same effective args as `PreToolUse`, and Claude Code `tool_input` no longer includes Forge's injected `cwd`. `/mcp` says why a malformed `.forge/mcp.toml` is ignored instead of claiming no servers are configured (`crates/forge-cli/src/mcp_serve.rs`, `crates/forge-config/src/lib.rs`, `crates/forge-core/src/hooks.rs`, `hooks_cc.rs`, docs/features/hooks.md).
- **Session and context batch (ML-G02).** `@` picker lists untracked files and Enter with no match submits the prompt; `forge run` expands `@file` mentions; stream-json `tool_use`/`tool_result` carry paired ids; `forge attach` exits on detach without needing Enter; `POST /fork` rejects a negative `at_seq`; session token counters keep compacted calls; a disabled OpenCode Go provider is no longer polled for usage; `/help` groups each heading once; a steered prompt no longer glues onto the preceding reply; the completeness review works in repositories with no commits; a question-only turn no longer re-verifies an old all-Done task list.
- **Autonomy and subagent robustness (batch ML-G11).** Harness nudges no longer replay as your own prompts; "do not change/modify anything" counts as read-only; schedule units escape `$` and newlines and honour `XDG_CONFIG_HOME`; `spawn_agents` over the cap is rejected with a message instead of silently dropping agents; worktree conflict reports name the real file; the git-commit reminder sees files in new untracked directories; background job files are git-ignored; `forge queue run` exits non-zero when every task failed and no longer double-records a shadow win for the same model; `/heartbeat status` shows no countdown for a paused heartbeat (`crates/forge-core/src/turn_contract.rs`, `replay.rs`, `worktree.rs`, `git_hygiene.rs`, `subagent/requests.rs`, `crates/forge-cli/src/cli/commands/queue.rs`, `schedule/native.rs`).
- **Autonomy and subagent robustness, second batch (ML-G11).** A polled command whose output keeps changing no longer trips the doom-loop guard; `/goal` and `/loop` refuse a non-numeric `--max-tokens`/`--max-minutes` instead of running uncapped, and `forge run "/loop ..."` says it is interactive only; a headless goal's budget stop reads "goal not met"; `forge schedule add` validates `--mode`; `forge queue run` records the child's stderr when it dies without a result and marks a crashed assay gate `gate-error` instead of `gated`; `/duel` refuses a read-only temper, and `/workflow run` for a missing workflow leaves no failed run row; plan mode no longer advertises `append_file`/`multi_edit`; `/execute` keeps a non-plan temper; subagent model calls use the first-event timeout; delegation prompts keep tools on the trivial tier; plain output starts a warning on its own line (`crates/forge-core/src/completeness.rs`, `duel.rs`, `orchestration.rs`, `subagent.rs`, `crates/forge-tui/src/arg_parse.rs`, `headless.rs`, `crates/forge-cli/src/cli/commands/queue.rs`, `run/one_shot.rs`).
- **More daemon and remote-client fixes (batch ML-G04, part 2).** `forge serve` prints the connect URL, token and QR only on a terminal (under `forge service` the journal gets a redacted URL). `/model <id>` rejects an unknown provider prefix instead of pinning it, and typing over the board's `/model` prefill replaces it. A malformed WebSocket input now gets a `{"keepalive":true,"input_error":...}` control frame instead of vanishing, and an `answer` sent with no question pending gets a note pointing at Allow/Deny. `GET /api/sessions/{id}/diff` includes files created in the worktree, cancelling a plan clears it from the snapshot and `/api/plans`, uploads are git-ignored, the diagnostics database check really queries the store (and the probe no longer scans every process), and concurrent first-use voice transcriptions share one model download and reject bad audio before downloading. Switching the permission mode edits only that line of `config.toml`, keeping comments (`crates/forge-cli/src/serve.rs`, `serve_git.rs`, `serve_diagnostics.rs`, `remote.rs`, `voice.rs`, `crates/forge-voice/src/model.rs`, `crates/forge-tui/src/app.rs`, `board/`, `crates/forge-config/src/lib.rs`).
- **Skills, agents and instruction files, medium/low batch.** `CLAUDE.md` and the user-global instruction files (`<config>/forge/AGENTS.md`, `~/.forge/AGENTS.md`, `~/.claude/CLAUDE.md`) are now injected when the project has no `.forge/AGENTS.md` or `AGENTS.md`; user-scope agents in `<config>/forge/agents` (where `forge import claude` writes them) load at runtime; the `use_skill` listing clips descriptions on a word boundary and keeps a `Use when ...` clause; an untrusted project skill no longer hides the user's or built-in skill of the same name; `forge plugin update` keeps a hand-written skill the pack does not own, and install/update say what was kept and which plugin components (hooks, agents, commands, `.mcp.json`) are not installed; `forge skill normalize` maps `claude -p` / `codex exec` to `forge run`; a model in `[mesh] disabled` is reported as disabled, not quota exhausted; keybind key names are case-insensitive; a branch widget in the unrendered statusline center/right no longer removes the location chip; Alt-R says what it reloads; `PUT /api/config` errors drop the figment internals (`crates/forge-core/src/lib.rs`, `crates/forge-config/src/agents.rs`, `crates/forge-skills/src/summary.rs`, `crates/forge-cli/src/cli/commands/marketplace.rs`).
- **File, shell and web tools: a batch of correctness fixes.** `edit_file`/`multi_edit` keep a file's CRLF endings, re-indent a whitespace-matched replacement to the real block, refuse an empty or identical `old`, count `replace_all` replacements, no longer cry "already applied" for a short `new`, and number a failing multi-file edit overall; the truncation guard understands escaped backslashes, char literals, inch marks, `#` comments and single-quoted strings, and `multi_edit` checks bracket balance on the combined result. Codex-envelope `apply_patch` is all-or-nothing and performs `*** Move to:`; a multi-file unified diff without `diff --git` lines applies (`--recount` is now a retry). `read_file` refuses FIFOs and devices, explains past-EOF and inverted ranges, rejects `path` plus `paths`, and chunks a one-line giant; `search` skips FIFOs; absolute `glob` patterns work; missing-path errors name the path and similar files. `shell` keeps the tail of output over 1 MiB, returns right after SIGTERM when the group exits, shows readable text around NUL bytes, refuses contradictory arguments, no longer feeds RTK-formatted data into pipelines or `git status --porcelain`, hides provider keys Forge injected from its children, and keeps `.forge/jobs` out of git. `web_fetch` refuses hosts that resolve to private addresses (and `localhost.`), reports binary bodies, and honours Latin-1 charsets. The bridge caps `list_dir`/`glob`. A successful poll-ready or background-start no longer triggers the shell-error interceptor; natural "don't change the existing tests" prompts enable the test-edit guard; `[mesh.subagents] enabled = false` rejects a `spawn_agents` call; plan files stay out of git.
- **TUI polish batch (medium/low findings).** Esc no longer lets the aborted turn's queued events render as a ghost block; Enter on a typed short alias (`/sl`, `/ls`, `/r`) runs what you typed instead of the palette's highlight; the `/model` picker hides models from disabled providers; a batched shell call that exits non-zero no longer shows a green check; a tool refused by task scope now draws a denied card; `/btw` records its usage; the composer moves and deletes by grapheme cluster; the fullscreen transcript wraps prose at spaces; `TERM=dumb` falls back to plain chat; SIGTERM/SIGHUP restore the terminal; the keybind configurator refuses a key another action owns and Esc discards; `/help` lists each group once and files `/rewind` under History; unknown commands suggest the closest one; `/new` resets the statusline counters; `/compact` says so when there is nothing to compact; `/effort` errors list `max`/`whitehot`; recaps keep the tail of long answers; `NO_COLOR` is honoured by error lines; `/duel` preconditions no longer read as internal errors.
- **Mesh, exit-code and config robustness batch.** Disabled providers/models no longer receive classifier or side-call prompts (nor are disabled bridge CLIs probed at discovery); `[mesh] daily_cap_usd` loads again; HTTP 413 fails over like a rate limit; `/duel` works with the default LLM classifier; `forge run` exits non-zero for step-ceiling, budget, no-output and unroutable turns and stream-json reports the real result subtype; read-only turns also deny `append_file`/`multi_edit`; shell `<(...)`/`>(...)` bodies are vetted like `$(...)`; `forge schedule` writes a systemd-valid `WorkingDirectory`; `forge plugin remove` resolves packs from the lockfile and rejects paths; a corrupt `secrets.enc` is no longer overwritten; worktree merge-back from a repo subdirectory no longer drops the change.
- **Headless, resume and store fixes.** `FORGE_TELEMETRY=0` no longer breaks config loading. `forge run` reads piped stdin (appended to the prompt, or the prompt itself), an unroutable turn exits non-zero with `is_error: true`, and `--output-format json`/`stream-json` print an error result on failure instead of nothing. Resuming through the picker or `/resume` restores the session's permission mode, pinned model and effort like `--continue`. Config writes are atomic (temp file plus rename) and provider removal refuses to rewrite a config that does not parse. `forge import claude` skips command and skill names that would escape the config directory. An unwritable data directory fails at once with the path instead of stalling 30 s, and `forge doctor` reports it. Full-screen PageUp scrolls one page from the current position after a tool card or a clear. Forking a compacted session keeps the compaction summary (or the verbatim rows when the fork point precedes the compaction). Side calls no longer pick a model from a provider listed in `[mesh] disabled` (`crates/forge-config/src/lib.rs`, `crates/forge-cli/src/cli/commands/run/one_shot.rs`, `crates/forge-store/src/fork_store.rs`).
- **Daemon, Anywhere, app and integration fixes.** A dispatch merge or session archive no longer fails (merging nothing with exit 0) in a repo that gitignores `.forge`, and `forge dispatch merge` exits non-zero when it stops. A daemon shutdown freezes dispatches instead of failing queued items, and a restored worker left idle by a restart is told to continue. A dequeued prompt is withdrawn from the running turn too. Cancelling a plan on a resumed session falls back to Ask, not Auto-edit. A session whose only usable model is busy now waits for it instead of failing the turn. Remote frames cap the in-flight partial line at 2 KiB so bandwidth stops growing with reply length. Anywhere: a queue stamp up to 15 minutes after the sealed envelope is accepted, a local edit of an adopted session continues the remote revision, an upload failure no longer starves downloads, sessions synced in from another host start in Ask mode, and `anywhere disable`/`enable` stop and restart the relay connector in the running daemon. `forge mcp login` finds the OAuth server through the 401 `resource_metadata` URL or the RFC 9728 path-aware URL. The LSP memory guard no longer counts Linux threads as processes, post-edit diagnostics wait for the server's publishes to settle, and Lattice indexes TypeScript/TSX functions, classes, methods, enums and type aliases (`crates/forge-core/src/worktree.rs`, `crates/forge-cli/src/serve_dispatch/supervisor.rs`, `crates/forge-cli/src/anywhere/`, `crates/forge-mcp/src/oauth.rs`, `crates/forge-lsp/src/server.rs`, `crates/forge-index/src/extract.rs`).
- **TUI, tools and provider reliability batch.** Single line breaks in model replies are kept instead of collapsing to spaces; Esc during a running shell command now kills the command's whole process group; an interrupted text turn leaves a marker so the next request no longer carries two bare user messages; the secret-file denylist now covers `paths[]`, `edits[].path`, `append_file`, `multi_edit`, `apply_patch` bodies and symlink targets; auto temper asks before an `apply_patch` writes outside the workspace; short edit prompts ("rename X everywhere", "add ...", "summarize README") keep their tools; HTTP 413 / "request too large" now shrinks the request and retries; genai's stream-start event no longer disarms the first-event timeout; `/export` write errors, a mistyped `/remote` flag, palette Tab highlight and browser-launcher stderr no longer break the session; the stale undo e2e test quits with `/quit`.
- **Compaction no longer thrashes on a small trigger.** Everything a compaction puts back (the summary, the verbatim user messages, the kept tool rounds, the restored files) is now budgeted from the trigger auto-compaction compares against, capped at the old ceilings, so a 24k `[compact_cap]` or a 32k-window model lands near half its trigger instead of above it. Restored files take only what is left under that target, and a context that has not grown by a quarter of the trigger since the last compaction is not summarized again. On `forge bench recall --scenario hard` at a 24k cap this took auto compactions from 43 to a handful. The summarizer chain now logs (`RUST_LOG=forge_agent_core=info`) which model heads it and why a candidate was skipped or failed.
- **Rewind, undo and checkpoints agree with the store.** A turn's checkpoint was keyed by the session's in-memory seq while the store deactivates by DB seq, and side calls (recap, suggestion, memory capture) insert inactive usage-anchor rows that push the DB seq ahead, so rewinding to turn 3 of 8 soft-deleted the tail of turn 2 (answer and tool results) while its prompt stayed active, and across a compaction it emptied the transcript and put turn 1's prompt back in the input box. The turn's user row now reports the seq it really landed on and the checkpoint, snapshot directory and `/undo` chain use it; `rewind_to` restores the files of every snapshotted turn at or after it, deactivates by DB seq and reloads the transcript from the store, so the live conversation is exactly what a resume would load, and the next row never reuses a rewound seq. After a rewind the cursor sits at the end of the restored prompt, and `/new` and `/resume` reset the context gauge and the token totals instead of showing the previous session's (`crates/forge-core/src/session_history.rs`, `lib.rs`, `snapshot.rs`, `crates/forge-store/src/checkpoint_store.rs`, docs/features/rewind.md).
- **Pickers have PageUp/PageDown/Home/End, stay fast with thousands of rows, and `/sessions` finds message text.** The rewind and session pickers ignored the jump keys; they now move a screenful or jump to the ends, the row filter no longer re-lowercases the whole list on every keystroke and frame (20,000 rows stay interactive), the app's picker overlay carries a 200-row window around the selection instead of every checkpoint in every snapshot, and `/sessions` also searches message bodies like the daemon's search does. The Tab view of `/sessions` lists archived sessions only, Del archives or un-archives by the row's own state, and resuming an archived session brings it back (`crates/forge-tui/src/commands.rs`, `crates/forge-cli/src/cli/commands/run/pickers.rs`).
- **`forge replay` prints each answer once, and `forge tree` shows the whole lineage.** The published `ui` copy of a final answer is dropped when it repeats the `llm_only` row before it. `forge tree` nests forks of forks, draws `├─`/`└─`, and shows a fork whose source was deleted as its own root instead of an empty list. `forge fork --model <bad id>` is validated before any session is written and exits non-zero (`crates/forge-store/src/transcript_read_store.rs`, `crates/forge-cli/src/cli/commands/replay.rs`).
- **The daemon no longer archives, deletes or resumes a session that is open in a terminal, and a terminal no longer resumes a daemon-driven one.** Deleting a terminal-live session removed its rows under the live writer (the TUI then re-created a stub with only later messages); resume left two drivers on one session. Archive, delete and resume answer 409 while the terminal's heartbeat is fresh; `forge chat --resume`, `--continue` and the `/resume` picker refuse a session a live `forge serve` is driving and point at `forge attach` (`crates/forge-cli/src/serve.rs`, `crates/forge-cli/src/cli/commands/replay.rs`).
- **Memories: `--global` ones are recalled, secrets are masked, and `forge memory` reports honestly.** Global memories were never injected into a session; they now share the recall slots with the project's. Credential-shaped text is masked before a manual or auto-extracted memory is stored (it used to be saved verbatim and replayed into every later prompt). Re-adding a known fact says "already remembered" with the stored text, an empty text no longer reads "connection pool: ...", and removing a memory that does not exist exits non-zero (`crates/forge-core/src/pinned_context.rs`, `auxiliary_policy.rs`, `crates/forge-store/src/memory.rs`, `crates/forge-cli/src/cli/commands/memory.rs`).
- **Every surface installs the commit-attribution hook it promises, and the hook is safe.** With `[git] coauthor = true` the prompt says a Co-Authored-By trailer is added automatically, but only the TUI installed the hook, so headless `forge run` and daemon sessions committed without it. The hook is now installed (and the model file kept current) by the session itself before each request. It resolves the hooks directory with `git rev-parse --git-path hooks` (linked worktrees, submodules), keeps an existing `prepare-commit-msg` hook as `prepare-commit-msg.pre-forge` and runs it first instead of overwriting it, reports `core.hooksPath` instead of claiming success (`forge git hook` prints the script), strips co-author lines in any casing, adds the trailer once across `--amend`s, and leaves an empty message empty (`crates/forge-core/src/git_attribution.rs`, docs/features/commit-discipline.md).
- **Forge's own scaffolding no longer dirties your repository.** The shared-target `.cargo/config.toml` is written only for Rust projects without their own, self-ignored; `.forge/worktrees/`, `.forge/worktree-owners/` and `.forge/forge.log` are ignored the same way, so `open_pr` no longer refuses in a daemon worktree session and `git status` stays clean (`crates/forge-core/src/worktree.rs`, `worktree_reclaim.rs`, `crates/forge-cli/src/main.rs`).
- **The review dock's git routes tell the truth.** `GET /api/git/diff` no longer shows an unchanged tracked file as a created file, pairs a staged rename with its source (`kind: renamed` with `orig_path`), and takes `offset`/`limit` so a reviewer can page past the 4,000-line cap (a full rewrite's additions used to be unreachable; each file reports `offset` and `total_lines`). Untracked rows in `GET /api/git/status` carry their line counts instead of +0 -0 (`crates/forge-cli/src/serve_git.rs`).
- **Tool results read honestly.** A shell command that exited non-zero is no longer drawn with a check mark, and a call you declined reports "denied by the user" to you and to the model instead of "denied by policy". The branch line is reported from a subdirectory of the repository and as `(detached HEAD at <sha>)` instead of vanishing (`crates/forge-core/src/tool_dispatch.rs`, `lib.rs`, `session_worktree.rs`).
- **Commit reminders keep their cadence.** The edit-20 reminder for a single file was dropped because it was byte-identical to the edit-10 one; reminders now carry the edit count, and an unchanged dirty set is re-announced at turn start only every third turn. `/commit` tells the model to name the paths it stages and never to use `--no-verify` (`crates/forge-core/src/git_hygiene.rs`, `crates/forge-cli/src/cli/commands/run/commit_prompt.rs`).
- **Long sessions keep what the person asked for.** The window trim used to protect only the newest user message, and the harness's own "your last response was empty" / failed-gate nudges (stored as user messages) could take that place; it now keeps every real user message of the current turn (the originating request and any steers), harness nudges are tagged and never count as the task, and protected messages too big for the budget are clipped from the front instead of dropped. Before the trim drops real history Forge compacts, and it warns once per turn when it still has to trim. Clock lines, tool-result hints and progress nudges are superseded by the newest of their kind. After a compaction the store keeps exactly the tail memory kept, so a resume matches the live transcript and never opens on an orphaned tool result; `/uncompact` no longer brings `/undo`-rewound messages back; a compaction that gives up clears its progress band; the idle auto-compact honors `[compact_cap]` and the free-model ceiling and reports as automatic; and the claude/codex bridges find the new messages of a resume by identity, so a front-trimmed view can no longer skip them.
- **Gateway model aliases no longer fall to a 32k context window.** A pinned alias such as InferHub's `inferhub::combo/glm-5-3-flash-fp` matched no known window, so the statusline showed 32k and long turns were trimmed to it. The cross-provider window match now reads a dash between two digits as a version dot and ignores a trailing quantization tag (`-fp`, `-fp8`, `-bf16`, `-int4`, `-awq`, ...), so the alias borrows `glm-5.3-flash`'s 1M window (`crates/forge-mesh/src/pricing.rs`).
- **Reconnecting after a busy stretch no longer replays hundreds of superseded snapshots.** Every session stream frame is a complete snapshot, yet a client reconnecting with `?rev=` was sent every frame it missed from the 512-entry replay log: measured on a live session, a phone returning after ~500 revisions received 503 frames (3.2 MB) one by one through the Anywhere relay before it reached the present, which kept the session on "reconnecting…" for a long time. A gap of more than 8 frames now gets the current snapshot once as a `resync`; short gaps still replay exactly (`crates/forge-cli/src/remote.rs`, docs/features/remote-control.md).
- **The mobile app over Forge Anywhere reconnects, loads the fleet and follows sessions much faster.** Every relay frame, in and out, used to write the whole credential blob to the keychain and re-render the app. Outbound sequences are now reserved in blocks of 1024 (one keychain write per block; a crash only skips unused numbers), and the inbound replay window is checked in memory at once and written at most every 1.5 s and when the app backgrounds, without re-rendering; job acknowledgements still write immediately. A refresh that returned the same hosts no longer rebuilds every transport (one leaked WebSocket and ticket per host each time), one relay connection is shared by all hosts and is opened in the background as soon as the app is signed in, and the Fleet shows the last-known sessions at once (marked as updating) instead of skeleton cards, with a faster first retry. Settings > Diagnostics gains relay round-trip and per-frame p50/p90 plus cold-start timings. On the host, the connector reserves outbound sequences 256 at a time instead of rewriting and fsyncing the state file for every envelope, and inbound acceptance no longer blocks an async worker. Opening a session starts its history fetch at press-in and shares one history request with the session shell instead of issuing two, and the host connector now keeps only the newest unsent full snapshot per stream (older snapshots and keepalives are dropped, the survivor is flagged `resync`, terminal output and closes are never dropped), so a slow relay no longer delivers a backlog of stale snapshots. Touches `mobile/src/lib/AnywhereProvider.tsx`, `mobile/src/lib/anywhereSequenceState.ts` and `crates/forge-cli/src/anywhere/connector/outbound_sequence.rs`.
- **Ctrl-C no longer throws away your prompt or quits on one press.** Idle Ctrl-C with text in the input clears it and saves it to prompt history (↑ recalls it); on an empty input the first press shows "Press Ctrl+C again to exit" and a second press within 2 s quits, any other key cancels. Mid-turn and overlay behavior is unchanged (`crates/forge-tui/src/app/idle_interrupt.rs`, `crates/forge-cli/src/cli/commands/run.rs`, docs/features/keybinds.md).
- **A remote image upload no longer prints a spurious `(binary)` skip.** The `@path` mention that keeps an uploaded image in history was also read as a text file and reported as skipped; image mentions are now recognised by content and attached once (`crates/forge-cli/src/cli/commands/run/atfiles.rs`).
- **Slash commands typed while a turn is running now do something.** `/btw <question>` used to be refused with "commands run when the turn is idle" on every input surface before its busy path was reachable. Commands are now classified in one place (`busy_policy`): `/btw`, `/help`, `/keys`, `/sessions`, `/config`, `/clear`, `/remote`, `/statusline`, `/voice` and a few more run immediately beside the turn; commands that need the session lock (`/compact`, `/model`, `/effort`, `/usage`, `/plan`, file commands, and so on) queue in the existing prompt queue and run in typed order when the turn ends (never steered into the model turn; an Esc interrupt still runs them); only `/resume <id>`, `/undo`, `/checkpoints`, `/claude` and `/quit` stay refused. Same rules on the local keyboard, remote input (phone, web, `forge attach`) and daemon-hosted sessions (`crates/forge-cli/src/cli/commands/run/busy_commands.rs`, docs/features/command-skill-system.md, docs/features/side-questions.md).
- **Scheduled wakeups, `/heartbeat` ticks and peer messages no longer show up as something you typed.** Like background-job exits, these reach the model as user-role messages (`[scheduled wakeup] <prompt>` + `(scheduled because: <reason>)`, `[heartbeat: <label>] <prompt>`, `[message from <who>] ...`), and the TUI used to echo them under a "you" header. One classifier, `forge_types::parse_harness_notice`, now covers jobs, wakeups, heartbeats and peers, and the generators build their text with the same `forge_types` format functions so they cannot drift. The TUI (live, mid-turn echo, resume) shows dim rows (`⏰ wakeup · <prompt…>` with the reason underneath, `♻ heartbeat <label> · <prompt…>`; long prompts cut to two lines), headless and the `mcp_agent` log print the same, the remote history API tags the row `kind: "harness_notice"` (live rows carry it with `meta` = `wakeup`/`heartbeat`/`peer`; `job_notice` is unchanged), and the web app and mobile app render dim system rows. The model transcript is untouched. A prompt you type that is literally in one of these shapes (for example `[heartbeat] x`) is rendered as a notice too (`crates/forge-types/src/harness_notice.rs`, `crates/forge-tui/src/app/harness_notice.rs`, docs/features/shell-tool.md).
- **Background-job exit notices no longer show up as something you typed.** When a `shell{background:true}` job exits, Forge still tells the model through a user-role message, but the TUI (live, mid-turn echo and resumed history), headless output, `forge attach`, the remote web app and the mobile app now show it as a harness notice (`⚙ job 3429249 ✓ exited 0 · <cmd>`, failures `✗`, the log tail dim underneath) instead of a "you" row. The remote history API tags such rows `kind: "job_notice"` and live rows carry the same kind, additively (`crates/forge-types/src/job_notice.rs`, `crates/forge-tui/src/app/job_notice.rs`, docs/features/shell-tool.md).
- **Typing an `f` no longer replaces your prompt with a shell command.** After a failed shell command got a diagnosis with a suggested fix, any `f` or `F` typed while Forge was idle swapped the whole input for that fix command, mid-sentence. The shortcut is removed; the fix still shows in the diagnosis card and reaches the model (`crates/forge-cli/src/cli/commands/run.rs`).
- **Streamed tool calls whose `index` starts above 0 no longer add nameless "unknown tool" calls.** Gateways that relay an Anthropic upstream number tool calls by content block, so the first call arrives at index 1 or 2; the gap slots below it were emitted as empty calls the model then had to recover from. Never-named placeholders are now dropped at the end of the stream (`vendor/genai-0.6.5/src/adapter/adapters/openai/streamer.rs`).
- **Design-level fixes: bridge teardown, in-turn run budgets, daemon project root (design-c).** A `claude-cli`/`codex-cli` turn that ends or is aborted no longer orphans a shell command still running in its own process group (the teardown now walks the bridge's process tree; `shell{background:true}` jobs, which are `setsid`, keep running); `/goal` and `/loop` `--max-tokens`/`--max-minutes` are enforced inside a turn, ending it cleanly with stop reason `budget_exhausted` instead of overshooting until the turn finishes; a daemon-hosted session resolves its project config, trust, hooks, `.mcp` and project commands/skills against its own workspace root rather than the daemon's cwd (`forge_config::load_in`); a session's title comes from what the user typed, not the prompt a hook appended to; `--mock` usage rows are filed under the `mock` provider instead of a real seed model's; a resumed session shows its last outcome and model before the next turn. `/goal` through the daemon already parses and runs like the TUI (covered by a new test).

## [2.20.0] - 2026-10-14

Sessions follow worktrees and host moves, `/goal` checks its own progress (also in a plain `forge run`), LSP diagnostics are sent as deltas, codemode gets used, and InferHub models run lean by default.

### Added
- **Sessions can move into git worktrees and back with the new `worktree` tool.** `create`, `enter` (including worktrees outside the repo, subject to `confine_to_workspace`/`extra_roots`), `list`, `exit` (optionally removing the worktree) and `remove` move file tools, relative paths, the shell's cwd, Lattice, the branch and the stored session cwd together, on both the direct path and CLI bridges. Removal refuses dirty worktrees and unmerged branches unless forced, `/undo` snapshots survive removal, and entering a worktree is never mistaken for a host move (`crates/forge-core/src/session_worktree.rs`, docs/features/session-worktrees.md).
- **Sessions moved to another host now re-ground themselves instead of chasing the old machine's paths.** Forge records each session's host fingerprint (machine, workspace, repo, git worktrees and remotes; store migration 44). On resume, import or handoff to a different machine or workspace it adds one persisted notice with the current layout and a path map matched by git remote, telling the model its tool outputs are real and only remembered old-host paths may be stale. File tools aimed at a vanished old-host path get an error naming the local equivalent, a missing workspace is relocated to the local checkout of the same repo, and `/reground` re-summarizes history without the stale host facts. Handoff capsules now also carry the compaction summary, which they used to drop (`crates/forge-core/src/host_move/`, docs/features/session-host-move.md).
- **`/goal` now checks its own progress after every turn.** A fast side-call model compares the goal with what was actually done (task plan, final answer, commits, diff, gate results, recorded checks) and tells the agent what is still missing and the single next step, instead of repeating the same "continue the goal" prompt. A `🎯 2 left · next: …` line shows each verdict in the TUI, headless output and on the phone. The goal stops once the evaluator judges it achieved and the quality gates pass, or as wedged when nothing has moved for three evaluations. Turn it off with `[goal] evaluator = false` or pick the model with `[goal] evaluator_model` (docs/features/goal-evaluator.md).
- **InferHub models default to the `minimal` harness profile.** A 72-run ablation on `inferhub::glm-5.3-flash` measured 14% fewer input tokens and 17% fewer model calls with no pass-rate change. Opt out with `[mesh.harness_profiles] "inferhub::*" = "guarded"` (`crates/forge-config/src/harness_profile.rs`).
- **`forge run "/goal …"` now loops to completion without a daemon.** A local headless run (no daemon, `--no-publish-to-fleet`, `--usage-json` or a machine output format) used to send the text as one prompt and exit after a single turn. It now runs the same turn, evaluator and decision loop as the daemon (same iteration ceiling, `--max-tokens`/`--max-minutes` budgets, wedge detection and `--gate` checks in the session's workspace), exits 0 only when the goal is met, exits 1 with the reason otherwise or on Ctrl-C, and `--usage-json` totals every iteration (`crates/forge-cli/src/cli/commands/run/goal_headless.rs`, docs/features/goal-evaluator.md).

### Changed
- **LSP diagnostics after an edit are now a per-file delta.** The first edit to a file shows its errors and the findings near the edit, plus a total count; later edits show only new diagnostics and a resolved count. Errors the edit introduced are always shown in full, everything else is capped at about 2,000 characters in a stable order. On the Python bench this cut diagnostics text on the wire by 84% and input tokens per task by 20% with the same pass rate. Configure with `[lsp] diagnostics = "delta" | "full" | "off"` (default `delta`; `crates/forge-core/src/lsp_delta.rs`).
- **Direct-path models now actually use `run_code`.** A fixed guidance block is sent only when `run_code` is advertised, alongside a when-to-use tool description and a one-time hint after three single-call exploration steps (off under the `minimal` harness profile, never on bridges). Script errors now name the failed call and the line, `tools.x("arg")` shorthand works, `.join()` on a result string no longer throws, and diagnostics a script returned are no longer delivered twice. On six InferHub tasks adoption went from 0/18 to 4/18 runs (glm-5.3-flash) and 6/18 (kimi-k3), pass rate unchanged (`crates/forge-core/src/codemode/guidance.rs`).
- **InferHub models are scored on the right benchmark row and priced from the live marketplace.** The benchmark matcher now requires equal version numbers and matching tier words, so `kiro-opus-5` no longer takes Opus 5.5's score and image models no longer borrow chat scores (InferHub audit: 43 wrong matches down to 0). Custom providers whose `/models` lists marketplace asks are priced at the median ask (`pricing_stat = "median" | "min" | "mean" | "official"`), the router now uses fetched prices, a provider-reported `usage.cost` is recorded as actual spend, and per-model `reasoning_levels` limit which effort rungs Forge sends (`crates/forge-mesh/src/bench/model_id.rs`, `crates/forge-provider/src/custom_models.rs`).

### Fixed
- **A shell batch (`commands: [...]`) now shows its first command in the TUI tool card headline** (`cargo build (+1 more)`) instead of an empty or cwd-only title (`crates/forge-tui/src/app/tool_cards.rs`).
- **Workflow scripts that open with `export const meta = {...}` now run instead of failing with "unsupported keyword: export".** The header (also plain `const meta`) is parsed as a literal for a name/description/phases banner, the `export` keyword is dropped, and unsupported module syntax gets an error that spells out the accepted format. The `run_workflow` tool description now documents the header (`crates/forge-core/src/workflow_meta.rs`, docs/features/forge-workflow.md).
- **The build spend cap now recognizes clang and the rest of the native toolchain.** `clang`, `clang++`, `c++`, `ld`/`lld`/`mold`, `zig cc`, versioned names (`clang-19`, `gcc-14`), cross triples (`aarch64-linux-gnu-g++`), absolute paths, env prefixes and `ccache`/`sccache`/`sudo`/`bash -c` wrappers all count as build commands now; before, only bare `gcc`, `g++`, `cc` and `clang` did, so failed clang builds never armed the cap (`crates/forge-core/src/build_command.rs`).
- **Auto-memories follow a project across a workspace move.** Memories stay keyed by workspace path, but are now mirrored under a stable per-git-remote alias, so a workspace that changed path (host move, re-clone, linked worktree) recalls the memories of its old path. The old key keeps its memories (`crates/forge-core/src/memory_scope.rs`, docs/features/session-host-move.md).
- **Esc no longer records a queued prompt twice.** When the running turn had already absorbed a queued prompt as a steer and you pressed Esc before it answered, the prompt was re-sent as a brand-new turn: it showed twice in the transcript and the phone snapshot, and the model and the stored session saw it twice. The re-driven turn now only continues from the absorbed prompt (`Session::run_turn_resuming`), on the local TUI, remote interrupts and the `forge serve` driver.
- **A long session no longer stops after the first step of every turn.** The stagnation guard carried its pending nudge and stale-step window across user turns; in long sessions nearly every read repeats output seen earlier, so after one nudge each later turn halted after a single tool call and answered without tools. A new turn now gets a fresh nudge-then-halt budget, and a loop that continues is still caught within the turn (`crates/forge-core/src/loop_progress.rs`).
- **Esc after queueing a prompt mid-turn now always sends it as the next turn, and prompts typed while Forge is busy show up in ↑ history right away.** Esc used to work only if the running turn had not yet picked the queued prompt up at a tool step; otherwise it stopped with the prompt unanswered. Busy prompts now reach history on submit and ↑/↓ recall works mid-turn. Closing a `/btw` card before its answer arrives no longer lets that answer reopen the card and swallow the next Esc. Applies to the TUI, `forge serve` sessions and `forge attach` (`crates/forge-cli/src/cli/commands/run/busy_queue.rs`, `crates/forge-tui/src/prompt_history.rs`).
- **`/goal` and `/loop` quality gates run in the session's current worktree**, not the directory Forge was launched in (`crates/forge-cli/src/cli/commands/run/gates.rs`).
- **`shell.sandbox` no longer breaks every git command.** Landlock blocked writes to `/dev/null`; a sandboxed shell also refused any cwd outside its first root, and `/duel` merged winners into the launch directory instead of the checkout the duel came from.

## [2.19.0] - 2026-10-14

Reason Agent 1.1-inspired release: cache-first context, honest bench numbers, codemode, lean tools, evidence-backed PRs.

### Added
- **Forge now measures and stops its own prompt-cache misses.** Each model call is labelled with why it did or didn't reuse the provider's cached prompt (`append`, `harness_rewrite`, `floor_advance`, `compaction`, `history_rewrite`, `provider_eviction`, …). `forge stats cache [--session]` shows the hit rate, the cost of uncached input over cache-read pricing, and the cause of each miss per model. The TUI warns when Forge rewrites context it already sent twice in a row; rewinds and full-history reloads don't count. Old tool output is now shortened only when a single forward-only elision floor advances, in blocks, instead of at every turn and every few steps. Auto-compaction goes down to a low-water mark (`[mesh] compact_low_water`, default 0.62) in one step. Turn contracts stay in the prompt, and a changed contract says it supersedes earlier ones. The git branch and the full clock instruction are only sent when new. Tuning: `[mesh] context_floor_block_tokens` (default 32768). Store migration 41.
- **`forge bench report` now scores each run against its own eval report and shows cost per solved task.** Pair runs with the repeatable `--run metrics.jsonl=eval.json`, or by position with `--metrics`/`--eval`. A count mismatch is now an error, and `--shared-eval` keeps the old combined behaviour. Before this, a solve by one agent or seed was also credited to the others. Runs are grouped by agent and resolved model. Each group shows:
  - mean pass@1 ± stdev and pass@k
  - tokens per attempt and cache hit rate
  - cost per attempt and cost per solved task
  - notional (list-price) cost for subscription and free routes, labelled as not spend

  A Pareto block is printed only when every attempt has at least 20 instances that count toward pass@1. Attempts where tools were unavailable no longer count toward pass@1, and detected refusals score 0. Metrics sidecars now record the attempt number, session id, cache-write and reasoning tokens, model and tool call counts, and the outcome. Store migration 42 adds nullable `usage.cache_write_tokens` and `usage.reasoning_tokens`, filled for Anthropic, Bedrock and OpenAI-compatible providers, the Claude and Codex bridges, and codex-oauth.
- **Zero-data-retention routing and a cross-session prompt-cache key.**
  - `[privacy] zero_retention = true` (off by default) routes only to providers known not to retain prompts. Unknown providers count as retaining. Only local Ollama is built in, and Ollama cloud models (`:cloud` / `-cloud` tags) do not count as local. Vouch for other providers with `[privacy.retains] <provider> = false`.
  - In this mode retaining models are dropped from routing, failover and side calls, and from the LLM tier classifier, the bridge auto-mode classifier, `forge skill` and `forge assay`. A retaining pin fails with an explanation instead of being sent. OpenRouter requests carry `provider.data_collection = "deny"`. `forge mesh explain` lists what was dropped.
  - New sessions now share a provider prompt-cache key derived from model, tools, system prompt and workspace (`[mesh] cross_session_cache_key`, on by default), so they reuse the fixed prefix an earlier session already warmed. The key is set at a session's first request and never changes mid-session; `/new` and `/resume` pick it again.
  - Repetition penalties now actually reach OpenRouter.
- **Minimal harness profile per model.** Set `[mesh] harness_profile = "minimal"`, or a per-model glob such as `[mesh.harness_profiles] "bedrock::*" = "minimal"`, to turn off Forge's advisory nudges for that model: the progress-note ask, stall and stagnation nudges, commit and push reminders, the auto-orchestrate prompt, and the mid-intent and continue nudges. The safety guards stay on: the permission broker, the doom-loop, failure-loop and stagnation halts, the phantom-edit gate, the step cap and the spend caps. The profile follows the model running each step, so a failover can change it. The default, `guarded`, keeps today's behaviour. Separately, the "your last response was empty" nudge is now sent once per streak of empty replies instead of twice: a model that answers it with another empty reply is benched and the turn fails over straight away.
- **Secrets are now masked in everything Forge exports, and bench attempts are checked for test tampering.** By default, `forge replay` redacts all of its output (transcript, `--json`, `--html` and diffs); use `--no-redact` to see the raw text. `/export` and `forge anywhere share` always redact. Masked values become `[redacted]`. That covers configured API keys (from env and the keyring), values of env vars named like secrets, common credential formats (`sk-…`, `ghp_…`, AWS keys, JWTs, private-key PEM blocks, `Bearer …`) and `password=…`-style values. Git SHAs and UUIDs are left alone. `forge bench swe` also writes `<out>.integrity.jsonl`. An attempt scores 0 if it adds skip/xfail markers, edits test collection, deletes asserts, special-cases FAIL_TO_PASS test names, reads grader data, or reads git history past the base commit. The last two are checked from the Forge agent's recorded tool calls, including turns that compaction folded away. Touching test files is only a warning. The new `forge bench integrity --predictions … --dataset …` command runs the patch checks on any harness's predictions. An optional `--judge <model>` can mark an attempt as a fail but never clear one.
- **Sessions now keep an evidence ledger, and `/pr` lists the proof.** Browser and device screenshots are saved to `.forge/evidence/<session>/` with numbered names and an optional `caption`, so a new shot no longer overwrites the previous one. Shell checks that pass (test, lint, typecheck or build commands that exit 0) are saved with the command, exit status and the last 40 lines of output. Every capture adds a line to `manifest.jsonl`: timestamp, kind, file, caption, page URL without its query string, and sha256. `/pr` adds a fixed "Evidence (captured, not reviewed)" section before the provenance block. Forge builds this section from the manifest; the model never writes it, and values of env assignments such as `API_KEY=…` in commands are masked. Evidence is recorded on both the direct path and the CLI bridge, and git ignores the directory.
- **Preview URLs for dev servers the agent starts.** `shell_job {"action":"expose","id":<job>,"port":5173}` opens a cloudflared/ngrok quick tunnel in front of a running background job and returns the public `https://` URL. The tunnel stops when:
  - the job stops or exits;
  - `[preview] ttl_minutes` (default 120) runs out, enforced by `timeout` where it is on PATH so it holds even if Forge crashes;
  - `forge serve` startup or `forge doctor` finds it orphaned or expired.

  Forge always asks before exposing a port, whatever the mode or allow rules. The prompt names the port and job, and "always" counts only once. Read-only/plan mode, headless runs and `forge mcp-agent` refuse it. Turn it off with `[preview] enabled = false`. The tunnel code `/remote` uses now lives in `forge_tools::tunnel` and is shared with this feature.
- **Lean direct-path tool surface with on-demand tools.** By default the model sees the core coding tools plus `tool_search`/`tool_call`. Platform tools (browser, device, web, heartbeats, fleet and others) and connected MCP server tools (`server__tool`) load on demand. With subagents on and no skills, lattice or MCP, the tool block goes from about 42K to 17K characters per request. A `tool_call` counts as the tool it runs everywhere: permissions (Plan mode still denies writes), hooks, audit, the phantom-edit gate, the verification ledger and the loop guards. Calling a deferred tool by name still works, and `mcp_search_tools`/`mcp_call` remain as aliases. To turn it off, set `[mesh] direct_lean = false` or `FORGE_DIRECT_LEAN=0`. To keep specific deferred tools advertised, use `[mesh] lean_core_extra`. The verification ledger now also counts `multi_edit` and `notebook_edit` as mutations. The bridge's `tool_call` accepts the same argument forms as the direct path (an object, a JSON string, or `args`/`input`).
- **Codemode (`run_code`)**: direct-path models can now write a short JavaScript program that calls Forge's tools as `await tools.<name>({...})` and returns only what it needs, so one step can search, read, edit and test.
  - Every inner call goes through the normal permission broker, hooks, /undo snapshots and audit, and counts toward the phantom-edit gate, the verification ledger and the loop guards exactly like a plain call.
  - Only the script's result enters the context.
  - The deadline is checked only between calls, so a call in flight is never cancelled, and time spent waiting on a permission prompt does not count.
  - On by default; set `[codemode] enabled = false` to turn it off. CLI bridges never see it.
  - `forge stats codemode` reports the script error rate and the output tokens per `run_code` step. See docs/features/codemode.md.
- **`open_pr` tool and a one-line `/pr`**: Forge can now open a GitHub pull request in one deterministic step. First it refuses a dirty tree, a detached HEAD, the base branch, an archived repo, or a repo where `gh` reports less than write access. It never force-pushes. If the branch already has an open PR from the same repository, it pushes to that PR. Otherwise it runs `gh pr create` with the model's title and body, followed by a generated Evidence section and a Provenance section, and secrets are redacted from both. The Evidence section lists passing checks, inline screenshots and live preview URLs. The Provenance section gives the session, models, spend and diff stat. Screenshots are committed with git plumbing only to an orphan `forge-evidence` branch, and only intact captures from the session's evidence directory are published. Set `[pr] publish_evidence_images = false` in the user config or in a repo's `.forge/config.toml` to keep them local. Preview URLs from `shell_job` expose are now recorded as evidence. The tool asks for permission before it runs, except in bypass mode, and is denied in plan mode, on both the direct path and the CLI bridge.
- **Trajectory export and generic task suites.** `forge bench tasks --suite <dir>` runs Terminal-Bench-style suites, where each task directory holds instruction.md, an optional setup.sh and test.sh, and test.sh exiting 0 means solved.
  - Every attempt runs in a fresh temp workspace that has a git baseline commit.
  - Script process groups are killed when the attempt ends.
  - It writes results, a `.metrics.jsonl` and a `.eval.json`, which `forge bench report --run` reads.
  - `forge bench export --metrics <f> --out <dir>` writes one redacted trajectory JSONL per attempt: an attempt header with outcome, integrity verdict and usage, then one line per transcript step. Every field is capped at 8 KB.
  - `forge run --usage-json <path>` writes the run's tokens (including cache splits), calls, cost, notional cost and wall time. With `--resume` it counts this invocation only, and it always runs locally.
  - Redaction now masks JSON string values stored under secret-named keys (for example `{"api_key": "..."}`) in tool-call arguments, which covers replay, export and PR evidence.
- **Measured cost per solve now feeds routing.** `forge bench report --run m.jsonl=e.json --record` stores each scored Forge run that is pinned to one `provider::model`. The run must be scored by its own eval and its cost must be captured; runs that don't qualify are skipped with the reason. Once a model has 20 scored attempts and a second model has been measured, the mesh adjusts its score by −0.75 × log2(cost per solve / median), capped at ±1.25. `forge mesh explain` shows the adjustment (`· $/solve +x`, one line per measured candidate, and a `solve_cost` JSON object), and the `/mesh` overlay adds a note to its rationale.
- **Adaptive reasoning effort** (`[mesh] adaptive_effort`, default on). After a tool round where every call was read-only and succeeded, the next request goes out at low effort. The configured effort comes back after a failure, a write, a step with no tool call, a steer or a new user turn. It only lowers once a response in the turn has reported reasoning tokens, and it never overrides `/effort exact`. It only applies to providers listed in `[mesh] adaptive_effort_providers` (default: openai, codex-oauth and gemini, where an effort change keeps the prompt cache). Anthropic, Bedrock, OpenRouter, open-weight hosts and the CLI bridges are off by default. See docs/features/adaptive-effort.md.
- **`[mesh.context_windows]` sets a model's context window in config.** A custom
  OpenAI-compatible gateway whose `/models` lists no window, serving a renamed model
  (`glm-5.3-abliterated`) that matches nothing published elsewhere, fell to the 32k conservative
  floor, so a million-token model trimmed and compacted long turns for no reason. A configured
  entry, keyed by full model id, now wins over every fetched or inferred window, and discovery
  writes it into the store so the gauge and pickers agree with routing.
- **`forge switch` moves one conversation between Claude Code and Forge, either way, as often as
  you like, with nothing duplicated or lost.** Leave either tool, run `forge switch` in the same
  directory, and the conversation reopens in the other (`claude --resume` or `forge chat --resume`
  replace the shell's process): one line says what moves, e.g. `Claude Code "fix the parser" (412
  messages, active 3m ago) → Forge`. `forge switch <id|path> [--to claude|forge] [--dry-run]` names a
  session explicitly; `/claude` in the Forge TUI leaves it cleanly and switches the session you are
  in. A link per conversation (store migration 40, `cc_session_link`) records how far each side has
  been mirrored, so a later hop reads and writes only the new turns (a 400 MB transcript costs
  milliseconds) and a second hop with nothing new changes nothing. Forge turns reach Claude Code as
  real transcript entries (tool calls mapped to `Bash`/`Read`/`Edit`/..., Forge-only tools as
  `mcp__forge__*`, one result per call, compaction as a `compact_boundary`); Claude Code turns reach
  Forge through the `forge import claude-session` conversion, which now records the same link so an
  imported session can switch back. The permission mode and Claude model carry over. If both sides
  changed, or Claude Code rewound or compacted past what Forge holds, it says so in one line and
  keeps every turn (a rewind re-imports as a new Forge session). Refuses (`--force` overrides) while
  the session is busy in the daemon or open in a running `claude`. Forge's own claude-cli bridge
  transcripts are never taken for your Claude Code session. See `docs/features/session-switch.md`.
- **Sessions can find and message each other: `list_sessions` + `message_session`.** `list_sessions`
  shows the live sessions on this machine (daemon-hosted and `forge chat` terminals) with id prefix,
  name, directory, model, busy/idle/waiting and which one is you; with Forge Anywhere on it also
  lists the account's other hosts by name (an unreachable host shows as offline with its last-seen
  time). `message_session` now resolves names, id prefixes and `host/name`, works from a terminal
  session and the claude/codex bridge, and can deliver into another machine over the end-to-end
  encrypted relay (new typed `SessionMessage` bridge route; no plaintext at the hosted service). The
  recipient sees a labelled peer message ahead of the text (never a user instruction, no permission
  change); an idle one wakes to handle it. Guards: no self-send, 10 messages per minute per pair,
  and a conversation-depth guard that tells the recipient not to auto-reply from the 6th consecutive
  agent message and refuses past the 12th. `message_session` is refused in Plan mode, like
  `push_notification`. `forge send` uses the same routes. See `docs/features/session-messaging.md`.
- **The iPhone lock screen shows a `push_notification`'s real title and message, even with the app
  killed, without the relay or Apple being able to read it.** The app now ships a Notification
  Service Extension (`dev.adulari.forge.notification-service`). The phone generates a P-256 key kept
  in a keychain group shared with the extension and registers only the public half with its daemon
  (store migration 39). The daemon seals the notice to that key (RFC 8291) and sends the ciphertext as
  an opaque `enc` field beside the usual generic alert; the hosted relay validates its size and
  shape, forwards it, and cannot open it. Any failure, a device without a key, or the app lock being
  on leaves the generic alert. Needs a new native build (changes the OTA fingerprint); existing
  installs register their key on next open. On Android a fetched notice now raises a local
  notification (there is no remote push there). ADR-0012 gains an addendum.
- **`push_notification` tool: the agent can notify your phone and desktop.** Available on the direct
  path and the claude-cli/codex bridge (deferred behind `tool_search` on a lean bridge). Fans out to
  iOS native push, Forge Anywhere, web push and a desktop notification, reports which delivered,
  and degrades to a "no channel available" result. Hosted channels never carry the text (generic
  alert; the real text is served end-to-end encrypted from the new `GET /api/notices`). Limited to 4
  per minute and 20 per hour per session with identical messages suppressed; refused in Plan mode.
  See `docs/features/push-notification-tool.md`.
- **App: notices from `push_notification` show their real text.** The hosted push can only say
  "Open Forge to view an update", so the app (iOS, Android, web, desktop) now reads the daemon's
  `/api/notices` inbox over the direct or Anywhere transport on launch, on return to the foreground,
  when a push arrives or is tapped, and every 30 s while a host is connected. New notices raise an
  in-app banner (tap opens the session) and fill a per-host Notices list under Inbox; only a
  per-host cursor and read marker are stored, never the text. Web and desktop raise an OS
  notification with the text when the window is in the background; iOS raises a local one only if
  the app is still running, and keeps the generic text off the lock screen while the app lock is on.
  JS-only, ships over the air.
- **`forge import claude-session <id|path>` moves a Claude Code session into Forge.** The active
  chain of the transcript (streamed, so multi-hundred-MB files import in seconds) becomes a new
  session with its tool calls and results; Claude Code's latest compact summary plus what followed
  it is the model context, older turns stay in history and replay (`/uncompact` brings them back),
  and the session is pinned to the transcript's model on the claude-cli bridge. Prints the new
  session id on stdout; continue with `forge chat --resume <id>`. A resumed session now also
  restores its stored model pin unless `--model` is given.

### Fixed
- **Long tool-only stretches now show progress.** A model could run hundreds of distinct shell
  commands without a word, and no loop guard could fire (they key on repeated arguments, failures,
  unseen output or repeated wording). A silence guard counts tool steps since the last visible text:
  every 12 (or after 3 minutes) it asks the model for a one-line progress note alongside its next
  tool calls and shows a harness-written `⋯ 12 steps since the last update: …` line, so progress is
  visible even if the model ignores the ask. It never halts a turn.
- **Shell work takes fewer round trips.** `shell` accepts `commands: [...]` (up to 20, run in order,
  one section and exit code per entry, stop at the first failure unless `stop_on_error: false`), and
  several provably read-only shell calls in one response now run concurrently instead of one by one.
  Permissions judge every batch entry like a line of one command, and a failed batch gets one
  diagnosis for its failing entry only.
- **The statusline updates live during a turn.** Spend, token totals, per-turn token deltas,
  throughput and the context gauge only refreshed when a turn ended; they now update after every
  model call.
- **`/btw` answers in an overlay card, like Claude Code.** The answer used to be printed into the
  scrollback between real turns. It now opens in a card over the live view (Esc, Enter or Space
  close it, arrows scroll), never enters the scrollback, reaches remote clients as an overlay, and
  no longer takes over the turn spinner when asked while idle.
- **`/btw` works while a turn is running and sees the conversation, like Claude Code's.** It took
  the session lock the running turn holds, so a side question waited for the whole turn to end, and
  it saw only the question itself. Each turn start now registers a detached asker built from the
  session's shared parts; a `/btw` sent while busy runs beside the turn without touching it, and
  every `/btw` carries a bounded excerpt of the session read from the store (recent messages, tool
  names, shortened tool output, the compaction summary). Still never written to the transcript.
- **The file tools no longer refuse paths outside the session workspace, so models stop writing
  files through `shell`.** `write_file`, `apply_patch`, `read_file`, `list_dir` and the other
  structured file tools rejected any path outside the workspace (plus `[tools] extra_roots`) in
  every mode, Full included, while `shell` could reach the same path unconfined. A session asked
  to write a doc into a sibling repo, read Claude Code's memory files, or list a parent directory
  hit "path escapes session workspace" and fell back to `cat >` and heredocs. Outside paths now go
  through the permission broker: Full allows, Ask/Auto/Auto-edit ask, Read-only denies, and the
  secret denylist still applies in every mode. A write into another agent's Forge-managed worktree
  stays refused, and main-checkout paths in a linked worktree are still redirected into it.
  `[tools] confine_to_workspace = true` restores the old confinement.
- **An endpoint's small default output cap no longer cuts large file writes off mid-call.** With
  no `[mesh] max_output_tokens`, Forge sent no `max_tokens`, and some OpenAI-compatible endpoints
  then default to 1,024 output tokens (Refuseless does), so every large `write_file`/`append_file`
  ended with "the model's native output limit interrupted this tool call". When a reply stops at
  the limit within 8,192 tokens and no cap was sent, Forge now sends `max_tokens = 32768` for that
  model for the rest of the process and tells the model to retry the same call unchanged. It is
  learned only after such a cut, so endpoints that reject large `max_tokens` values are never sent
  one up front.
- **Forge Anywhere sync keeps up with an active host and a late-joining device.** Each cycle
  uploaded one batch of ten journal entries and fetched one feed page of a hundred objects one at
  a time, about a minute in all, while a host with a busy session journals ~14 records a minute:
  its 9,000-entry backlog only grew, and a device reading the feed from the start needed ~35 hours
  to catch up. Uploads, feed pages and staged applies now each drain for up to 20 seconds per
  cycle, and a page's objects download eight at a time (still staged in feed order).
- **Forge Anywhere sync no longer wedges on a change whose encrypted object is gone.** A device that
  joined later reads the change feed from the start, and one early entry whose R2 object no longer
  exists answered 404 on every poll: the pass failed before advancing the cursor, so that device
  never downloaded anything, and the failure backoff starved its uploads to ten entries per five
  minutes (8,353 journal entries pending on one host). A 404/410 download is now skipped with a
  warning and the feed moves on; other failures still retry.
- **A headless `forge serve` could not see the `claude` CLI, and later runs warned about a valid pin.**
  A `systemd --user` unit on a server has an empty environment, so the daemon's `PATH` lacked
  `~/.local/bin`: the `claude` bridge counted as not installed, discovery cached a catalog with no
  `claude-cli::` models, and `forge chat --model claude-cli::claude-opus-5-5` then warned "unknown
  model". Also, agent shell tools in daemon sessions ran with that bare `PATH` (no `cargo`, `node`).
  `forge serve` now merges the login shell's `PATH` in at startup (3 s timeout; skipped on a TTY or
  with `FORGE_NO_LOGIN_PATH=1`); bridge CLIs are also looked up in `~/.local/bin`, `~/.claude/local`,
  `~/.npm-global/bin`, `~/.bun/bin`, `/opt/homebrew/bin` and `/usr/local/bin`; and a pin on a
  routable bridge is no longer judged by the cached catalog, which is refreshed when it lacks that
  bridge. A typo'd bridge alias still warns.
- **A toolless "Implemented ... / Added ..." summary was accepted as a finished edit (#1428).** The
  phantom-edit gate only recognised first-person phrases, and "Modify ..." / "Replace ..." were not
  treated as change directives, so a turn that made zero tool calls ended as success. Change
  directives now include modify/replace/edit/delete/rewrite/extend/patch/insert, and a no-tool
  reply that reports past-tense changes to a file or `identifier` is nudged (bounded) before the
  task-completion gate, so stale all-Done tasks cannot mask it. Questions, plans, read-only and
  explicit no-change answers are not nudged.
- **`forge import claude` is idempotent.** Re-running it appended a full second copy of every permission
  rule whenever the importer's marker comments had been lost (any tool that re-serializes
  `config.toml` drops them), and left the config rewritten each time. A rule an equivalent entry
  already provides (same tool and allow/deny/ask pattern, any key order) is now skipped and reported
  as "already present", and the file is untouched when nothing changes. MCP servers whose launch
  command does not exist here (a missing absolute path, or a bare name not on `PATH`) are skipped
  with the reason instead of being imported as servers that can never start.
- **Compaction ignored `mesh.disabled`.** The summarizer chain skipped the disabled-model and
  missing-key check the main loop applies, so a disabled provider still received the session's
  transcript to summarize. The chain now honors both; the session's own model is always kept.
- **Long Claude bridge sessions never compacted.** claude's own window held ~650k tokens per request
  while Forge counted ~18k, because the bridge's tool results never enter Forge's transcript. The
  bridge's real per-request context now drives the gauge and auto-compaction.
- **A turn that committed its work was nudged "You have not modified any files".** Progress compared
  `git status` only, which is identical before and after a commit; it now includes HEAD. One 40-turn
  claude session was re-driven 23 times for work it had already done.
- **Repeated end-of-turn nudges vanished from the request.** The review pass and the completion gate
  use the same text every turn, and system-context deduplication dropped every copy after the first,
  so the model was re-driven with nothing to act on and answered "nothing new is requested" as the
  turn's final reply. A live bridge process is also no longer re-sent the whole transcript when there
  is nothing new to say.

## [2.18.0] - 2026-10-10

### Added
- **Multi-file `multi_edit` and batched ranged reads** (#1497). One `multi_edit` call edits several
  files; `read_file` batches take line ranges (`src/a.rs:120-180`); `search` gains
  `case_insensitive`, `output_mode`, `head_limit` and Claude Code aliases; edits return a numbered
  snippet. On a 6-task tool benchmark Forge (haiku) now beats `claude -p` haiku: 34 vs 40 tool
  calls, 0 vs 3 failed calls, 82s vs 108s, both 6/6.
- **Spend guards** (#1489). Per-turn and per-session dollar limits in `[mesh.budget]`
  (`turn_warn_usd` 5, `turn_cap_usd` 25, `session_warn_usd` 25, `session_cap_usd` 100; 0 disables,
  `FORGE_BUDGET_OVERRIDE=1` bypasses). Side calls (compaction, memory, recap, suggest, diagnose,
  refine) are now priced instead of recorded as free.
- **Shared MCP connections in the daemon** (#1492). One connection per HTTP/SSE server definition
  for all sessions; stdio servers stay per session unless `shared = true`.

### Changed
- **Lean bridge requests** (#1496). Core tools are advertised; the rest sit behind `tool_search`
  (`FORGE_BRIDGE_LEAN=0` opts out). The completeness review runs only for multi-requirement
  requests, large/multi-file diffs or hedging answers; no orchestration message on simple prompts,
  no Lattice context when the prompt names the file, no duplicate AGENTS.md. On 8 everyday tasks
  Forge claude-cli sonnet went from 1.79x to 1.08x `claude -p`'s tokens and 1.23x to 1.08x its
  wall time, 8/8 passing.
- **Responsive TUI while replies stream** (#1492). Finished blocks render once and only the open
  block re-renders; keystrokes echo without the 16 ms sleep (echo p95 63 ms → 8 ms with a 38k-message
  session open). `FORGE_TUI_PERF=<file>` writes a render/echo histogram.
- **Daemon memory** (#1494). glibc arenas capped at 2 with `malloc_trim` after each turn
  (`FORGE_ALLOC_POLICY=off` disables), and the remote replay ring is bounded to 8 MiB.

### Fixed
- **Tools in worktrees outside the session directory** (#1497). Absolute main-checkout paths are
  remapped into the active worktree on the direct path, the bridge and isolated subagents; reads
  fall back to the main checkout for files only it has; `git commit` works under the shell
  sandbox; the bridge pins `mcp-serve` to the session workspace.
- **Code-block copy** (#1488). Mouse selection copies only the code: borders, gutters, quote bars
  and padding are excluded, wrapped lines rejoin, and code containing box characters survives.
- **Long claude-cli sessions never compacted** (#1493). Forge now reads the bridge's own context
  usage; a committed turn no longer triggers the false "you have not modified any files" nudge;
  repeated end-of-turn nudges are no longer dropped (which caused "nothing new" replies and full
  transcript re-sends); the compaction summarizer honours `mesh.disabled`.
- **Real-prompt replay findings** (#1490). Mid-turn narration no longer glued onto the final answer;
  a verification re-drive no longer replaces the summary; `write_file`/`append_file` create missing
  parent directories; `.forge/` ignores itself so it no longer shows in `git status`; "nothing left"
  answers are not re-driven; the review nudge sees new files.

## [2.17.2] - 2026-10-10

### Fixed
- **`apply_patch` rejected Codex-format patches** (#1486). 59% of historical `apply_patch` failures
  were `*** Begin Patch` envelopes that `git apply` refused outright. They are now applied natively,
  each hunk through the same tolerant matcher `edit_file` uses; failing unified diffs get one
  whitespace-insensitive retry. Not-found edit errors now name the closest matching line, and
  `edit_file`/`multi_edit` accept `replace_all`.
- **Startup discovery noise and rediscovery loop** (#1485). Bridge providers no longer count as keyed
  providers in the discovery retry loop (one slow `agy models` re-ran the whole sweep every 10
  minutes for the daemon's lifetime); retries stop after three rounds. A provider that fails a round
  keeps its last-known model list for up to 7 days instead of leaving the cache stale. `agy models`
  gets a 30s budget and an hour-long cache. Transient MCP connect failures log one concise line.
- **iOS OTA dispatch in the release workflow** (#1484): the hosted job now has its repository context.

## [2.17.1] - 2026-10-10

### Added
- **Model-assisted auto mode** (#1478, #1480). Auto mode now sorts each shell command into safe,
  risky or unknown. Unknown commands (project scripts, `make` targets, unfamiliar binaries) get one
  cheap model check that fails closed: a timeout, error or unusable reply asks you instead. The
  model can never override a risky verdict or a deny rule. Verdicts are cached per session, and the
  check also runs on the Claude CLI bridge. Turn it off with `[permissions] auto_classifier = false`
  (unknown commands then always ask).

### Changed
- **Faster releases** (#1477). Stable cache keys for the hosted Windows/macOS builds plus a
  keepalive workflow so caches stop expiring between releases, a persistent build slot for the
  assets job, no per-run toolchain reinstall in the Linux containers, preflight and OTA moved to
  hosted runners, and parallel packaging.

### Fixed
- **Completeness review replaced the answer after shell-only turns** (#1481). The review now runs
  only when the git working tree changed during the turn, and the original answer is kept unless
  the review itself changed the tree.

## [2.17.0] - 2026-10-09

### Added
- **Auto permission mode** (#1463). `--mode auto`, `permission_mode = "auto"`, the `/mode` picker and
  the SHIFT+TAB cycle now include a mode that proceeds unless a command looks risky and asks
  otherwise. `permission/auto.rs` classifies each shell segment (recursive/forced deletes, hard
  resets and force-pushes, destructive SQL, `dd`/`mkfs`, `sudo`, publishing, uploads, pipe-to-shell,
  writes outside the workspace, credential paths). The built-in deny rules remain the floor.
- **`schedule_wakeup`, agent types and a commit policy** (#1463), matching Claude Code.
- **Statusline command, notifications and reply language** (#1460). `[statusline] command` runs a
  Claude Code style script (session JSON on stdin, first stdout line shown as an extra row);
  `forge import claude` copies `statusLine.command`. `[notifications]` (off by default) fires
  `notify-send`/`osascript` and a bell when a turn ends after `min_turn_secs` or a permission or
  `ask_user` prompt appears while the terminal is unfocused. The system prompt now asks the model to
  answer in the language of the user's latest message.
- **Attach the browser tool to your own logged-in browser** (#1466). `[browser] attach` / `cdp_url` /
  `FORGE_BROWSER_CDP` connect over CDP, open Forge's own tab and never list, adopt or close yours.
  `forge browser attach` launches Chrome, Chromium, Brave or Edge on a dedicated persistent profile.
- **Claude Code hook parity** (#1452). Stop and SubagentStop hooks get `last_assistant_message` and
  honour `continue:false` and `additionalContext`; SessionStart output reaches the model;
  PreToolUse `updatedInput` is applied; tool payloads carry `session_id` and Claude Code tool names
  (so `rtk hook claude` matches); finished background jobs wake the session.
- **`forge run --output-format json`** (#1456). Usage and cost in the result are per invocation, also
  with `--continue` (#1462).
- **Plan mode can run provably read-only shell commands** (#1468) from a strict allowlist (every
  segment known, no write flags, no redirects other than `/dev/null`, no assignments or command
  substitution writers), so it can list and measure files instead of looping.
- **Persistent Codex reasoning** (#1447, #1459). The Codex (ChatGPT OAuth) provider requests and
  replays encrypted reasoning items, and they are now stored (migration 38) so resume and daemon
  restarts keep the reasoning chain and prompt-cache prefix.

### Fixed
- **Security: permissions no longer leak in from an ancestor config** (#1464). The project config was
  loaded with a search through every parent directory, so a stray `~/.forge/config.toml` full of old
  `allow = "*"` rules applied to every directory under `$HOME`, auto-approving shell commands in
  Ask, Auto and Auto-edit modes. The project file is now read from the exact path only. A project's
  `[[permissions.rules]]` no longer replace the user's rules (including denies imported from Claude
  Code), and the TUI's "always allow" now persists a rule scoped to the command instead of the
  whole tool.
- **claude-cli bridge tools were invisible to Claude Code 2.1.29x** (#1461). `tools/list` omitted
  `ttlMs` and `cacheScope`, which claude 2.1.295 rejects, so the model saw no Forge tools. Both are
  now set; edit-and-test and refactor tasks ran about twice as fast in A/B runs.
- **Codex OAuth** (#1447): reasoning items round-trip, a rejected WebSocket token refreshes and
  retries, refresh is serialised across processes with a file lock, and `response.incomplete` is
  handled.
- **Loop guards** (#1445, #1448, #1465, #1457). Turns that end on an announced but undone action are
  nudged (up to twice per idle stretch); repeated-opening detection counts only steps that made no
  progress, so productive edit runs are no longer halted and real loops are caught sooner; a
  successful edit clears failure-loop streaks, ending false "failed the same way" nudges; side calls
  that fail are benched so the same doomed call is not repeated.
- **`read_file` honours Claude Code's `offset`, `limit` and `file_path`** (#1449); paging no longer
  returns the whole file.
- **Provider requests** (#1454): continuation nudges stay in place instead of being hoisted to the
  front, which had dropped prompt-cache reads (Bedrock Kimi 98% to 9%); blank replies are retried;
  DeepSeek DSML tool calls are recovered and their markup hidden from the live stream (#1459).
- **Compaction** (#1453): large tool results are clipped when the tail alone fills the budget, a
  deterministic digest is used when every summariser fails, double-counted usage is fixed, and side
  calls record their model.
- **Daemon startup** (#1451): model discovery retries when DNS is not ready, dead sessions with a
  deleted workspace stop being restored on every start, and a dead model no longer holds a first
  event for the full idle window.
- **Final answers** (#1468, #1473): a turn halted by a loop guard ends with a real answer instead of
  an empty result, and the completeness review runs only after edits and never replaces the answer
  to a question.
- **Bridge** (#1446): one long-lived `claude` process per conversation with `--resume` on respawn,
  instead of a cold respawn and full-transcript flatten every turn.

### Changed
- **Faster session open and startup** (#1450, #1458). Migration 37 indexes the cascade-delete
  children, so the first run on a 2.7 GB store dropped from 4m32s to about 6s; `mesh`, `models`,
  `benchmarks` and `doctor` no longer repeat discovery; startup catalog refresh is skipped when the
  cache is under an hour old.
- **Large-session resume** (#1467): the daemon replays only the transcript tail, the last model is
  one indexed lookup, and outcome calibration is a per-model probe.
- **Fewer wasted side calls** (#1456): unrendered suggestions and probes of dead local servers are
  skipped.

## [2.16.2] - 2026-09-30

### Added
- **`[mesh] min_tier`** — the lowest tier a turn may be routed at (`"standard"` or `"complex"`). A
  turn classified below it is routed as the floor tier; an explicit tier hint, an in-session tier
  pin, or a pinned model keep their own choice (a pinned model still takes the floor's tier, and so
  its tool surface). For embedders whose every question needs tools: a `trivial` turn went to the
  cheapest models with no tools unless its prompt named a workspace action, so "how many habits do
  I have" was answered "I'll check your habits now" with nothing to call.

## [2.16.1] - 2026-09-29

### Fixed
- **Claude models the picker names only by alias now appear by id** (#1438). Claude Code 2.1.284
  advertises the newest Opus and Sonnet only as `opus`/`sonnet`/`default`, resolving to
  `claude-opus-5-5` and `claude-sonnet-5-5`; discovery kept the aliases and dropped the ids, so
  neither reached `forge models` or `GET /v1/models`. Each entry's resolved id is now advertised too.
- **Benchmark scores for new models, and for every model without an AA coding index.** Artificial
  Analysis publishes no coding index for 406 of its 665 rows, including Opus 5.5, GPT-6 Sol and
  GPT-6 Luna. Forge copied the intelligence index into the gap, and the two indices sit on
  different scales, so those models ranked about 20 points low on code-heavy work (GPT-6 Sol read
  47.5 against GPT-5.6 Sol's 77.4). The gap is now estimated from rated models of similar
  intelligence and capped at the best measured coding index. The estimate is 4.7 points off on
  average; copying intelligence was 21.3 points off. A cache holding estimates is re-fetched daily,
  so a published coding index replaces the estimate. Separately, Opus 5 and Opus 5.5 shared one
  matching key: Opus 5 was scored as 5.5 and their per-effort ladders were merged.

## [2.16.0] - 2026-09-23

### Added
- **Amazon Bedrock runs Kimi K3 for a whole session** (#1429–#1432). `forge auth bedrock` stores a
  Bedrock API key; `[providers.bedrock] models = ["global.moonshotai.kimi-k3"]` puts the model in
  every picker (Bedrock cannot be listed with an API key, so the configured ids are the catalog).
  The first live session found nine defects, each confirmed against Bedrock itself: Converse rejects
  a `cachePoint` for models outside its caching list, and a `temperature` or `topP` at any value for
  Moonshot models; the stream stopped one frame before the token counts, so every Bedrock reply
  (Claude included) recorded 0 tokens and $0; `inputTokens` excludes cached tokens, so a nearly
  full context read as empty; parallel tool results were split across messages and an unanswered
  call failed every later request; `Inference Profile ARN not found` is a routing blip, now retried;
  and a Bedrock id matched no published window, so a 1M-token model was trimmed to 32k. Effort is
  not offered there: Bedrock accepts `reasoning_effort` for Kimi and ignores it.
  (vendor/genai-0.6.5/src/adapter/adapters/bedrock/, crates/forge-provider/src/cache_hints.rs)
- **Kimi Code as a subscription provider** (#1374): `forge auth kimi`, K3 and Kimi for Coding over
  Kimi's coding endpoint, with its own low/high/max effort ladder.
- **Per-provider tuning in config** (#1376, #1377, #1380): `[system_prompt_overrides]` swaps the base
  prompt for one provider (`@path` loads a file); `[reasoning_prefill]` seeds a provider's thinking;
  `[model_effort]` and `/effort exact <level>` send one rung verbatim instead of treating it as a
  ceiling; `[compact_cap]` sets an auto-compaction ceiling per model or provider (Kimi Code ships at
  120K). `[mesh.pricing]` entries take an optional `cache_read_per_1k`.
- **Large whole-file reads return an outline** (#1388) instead of flooding the context, and a reply the
  provider cut for repeating itself is re-sampled instead of accepted.
- **A bounded working set inside a turn** (#1387), and search results that show every matching file.
- **The model is told the current time** (#1368).
- **Bridged claude/codex turns ask before a permissioned tool** (#1407) instead of refusing it outright;
  the question reaches the TUI and the phone.
- **Sessions report whether their model is pinned** (#1419), and Automatic sessions are labelled auto.
- **Full tool output viewer, and the live repo and branch in the statusbar** (#1378).

### Fixed
- **A session mid-task keeps its tools for "continue"** (#1432). A bare "continue" or "yes" classified
  as a trivial chat reply, which advertised no tools, so every nudge re-sent a tool-less request and
  the model could never call a tool again.
- **Forge Anywhere lockout** (#1426). A refresh token the service had revoked still counted as signed
  in, so `forge anywhere setup` skipped sign-in, failed, and advised running `setup`. A rejected token
  is now cleared and the error names the recovery routes; the approval challenge prints as a QR code.
- **The phone can approve a new device** (#1425): the approval screen ran on the mock client and never
  called the service.
- **Compaction and long turns** (#1379, #1381, #1382, #1385, #1386): echoed reasoning counts toward the
  transcript estimate; an empty summary is never committed; fitting never orphans a tool result; the
  last tool rounds stay whole when pruning mid-turn; one ceiling applies to any tool result.
- **Stalls and loops** (#1383, #1384, #1395, #1398, #1415): a model is not written off after one idle
  answer; the request after a stall nudge must call a tool; interleaved narration loops are caught;
  Forge stops manufacturing repetition loops it used to nudge into; an answer given before an
  open-task re-drive is kept.
- **Long reasoning streams** (#1364): the 90s inner idle timeout no longer kills them.
- **Forge Anywhere relay** (#1365, #1389, #1397, #1399): one oversized record no longer jams the sync
  outbox; relay bodies are UTF-8 and history stays live during long turns; the blob token is resolved
  per request; Files work over the relay, and slow requests no longer stall the connector.
- **Store** (#1403): opening it no longer takes the write lock for the sync flag.
- **Companion app, from a full Android retest** (#1393–#1396, #1400–#1402, #1404–#1406, #1408,
  #1410–#1414, #1416–#1418, #1420–#1424): session creation no longer blanks the screen; lists stop
  spinning on every poll; the first tap while typing lands; a lost terminal attach retries; sessions
  survive daemon restarts and open from inside another session; plans are approved where they can be
  read; replay links decrypt the real replay; Remote jobs and Encrypted storage show real data;
  commands and code render without font ligatures.
- **Other fixes**: the statusline follows a session into a resumed workspace (#1392); no warning for a
  session started in the home directory (#1409); the task list is no longer deleted mid-work (#1366);
  iOS prebuild works again with the xmldom override bounded (#1371); a tools history page is sized for
  the relay (#1369).
- **Security**: rustls 0.23.45 for RUSTSEC-2026-0285 (#1390).

### Added
- **Plan and dispatch — split one prompt into parallel sessions from the board.**
  `forge dispatch start`, `/dispatch` in chat, or `D` on `forge board` sends a project to a
  coordinator session that reads it and proposes a split into work items with the
  `dispatch_sessions` tool, then stops for review. Approve all of it, a subset, ask for a
  revision, or cancel; approved items run as ordinary sessions — a worktree apiece by default —
  up to `max_running` at a time, held back while their dependencies are still running. The
  daemon tells the coordinator each time an item finishes and asks it for a final summary once
  everything is done; worker worktrees are merged back or discarded explicitly. See
  docs/features/plan-and-dispatch.md.
- **A live project board in the CLI — `forge board`.** A full-screen kanban view of every session
  on a project — Needs you · Working · Ready · Done — so "what needs me" has one answer instead of
  `forge sessions`, `journalctl`, and a handful of tmux panes. Cards show model, current task, last
  line, cost, context fill, and the signals that used to require digging by hand: a pinned model
  Forge has already benched, a turn repeating its own opening sentence, a stalled task, a turn gone
  quiet or silent. Opening a card gets five tabs (Overview/Live/Tasks/Changes/Tools) and the same
  seq-checked answer/allow/interrupt/re-pin/mode/archive actions the remote-control page has, over
  exactly its daemon surface. See docs/features/project-board.md.
- **Stalled tasks stop driving the session.** Forge treats an unfinished task as "the turn is not
  over" and re-drives the model — which loops forever once a task becomes unresolvable (a real
  session lost the context behind one to a compaction and then spent days re-reading files to work
  out what it meant; re-reading counts as progress, so the nudge budget never ran out). A task
  whose title and status have not moved for three turns now gets one system line naming it and the
  ways out — do it, mark it Done, drop it, or `ask_user`; two further unchanged turns and Forge
  removes it from the list itself, telling both the model and the user. When a compaction happened
  while the task was open the escalation says so, because that is why the model cannot reconstruct
  it. Inside a turn, the second and later continue-nudges now name the open tasks instead of
  repeating the same generic instruction. See `docs/features/stalled-tasks.md`.

### Fixed
- **DeepSeek can use tools.** `deepseek::*` models are thinking-mode-only and enforce an echo: every
  assistant message in the transcript must carry `reasoning_content`, or the next request fails with
  `The reasoning_content in the thinking mode must be passed back to the API`. Text-only replies
  worked because they never make a follow-up request; every tool-using turn died the moment the
  first tool result came back. Forge now keeps a reply's reasoning (`Message::reasoning`, live-turn
  only — it is not persisted) and replays it. Two details the live API settled: a reply that calls a
  tool often streams no reasoning at all, and presence of the field is what is checked, so an empty
  value is sent where there is none; and a reply carrying both prose and tool calls becomes two
  assistant messages, both of which need the field. Also fixed in the vendored genai: the streamer's
  tool-call branch consumed the delta without ever reading `reasoning_content`, so exactly the
  replies that needed it lost it.
- **A tool-call id reused across turns no longer 400s the whole request.** Tool ids are only unique
  within a turn (`shell:0` is that turn's first shell call), and `normalize_tool_pairs` deduplicated
  them only *inside* one assistant message — so once a compaction retained two turns' tails, the
  request went out with two `shell:0` tool responses and every turn died on "Duplicate tool response
  for tool_call_id='shell:0'. Each tool_call must have exactly one matching tool response". A live
  session hit this on every retry, including straight after compacting 465 messages to 7. Ids are now
  unique across the whole request: a repeat is renamed (`shell:0#2`) and its call and result are
  renamed in lockstep, so both turns keep their real context instead of one pair being dropped.
- **A dead pinned model no longer looks like a broken session.** A pin overrides routing *and* the
  health table, so Forge kept calling a model it had already benched and said nothing: one session
  spent twelve turns on zero-token empty completions while `forge models` listed that exact id as
  `benched`, and the turn error never mentioned the pin. Now a pinned turn whose model is benched
  opens with a warning naming the model, the reason, the time left and the ways out (`/model`,
  `forge models --probe`); an empty-responding model is benched even when failover is off (it used
  to be skipped on pinned turns, so the health table never learned); and the stopping error names
  the pin and says the conversation resumes once it is cleared.
- **Narration-stall guard.** A 27k-message session spent 23 steps opening every reply with the
  same sentence ("You're right — I looped. Answering the 429 question …") while reading a slightly
  different slice of the same file each time. Neither existing loop guard could see it: the
  identical-call guard needs identical arguments, the failure guard needs failures. The model loop
  now tracks the opening of each tool-calling reply; the fifth near-identical opening in a row gets
  one nudge to answer in text from what it already read, two more end the turn with a clear error
  instead of running to the step cap. (`crates/forge-core/src/stall_guard.rs`)

### Added
- **Steer from the phone.** The companion app's composer no longer turns into a bare Stop
  button while a turn runs: typed text sends as ⚡ *steer* — the daemon queues it and hands it to
  the model at the turn's next step, exactly like typing during a turn in the CLI — with a small
  Stop kept next to it. Queued prompts show as ⚡ chips (tap to dequeue). Ships over the air.

### Added
- **Prompt history survives closing the session.** The composer's ↑/↓ history used to start
  empty every time `forge chat` opened. It is now seeded from the store: the session's own past
  prompts (rewound and compacted ones included — history is what you typed), preceded by the most
  recent prompts from other sessions in the same workspace, up to 500. Harness-written user rows
  (continuation nudges) are skipped; consecutive duplicates collapse.
- **"↑ Previous message" bar.** The full-screen transcript had a jump-to-bottom bar but no way
  back up: a bar at the top (and `Ctrl+Home`) jumps to the most recent user message above the
  view; pressing it again walks to the one before, and so on.

### Added
- **Commit discipline.** A model left alone in a repository edited for days and hardly ever
  committed (one real session: 2,600 edits, 49 commits, 2 pushes, two whole days at zero). Forge
  now carries the habit: the system prompt asks for a focused conventional commit per verified
  unit of work (specific files, never `git add -A`, never push unasked); while files *this
  session* edited are still uncommitted every turn opens with one line naming them; every
  `[git] commit_nudge_edits` (default 10) further edits a mid-turn reminder lands after the tool
  result; and when the branch is `push_nudge_ahead` (default 3) commits ahead of its upstream the
  model is told to ask the user whether to push. Files the user dirtied themselves are never
  mentioned; a commit made outside Forge, a revert or a `/rewind` silences the reminder. New
  `/commit [hint]` runs one turn that groups the uncommitted work into conventional commits and
  reports the hashes without pushing. `[git] commit_nudge = false` turns the reminders off.
  (docs/features/commit-discipline.md)
- **`ask_user` is a form.** One call can carry up to 6 questions (`questions: [...]`), each with
  a short `header`, `options`, `multi_select`, `allow_other` and `allow_note`. The TUI answers
  them in the live region with a tab strip, radio / checkbox rows, an inline "Other…" field and an
  optional note per question (`↑↓`/digits/`Space`/`Enter`, `←→` between questions, `Esc`
  dismisses); the companion app gets a stepper card with the same affordances and sends the whole
  form in one `answer`; headless prompts accept `1,3` on a multi-select and ask for a note. The
  original single-question call shape, the page's numeric answers and plan approval keep working.
  The model reads back one line per question (`Database: Postgres — note: managed`).
  (docs/features/ask-user-question.md)
- **Steer a running turn from the queue.** A prompt typed while Forge is busy was queued and only
  sent after the whole turn finished. It is now handed to the model at the turn's next boundary —
  after a tool step's results, or where the response would otherwise have ended the turn — as a
  persisted user message, echoed in place as `you ⚡ steer` and dropped from the pending list;
  anything still queued when the turn ends starts the next turn as before. Same semantics as
  Claude Code / Codex (docs/features/steering.md).
- **`/rewind` (alias `/undo`) and Esc Esc.** Rewinding now also removes the rewound turns from
  the model's *context*, not just the transcript: the CLI bridge's server-side session is keyed on
  a per-session history epoch so a rewound `claude`/`codex` conversation is never resumed (it
  still held every removed turn — and a rewind followed by a re-prompt often leaves the transcript
  the same length, so the old "did it shrink?" check passed); AGENTS.md / white-hot "already
  injected" latches are re-derived from what survived; a rewind into compacted history undoes the
  compaction first instead of leaving the model with nothing; turn seq, queued hints, pending
  images and steers are reset. Two idle Esc presses open the picker; a bare Esc no longer quits
  (Ctrl-C does) (docs/features/rewind.md).
- **Native RTK and Headroom token savings** (docs/features/token-savings.md). `[shell] rtk = auto`
  routes eligible shell commands (`cargo test/check/build/clippy`, `ls`, `find`, `wc`, `git
  status`, JS test/lint runners) through `rtk` when it is on `PATH`; measured 68–99% fewer tokens on
  cargo output, 70–87% on directory listings, with compile errors preserved in full. Commands whose
  RTK rendering was measured lossy (`grep`/`rg`, `cat`, `git diff/show`) are never rewritten and the
  result header says when a rewrite happened. `[mesh] headroom = auto` routes direct-API calls
  (OpenAI-wire, Anthropic, Gemini adapters) and the `claude` bridge through a running Headroom
  proxy. Default `off`: measured on Forge's own traffic the proxy removed 0.2% of input tokens
  for ~0.4 s added latency per request (Forge already bounds and prunes tool output). The doc also
  records the Headroom bug that made it compress nothing on Codex — Codex sends tool results as a
  list of text parts, which Headroom's extractor did not recognise — with the patch that fixes it
  (`docs/patches/`, 3,478 → 2,556 tokens on one tool result once applied). A session routed
  through the proxy was also observed degrading into empty `{}` tool calls (the proxy rewrites
  prior turns and tool schemas); Forge now warns once, naming the proxy, when a tool call arrives
  with invalid arguments while routing is active.
- **`FORGE_SQL_PROFILE=<file>`** appends one `<micros>\t<thread>\t<sql>` line per executed
  statement on every store connection — the tool that found the startup I/O below.

### Fixed
- **Mobile/remote transcript showed every tool call twice, with a stray "running" line.** The
  expandable tool card rows are laid out for a terminal width (status pushed to the right margin,
  `◍ running` until the result lands); the phone's transcript ring is append-only, so the padded
  running row wrapped into its own "running" line and stayed there next to the finished row.
  Tool rows now enter the ring with padding collapsed and the transient mark dropped.

### Changed
- **`forge chat` opens instantly and resumes big sessions in a fraction of the time.** The terminal
  is drawn first and the session (store, keyring, catalog, skills, MCP) comes up behind a
  `starting session…` spinner instead of a blank screen. The background Lattice update no longer
  starts the moment the session exists: it waits 1.5 s and runs at idle I/O + low CPU priority,
  because on a cold page cache it pulled ~100 MB of a multi-GB store through the same disk the
  first frame was waiting on (3.2 s to first frame cold, measured). Resuming renders only the
  newest 600 messages into scrollback — the log ring holds a few thousand lines, so the rest was
  loaded, markdown-rendered and immediately trimmed again. And the real cost of resuming that
  23k-message session turned out to be one unindexed join: `routing_decision` had no index on
  `message_id`, so the per-session model lookup at resume walked it for every message row —
  5.5 s in a single statement. Migration 34 adds `idx_routing_decision_message`.

## [2.15.0] - 2026-09-08

### Added
- **Expandable tool cards in the chat transcript.** A tool call printed two separate scrollback
  lines — a `↳ name {raw json…}` line truncated mid-JSON and, later, an unrelated-looking
  `✓ name: exit 0 in 132ms` line — and the tool's actual output never reached the screen at all,
  because the presenter only ever received the result's first line. A call is now ONE row that
  carries its own outcome at the right margin; clicking it (or `Ctrl+T` for the most recent,
  rebindable as `toggle_tool_card`) expands it in place into the decoded arguments and the output,
  and clicking again collapses it. `PresenterEvent::ToolResult` and `LiveEvent::ToolResult` gained
  a bounded `detail` field (200 lines / 8000 chars) to carry that output, so cards work in
  `forge attach` and daemon-hosted sessions too. Inline mode (`--inline`) keeps the two-line
  rendering: the terminal's native scrollback cannot be rewritten after the fact
  (`crates/forge-tui/src/app/tool_cards.rs`, `docs/features/tui-tool-cards.md`).
- **`/subagents [free|pinned]` — let one pinned session fan its children out onto other models.**
  Pin inheritance was config-only (`mesh.subagents.inherit_pin`), so releasing subagents from the
  parent's pin meant editing `config.toml` and restarting, and it then applied to every session.
  The command sets a per-session override that `Session::subagents_free` resolves against the
  config default and hands to children as `AgentCtx::inherit_pin` — `route_child` now reads only
  that field, so the runtime command and the config default meet in one place. Bare `/subagents`
  toggles and reports the state. The override is persisted on the session row (migration #33) and
  restored on resume, so a daemon restart mid-goal no longer silently drags every child back onto
  the pin. Default is unchanged: children inherit the pin. A released session stays pinned itself;
  only its children route the full mesh (`crates/forge-tui/src/commands.rs`,
  `crates/forge-core/src/{session_controls,subagent,orchestration}.rs`,
  `crates/forge-cli/src/cli/commands/run/dispatch.rs`).
- **Structured file tools could not touch a path outside the workspace, even to clean up their
  own scratch files.** `validate_workspace_args` hard-rejected any `path`/`cwd`/`paths` resolving
  outside `workspace.root()` for read_file/write_file/edit_file/multi_edit/apply_patch/
  append_file/notebook_edit/delete_file/list_dir/search/glob — but the `shell` tool reaches any
  path via its command string, so an agent doing analysis work against scratch/capture files in
  `/tmp` could write and read them with `shell` but not with the structured tools, forcing a
  `cp into workspace; read; rm` dance that polluted the git tree. Added an opt-in allowlist,
  `tools.extra_roots` (`[tools]` in config.toml), of absolute paths outside the workspace that
  the structured file tools may also read and write; default empty, so existing configs see zero
  behavior change. Honored by both validator copies — `crates/forge-core/src/lib.rs`
  (`validate_workspace_args`, now also consulted from `crates/forge-core/src/tool_dispatch.rs`
  via a new `Session::extra_tool_roots`, populated in `crates/forge-core/src/session_lifecycle.rs`
  from `config.tools.extra_roots`) and `crates/forge-tools/src/workspace.rs`
  (`WorkspaceTool`/`ToolRegistry::bind_extra_roots`, wired at both real binding sites:
  `crates/forge-cli/src/cli/commands/run/session.rs` via `Session::build` and the `mcp-serve`
  CLI-bridge path in `crates/forge-cli/src/mcp_serve.rs`). A deliberately-kept regression test
  (`workspace_validation_rejects_peer_repository_paths`) still asserts that an unlisted sibling
  temp-dir path is rejected; `tools.extra_roots` is separate from `shell.sandbox_writable` (the
  Landlock write-sandbox) and does not change `shell`, which was already unconfined.

### Changed
- **The repository's `.mcp.json` no longer registers Forge as an MCP server for Claude Code.**
  Every Claude Code session in this checkout spawned a full Forge agent (session, index, file
  watcher) whether or not it was ever used; the two runaway processes above were exactly those.
  Add the entry back locally if you want `forge_chat` from Claude Code.

### Fixed
- **A session never picked up an `AGENTS.md` that appeared or changed while it was running.** The
  body was read once at construction and a resume set the "already injected" flag without reading
  at all, so the session that *wrote* the file (or ran `/init`) never saw it, and a long-lived
  daemon session stayed on whatever existed the day it started — restart after restart, because a
  restart is a resume. Each turn now re-checks the file in the post-persist window the git-branch
  refresh already uses (one `stat`; the body is read only when the fingerprint moves) and injects
  it only when the transcript does not already carry that exact text — so an edited or newly
  written `AGENTS.md` reaches the next turn, and an unchanged one is never restated
  (`crates/forge-core/src/session_controls.rs` `refresh_project_instructions`).
- **`tools.extra_roots` was still refused by the file tools' in-process safety net.** The
  allowlist reached the two argument validators but not `confine()` in
  `crates/forge-tools/src/core_tools.rs`, whose `workspace_roots()` only trusted the workspace
  (and, for standalone runs, the system temp dir) — so a daemon-hosted `read_file`/`write_file`
  on an allowlisted path still failed with "resolves outside the workspace (workspace-confinement
  safety net)". The extra roots now ride a `SESSION_EXTRA_ROOTS` task-local scoped alongside
  `SESSION_WORKSPACE` by the tool wrapper, and `confine()` honors them, so all three confinement
  layers agree (`crates/forge-tools/src/lib.rs`, `workspace.rs`, `core_tools.rs`).
- **Every visible chat message re-rendered on every ~30 ms WebSocket frame.** `useSessionCtx()`
  exposed one context whose value object was rebuilt on every snapshot, so any consumer —
  including `MessageRow` (already `React.memo`'d) and the 1000+ line `Composer` — re-rendered
  per frame regardless of whether it read `snapshot` at all; `Composer` only needed
  `snapshot.model`/`snapshot.effort`, and the session shell's `SessionHeader`/`StatusStrip`
  received ~25 fresh inline-arrow-function props on every frame too. `sessionContext.tsx` now
  splits into a `live` context (`snapshot`, `snapshotTimedOut`, `connectionState`, changing per
  frame) and a `stable` context (session id, `send`, drafts, pending answer, header height,
  focus signal — changing only when one of those actually changes); `useSessionCtx()` still
  merges both for existing callers, and new `useSessionStable()`/`useSessionLive()` hooks let a
  component opt into just the slice it needs. `MessageRow` and `Composer` now read
  `useSessionStable()` only; `Composer` takes `model`/`effort` as props from its caller instead
  of reading them off `snapshot` itself, and is wrapped in `React.memo`. `SessionHeader` and
  `StatusStrip` are also `React.memo`'d, with their handlers hoisted into `useCallback`s and the
  `weekly`/`transport` object props memoized in the session shell so the memo isn't defeated by
  a fresh object every render (`mobile/src/lib/sessionContext.tsx`,
  `mobile/src/components/chat/MessageRow.tsx`, `mobile/src/components/chat/Composer.tsx`,
  `mobile/src/components/session/SessionHeader.tsx`,
  `mobile/src/components/session/StatusStrip.tsx`, `mobile/src/app/session/[id]/_layout.tsx`,
  `mobile/src/app/session/[id]/index.tsx`).
- **The mobile app burned battery just for being on screen.** v2.13.6's desktop performance
  monitor ran on every platform, not just Tauri: `startDesktopPerformanceMonitor()` scheduled a
  `requestAnimationFrame` loop that pushed to an unbounded array and re-sorted the whole thing
  every frame to find the median — measured at 0.9 ms/frame after 1 minute on screen, 33 ms/frame
  after 30 minutes, with `frameIntervals`/`composerSamples`/`composerImeSamples`/
  `composerInputEvents`/`composerImeEvents` all growing forever. Separately, the iOS Home Screen
  widget was resynced (an app-group write plus a `WidgetCenter` reload) on every fleet refetch —
  up to twice a second while any session streams — even when nothing the widget renders had
  changed, and the session timeline rebuilt its whole transcript from history on every ~30 ms
  WebSocket snapshot instead of only when history actually changed. The sampler now only starts on
  Tauri (the only platform with a consumer for it); the diagnostics and perf-fixture screens start
  and stop it themselves on demand elsewhere. Frame intervals live in a bounded 1024-entry ring
  buffer with an O(1) running estimate for dropped-frame detection instead of a per-frame sort,
  composer sample/event arrays cap at the newest 512 entries, and a new
  `stopDesktopPerformanceMonitor()` cancels the loop and long-task observer. `syncWidgetSessions`
  now skips the write and reload when the top-4 snapshot is byte-identical to the last one synced.
  The session screen memoizes `buildTranscript` on `historyRows`/`transcriptRows` instead of
  rebuilding it on every snapshot, and derives the live tool-activity ledger through a
  content-keyed memo so `FlatList`'s `renderItem` identity — and therefore every visible cell's
  render — only changes when the ledger's actual content changes
  (`mobile/src/lib/performance.ts`, `mobile/src/app/_layout.tsx`, `mobile/src/app/diagnostics.tsx`,
  `mobile/src/app/perf-fixture.tsx`, `mobile/src/lib/widgetData.ts`,
  `mobile/src/app/session/[id]/index.tsx`).
- **MCP servers launched through a wrapper outlived the session.** Forge's teardown killed only
  the direct child, but `npm exec …` / `uvx …` run the real server as a grandchild, so ending a
  session left it under pid 1 — nine `token-counter-mcp` node processes (~500 MB) from sessions that
  had ended hours earlier were found on one laptop. Stdio servers now start in their own process
  group, every teardown path (disconnect, reconnect, shutdown, manager drop) signals the whole
  group, and on Linux the child asks for `SIGTERM` when Forge itself dies, so even a SIGKILLed Forge
  leaves nothing behind (`crates/forge-mcp/src/transport.rs`).
- **A removed worktree left its whole code index behind.** Each daemon worktree session indexes its
  own copy of the repository and nothing pruned it afterwards: 48 dead `forge-wt-*` copies were
  ~1 GB of a 2.5 GB store. Session start now drops the index of any root whose directory no longer
  exists, before the incremental update (`Lattice::prune_stale_roots`). Roots that still exist are
  never touched; the file shrinks after `forge lattice prune --stale --vacuum`.
- **Markdown tables in `forge chat` rendered as one long line of pipes.** The transcript renderer
  parsed with no extensions, so a GFM table was just a paragraph whose rows were joined by soft
  breaks — every model-written comparison table came out as `| Field | App | … |---|---| …` on a
  single wrapped line. Tables now parse and lay out as aligned columns with a header rule; cell
  styling (inline code, bold) is kept, and an over-wide cell is clipped with an ellipsis so one
  long value cannot wrap every row. `~~strikethrough~~` no longer leaks its tildes either
  (`crates/forge-tui/src/render.rs`).
- **A session pinned to a free model auto-compacted on a ChatGPT-plan model.** Two pins exist:
  the router's `--model` from session creation, which `/model` never updates, and the session's
  own `/model` pin. Compaction and refinement built their candidate chain from the ROUTER's pin —
  a stale `codex-oauth::gpt-6-astra` from two days earlier — ahead of the session's current free
  pin, and when the free shortlist was exhausted the summary ran on the low-allowance
  subscription. A session-level pin now drops the routed hop entirely, and a subscription model
  that is not the model the session is running on is never a candidate for compaction,
  refinement, or any other side call (recap, suggestion, memory, shell diagnosis)
  (`crates/forge-core/src/compaction_policy.rs`, `refinement.rs`, `routing_policy.rs`).
- **The mobile app flipped a whole conversation to plain grey text and stayed there.** Whenever
  REST history was late, failed, or came back empty — a daemon restart, a server switch, the
  first paint — the screen fell back to the socket snapshot's transcript painted as bare lines,
  and nothing asked for history again until a turn completed. The fallback now renders the v9
  `transcript_rows` through the same message and tool rows history uses (the two states are
  indistinguishable), history is refetched on every socket reconnect, and an empty history page
  that contradicts a non-empty snapshot is re-asked for every 5 s until it agrees
  (`mobile/src/lib/transcriptFiller.ts`, `mobile/src/app/session/[id]/index.tsx`).
- **Mobile reconnect could stall until the app was reopened.** A WebSocket constructor that threw
  fired no `onclose`, so no retry was ever scheduled; and returning to the foreground reused the
  backoff accrued while backgrounded, so the first visible attempt could wait 15 s. Construction
  failures now take the normal backoff path, and foregrounding resets the attempt counter so the
  first reconnect is immediate (`mobile/src/lib/ws.ts`).
- **The lattice file watcher pinned a CPU core per Forge process, indefinitely.** Two Forge MCP
  agents on one laptop each burned ~90% of a core for their whole lifetime with nothing changing on
  disk — package temperature 97–100 °C and 46,000 thermal-throttle events in 32 minutes. The
  watcher's inotify backend subscribes to `IN_OPEN`, so every file the reindexer *read* to hash it
  came back as an event for that same file, which queued another reindex, which read it again:
  a loop that fed itself forever once the initial index walk seeded it (measured: every source
  file re-opened 73 times in 6 seconds, 97,000 events). Access-only events are now dropped before
  they reach the worker; a real write still reindexes. A regression test reproduces the loop
  shape — a reindex action that reads its file — and proves the count stops moving once the tree
  is quiet (`crates/forge-index/src/watch.rs`).
- **The watcher registered ~55,000 inotify watches on the Forge repository.** One recursive watch
  on the project root pulled in `target/`, `node_modules/`, `.git/` and every worktree, so each
  cargo build or checkout anywhere under the tree woke the watcher thread for nothing. It now
  registers one watch per directory the indexer actually walks (~300 here), sharing the indexer's
  walker so the watched set and the indexed set cannot drift, and follows the tree as directories
  appear. The debouncer crate and its extra thread are gone; the existing coalescing worker does
  that job.
- **`forge mcp agent` launched a second full agent beside itself.** The self-MCP guard compared
  executable *basenames*, but the repo's MCP launcher runs a snapshot copy named
  `forge-<hash>`, so a `target/debug/forge mcp agent` entry in `.forge/mcp.toml` was not
  recognised as self and was spawned — a second session, index and file watcher for every real
  one. Any forge-named binary invoked as `mcp agent` is now treated as a nested Forge agent
  (`crates/forge-cli/src/cli/commands/run/session.rs`).
- **A `forge mcp agent` could outlive the process that spawned it.** Stdin EOF ends the server
  loop when the parent exits cleanly, but a parent killed outright left the agent — session, index,
  watcher — running until someone found it in `top`. On Linux the agent now asks the kernel for
  `SIGTERM` when its parent dies (`PR_SET_PDEATHSIG`), so a dead orchestrator can no longer leave
  a live Forge behind (`crates/forge-cli/src/mcp_agent.rs`).

## [2.14.1] - 2026-09-07

v2.14.0 was tagged but never published: its release build failed 24 seconds in, before compiling
anything, because Debian Bullseye is EOL and its security pool rotated out from under the live
mirror — the apt index advertised `libc-dev-bin 2.31-13+deb11u14` while `deb.debian.org` had
already deleted that `.deb`. Both Linux targets died identically. This release is the same work
plus the build fix; per RELEASING.md a release tag is never moved, so it ships as a new version.

### Fixed
- **The portable Linux release build could not install its toolchain packages.** The container now
  pins apt to a timestamped `snapshot.debian.org` view, which keeps the security suite and cannot
  rotate underneath a build — making the step reproducible the same way the image digest and Rust
  toolchain already are (`.github/workflows/release.yml`).

### Added
- **`shell` can start something that keeps running.** Every attempt before this died the moment the
  call returned, and nothing said why — a session spent hours concluding "the sandbox kills
  background processes" and reaching for transient systemd units. The cause was three lines: after a
  command exits, the tool SIGKILLs its whole process group so a leaked descendant cannot hold the
  output pipes open, and `nohup cmd &` does not escape that (nohup detaches from the *terminal*, not
  the process group), nor does `systemd-run --scope`. `shell{background:true}` now spawns into its
  own session (`setsid`) with output on a log file, so the job outlives the call, the turn, and
  Forge itself; the new `shell_job` tool lists, tails, inspects and stops those jobs from state on
  disk under `.forge/jobs/`, so a later turn — or a whole new session — can find what an earlier one
  started. Jobs deliberately survive shutdown: an emulator that took two minutes to boot must not
  die because a turn ended (`crates/forge-tools/src/shell/background.rs`,
  `crates/forge-tools/src/shell.rs`).
- **A foreground call now says when it killed what the command left running**, naming the processes
  and pointing at `background:true`. The silent kill is what turned a three-line problem into hours
  of dead ends (`crates/forge-tools/src/shell/background.rs`).
- **A local SearXNG is now the default search backend**, with `scripts/searxng-setup.sh` to stand it
  up in one command and `FORGE_SEARXNG_URL` to point at another instance
  (`crates/forge-tools/src/web/search.rs`, `scripts/searxng-setup.sh`).

### Fixed
- **`web_search` was effectively down without a key.** The keyless DuckDuckGo default answered the
  FIRST query from an IP and then returned HTTP 202 with an empty body — one query per session is
  not a search tool — and the error it produced advised setting a Brave key "for reliable results",
  advice that expired when Brave retired its free tier in February 2026. Search is now a chain
  (local SearXNG → DuckDuckGo → keyless Bing) so one engine being throttled no longer takes it down,
  and every result says which engine answered. That attribution is load-bearing: keyless Bing never
  throttles but returns confident nonsense — asked for `tokio select macro` it returned ten
  well-formed results for plumbers near 1 Microsoft Way — so it is last and labelled, never trusted
  silently. Bing's `/ck/a` tracking redirects are decoded to real URLs
  (`crates/forge-tools/src/web/search.rs`).
- **The emulator booted on a renderer that segfaults it.** A headless `emulator_start` picked
  `-gpu swiftshader_indirect`; SwiftShader renders in JIT-compiled shader code, so a bad guest draw
  call faults *inside* it — an out-of-bounds SIMD load — and the SIGSEGV takes the whole emulator
  down mid-test. It killed the local AVD twice in a row. Headless now uses `-gpu auto-no-window`,
  the emulator's own renderer selection, which also drops the CPU cost of compositing a phone-sized
  screen (`crates/forge-device/src/emulator.rs`, `crates/forge-tools/src/device.rs`).
- **A language server that could never succeed was retried forever.** `clear_failure()` ran on a
  successful *handshake*, but rust-analyzer initializes in milliseconds and dies minutes later while
  indexing — so the counter reset to zero before every failure and the exponential backoff never
  once doubled. One project logged 386 identical "retrying in 30s" lines over three days, ~90% of
  its session log, each cycle also injecting an unactionable "diagnostics unavailable" notice into
  the model's context after every write. Only delivered diagnostics clear the failure state now, a
  `(language, root)` pair is given up on after five consecutive failures with a reason naming
  `lsp.memory_limit_mb`, and outage notices stay in the log where they belong
  (`crates/forge-lsp/src/registry.rs`, `crates/forge-core/src/lsp_hints.rs`).
- **A hard guard abandoned the work it was ending.** The error named 60 modified files and stopped,
  leaving a human to reconstruct what a 400-step turn had been in the middle of. Hard guards now
  snapshot tracked edits with `git stash create` under `refs/forge/aborted-turns` — nothing moves,
  the files stay exactly where they are — and the guard messages print billable *and* cache-inclusive
  input, because printing only the latter made a working token ceiling look broken
  (`crates/forge-core/src/turn_guards.rs`, `crates/forge-core/src/lib.rs`).
- **`forge run --output-format stream-json` answered with prose on any machine running a daemon.**
  The fleet-publish check ran before the stream-json branch, so an explicit machine-readable format
  was silently replaced by two human-readable lines and exit 0 — every parsing caller saw a
  successful run and no events. It went unnoticed because CI has no daemon: the e2e test passed
  everywhere except a developer's own box. An explicit machine-readable format now opts out of the
  fleet (`crates/forge-cli/src/cli/commands/run/one_shot.rs`).
- **The emulator inherited Forge's process group**, so any group-directed kill threw away a
  two-minute boot for a reason invisible from the device side (`crates/forge-device/src/emulator.rs`).

## [2.13.9] - 2026-09-03

### Fixed
- **A fresh install could fail its very first prompt with a usable model sitting right there.** With
  no catalog yet the router only ever tried the classified tier's built-in seeds; the trivial tier
  is a local ollama plus groq, so on a machine with no API keys and ollama not running the chain
  exhausted and the turn failed — while a logged-in `codex` CLI could have served it. The seed chain
  now carries the other tiers as a deduped tail (the tier's own candidates still lead, so the
  primary pick is unchanged), and `codex-cli::` joins `claude-cli::` as a default seed: it was in no
  tier list at all (`crates/forge-mesh/src/lib.rs`, `crates/forge-config/src/lib.rs`).
- **A first-run failure named an adapter instead of a next step.** The verdict only offered setup
  guidance when EVERY attempt reported missing credentials, and a keyless candidate fails as
  "provider unavailable", so a machine with nothing configured was told "attempted providers failed
  for mixed reasons". It now leads with `forge setup` / `forge auth` whenever no API key and no
  logged-in CLI exist, keeping the failure mix after it. Neither "binary on PATH" nor "not known
  logged out" proves a login, so the check requires positive evidence
  (`crates/forge-core/src/failure_verdict.rs`).
- **A read-only turn inside a build session is no longer re-driven to produce a diff.** #1266 fixed
  which contract such a turn derives, but the empty-diff nudge and the code-change classification
  still read the session-wide flag a worktree daemon session arms for its whole life, so the turn
  was still pushed to "implement the fix now" against its own instruction
  (`crates/forge-core/src/lib.rs`).
- **The CLI-bridge terms notice reads like Forge**, not a raw timestamped log line wedged between
  the routing line and the model's first token. It also survives the failover path, which is how a
  keyless run reaches a bridge at all (`crates/forge-provider/src/lib.rs`).
- **A bare bridge id reads as what it means.** `claude-cli::` is a valid pin for "whatever model
  that CLI is configured to use" and the first built-in complex-tier default, but printed verbatim
  it looked like a truncated id on a new user's first turn. It now renders as
  `claude-cli (its default model)` (`crates/forge-tui/src/lib.rs`).
- **A relay link that has never exchanged is no longer reported as disconnected.** The two states
  call for different actions: one is still coming up, the other needs a `forge serve` restart
  (`crates/forge-cli/src/anywhere/state.rs`).

### Changed
- **The `use_skill` listing costs about 626 fewer tokens on every tool-bearing turn.** It advertises
  every skill in its own description — 64 skills at 100 characters was 7,438 characters riding every
  request, a quarter of the whole tool payload and more than the system prompt. Each summary is
  clipped to 60 characters; discovery is unchanged since every skill is still listed by name
  (`crates/forge-core/src/lib.rs`).

## [2.13.8] - 2026-09-03

### Fixed
- **A long tool loop threw away the task it was working on.** When the transcript overflowed the
  model's window, the fit kept every system message and a newest-first suffix of history — and the
  user's own message was neither, so a tool loop that filled the budget evicted the instruction
  while keeping the output it produced. The model then reported that no task had arrived and
  invented work from what was left. The newest user message is now reserved before the walk, and
  clipped rather than dropped when it alone exceeds the budget (`crates/forge-core/src/context_pipeline.rs`).
- **A model behind a gateway assumed a 32k window when its real one is a million tokens.** OpenCode
  Zen and Go list models without a context length, as do most custom OpenAI-compatible endpoints, so
  every model behind one fell to the conservative floor and trimmed long turns for no reason. A
  model with no window row of its own now inherits the window published for the same model under
  another namespace, lowest match winning (`crates/forge-mesh/src/pricing.rs`,
  `crates/forge-core/src/routing_policy.rs`).
- **An explicitly read-only turn was re-driven to produce a diff.** A worktree-backed daemon session
  arms a code-change expectation for its whole life, and that beat the prompt, so a turn that said
  "do not edit anything" ended with no edits, tripped the empty-diff guard, and was pushed to
  "implement the fix now". An imperative read-only instruction in the prompt now wins, and the
  phrase list recognises what an operator actually types (`crates/forge-core/src/turn_contract.rs`).
- **OpenCode Zen's Responses-only models could not be called at all.** `muse-*`, `gpt-*` and `grok-*`
  answer only on `/responses` there, while Forge always built an OpenAI-chat request and got an
  instant 500. Zen now reuses the per-model wire-format matrix the Go adapter already carries
  (`crates/forge-provider/src/genai_provider.rs`).
- **A new model silently inherited an older sibling's benchmark score.** The cross-version guard
  compared version numbers by membership, so a shared major digit let a different minor through:
  `muse-spark-1.3` matched "Muse Spark 1.2" on the shared `1`. Comparison is positional now, and a
  benchmark row missing one of its two indices is kept on the index it has rather than dropped
  (`crates/forge-mesh/src/bench.rs`, `crates/forge-cli/src/benchmarks.rs`).

- **An Ask temper on the CLI bridge approved silently instead of refusing.** A bridged turn had no
  one to answer a permission prompt, so the gate resolved the wrong way
  (`crates/forge-core/src/permission.rs`).
- **Routing stalled on rediscovery when a cached catalog already existed**, and the daemon's models
  page served the last terminal's catalog rather than its own
  (`crates/forge-cli/src/cli/commands/models/discovery.rs`, `crates/forge-cli/src/serve/serve_models.rs`).
- **An over-pace subscription pool is now held entirely**, and the last-resort override is
  re-checked per failover hop instead of being spent once and left open
  (`crates/forge-mesh/src/lib.rs`, `crates/forge-core/src/model_request.rs`).
- **An unattended bridge turn that stalled with tasks still open now fails** instead of reporting
  success (`crates/forge-core/src/turn_guards.rs`).
- **An explicitly configured `[mesh.models]` tier is honoured** over an auto-discovered one
  (`crates/forge-mesh/src/catalog.rs`).
- **`forge doctor` reports a provider-rejected key as invalid** rather than unreachable, and
  `forge models` says which listed models have no key
  (`crates/forge-cli/src/doctor_health.rs`, `crates/forge-cli/src/cli/commands/models.rs`).
- **A keyless first run skips network enrichment**, bare `forge` shows a first-run panel instead of
  the full command list, non-tty setup is actionable, and CLI bridges known to be logged out are no
  longer probed (`crates/forge-cli/src/cli/commands/run.rs`, `crates/forge-provider/src/lib.rs`).
- **Claude quota is read from `unifiedWindows`**, and model reasoning is no longer printed as answer
  text on a non-tty (`crates/forge-provider/src/claude_quota.rs`, `crates/forge-tui/src/lib.rs`).

- **Opt-in: a standalone `forge run` can execute in the daemon and show in the Anywhere fleet.**
  `[remote] publish_local_runs` (default off) and per-run `--publish-to-fleet` /
  `--no-publish-to-fleet` hand the prompt to the local daemon, which creates a session carrying the
  cwd, model and a title from the prompt's first line. A one-shot run was previously invisible to
  the phone however healthy the relay was. Failure is soft: no daemon means the run proceeds locally
  exactly as before. Output is not streamed back to the handing terminal, which prints the session
  id and the `forge attach <id>` command (`crates/forge-cli/src/cli/commands/run/one_shot.rs`).
- **The empty-diff nudge and the code-change classification read the turn's contract**, not the
  session-wide flag a worktree daemon session arms for its whole life, so an explicitly read-only
  turn is no longer re-driven with "implement the fix now" (`crates/forge-core/src/lib.rs`).
- **`forge run` no longer stalls on rediscovery when a cached catalog exists**, and one reader now
  serves both the router and the daemon's models page
  (`crates/forge-cli/src/cli/commands/models/discovery.rs`).
- **The mesh explanation marks a rank a routing rule decided** instead of restating the score as
  something it is not (`crates/forge-mesh/src/explain.rs`).

### Added
- **`POST /api/sessions/{id}/interrupt`** ends a fleet session's current turn and leaves it live and
  idle. The daemon accepted an interrupt over its WebSocket but had no HTTP route, so a script could
  only stop a runaway turn by ending the session; `--steer` is no substitute, since it lands at the
  next turn boundary and a session stuck in a tool loop never reaches one (`crates/forge-cli/src/serve.rs`).

### Changed
- **`release-build` no longer gates pull requests.** The release compile plus its upgrade,
  reconnect and rollback end-to-end is the longest job in the pipeline, and every heavy job
  serializes on one runner, so running it per pull request set the merge throughput of the project.
  It runs on the push to main after a merge, on the weekly schedule, and on the dispatch the release
  workflow fires — still before anything ships (`.github/workflows/ci.yml`).

## [2.13.7] - 2026-09-02

### Fixed
- **Every claude bridge on the machine stopped reporting `Not logged in · Please run /login` after a
  Forge process ran inside a bridged turn.** `real_claude_config_dir` honoured an inherited
  `CLAUDE_CONFIG_DIR` even when it was Forge's own isolated mirror, so `forge mcp-serve` (and any
  child session it spawned) rebuilt the mirror onto itself and every entry became a symlink to
  itself, `.credentials.json` included. The inherited value is now ignored when it is the mirror,
  `prepare_claude_bridge_home` refuses to mirror a directory onto itself, and a first auth failure
  seconds after the same provider completed a turn benches one model for five minutes instead of
  excluding the provider for thirty (`crates/forge-provider/src/claude_bridge_home.rs`,
  `crates/forge-core/src/compaction_policy.rs`).
- **Unattended sessions no longer die on failover with `rate limited: HTTP error.` while 150+
  models are usable.** The headless presenter answers the compact-on-switch prompt with
  `NO_ANSWER`, which the consent gate read as "No", skipping every smaller-window fallback until
  the chain ran dry. A non-answer now compacts and continues (`crates/forge-core/src/compaction_policy.rs`).
- **Unattended turns run past the soft step cap instead of exiting with uncommitted work.** A
  headless `forge run` (or `--mode bypass`) treats `mesh.max_steps` as a checkpoint: one warning
  with the step count and the turn's cumulative tokens, then it continues to
  `mesh.max_steps_unattended` (default 400) and ends with an ERROR naming the uncommitted work.
  Attended sessions still pause. A new `mesh.max_turn_input_tokens` ceiling (default 10M) ends a
  runaway turn on every surface, and both guards latch so re-drives cannot reset the counters
  (`crates/forge-core/src/turn_guards.rs`, `crates/forge-config/src/lib.rs`).
- **Failover hops obey the subscription pacing verdict, not just the primary pick.** Two builder
  sessions failed over onto a held codex model and burned 5–7M input tokens each. Held models are
  now parked until every non-held candidate is exhausted, reached only as a last resort with the
  rationale `last resort: pacing hold overridden`, and `forge mesh` marks the hold per model
  (`crates/forge-core/src/model_request.rs`, `crates/forge-mesh/src/lib.rs`).
- **A resumed follow-up turn inherits the session's routing tier.** "continue" on a complex
  session was classified on its own text and handed to a free trivial-tier model; the turn is now
  floored at the session's most recent routing tier unless a pin or explicit effort overrides it,
  with `tier inherited from previous turn` in the rationale (`crates/forge-core/src/routing_policy.rs`,
  `crates/forge-store/src/provenance_store.rs`).
- **Gemini 3.x no longer rejects a transcript whose tool calls came from another model.** Unsigned
  `functionCall` parts get the documented placeholder signature, captured signatures are replayed
  intact, and an HTTP 400 that names a transcript-compatibility problem classifies as a per-model
  capability failure so failover continues instead of ending the turn
  (`vendor/genai-0.6.5/src/adapter/adapters/gemini/adapter_impl.rs`,
  `crates/forge-provider/src/genai_provider/error_policy.rs`).
- **The Antigravity bridge stops being killed at exactly 120 s on healthy turns.** `agy -p` printed
  nothing until the whole answer was ready, so the idle watchdog killed every complex turn and the
  mesh walked through -high/-low/-medium for six minutes per cascade. agy now runs with
  `--output-format stream-json` and a 600 s print timeout, its usage block is recorded (no more
  `↑0 ↓0`), and a stall names the budget that fired in `model_health`
  (`crates/forge-provider/src/cli_provider.rs`, `crates/forge-provider/src/cli_provider/cli_stream.rs`).
- **OpenCode Go burn weights account for each model's own weekly quota.** The dashboard's weekly
  percentage is the sum of per-model percentages against $7.50 / $15 / $30 quotas, so a dollar on
  Grok 4.6 or Kimi K3 drains the pool four times faster than a dollar on Muse; the price-derived
  weight is now multiplied by `largest quota / model quota` (fallback table, since the usage
  endpoint exposes no per-model data) and `forge mesh` prints the quota buckets
  (`crates/forge-mesh/src/subscription_cost.rs`).
- **A binary compiled alongside the test suite says so instead of reporting "no keys".**
  `forge auth --list`, `forge models`, `forge mesh` and `forge doctor` name the active secret-store
  backend and warn loudly when it is the `test-secrets` in-memory store
  (`crates/forge-config/src/secret_store.rs`).

### Added
- **The subscription pacing verdict is visible everywhere routing acts on it.** `forge mesh`, the
  TUI usage overlays, the daemon usage API and the mobile usage screen show used vs allowed, the
  elapsed fraction and whether models are being held (`crates/forge-types/src/subscription_pacing.rs`,
  `mobile/src/app/usage.tsx`).

## [2.13.6] - 2026-09-02

### Fixed

- **OpenCode Go's top-ranked models now reach the endpoint they actually implement instead of
  failing or spending 6m47s in “recovering provider”.** The service exposes three incompatible
  wire formats without identifying them in `/models`: `gpt-5.6-luna`, `grok-4.5`,
  `grok-4.6`, and `muse-spark-1.2-contributor` reject Chat Completions immediately and answer
  only on Responses, while the other Go models do the reverse. Forge now seeds that measured
  matrix, learns an unknown model's endpoint only after its characteristic rejection and a
  successful one-shot Responses retry, and omits unsupported temperature parameters by model
  family. Two identical errors returned within two seconds are treated as a rejection, so a pinned
  model surfaces the real error immediately instead of consuming the 600-second outage budget;
  live turn-loop checks answered on Muse, Luna, Grok, and GLM in 7–12 seconds
  (`vendor/genai-0.6.5/src/adapter/adapters/opencode_go/adapter_impl.rs`,
  `crates/forge-provider/src/genai_provider.rs`, `crates/forge-core/src/model_request.rs`).

- **Claude CLI tool and filesystem errors no longer disable a valid login for 30 minutes.** A
  working `claude-cli::opus[1m]` was stored as `excluded: auth failed: auth failed` because the
  permanent-auth phrase list accepted generic “permission denied” and “credentials” text emitted
  by tool gates, OS errors, and keychain notices. Only text that identifies the login can now earn
  that provider-wide verdict, and the stored health row retains up to 240 characters of the CLI's
  actual evidence instead of repeating the classification. Discovery also unions Claude 2.1.257's
  initialize picker with its documented aliases, so Fable is available even though initialize
  advertises only Opus, Sonnet, and Haiku (`crates/forge-provider/src/cli_provider.rs`,
  `crates/forge-provider/src/cli_provider/error_policy.rs`,
  `crates/forge-core/src/compaction_policy.rs`).

- **Reinstalling the daemon service now applies the new binary instead of merely rewriting the
  unit.** `systemctl --user enable --now` is a no-op for an already-active unit, so the rendered
  `ExecStart` could point at the release while the old process kept serving; in the observed
  failure this ended in a `203/EXEC` service outage. Active systemd units are explicitly restarted,
  loaded launchd agents are reloaded, and active Windows scheduled tasks are ended and re-run.
  Install and status inspect the live process before and after activation, report its executable
  and version, and fail honestly when the replacement cannot be established
  (`crates/forge-cli/src/cli/commands/service.rs`,
  `crates/forge-cli/src/cli/commands/service_report.rs`).

- **`forge doctor` reports the daemon's version, not the version of the doctor binary printing the
  report.** A unit stamped 2.12.2 with a daemon actually running 2.13.5 was reported as “running
  2.13.2” because 2.13.2 happened to be the separately installed CLI invoking doctor. Version
  evidence now comes from the live daemon's authenticated `/api/identity`, then the unit's
  `ExecStart --version`, otherwise an explicit unknown; the report labels the unit stamp, daemon
  binary, and current CLI separately so upgraded-on-disk-but-not-restarted processes are visible
  (`crates/forge-cli/src/doctor.rs`, `crates/forge-cli/src/doctor_daemon.rs`).

### Added

- **Routing prices now follow current model economics instead of stale hardcoded burn weights.**
  OpenRouter has no GPT-5.6 rows, leaving Codex decisions at `$0`, while the fallback
  Sol/Terra/Luna ladder of 5/2.5/1 predated current $4/$20, $2/$12, and $0.20/$1.20 per-million-token
  prices—roughly 17.5× and 10× Luna for Sol and Terra. Forge fetches models.dev beside OpenRouter,
  maps its prices onto native and CLI-bridge namespaces, preserves bundled rates on fetch failure,
  and resolves override → fetched/bundled price → table. A nonzero subscription floor prevents a
  heavier sibling winning on a marginal score at zero pressure; that old behavior burned 64% of a
  fresh $12/5h OpenCode Go pool in two hours on Kimi K3 over a 0.14-point advantage
  (`crates/forge-cli/src/context_windows.rs`, `crates/forge-mesh/src/pricing.rs`,
  `crates/forge-mesh/src/subscription_cost.rs`, `docs/features/mesh-routing.md`).

- **Subscription routing accounts for the size of the pool and the share consumed by one request.**
  At OpenCode Go 28% and Codex 25%, the former's Kimi K3 scored 3.27 over Codex OAuth's Sol at
  2.96 even though one Kimi request consumed about 1% of its $12/5h pool and Sol used a fraction of
  a much larger plan. Providers now carry an explicit capacity class—OpenCode Go is Tiny; captured
  CLI plan slugs map 20x to Large, max/pro to Medium, plus/team to Small, and an unset plan remains
  Unknown—and ranking applies request share times model burn times scarcity, with scarcity capped
  at 3×. Equal models therefore prefer the larger, fuller pool without guessing an unknown plan
  (`crates/forge-mesh/src/catalog.rs`, `crates/forge-mesh/src/subscription_cost.rs`).

## [2.13.5] - 2026-09-01

### Fixed

- **An Expo patch publish can no longer turn a tagged release red on its own — this is what kept
  v2.13.4 from ever publishing.** expo-doctor's "packages match versions required by installed Expo
  SDK" check resolves the SDK's expected patch versions over the network, so the answer lives
  outside the repository: Expo shipping `expo@57.0.19` upstream was enough to fail `app preflight`
  on a commit that had passed CI unchanged, and because `app-desktop.yml` checks out
  `refs/tags/<release_tag>` no fix landing on `main` can rescue the already-cut tag. That is the
  fourth occurrence of this exact failure mode (#993, #1129, #1160, and v2.13.4). The eleven
  drifted SDK packages are realigned with a lockfile regenerated under npm 10 to match CI's Node 20
  toolchain (`mobile/package.json`, `mobile/package-lock.json`), and the release path now runs
  `scripts/ci/mobile-release-check.sh`, which suppresses only that one check through expo-doctor's
  own `EXPO_DOCTOR_SKIP_DEPENDENCY_VERSION_CHECK` while the other 18 checks, ESLint, `tsc --noEmit`
  and Vitest stay fully enforcing (`.github/workflows/app-desktop.yml`,
  `.github/workflows/app-web.yml`, `scripts/ci/test-mobile-release-check.sh`,
  `.github/workflows/ci.yml`, `mobile/README.md`). PR CI still runs the plain `npm run check`, so
  version drift is still caught — just where a human can act on it instead of where it strands a
  release.

- **The mobile lockfile moves to `browserslist` 4.28.8 for GHSA-73wf-gq98-2v4g and
  GHSA-c83g-rgw3-j3cx.** Both advisories were published after `main`'s last green run and cover
  `browserslist <= 4.28.6`, which `main` carried at 4.28.4; every dependent range is `^4.x`, so a
  lockfile bump clears the audit gate with no override (`mobile/package-lock.json`). Same
  non-hermetic class as the expo-doctor failure above, on the audit gate rather than the doctor
  gate.

## [2.13.4] - 2026-09-01

### Fixed

- **The two macOS legs that failed the v2.13.3 release now compile — this is the release that
  reaches users.** `pid_alive` read errno through `libc::__errno_location()`, which is a glibc
  symbol; Apple's libc spells it `__error`, so `aarch64-apple-darwin` could not build at all. errno
  now comes from `std::io::Error::last_os_error()`, which is portable
  (`crates/forge-core/src/worktree_reclaim.rs`). Separately, `release.yml` installed its
  cross-compilation targets with `dtolnay/rust-toolchain`'s default `stable` channel while cargo
  obeyed `rust-toolchain.toml`, so the pinned channel had only the host target installed and
  `x86_64-apple-darwin` died with `can't find crate for std`; the workflow now resolves the channel
  out of `rust-toolchain.toml` and pins the action to it (`.github/workflows/release.yml`,
  `.github/workflows/app-desktop.yml`). Both defects passed CI green because `release.yml` is
  `workflow_dispatch`-only and no PR-time job ever builds a macOS target. Two guards close that
  blind spot at PR time: `scripts/ci/release-targets-guard.sh` asserts every target the release
  builds resolves against the pinned toolchain and that each installing step pins that toolchain,
  and `scripts/ci/linux_only_libc.py` rejects glibc-only libc symbols in code the macOS legs
  compile (`.github/workflows/ci.yml`, plus `scripts/ci/test-release-targets-guard.sh` and
  `scripts/ci/test_linux_only_libc.py`).

- **A rejected Codex `previous_response_id` no longer pins a session to full-HTTPS resends for the
  rest of its life.** The backend spells the rejection ``Invalid `previous_response_id`.`` with
  backticks, but `is_stale_previous_response_error` tested for `invalid previous_response_id` —
  space, no backticks — so the one-shot reconnect-and-resend-in-full recovery never fired. The
  failed request then stashed its dead chain as `resume_history`, which every following turn
  restored and replayed, so a single transport drop downgraded the whole session permanently. The
  matcher now strips backticks before comparing, and a request rejected for a stale chain clears
  both `resume_history` and the socket instead of re-stashing them
  (`crates/forge-provider/src/codex_websocket.rs`, `crates/forge-provider/src/codex_oauth.rs`). The
  existing unit test asserted the matcher against a message shape the backend never sends, so it
  passed while the feature was dead; it now covers the real backticked spelling alongside a live
  end-to-end test that a rejected chain resends in full exactly once and does not poison later
  turns.

### Added

- **`forge worktree` accounts for Cargo `target/` bytes separately and can prune them from
  worktrees that must otherwise be kept.** `list` reports source and build-artifact sizes as
  distinct numbers, and `reclaim --artifacts` deletes stale `target/` directories on their own
  lifecycle: never from a worktree with a live session or an in-flight build, never without a
  readable modification time, and only after a quiet period (`--artifact-min-age-hours`, default
  24). This matters because reclaimable-by-worktree badly understates what is recoverable — on the
  machine this was built on, `forge worktree list` reported 146.9 GB across 76 worktrees but only
  107.4 MB reclaimable, since roughly 94% of every worktree is regenerable build output held in
  place by a live session, an unmerged branch, or uncommitted changes
  (`crates/forge-core/src/worktree_reclaim.rs`, `crates/forge-cli/src/cli/commands/worktree.rs`,
  `crates/forge-cli/src/cli/args.rs`, `crates/forge-core/tests/worktree_reclaim.rs`).

## [2.13.3] - 2026-09-01

### Fixed

- **Desktop releases can be published again — this is the release that actually reaches users.**
  linuxdeploy probes a plugin by invoking it with no `--appdir`; the AppImage plugin shim answered
  that probe with `exit 1`, and under `set -euo pipefail` the `Prepare Arch Linux webview deps`
  step died before the `linux-x86_64` AppImage was ever built. The publish job then correctly
  refused to ship an incomplete signed platform set and the release stayed a draft. That one line
  is why v2.13.0, v2.13.1 and v2.13.2 are all unpublished and why the newest installable Forge was
  still v2.12.1: four of five desktop legs succeeded every time. The probe path now logs and falls
  through to the real plugin, whose exit status propagates as before, and the shim moves out of an
  inline heredoc into `scripts/ci/appimage-plugin-shim.sh` so it can be tested
  (`scripts/ci/test-appimage-plugin-shim.sh`, `.github/workflows/app-desktop.yml`,
  `.github/workflows/ci.yml`). The desktop matrix also gains a name expression, because
  `matrix.os` is a label array for the self-hosted Linux leg and every leg rendered as
  `build (Array)` — the failing platform could not be identified from the checks list.

- **A mistyped field in `POST /api/sessions` no longer silently starts the session in the wrong
  directory.** The daemon's request bodies deserialized with unknown fields ignored, so a client
  sending `workspace` instead of `cwd` got a session rooted in the daemon's own cwd — `$HOME` —
  rather than the intended repository, with no error anywhere. `CreateSessionReq` and
  `SendFleetMessageReq` now use `deny_unknown_fields`, and create-session rejections are returned
  as a `400` naming the offending field instead of axum's default empty body
  (`crates/forge-cli/src/serve.rs`).

- **Stopping the daemon no longer reports an aborted turn as a successful one.** `forge serve`
  handled only Ctrl-C and killed session drivers immediately, leaving the last snapshot claiming
  success. Shutdown now listens for SIGTERM as well, marks the registry as draining, refuses new
  sessions and fleet messages with `503`, and gives active turns a bounded 30s window to finish;
  a turn cut short by shutdown ends with the new `StopReason::DaemonShutdown` rather than a stale
  success (`crates/forge-cli/src/serve.rs`, `crates/forge-cli/src/cli/commands/run/driver.rs`,
  `crates/forge-types/src/turn_outcome.rs`).

- **A model the account cannot call is now benched instead of being re-selected every turn.** groq
  answers a decommissioned id with `404 model_not_found`, which matched no capability marker and
  fell through to the catch-all `ProviderError::Request` — neither retryable nor benched, so
  nothing was written to `model_health` and ranking kept picking the model. It is classified as a
  new permanent `ProviderError::NoModelAccess` on all three paths (typed status, structured stream
  body, free text), and permanent failures are persisted before the pinned-model policy returns
  and on the non-failover terminal arm, so even a pinned `--model` run leaves a health row. The
  exclusion is scoped to the model: the provider's other models keep working
  (`crates/forge-provider/src/error.rs`,
  `crates/forge-provider/src/genai_provider/error_policy.rs`,
  `crates/forge-core/src/model_request.rs`).

- **A healthy ChatGPT subscription is no longer disabled by a malformed probe.** The Codex OAuth
  entitlement probe sent `stream:false`, got a `400` about request shape, and Forge misfiled it as
  an auth failure — excluding `codex-oauth` provider-wide and pushing every OpenAI turn onto the
  `codex-cli` bridge. The probe now sends `stream:true` with an `Accept: text/event-stream` header,
  and only `401`/`403` count as credential verdicts; anything else is reported as itself. Token
  refresh becomes a first-class path: an expired access token with a live refresh token rotates
  instead of failing, only a server-rejected refresh becomes `ProviderError::Auth` while a
  rate-limited or unreachable token endpoint becomes `ProviderError::Unavailable` so the
  subscription stays enabled. The upstream status and message are surfaced verbatim, replacing the
  bare `auth failed` (`crates/forge-provider/src/codex_oauth.rs`,
  `crates/forge-provider/src/codex_oauth/quota_headers.rs`,
  `crates/forge-config/src/provider_oauth.rs`).

- **`forge doctor` reports whether a provider will actually route, not just whether it answers.**
  A provider can serve discovery while the mesh has excluded it or its subscription is exhausted.
  Doctor now derives a routing verdict from the reachability evidence and reports both, and the
  "usable provider" summary follows the routing verdict; the reachability rows are relabelled
  accordingly (`crates/forge-cli/src/doctor.rs`, `crates/forge-cli/src/doctor_health.rs`).

- **Subscription windows have a rollover date again.** Every `subscription_usage` row was written
  with an empty `resets_at`, so the phone showed no rollover and pacing had to assume a nominal
  window length. The reset instant is now threaded through from the source that already reports it
  — Claude's `anthropic-ratelimit-unified-*-reset` headers — into the store
  (`crates/forge-store/src/lib.rs`, `crates/forge-store/src/quota_store.rs`,
  `crates/forge-cli/src/bridge_stats.rs`, `crates/forge-core/src/session_controls.rs`).

### Added

- **`forge worktree list` / `reclaim` — recover disk from worktrees Forge abandoned.**
  `WorktreeGuard`'s `Drop` only covers a clean exit, so a killed session left its worktree behind
  forever; unreclaimed ones filled a 1.8 TB disk to zero free on the development machine and killed
  a linker with a bus error. A survey measures every registered worktree (size including
  `target/`, merged, dirty, unpushed, live, age) and a classifier clears **only** a prunable
  registration or a merged, clean, unreferenced worktree; ownership is recorded at creation so a
  dead session's worktree is still recognisable. `list` shows the worst offender first, `reclaim`
  is a dry run unless `--yes` and prints every kept worktree with its reason, and `forge doctor`
  gains a worktree-disk section that fails when free space is low with something to reclaim
  (`crates/forge-core/src/worktree_reclaim.rs`, `crates/forge-cli/src/cli/commands/worktree.rs`,
  `crates/forge-cli/src/doctor_environment.rs`, `crates/forge-store/src/live_session_store.rs`).

- **The mesh paces a subscription that is burning ahead of its window.** While a subscription is
  over its linear allowance, routing keeps only that provider's cheapest usable model, and if the
  only usable sibling is also the provider's highest-burn model the provider is dropped entirely —
  ordinary pacing never spends a flagship through the protected reserve
  (`crates/forge-mesh/src/lib.rs`, `crates/forge-types/src/subscription_pacing.rs`).

- **The README lists the features that shipped without it, and the Rust toolchain is pinned.**
  `/refine`, `/btw` and `/heartbeat` each shipped with a docs page and a changelog entry but no
  README mention — the one document a new user reads first. `rust-toolchain.toml` also tracked
  `stable`, so a new Rust release could land new warn-by-default lints on every open PR at once and,
  with CI's `RUSTFLAGS=-D warnings`, arrive as hard failures nobody chose the moment for. It is
  pinned to 1.98.0, the current stable, so nothing about what compiles changes today
  (`README.md`, `rust-toolchain.toml`).

## [2.13.2] - 2026-08-30

### Fixed

- **Bridged Codex sessions no longer abort their first turn by believing Forge tools are missing.**
  The provider harness now tells each CLI how Forge MCP tools are actually acquired: Claude receives
  schemas up front, Codex must resolve deferred `mcp__forge__*` registry entries, and Antigravity no
  longer receives a Forge MCP preamble it cannot use (`crates/forge-provider/src/cli_provider.rs`).

- **Subscription quota displays no longer present stale or unknown readings as live headroom.**
  Expired quota snapshots are dropped before display and unknown fractions stay unknown instead of
  rendering as `0%`, while Codex CLI and OAuth share one subscription quota view because they spend
  the same account (`crates/forge-store/src/quota_store.rs`, `crates/forge-cli/src/serve/serve_usage.rs`,
  `crates/forge-cli/src/cli/commands/models/presentation.rs`, `mobile/src/app/usage.tsx`).

- **Codex bridge usage no longer counts cached input as fresh or invents zero cache use.**
  Cached input reported by Codex is now recorded for context, budget, cost and routing calculations,
  while providers that omit it store an unknown value instead of zero
  (`crates/forge-provider/src/cli_provider.rs`, `crates/forge-types/src/lib.rs`,
  `crates/forge-store/src/migrations/usage.rs`, `crates/forge-mesh/src/pricing.rs`).

- **CLI model discovery failures no longer masquerade as live model inventory.**
  `forge models` and `forge doctor` now show each list's provenance and the probe error, reuse the
  last successful live inventory while retrying, and discover Codex 0.149+ models through
  `codex debug models` (`crates/forge-provider/src/cli_provider.rs`,
  `crates/forge-cli/src/cli/commands/models/discovery.rs`,
  `crates/forge-cli/src/doctor_bridge_models.rs`).

- **A turn that produces no assistant text and no successful mutation is reported as a failure.**
  Forge now classifies that outcome as `no_output` and exposes the result in the TUI, remote
  snapshots and fleet session rows, so phone clients and orchestrators can distinguish a no-op turn
  from a successful one (`crates/forge-core/src/lib.rs`, `crates/forge-cli/src/remote.rs`,
  `crates/forge-cli/src/serve.rs`, `crates/forge-types/src/turn_outcome.rs`).

## [2.13.1] - 2026-08-30

### Fixed

- **Desktop releases could not publish at all.** Every one of the five `app-desktop.yml` build legs
  for v2.13.0 failed, for three independent reasons, after all the CLI and desktop binaries had
  already been built. The release correctly stayed a draft, so nothing shipped and v2.12.1 remained
  Latest and installable — but no desktop bundle, `desktop-checksums.txt` or `latest.json` could be
  produced until all three were fixed. This release carries those repairs.

- **Updater artifacts were never generated, which killed three legs.**
  `bundle.createUpdaterArtifacts` in `mobile/src-tauri/tauri.conf.json` had been flipped `true` →
  `false` inside the unrelated perf/parity squash #954 (`a2b8a29e`, merged 2026-08-04), whose
  description never mentions updater artifacts. In tauri-cli 2.11.4 that one flag decides whether
  anything is signed: the interface builds updater settings only when it is not `false`, and
  `sign_updaters` returns immediately when those settings are absent. So no `.sig` was written on
  any platform despite `TAURI_SIGNING_PRIVATE_KEY` being present — the Linux leg bundled both its
  AppImage and `.deb`, logged that updater signing was enabled, and then died on
  `cp: cannot stat '...AppImage.sig'`. The signing key was never the problem; nothing had been
  asked to produce a signature. The flag is restored to `true`, the v2 self-contained layout the
  workflow actually collects.

- **The Windows leg failed before compiling anything.** The same squash replaced the
  `beforeBuildCommand` with `mobile/scripts/export-web-clean.mjs`, which spawned the `npx.cmd`
  shim directly. Since the CVE-2024-27980 mitigation (Node ≥ 20.12.2, and the runner pins Node 20)
  `spawnSync` refuses a `.cmd` target without `shell: true` and reports `EINVAL` in `result.error`
  with a **null** status; the script checked only `result.status !== 0` and exited without printing
  anything, so the leg failed in ~50 ms with no output at all. The script now resolves the
  installed Expo CLI from `expo`'s own manifest and runs it under `process.execPath`, taking the
  `.cmd` shim out of the picture on every platform, and reports spawn errors, signals and exit
  codes.

- **The self-hosted `linux-x86_64` leg never reached the build.** Its `linuxdeploy` AppImage plugin
  was pinned by SHA-256 but downloaded from linuxdeploy's rolling `continuous` release, which keeps
  one asset name and republishes its bytes; the upstream rebuild on 2026-08-01 changed the payload
  out from under the reviewed digest and `Prepare Arch Linux webview deps` failed closed with
  `checksum mismatch for pinned linuxdeploy AppImage plugin`. That check behaved exactly as
  intended — the pin was pointing at a moving target. It now names the immutable tagged release
  `1-alpha-20250213-1`, whose assets cannot be rewritten, and the mismatch message prints the digest
  actually served.

- **A CI guard so the updater flag cannot regress silently again.**
  `scripts/ci/desktop-updater-guard.sh` (with `scripts/ci/test-desktop-updater-guard.sh`) fails any
  PR that disables updater artifacts or selects the deprecated `"v1Compatible"` layout, wired into
  the same CI job as the existing desktop version guard. The original flip sat unnoticed for four
  weeks because nothing checked it. See `docs/known-issues.md`.

## [2.13.0] - 2026-08-28

### Added

- **Continual Harness — `/refine`, a session that gets better at your project.** Forge can now
  learn from how a session actually went and carry that forward: it journals prompt notes, skill
  descriptions and subagent specs, injects them into later turns, and can roll any of it back.
  `/refine` runs a pass on demand and `/refine status` shows what has been learned; every entry
  records the trajectory evidence that produced it, so a lesson can be traced rather than taken on
  faith. Automatic refinement ships **off by default** (`harness.auto_refine = "off"`; set it to
  `"compact"` to run a pass after every `/compact`, or `"turns"` to run one every
  `harness.auto_refine_turns` turns) — a self-modifying layer is opt-in until you have looked at
  what it wants to change. See `docs/features/continual-harness.md`.

- **`/btw` — ask a side question without derailing the turn.** A question asked mid-task previously
  either waited for the turn to finish or knocked the agent off what it was doing. `/btw` answers
  out of band and leaves the main thread's objective and context intact. The same change adds HTML
  session export, so a transcript can be handed to someone who does not have Forge.
  See `docs/features/side-questions.md`.

- **Long-running autonomy now has explicit controls.** `/goal` and `/loop` can enforce quality
  gates and token or wall-clock budgets; asynchronous subagents retain admission handles so they
  can be followed up without losing ownership; and session heartbeats can re-enter live work at a
  bounded interval through `/heartbeat` or the `manage_heartbeats` tool.

- **`forge doctor` now reports the two things that actually take this setup down.** It checks the
  background daemon — whether `forge serve` is running, failed, or restart-looping — and flags a
  systemd unit that predates the running binary, which is how a unit silently keeps missing the
  restart guards a newer release ships. Previously both failures were invisible to the one command
  a user runs when something is wrong.

- Forge Anywhere surfaces local terminal sessions read-only in the fleet, so a session started in a
  terminal is visible from the phone instead of appearing only after it is daemon-hosted. A pending
  device enrollment can also be approved without going through the inbox.

- The desktop app connects on launch instead of asking first, and follows a daemon that moves.

- Models and providers can be enabled or disabled from the CLI, provider discovery reports its
  progress instead of failing opaquely, and Forge Anywhere can show a bounded read-only git diff
  for review from a phone.

- **Fleet messaging — `forge send` and the `message_session` tool.** Any two daemon-hosted
  sessions can now exchange messages, and a shell can message any of them: previously only a
  parent could reach its own subagents inside one session tree, so a session in another window was
  unreachable. `forge send <target> "<msg>"` addresses a session by name or unique id prefix;
  `--steer` delivers at the target's next turn boundary ahead of its queued backlog, while the
  default queues for when it goes idle. Neither ever interrupts a turn already streaming. The
  `message_session` virtual tool gives an agent the same reach, advertised only when the session
  is `forge serve`-hosted. Pending messages persist and the backlog flushes whenever a target
  rejoins the fleet, so a daemon restart between send and delivery strands nothing. 16KB per
  message, 8 pending per sender/target pair. See `docs/features/fleet-messaging.md`.

- The companion app now has a cross-surface Diagnostics & Updates page backed by an authenticated,
  bounded `/api/diagnostics` projection. It reports daemon/client protocol compatibility, aggregate
  runtime and resource state, safe host checks, and one centralized signed-desktop-updater state.
  Forge Anywhere exposes the same projection through a dedicated typed read-only route; arbitrary
  daemon paths remain prohibited. The copied support summary uses a strict whitelist and excludes
  host names, connection details, tokens, credentials, workspace data, prompts, logs, environment
  values, and daemon-provided free text.
- Native iOS and Android clients can receive shared text or a web URL into a durable new-session
  draft. The iOS share extension, Android share intent and notification channel/icon, in-app
  privacy/license/support surface, public privacy policy, and store-listing source are generated or
  version controlled with the release configuration.

### Changed

- Forge's shared workbench now spans desktop and mobile, with the session, fleet, diagnostics,
  update, keyboard-shortcut, annotation, and review surfaces using the same interaction model.

- The mobile native dependency set is aligned to Expo SDK 57's current compatible patch versions,
  including Expo 57.0.9 and React Native 0.86.2, before producing the next native archives.

### Fixed

- **The LLM task classifier was dead for every user on defaults.** `mesh.classifier_model` defaulted
  to `groq::llama-3.3-70b-versatile`, an id that answers *"The model does not exist or you do not
  have access to it"* — so classification failed on every turn and silently degraded to the
  heuristic, reporting only "all LLM candidates unavailable" with no cause. The failure now names
  the model and reason, a permanently-dead candidate is marked and skipped instead of being retried
  every turn, and the single candidate became a fixed ordered fallback chain whose every entry was
  verified with a real call rather than read from provider discovery. Provider `/models` listings
  are not evidence a model is callable: groq still advertises two uncallable ids as free, and
  cerebras advertises one that answers "Payment required".

- **A daemon spawned by Forge could be left permanently broken by a deleted working directory.**
  `ollama serve` and the Anywhere-managed `forge serve --local` were both spawned without a working
  directory, so they inherited Forge's — often a worktree or temp dir that is later removed. Once it
  was gone, every ollama model load failed with `cannot get current path`, while `forge doctor` still
  reported the server healthy and `/api/tags` still answered; only inference was dead, so the mesh
  quietly failed over to a worse model. Both now start from a directory that outlives Forge.

- **The daemon deadman alarm never fired.** `OnFailure=` was rendered into the systemd `[Service]`
  section, where systemd silently ignores it — no warning, no parse error. The alarm added for
  unattended daemon failures had therefore never fired on any install while appearing correctly
  configured. It is now emitted in `[Unit]`, and the regression test pins the section rather than
  merely asserting the directive is present.

- **The Windows bridge home copied files it should not.** When Windows cannot create a symlink the
  bridge falls back to copying, and what was safe to copy was decided by a denylist matching
  `credential`/`secret`/`token` in the filename — fail-open, so entries like `daemon` and
  `control.key` were copied. It is now a two-entry allowlist that fails closed.

- Images sent to the OpenAI-compatible `forge api` endpoint were silently dropped and the request
  routed as if it had no image. Image content parts are now decoded and the vision signal reaches
  routing; only `data:` URLs are accepted, since fetching a remote URL from the daemon would make any
  request an SSRF primitive.

- `chacha20` was pinned to a yanked release in both the workspace and vendored lockfiles.

- Model selection now treats a pin as a hard constraint across resume, subagents, model sets,
  inherited scores, and last-resort routing. Context accounting includes tool schemas and asks
  local servers for their actual context window, while provider transcripts hoist system messages
  and serialize wire parameters consistently.

- Shell confinement now fails closed when Landlock setup cannot be completed, with the ruleset
  prepared before forking. Background failures across Store, Lattice, LSP, MCP, provider bridges,
  replay, voice, shell cleanup, and daemon services are surfaced instead of being silently ignored.

- **A development build no longer migrates the store the installed Forge depends on.** This class of
  failure took the daemon down repeatedly: something built from a working tree opened
  `~/.local/share/forge/forge.db`, migrated it to whatever schema that branch carried, and the
  installed release binary then correctly refused to open it — leaving `forge serve` crash-looping
  and Forge Anywhere dark. It is now closed from both directions. A build with `debug_assertions`
  resolves to `forge-dev.db`, covering `cargo run`, `target/debug/forge` and dev-built MCP agents;
  unit tests get a per-test store instead of falling back to the default path. `FORGE_DB` still
  overrides everything, so deliberately debugging against real data is unchanged. The repo's own
  MCP configuration and the quota probe's child process, which each reached the real store by a
  different route, are fixed alongside.

- **A store written by a newer Forge now explains how to recover.** The error said only to run
  `forge doctor` or retry with `RUST_LOG=debug`, neither of which leads to the remedy. It now names
  it: install a build at least as new as the store, and use `forge doctor` to see the store path and
  both schema versions.

- **`forge serve` stops retrying a failure that retrying cannot fix.** A permanent configuration
  error previously restarted forever, roughly every ten seconds, silently. It now exits in a way
  systemd can recognise as terminal, so the service stops and shows as failed rather than looping
  invisibly. The fleet is also restored after a daemon restart instead of being lost.

- Each usage row records the model the mesh actually routed to, so cost and token attribution
  reflect what ran rather than what was requested.

- A provider error carrying a status code inside a longer number is no longer misread as that
  status — the classifier matched digits anywhere in the string, so an unrelated identifier could be
  mistaken for a rate limit or a permanent capability failure and change failover behaviour.

- Desktop: AppImage users get native Wayland instead of XWayland; the changelog sheet no longer
  covers the connect flow; and a local desktop build no longer ships a stale pinned version.

- Mobile `lucide` barrel imports are rewritten to per-icon files, cutting the entry bundle that owns
  a large share of cold start.

- The voice recorder retries a spawn that fails with `ETXTBSY` instead of losing the take.

- The TUI styles only the zoomed row's transcript rather than restyling every row each frame.

- Production iOS OTA safety now checks the complete range from the installed native archive's
  source commit instead of only the latest push. A native change followed by a source-only push can
  no longer publish a combined bundle under the old runtime fingerprint; push, release, recovery,
  and reconciliation dispatches all use the same non-bypassable baseline.


### Fixed

- **A first run with no credentials now says so.** Starting Forge without any provider configured
  ended in "every routed/fallback model is rate-limited or down" — a sentence that was simply untrue
  and sent new users to check quotas and status pages for a problem that was a missing login. The
  verdict is now derived from what actually happened in the failover chain, so a chain that failed
  for lack of credentials says exactly that and points at `forge setup`. The same change stops the
  chain re-offering a model it already failed earlier in the same turn as its "last resort", and
  stops ranking local Ollama models that are not actually pulled (#1175).

- **Forge no longer refuses to write when the task is *about* not writing.** Whether a turn could
  modify files was decided by substring-scanning the prompt for phrases like "read-only". Any
  occurrence anywhere stripped every mutating tool for the whole turn, so a task whose *subject* was
  restricted-write behaviour could not write — describing the behaviour was indistinguishable from
  commanding it. It failed worst exactly when Forge was asked to work on its own permission
  machinery, and it failed expensively: a full context window spent producing an analysis instead of
  a patch, reported as a successful turn. Bare tokens must now open a segment to count as a
  directive, while genuine imperatives still bind anywhere. The same change stops a parked CLI-bridge
  child being reused across permission modes, where it would keep enforcing the temper it started in
  (#1178).

- **MCP server registration is no longer bridged over the Anywhere relay.** `POST /api/mcp` accepted
  a caller-supplied stdio command with no validation of the command itself, and persisted it to
  `.forge/mcp.toml` — persistent code execution reachable from outside the LAN, gated only by device
  enrollment. Git over the same relay has always been deliberately read-only on the reasoning that a
  commit is durable and attributable; registering an arbitrary command is strictly more powerful and
  equally durable, so it is now refused the same way — structurally, with a regression test that
  fails if the route is re-added. Configuring MCP servers from the machine itself is unchanged
  (#1176).

- **The Anywhere connector reconnects, and its health checks tell the truth.** A dead relay link
  produced no reconnect attempt, no error and no log line — one "connector online" message at
  startup and then fifteen hours of silence, while the phone showed an empty session list. Both
  `forge anywhere status` and `forge anywhere doctor` reported "healthy" throughout, because they
  measured registration state and a local process rather than the live link. Dead links are now
  detected and reconnected with bounded backoff, both commands report time since the last successful
  relay exchange, and connection state changes are logged (#1179).

- **A CLI-bridge crash no longer benches your whole subscription.** A transient non-zero exit with
  stderr was classified as an authentication failure because the advice text Forge appends ("is it
  authenticated?") was being fed to the classifier as evidence. Auth failures are permanent, so one
  segfault or network blip excluded claude-cli, codex-cli and agy-cli from routing for a full day,
  silently degrading every later turn while the subscription was perfectly healthy. Classification
  now uses the process's real stderr (#1169).

- **The agy bridge works again.** Forge built agy's command line as `agy -p --dangerously-skip-permissions <prompt>`;
  agy binds `-p` to the next token, so it took the flag as the prompt, dropped the real prompt and
  exited 2. Every agy call had been failing, and it was reported as an authentication problem, so the
  suggested fix — log in again — could never help. The prompt is now attached to the flag, a non-zero
  exit is classified from real stderr rather than defaulting to auth, and the exact argv each bridge
  constructs is now asserted by tests (#1177).

- **Project-scope skills are gated on the model path too.** Skills that arrive with a cloned
  repository are attacker-authored. The interactive `/skill` path already required project trust; the
  model-facing path did not, so a bridged session would advertise and load them regardless (#1168).

- **The mobile approval inbox shows pending device requests.** Every entry was rejected with
  "unsupported pairing version". The listing endpoint deliberately returns only a device name, an
  expiry and a safe reference — the raw pairing id and keys are withheld precisely so a stolen
  account token cannot approve a device on its own — but the app validated entries as though they
  were full pairing details, so no request could ever be displayed. The inbox now matches the real
  contract, explains that approval needs the challenge shown on the waiting device, and names the
  actual invalid field when validation does fail. Strict validation on the scanned-QR path, where
  full details genuinely arrive, is unchanged (#1181).

- **Sessions hosted by the daemon can be given a permission mode.** `POST /api/sessions` ignored any
  mode and there was no endpoint to change one, so a headless session ran at whatever the config
  resolved to — typically "ask before every write", with nobody present to answer. Creation now
  accepts a mode, `POST /api/sessions/{id}/mode` changes it on a live session, and the mode appears
  in session listings and snapshots so remote clients can see it. Remote protocol bumped to v10
  (#1180).

- **Lattice stops asserting stale code and failed parses as fact** (#1170), **the shell refuses to
  run unconfined when a sandbox was requested but is unavailable** (#1171), **a runtime "always
  allow" is scoped to the command actually approved** rather than matching everything (#1172), **a
  failed heartbeat replacement no longer leaves a session with none** (#1173), and **live-event
  appends retry through a writer stall instead of being dropped** (#1174).

## [2.12.2] - 2026-07-30

### Fixed

- The crates.io `forge-agent` tarball now carries a byte-identical package-local changelog and a
  CI packaging guard, so its compiled `/api/changelog` feed no longer reaches outside the package
  and clean registry verification succeeds. The broken `forge-agent@2.12.1` binary package was
  correctly withheld; its already-published dependency crates remain valid.

## [2.12.1] - 2026-07-30

### Changed

- Main-branch governance now strictly requires the aggregate `CI`, `mobile checks`, and
  `security checks` results from the current branch, with no bypass actors
  (`CONTRIBUTING.md` and repository ruleset 17796318).

### Fixed

- Persistent CI and release-runner storage is bounded after every relevant job: aggregate Cargo
  targets are capped at 24 GiB, mobile `node_modules` at 4 GiB, and the exact allowlisted release
  Docker volumes at 24 GiB, with dry-run and destructive-behavior regression coverage
  (`scripts/ci/trim-runner-cache.sh` and workflow wiring).
- Forge Anywhere prunes acknowledged superseded local revisions and terminal remote staging rows
  after successful sync while retaining pending uploads, newest anchors, conflicts, cursors, and
  materialized data (`crates/forge-store/src/sync_journal.rs`).
- The mobile production graph pins patched `brace-expansion` and now fails the required mobile gate
  on high-severity production advisories; its lockfile is compatible with CI's npm 10 resolver
  (`mobile/package.json`, `mobile/package-lock.json`, and `mobile-typecheck.yml`).
- crates.io publication now covers all 16 publishable Forge crates in dependency order, including
  `forge-agent-anywhere-protocol`, and publishes the exact vendored provider fork as
  `forge-agent-genai@0.6.5-forge.1` instead of silently falling back to unpatched upstream source
  (`docs/RELEASING-crates.md` and `scripts/ci/test-crates-release-order.sh`).

## [2.12.0] - 2026-07-30

### Added

- **Forge has a logo.** Every icon the product shipped was the stock Expo placeholder — the blue "A"
  on the iOS app icon, the Android adaptive icon, the desktop bundles, the PWA manifest and the
  favicon, and the grid-and-circles placeholder on both splash screens, which is the "stock Expo
  flash" visible on every cold start. The only real mark lived in `docs/` and no app target used it.
  The new mark is a pair of tongs closing on a billet at welding heat; reduced to its silhouette it
  also reads as `< >`, so it means smithing and code at once. Symmetric, flat single colour, and it
  holds its shape down to 16px.
- **One vector source for every icon.** `scripts/brand/forge-mark.svg` is now the only place the mark
  is drawn, and `scripts/gen-brand-assets.py` renders all 25 shipped assets from it: iOS app icon,
  Android foreground/background/monochrome, both splash marks plus the six committed native splash
  images, the web manifest icons, two favicons, a multi-size `.ico` and a `.icns` (written directly,
  since `iconutil` is macOS-only). There was previously no vector source anywhere and each target
  carried its own hand-placed PNG, so changing the logo meant finding them all and missing some.
  `scripts/gen-splash-light-variant.py` is superseded and removed.
- The web root now serves a real `favicon.ico`. Browsers request `/favicon.ico` unprompted and there
  was nothing there.

- **Reproducible, history-safe benchmark cells for Codex, Claude, and full-mesh routing.** The
  harnesses now recreate exact source trees, gate model/effort/CLI identity, include child-session
  usage, preserve superseded attempts, and publish official-evaluator plus quota/integrity evidence.
  The matched July samples retain the important caveat: they are evidence for those tasks, models,
  hosts, and dates, not population-wide performance estimates.

### Changed

- **Single coding tasks stay direct and recursive delegation is opt-in.** Completeness, named-API,
  and migration guidance is stronger without paying for redundant orchestration or repeated audits;
  failed environment setup is bounded and child-session cost is included in benchmark accounting.
- **Claude's persistent bridge is stricter and more resilient.** Authoritative model discovery,
  bounded tool aliases, MCP readiness, partial-message deduplication, safe no-replay behavior, and a
  bounded extra idle window for known long-running tools make subscription-backed Claude sessions
  less prone to stalls, duplicate activity, or silent capability drift.
- **Long mesh sessions retain quality with less repeated context.** Complex task-defining turns get
  a usable quality anchor, continuations keep controlled diversification, verified session/model/
  account boundaries can reuse provider prefixes and Codex response chains, completed tool logs are
  pruned, and task-list bookkeeping no longer consumes an independent model round trip.
- **Runtime ownership is split behind narrower internal boundaries.** Core, Mesh, Store, CLI, TUI,
  Tools, Config, Provider, Anywhere, and Serve now use cohesive private modules, with no
  implementation owner above 5,000 lines. This is an architecture improvement, not a claim that the
  longer-term file-size distribution or numerical coverage targets have been reached.
- Auto-merge reconciliation now observes completed workflows instead of depending on events GitHub
  can drop, while still requiring the protected aggregate `CI` gate for code-bearing changes.

### Fixed

- **Rust Analyzer can no longer create an unbounded workstation burst.** Forge permits one live
  analyzer tree process-wide, uses a one-worker/one-Cargo-job lightweight profile, enforces a
  configurable aggregate RSS guard (2 GiB by default), reaps idle servers after 120 seconds, keeps
  healthy timed-out servers warm, and rejects diagnostics for stale document versions. A real
  workspace probe reduced the observed peak from 3.7 GiB/37 processes/about 14 cores to
  1675.7 MiB/four processes/about one core while still finding an injected Rust type error.
- Long-running sessions now handle queued steering, interruption cleanup, stale completion markers,
  context fitting, cancellation rollback, stream snapshots, and provider reconnect/recovery without
  advancing the wrong turn, repeating activity, or retaining detached work.
- OAuth pasted callbacks preserve CSRF-state validation; explicit model pins survive reservation
  pressure; context windows no longer borrow unrelated provider metadata; and usage-store failures
  no longer become plausible zero values.
- Serve now aborts timed-out or dropped drivers, prunes unexpectedly completed drivers, performs
  bounded shutdown joins, preserves malformed MCP catalogs during mutation, rejects project-path
  ambiguity and symlink escapes, serializes configuration writes, and includes stored pricing in
  model projections.
- Queue repository validation, gate exits and failed-task branches, Assay semantics, MCP dynamic
  registration/device-flow separation, Claude import policy and error propagation, Codex alias
  freshness, Gemini classification, and TypeScript protocol parity were corrected.
- Tauri desktop icons are generated as RGBA PNGs, so `tauri::generate_context!` accepts the shared
  brand assets instead of failing release builds on RGB-only icons.

## [2.11.0] - 2026-07-27

### Added

- **CI now reconciles what devices are running against what main contains.** Twice — #890 and #910 —
  a merge to main created no workflow run at all, so no OTA was published and nothing said so; both
  were found days later by a human noticing the fix had not arrived. A missing run cannot be caught
  by anything keyed off that run, so a scheduled job now works the other end: it finds the newest
  commit touching the OTA-safe paths, checks whether any successful `eas-update` run covers it (by
  ancestry, since a push's head can be a later commit than the change itself), and dispatches the
  publish for exactly the uncovered range if not. It reconciles against runs rather than the Expo
  update list because a missing run is precisely the defect, and it passes the range as `base_ref`
  so the existing OTA-safety guard still decides what may ship.
- **The app now says when it has updated, and what changed.** An OTA is applied silently on the
  launch after it downloads and a TestFlight build arrives with nothing in-app to mark it, so "did it
  actually update?" was unanswerable without reading CI. A sheet now appears once per update with the
  newest changelog section in it, distinguishing a native build from an OTA — a build that also
  brings an OTA is reported as one event, not two, because that is what the user experienced. A fresh
  install stays silent: there is no version it came from. The decision lives in `updateNotice()` as a
  pure function of running-versus-last-seen, so it is testable without a device, and the seen build
  is recorded when the sheet appears rather than when it is dismissed — a sheet swiped away is still
  a sheet that was seen. The changelog is read from the daemon, so with no server paired it says so
  rather than showing an empty panel.

- **Tabs page under the finger.** Dragging horizontally on Fleet / Inbox / History / Settings moves
  the content with the drag and peeks the neighbouring tab in behind it; releasing either springs back
  or completes, and dragging past the first or last tab resists instead of stopping dead. The bottom
  bar is no longer the only way across, and it is still the real one — `RNSTabBarController` is a
  genuine `UITabBarController`, so Liquid Glass, scroll-to-minimize and native badges are untouched
  and nothing about the bar is reimplemented in JS.

  The pager is a horizontal `ScrollView` with `pagingEnabled`, and that choice is the feature rather
  than an implementation detail. `canCancelContentTouches` means the moment the scroll view decides it
  is scrolling it **cancels the touches it has already delivered to its subviews**, so dragging across
  a row cannot press it. A first attempt drove the translation from a hand-rolled `Gesture.Pan` and
  could not achieve that: a horizontal drag across a full-width row stays inside that row's hit rect,
  so RN's `Pressable` kept the press and fired it on release — swiping History → Settings opened
  History's "Resume this session?" dialog, which then floated over Settings. There is no way to take
  that press back after the fact. `directionalLockEnabled` settles the vertical axis with the same
  owner instead of arm-wrestling the list, and paging supplies the peek, the rubber-band at the ends
  and the settle from the platform's own physics rather than from numbers picked by hand.

  Because a `UITabBarController` only keeps the SELECTED child's view laid out, the pager is rendered
  *by each tab route* rather than around the navigator, so a peeked neighbour is a second instance of
  that screen. Two consequences follow and are deliberate: neighbours mount once the tab is settled
  and then stay mounted — mounting them at the start of a drag was too late, since a state update plus
  a lazy import cannot finish inside a quick swipe and the neighbour slid past empty, while dropping
  them on blur put a whole screen mount on the exact frame a tab became visible — and they render as
  peeks (`useIsPeeking`), showing cached data and asking the network for nothing, because a screen
  sliding past under a thumb is not an arrival.

  Guarded by assertions that were each verified to fail when deliberately broken: `TAB_SWIPE_ORDER`
  matches the tab bar's declaration order in both navigators, each route passes the index its position
  implies — a wrapper wired to the wrong number would page to the wrong tab while looking perfectly
  correct in the bar — and `pagerGeometry` always pins a content width the resting page fits inside,
  which is what keeps the scroll view from clamping the offset onto the wrong tab.

### Changed

- Patched the four open high-severity advisories in the build toolchain: all ten transitive copies of
  `brace-expansion` in the mobile lockfile (to 1.1.16 / 2.1.2 / 5.0.8) and `fast-uri` in the promo
  video pipeline (3.1.4). None of them is reachable from the app bundle or the daemon — they hang off
  eslint, sucrase, `@expo/prebuild-config` and `@bacons/apple-targets` — so this clears noise rather
  than exposure. The mobile lockfile is regenerated with npm 10, which is what CI's `npm ci` reads;
  npm 12 prunes entries it needs.
- **Tab swiping no longer peeks; it switches immediately.** The peek rendered a second live instance
  of the neighbouring screen inside the current tab, because a `UITabBarController` only keeps the
  selected child's view laid out and iOS keeps its real tab bar here. That leaked in every direction,
  and a screen recording caught the worst of it: a horizontal drag across a full-width row stays
  inside that row's hit rect, so RN's `Pressable` retained the press and fired it on release —
  swiping History → Settings opened History's "Resume this session?" confirm dialog, which then
  floated **over** the Settings tab, because a peek that stays mounted keeps its state alive in a tab
  it does not belong to. Duplicate fetches and loading states from screens never navigated to, and a
  one-frame light flash at the handover, came from the same place. Each was fixable alone and the
  next appeared; they share one cause, which is a screen rendered outside the tab that owns it. A
  faithful interactive transition needs the platform to own it — a horizontal `ScrollView` with
  `pagingEnabled`, whose UIScrollView cancels touches in its subviews the moment it scrolls, or an
  interactive `UITabBarController` transition in Swift. The swipe, its thresholds and the absent
  arrival haptic all stay.

### Fixed

- Three `forge-index` watcher tests raced the watch they were testing. Each made its external edit
  once, immediately after `spawn_watcher` returned — but registration happens on the watcher's own
  thread, so the write could land before any watch existed, produce no event, and then no amount of
  polling could recover it (the polling backend has the same shape: an edit made before its first
  scan is simply part of the baseline). Alone the gap is too small to notice; run beside each other
  under load, as `cargo test --workspace` does, and it was wide enough to fail the release gate. The
  edit now repeats until the watch picks it up, which tests what the tests meant to test without
  weakening either assertion.
- **One stale frame from a phone took the whole Anywhere connector offline.** The host's list of open
  session streams is per relay connection, and that connection drops and reconnects on its own —
  three times in the last two days' logs, from resets and heartbeats. The phone's socket survives
  those drops, so it goes on sending frames for streams the reconnected host has no record of, and
  the host treated an unknown stream id as fatal: it tore down the connector, reconnected, and died
  again on the next frame. Every bridge request in those windows failed against a connector that had
  just reported itself online. An unknown stream is a race, not an attack, so the host now answers it
  with a close — telling the phone to stop using that socket — and keeps serving everything else.
- **Voice over Anywhere timed out on anything but a short clip.** The relay applied a flat 30s
  deadline to every bridge request, overriding whatever the caller asked for, so
  `transcribeAudio`'s 120s budget was never in effect. A bridge request is not a proxy hop: the host
  transcribes the entire clip before it answers — measured at ~4.5s for a 4s recording — so a voice
  memo of any real length could not come back in time. The caller's `AbortSignal` now reaches the
  relay and governs, which also means cancelling a recording actually cancels the request; the relay
  keeps a deadline of its own only for callers that set none.
- **The tab you swiped away from no longer flashes when you arrive.** Four previous attempts moved a
  corrective scroll earlier and earlier — onto a 150ms timer, then into a layout effect on arrival —
  and each one made the flash shorter without removing it. Shortening it was the clue: the correction
  was racing something rather than preventing it. A `UIScrollView` clamps its `contentOffset` into its
  `contentSize`, and the pager's content width was left to be measured from its pages, so any layout
  pass that measured it short pulled the offset to zero — and page zero is the neighbour on the LEFT,
  which is the tab you just swiped away from. The clamp happens inside layout, earlier than any scroll
  JS can schedule, which is why no delay could have been the answer. The content width is now pinned
  from the page count, so there is no pass in which it is too narrow and no clamp to recover from. The
  geometry moved to `pagerGeometry` in `lib/tabSwipe.ts`, where the invariant that makes the clamp
  impossible — the resting page fits inside the pinned width, for every tab — is asserted rather than
  reasoned about, along with the page-to-tab arithmetic that was previously inline and untested. The
  timer is gone; the pager now returns home on focus change, which needs no tuning and covers leaving
  a tab in the same commit that switches away from it.
- **A light grey flashed through the tab pager.** Frame analysis of a screen recording found it four
  times in 18.7s: once filling the whole content area for a single 17ms frame at the moment a swipe
  committed, and otherwise as light strips down the incoming edge mid-drag, 6–20% of the area for
  33–100ms. The colour is a uniform `#F0F0F0` — measured exactly, against pure white reading 255 in
  the same frame, so neither white nor the light theme's `#F5F4F1` — which makes it a native surface
  behind the JS view rather than anything the app draws. The pager's clipping container had no
  background of its own, so any moment it had not filled fell through to it: the frame between one
  tab's screen unmounting and the next one painting, and the first frames of a drag before the
  neighbour had mounted. It now paints `bg0`, so an unfilled moment reads as the app.
- **A swipe sometimes scrolled the page instead of changing tabs.** The pager needed 28px of
  horizontal travel before 15px of vertical cancelled it — a ratio of nearly 2:1 just to begin, and a
  thumb swipe arcs, so an ordinary curved swipe crossed the vertical limit first and the list
  underneath took the drag. Now 20px horizontal against 24px vertical, so intent that is even roughly
  sideways pages while a genuinely vertical drag still scrolls. 20px stays clear of SessionCard's
  10px archive pan, which is a descendant and so still claims a card drag first. The thresholds moved
  to `lib/tabGesture.ts` where the relationships between them are asserted, since the ratio — not
  either number alone — is what a thumb experiences.
- Tab neighbours now mount on the first drag and stay mounted, rather than unmounting on every
  settle. Each swipe previously waited on `runOnJS` → `setState` → render before the neighbour
  existed, which is what left the incoming edge empty at the start of a drag.

- **The app failed to open** — `undefined is not a function`, caught by the root error boundary — on
  the OTA that added tab-peek preloading. Preloading resolved all four tab route modules from an
  effect when the pager mounted, to spend `React.lazy`'s promise frame before a drag rather than
  during one. That was the only thing in that release which ran at app OPEN (the peek-aware refetch
  and the dropped haptic can only take effect during a drag), and forcing four route modules to
  evaluate inside the first mount — `settings.tsx` alone pulls some 25 local modules — reorders
  initialisation for the whole app. Peeks resolve on first use again, which keeps module evaluation
  where the code expects it: after the tree those modules depend on exists. The flash preloading was
  meant to fix is covered by the opaque Suspense fallback, which cannot reorder anything.

- **Swiping between tabs flashed, buzzed and reloaded.** Three separate causes behind one symptom:
  the peeked neighbour resolved its module through `React.lazy` *during* the drag, and Metro's
  resolution costs a frame — visible as a blank page at the moment the swipe starts, so peeks are now
  preloaded once the pager mounts; `useSessions` sets `refetchOnMount: "always"`, and a peek is a
  second mount, so every swipe between Fleet and Inbox fired a round trip mid-gesture and another one
  when the real screen arrived; and the commit fired a selection haptic, which nothing on a native
  tab bar does when you tap it. A peek is now marked as such (`useIsPeeking`) and asks the network
  for nothing, renders whatever is already cached, and no longer mirrors itself into the home-screen
  widget. `sessionsRefetchPolicy` carries that decision as a plain function so it is testable without
  a renderer — the original bug was invisible to every unit test and only showed up as a flicker on a
  phone. The Suspense fallback is also opaque now rather than transparent, since a see-through gap
  during a drag reads as a flicker of its own.

- **A PR with every check green and auto-merge armed could sit unmergeable forever.** Two settings
  combine into a deadlock: the `Protection branch` ruleset sets
  `strict_required_status_checks_policy: true`, so a branch must be up to date with `main`, while the
  repository has `allow_update_branch: false`, so GitHub never updates it. The PR parks at
  `mergeStateStatus: BEHIND` and stays there until a human runs `gh pr update-branch` — and it looks
  entirely healthy in the meantime, which is why it recurred unnoticed: #891 (left Homebrew pointing
  at a v2.10.0 that was never published), #894 (package managers stranded on 2.9.1 while 2.10.1 was
  current), #897 (an ordinary fix PR, so this was never specific to `dist/`), and #899. A new
  `unstick-automerge.yml` runs on every push to `main` — the moment PRs become BEHIND — updates any
  stalled auto-merge PR, and re-dispatches the required workflows afterwards, because a
  GITHUB_TOKEN push creates no `pull_request` runs and the new head SHA would otherwise carry no
  `CI` context and stall one state later.
- **Corrects the 2.10.1 note below**, which blamed this on `workflow_dispatch` runs not satisfying
  branch protection and on `pull_request` runs stuck at `action_required`. That was wrong. The
  ruleset requires exactly one context, `CI`, and the dispatched run does provide it — verified
  green on #899's stuck head SHA `af8371dc`, which was `BEHIND` and nothing else. The
  `action_required` runs were real but irrelevant.

## [2.10.2] - 2026-07-26

### Fixed

- **iOS notifications could not be enabled at all**, and the app blamed a permission the user had
  already granted. Every Xcode Cloud archive shipped `aps-environment: development`:
  expo-notifications' plugin writes that value, EAS Build rewrites it to `production` from the
  signing credentials, and Xcode Cloud performs no such rewrite. A distribution-signed app carrying
  a development entitlement installs and shows its permission as granted in iOS Settings, but
  cannot register with production APNs — `getDevicePushTokenAsync()` rejected and the toggle snapped
  back. The earlier fix for this was a hand-edit to the generated Xcode project pointing Release at
  a separate `Forge.Release.entitlements`; `expo prebuild` deletes that file and repoints Release
  back on every build, so it never reached an archive. Declared in `app.config.ts` instead (verified
  to survive prebuild), with `scripts/ci/verify-ios-entitlements.sh` failing the build if it ever
  regresses.
- **A push registration failure was indistinguishable from a declined permission.** The toggle had
  two channels — a resolved non-subscribed state meaning "user declined", a thrown error meaning
  "the network call failed" — and an OS-level registration refusal collapsed into the first, which
  is what made the entitlement bug undiagnosable from the device. It now raises
  `PushRegistrationError` and both toggles report it accurately; the Anywhere screen previously had
  no `catch` at all, so a throw there became an unhandled rejection.

- **The light-theme splash mark was effectively invisible.** `splash-icon.png` is inked in a light
  gray for the dark splash background; composited on the light one it measured **1.41:1**. The light
  variant now gets its own asset, the same geometry re-inked in `lightTokens.ink` at **15.65:1**,
  matching the dark pairing's 14.69:1. Both marks are baked in at prebuild, so this can only reach a
  device via a native build — regenerate with `scripts/gen-splash-light-variant.py`.
- **`chore(dist)` package-manifest PRs could never merge**, so Homebrew, AUR, and Scoop stayed on
  **2.9.1** while 2.10.1 was the published release. `release.yml` dispatches the required workflows
  against the manifest branch to work around GitHub's anti-recursion guard, but those
  `workflow_dispatch` runs do not satisfy branch protection — the `pull_request` runs it needs sit at
  `action_required` awaiting manual approval, forever.

- **A mobile fix could reach `main` and never reach a phone, silently.** The iOS OTA fired only on
  a push touching `mobile/src/**` or `mobile/assets/**`, and refusing to publish wrote a line to the
  step summary — which nobody opens — so the run went green either way. #890 is the concrete case:
  it changed 17 files under `mobile/src/`, no `eas-update` run was ever created for its merge commit,
  and the OTA had to be dispatched by hand hours later. Three changes close it: `release.yml` now
  hands off to `eas-update.yml` after publishing, so every release either ships the matching OTA or
  states why it cannot; a refusal emits a `::warning` naming the owed native build instead of a
  buried summary line; and the push filter widened to `mobile/**` so a native-config-only change
  produces a loud "native build required" run rather than nothing at all. The release hand-off
  passes `base_ref`, so it is classified by the same OTA-safety guard as a push — tying the OTA to
  releases must not become a way around the guard that stops a JS bundle expecting native code the
  installed binary does not have.
- **A Grok OAuth account hop replayed output the user had already seen.** `xai_oauth.rs` shared
  `codex_oauth.rs`'s hop-replay defect and not merely a resembling one: both route through
  `oauth_responses.rs`, where an in-stream `error`/`response.failed` SSE event is classified by
  message text into `Unavailable`/`RateLimited` — both of which `should_hop_account` accepts. A
  mid-stream failure after partial visible output therefore hopped to the next account, which
  re-streamed the reply from the top into the same sink. The hop guard now also requires that
  nothing visible has been streamed yet.
- **The workflow view re-wrapped its zoomed transcript twice on every frame** — once to measure
  `wrapped_len`, then again inside `transcript_lines`, with no memoisation. Cached on
  `(selected, wrap_width, revision)`, matching the activity viewer.
- **`Up` did nothing in a freshly opened activity viewer.** The scroll offset starts at a
  `usize::MAX / 2` tail sentinel, and the clamp that keeps the view at the bottom was applied to a
  local copy that was never written back — so `Up` decremented 9223372036854775807 and the view did
  not move. `Up`/`k`/`PageUp` now pin the offset to the real tail before scrolling.
- **Git review over Forge Anywhere failed 100% of the time behind a retry button that could never
  succeed.** `/api/git/*` is not in the Anywhere bridge's `RouteId` enum, so every request returned
  the raw `Forge Anywhere route is not allowlisted: GET /api/git/status` as though it were transient.
  The dock now detects the relay up front and explains that git review needs a direct connection.
  (Unlike the terminal dock, this never crashed: the git hooks reject a Promise where the terminal
  threw synchronously out of a `useEffect`.)
- **Forge Anywhere's notification toggle drove the wrong backend.** Settings → Forge Anywhere →
  Notifications called `lib/push` — the local-daemon subscription, `POST /api/push/subscribe`
  against the paired server's `baseUrl` — while the Anywhere hub row one screen up has always read
  the relay's `pushStatus`. So the toggle was disabled outright with no paired server, and otherwise
  reached for the daemon over the LAN: away from it, which is the situation Anywhere exists for, it
  failed with a reachability error and snapped back. It now registers through
  `AnywhereProvider.enablePush` (`POST /v1/push/subscriptions`), which needs no `baseUrl`.
  `enablePush`/`disablePush` also resolve with the resulting status instead of `void`, because
  `denied` is an ordinary outcome rather than a thrown error and the screen has to tell it from
  success.
- **Voice over Forge Anywhere was refused by the relay for most clip lengths.** Bodies do not travel
  as bytes: `body`/`bytes` are a JSON array of decimal literals — `Array.from(uint8array)` on the
  client, `Vec<u8>` through serde on the host — which measures at **3.57x** for random bytes. The
  relay caps an envelope at 300 KiB, and both sides offloaded to a blob only above 256 KiB, which
  seals into 1,048,896 bytes. Everything from roughly 84 KiB to 256 KiB was rejected with
  `relay_frame_too_large`, while both smaller **and larger** bodies went through; at 32 KB/s for
  16 kHz mono WAV that is every voice clip between about 2.7s and 8s. The single threshold is now
  two on both sides: still accept 256 KiB inline, since a peer on an older build offloads at its own
  threshold, but send inline only up to 64 KiB. The client test asserted that a 256 KiB body stays
  inline — true of the code and rejected by the real relay every time; it now asserts that the
  sealed envelope fits through the relay.
- **The Live Activity's forging card did not match its own design frame.** The frame draws one row —
  a dot, `title — forging 4m`, then `2/4 · 64% · $1.84` pinned right — and it was built as two
  stacked rows. A `Text` in `.timer` style reserves the width of the widest value it could ever
  show rather than of what it shows now, so inside one flat run of `HStack` items that reserve
  pushed everything after it to the trailing edge, leaving a stranded separator against the margin
  with a hole mid-row. `contextLimit == 0` means the host never reported a context window, and the
  `0% ctx` that fell out of it read as a measurement; segments now appear only when they mean
  something.
- **An untitled session's Live Activity read "Forge session" for its entire life.** `title` lives on
  `Attributes`, which ActivityKit freezes at `Activity.request()`, so an activity started on the
  idle→busy edge of a session that has not yet produced a title can never be corrected. `agentLabel`
  (the session's cwd) was already carried in `Attributes` and never rendered — it is now the
  fallback.

## [2.10.1] - 2026-07-25

Ships everything listed under 2.10.0 below. **2.10.0 was tagged but never published**: its
Windows desktop bundle failed to build, and the release-publication guard added in that same
version correctly refused to publish a release missing an installer-required asset, so it stayed
an unpublished draft rather than becoming a broken "latest". This is that release, with the
Windows failure fixed.

### Fixed

- **Every Windows desktop build failed in `expo export`** with `SyntaxError: Invalid regular
  expression … Unterminated character class`, which failed the whole five-platform desktop
  release. The `src-tauri/target` exclusion rebuilt `config.resolver.blockList` by passing it
  through `exclusionList()`, but Expo's default is already a finished array of RegExps — one
  being `/^(?:android[\\/]app[\\/]build|…)$/` — and `exclusionList` rewrites `\/` to
  `\` + `path.sep` in every pattern's source. On Windows that turns each `[\\/]` separator class
  into `[\\\]`, so merely requiring `metro.config.js` threw before any bundling began. On Linux
  and macOS `path.sep` is `/`, the rewrite is a no-op, and the defect is invisible — which is why
  it passed every local check and every non-Windows job (`mobile/metro.config.js`).

## [2.10.0] - 2026-07-25

### Added

- Claude Opus 5 (released 2026-07-24) is now a first-class model across the Model Mesh: bundled pricing, benchmark-backed ranking, frontier capability class, and the default Complex-tier model. Claude Sonnet 5 and Haiku 4.5 were also missing bundled prices — an unpriced model costs `$0.0`, so cost-tiered routing was treating the most expensive frontier models as the cheapest candidates (`crates/forge-mesh/src/pricing.rs`, `bench.rs`, `catalog.rs`, `crates/forge-config/src/lib.rs`, `crates/forge-core/src/lib.rs`).
- `forge lattice roots`, `forge lattice prune <root> [--vacuum]` and `forge lattice prune --stale` — there was previously no supported way to remove an unwanted index root or reclaim its space (`crates/forge-cli/src/cli/commands/lattice.rs`).

### Fixed

- **Shell permission floor was bypassable by a newline.** The built-in, documented-as-unoverridable denylist split commands only on `;`, `&&`, `||`, `|` and `&`, so `git status\nrm -rf ~` stayed one segment, collapsed to a single flat string, matched no anchored pattern, and executed both statements. Newlines and carriage returns are now statement separators, backslash line-continuations stay one command, `$( … )`/backtick substitutions are recursed into, and heredoc bodies are treated as data unless fed to a shell. The remaining evasion classes (variable indirection, `eval`, base64-staged scripts, `xargs`/`find -exec`/`ssh` wrappers) are documented in the source: this is a floor against accidents and obvious mistakes, not a sandbox (`crates/forge-core/src/permission.rs`).
- **The Lattice indexed the entire home directory.** `forge run`/`forge chat` built an index on the launch directory with no root check, so starting a session in `$HOME` walked the whole tree — 80,117 files and 1.44M symbols, 2.2 GB of Go-toolchain and Android-SDK symbols that were 77% of the database. The file watcher already refused `$HOME` and said so, while the indexer silently proceeded. Indexing now refuses the home directory and its ancestors outright, applies a file-count ceiling before writing any row, and skips toolchain/SDK trees (`crates/forge-index/src/root.rs`, `lib.rs`, `watch.rs`, `crates/forge-cli/src/cli/commands/run.rs`).
- **Every Anthropic model was budgeted at the conservative 32k floor.** The `/v1/models` parser read a `context_window` field that the API does not return; the correct field is `max_input_tokens`. A 1M-token Opus was being treated as 32k for context assembly and trimming. The old unit test passed only because its fixture used the non-existent key (`crates/forge-cli/src/context_windows.rs`).
- **Desktop releases could publish with no desktop artifacts at all.** `app-desktop.yml` is dispatched by the last step of the release job, which sat below the package-manager manifest work — so a single failure in that chain (run 29831376887 died on `gh workflow run codeql.yml` with HTTP 422) meant desktop bundles, `desktop-checksums.txt` and `latest.json` were never produced, and the documented installer aborted. The release is now published as a draft, the desktop/web dispatch runs immediately after, manifest bookkeeping moved to its own `needs: release` job, and the draft is flipped to published only once every installer-required asset exists. Manifest generation now enumerates all five platforms and fails closed — the two `.dmg` files were previously covered by nothing (`.github/workflows/release.yml`, `app-desktop.yml`, `ci.yml`).
- The release-publication CI guard (`scripts/ci/test-desktop-release-publication.sh`) existed but was wired into no workflow, so it never ran. It is now part of CI and additionally asserts the draft handoff, publish ordering, and installer↔manifest filename parity.
- Forge Voice and every file attachment failed before leaving the device when connected through Forge Anywhere on iOS/Android. The transport serialised request bodies via `new Request(...).arrayBuffer()`, which on React Native is whatwg-fetch — its `blob()` throws `could not read FormData body as blob`, and no content-type was set either. Multipart bodies are now encoded directly, handling web `Blob`/`File` and React Native byte adapters (`mobile/src/lib/transport/requestBody.ts`, `AnywhereTransport.ts`).
- The terminal dock was permanently blank in the desktop app: the Tauri WebSocket shim never set `binaryType`, so every binary PTY frame decoded to an empty string. Opening the dock while an Anywhere host was selected threw synchronously out of a React effect and took down the whole app via the root error boundary; it now explains that the terminal needs a direct connection (`mobile/src/lib/transport/index.ts`, `mobile/src/components/shell/TerminalDock.tsx`).
- Bottom sheets were hidden behind the iOS keyboard. Any sheet shorter than the keyboard — including the goal, fork, memory, checkpoint and schedule sheets — disappeared entirely when a field was focused, leaving only the scrim. Fixed once in the shared sheet so all callers benefit (`mobile/src/components/ds/Sheet.tsx`).
- The Fleet tab's task composer — the only way to start a session on mobile — was covered by the iOS keyboard, and was also positioned a full safe-area too high because the inset was counted twice (`mobile/src/app/(tabs)/index.tsx`).
- Command palette arrow-key and Enter navigation did nothing on web and desktop: the palette's own autofocused search field was treated as "editing" by the global key handler, which bailed out before reaching the navigation branches (`mobile/src/components/overlay/CommandPalette.tsx`).
- The composer's Enter-to-send and paste-image handlers were bound to a detached DOM node after a voice recording ended, because the input remounts on that transition and the effect never re-ran. Enter during IME composition no longer sends a half-typed message (`mobile/src/components/chat/Composer.tsx`).
- Mouse-wheel scrolling the transcript moved roughly 3px per notch in Firefox — the handler ignored `WheelEvent.deltaMode` — and swallowed horizontal gestures meant for nested scrollers such as code blocks (`mobile/src/components/ds/BoundedList.tsx`).
- Every transcript message was a keyboard focus stop showing a pointer cursor on web, despite having no press action (`mobile/src/components/chat/MessageRow.tsx`).
- The desktop split-pane divider was a 1px hit target with no resize cursor (`mobile/src/components/shell/SplitPanes.tsx`).
- Opening the activity transcript viewer with an empty activity list panicked and killed the session (`crates/forge-tui/src/transcript.rs`).
- Ctrl+O with a task list but no subagents captured every keystroke with no visible way out (`crates/forge-cli/src/cli/commands/run.rs`).
- Post-turn re-drives (empty-diff nudge, test-edit guard, autofix, stop-hook continuation) overwrote a completed answer with an empty string (`crates/forge-core/src/lib.rs`).
- A turn that exhausted the failover chain reported a generic cause and discarded the real provider error, hiding actionable messages such as an expired credential (`crates/forge-core/src/lib.rs`).
- Desktop Settings reported the shared mobile client version instead of the installed desktop release. It now shows both, clearly labelled (`mobile/src/app/(tabs)/settings.tsx`, `mobile/src/lib/appVersion.ts`).
- The WebSocket liveness watchdog stayed permanently disarmed on web when the page first loaded in a background tab, so a half-open socket froze the session view indefinitely (`mobile/src/lib/ws.ts`).
- `npm run lint` failed locally on generated Tauri build artifacts under the gitignored `src-tauri/target`, producing warnings nobody could act on (`mobile/eslint.config.js`).
- **A merge that failed for a hard reason archived the session anyway.** Only conflicts were special-cased; any other git failure — a failed worktree commit, a failed `merge-base`, an unparseable `git apply` error — fell through to deregister and archive a session whose worktree and branch were still fully intact, so it vanished from the fleet even though nothing had been merged. Only a clean merge archives now; anything else leaves the session running and visible with the cause reported (`crates/forge-cli/src/serve.rs`).
- **The write-ahead log was never truncated.** `journal_size_limit` was unset, so it defaulted to unlimited, and because `forge serve` holds a connection open continuously the last-close checkpoint never fired — leaving the WAL pinned at its high-water mark permanently (656 MB on the reference install). Bounded on every pooled connection (`crates/forge-store/src/lib.rs`).
- **Opening the store always wrote to it.** `Store::open` rewound `user_version` 19→17→19 on every open to force additive migrations to replay, and two unconditional `INSERT OR IGNORE` singleton seeds took the writer lock even when nothing changed — so opening a current database could fail on a read-only file or contend with the daemon. The version window is now resolved by replaying the idempotent public steps instead of rewinding, seeds read before writing, and the whole init group retries on a busy database. A database at a pre-release version is now renumbered only when a decisive marker proves it is pre-release, so a genuinely newer database is refused rather than silently downgraded (`crates/forge-store/src/lib.rs`, `schema.rs`).
- **Five Lattice queries ignored the repo root** while every structural query scoped correctly. `forge lattice status` reported every indexed project's counts as the current one's — which is plausibly why a 2.2 GB stray index went unnoticed for a month. Semantic ranking cosine-compared every root's vectors, `embed_pending` would have spent embedding calls on other projects' nodes, and `lattice_embedding_count` gates hybrid retrieval, so another project's vectors could switch semantic retrieval on for yours (`crates/forge-store/src/lib.rs`, `crates/forge-index/src/lib.rs`).
- **Fixing a bad credential did not restore the provider.** The first auth failure benched the whole provider for 24 hours, and the periodic re-probe its documentation promised did not exist — the only thing that cleared it was `forge models --probe`. Re-authenticating now clears the exclusion, while leaving rate-limit and outage benches intact, since those are not credential problems (`crates/forge-cli/src/cli/commands/local.rs`).
- **An OAuth account hop replayed a partly-streamed turn into the same sink**, so the user saw a truncated reply with a complete one appended while the transcript stored only the second. Once anything is on screen the error now surfaces and normal failover owns the retry (`crates/forge-provider/src/codex_oauth.rs`).
- **Auto-compaction reliably failed on long sessions and benched healthy models for it.** `compact()` sent the entire untrimmed older transcript to a trivial-tier summarizer, so every candidate returned a context overflow — and overflow was classified as a model fault, so each healthy cheap model was benched in turn and the routing damage outlived the failed compaction. The payload is now fitted to the summarizer's real window, eliding from the *middle* (keeping the opening task statement and the most recent state, with an explicit marker for the gap) because trimming the tail would discard exactly the messages the summary exists to replace. It is re-fitted per candidate, since the failover chain crosses models whose windows differ by orders of magnitude, and a context overflow no longer records a health penalty (`crates/forge-core/src/lib.rs`).
- **Floor and Plans were unreachable on iPhone.** Both were registered as hidden tab entries, but expo-router deletes a hidden trigger from the navigator entirely rather than keeping it addressable, and the native tab navigator throws when the focused route is one of them. They are now root-stack routes that push over the tab bar, so the URLs are unchanged and the native iOS tab bar is untouched (`mobile/src/app/floor.tsx`, `mobile/src/app/plans.tsx`, `mobile/src/app/(tabs)/_layout.tsx`), with a guard test that fails if a `(tabs)` route ever loses its trigger again.
- An explicit in-app theme override never reached native chrome, so the status bar and the iOS tab-bar material stayed on the system scheme (`mobile/src/theme/ThemeProvider.tsx`).
- "Sign out on this device" discarded the Anywhere device keys with no confirmation (`mobile/src/app/anywhere/account.tsx`).
- "Merge back" could never show its pending state, because the selection was cleared in the same tick the mutation fired (`mobile/src/app/session-tree.tsx`).
- The desktop "Forge a task" modal advertised ↵ to submit, but Enter only inserted a newline (`mobile/src/app/new-session.tsx`).
- Skills, Hooks and MCP servers rendered an empty void while loading, with no retry affordance on web where pull-to-refresh does not exist (`mobile/src/app/skills.tsx`, `hooks.tsx`, `mcp.tsx`).
- Keycap hints hardcoded the macOS ⌘ glyph, telling Windows and Linux desktop users to press a key that does nothing (`mobile/src/lib/platform.ts` and its callers).
- The splash screen, PWA manifest and Tauri window background still held pre-Machined colours, producing a visible colour step on every cold start across all six targets (`mobile/app.config.ts`, `mobile/public/manifest.webmanifest`, `mobile/src-tauri/tauri.conf.json`).

## [2.9.1] - 2026-07-24

### Fixed

- Forge Anywhere mobile and desktop now show encrypted storage usage together with the account cap and link directly to storage details and cleanup.
- Managed hosts now report live online/offline presence in the Anywhere app instead of appearing unavailable while the connector is healthy.

## [2.9.0] - 2026-07-24

### Added

- `forge run` accepts repeatable `--system` / `--append-system-prompt` instructions that reach the
  provider without polluting Model Mesh classification
  (`crates/forge-cli/src/cli/args.rs`, `crates/forge-cli/src/cli/commands/run/dispatch.rs`).
- Embedded API routing classifies the active user message plus bounded current-turn tool results
  while excluding system and developer content (`crates/forge-cli/src/api_serve.rs`,
  `crates/forge-core/src/llm_router.rs`).
- Model Mesh exposes documented fixed-classifier and activity-focused controls through configuration
  and `/config` (`crates/forge-config/src/lib.rs`, `docs/features/mesh-routing.md`).

### Changed

- Referential follow-ups such as “continue” retain the prior task complexity they depend on, while
  independent turns route from the new task instead of stale conversation complexity
  (`crates/forge-core/src/llm_router.rs`, `crates/forge-mesh/src/lib.rs`).
- LLM classification now uses a fixed capable model at zero temperature and falls back to the
  deterministic contextual heuristic instead of recursively routing the classifier itself
  (`crates/forge-core/src/llm_router.rs`).

### Fixed

- Large injected assistant, skill, and on-screen system prompts no longer promote trivial Helm,
  Forge API, or CLI requests to the complex tier (`crates/forge-cli/src/api_serve.rs`,
  `crates/forge-core/src/llm_router.rs`).
- `forge init` project setup is treated as mutating and can create missing repository guidance
  files instead of being denied by incidental read-only wording
  (`crates/forge-cli/src/cli/commands/run/dispatch.rs`, `crates/forge-core/src/turn_contract.rs`).
- Cross-platform real-provider smoke coverage again exercises a supported production route, and
  shared mobile, desktop, and web render failures emit only the existing privacy-safe anonymous
  release-health code (`.github/workflows/e2e.yml`, `mobile/src/lib/anonymousTelemetry.ts`).

## [2.8.5] - 2026-07-23

### Added

- QwenCloud Token Plan registration and API-key authentication through its OpenAI-compatible endpoint.
- Dynamic TUI progress reporting for provider requests, streamed output, tool execution, and subagent activity.
- Persistent manual end-to-end scenarios, including saved playable browser-game results.

### Changed

- Qwen 3.8 Max preview now uses its 1M-token context window and temporarily inherits Qwen 3.7 Max benchmark statistics until native results are available.
- Prompt caching is capability-aware across OpenAI-compatible APIs, Gemini, OAuth/Codex, and CLI bridges.
- Provider responses are no longer given implicit output caps, including in credit mode.

### Fixed

- Large streamed tool calls retain complete arguments instead of truncating or corrupting file writes.
- Rejected workspace paths recover cleanly instead of leaving the agent stalled.
- Long-running chat work exposes meaningful live progress instead of an indefinite generic working indicator.

## [2.8.4] - 2026-07-21

### Fixed

- Self-hosted CI now serializes every Rust-heavy job on the dedicated heavy runner and caps Cargo
  parallelism, preventing simultaneous PR checks from exhausting workstation memory and swap.
- Portable Docker release builds now enforce CPU, memory, swap, and process limits. The sandboxed
  release-runner service has matching persistent cgroup limits so a release cannot freeze its host.

## [2.8.3] - 2026-07-21

### Fixed

- Portable Linux release jobs now target the dedicated sandboxed release runner explicitly, so
  general-purpose repository runners without Docker access cannot claim and fail those builds.

## [2.8.2] - 2026-07-21

### Fixed

- Release portable Linux builds now copy the checked-out source through the Docker API, so
  containerized self-hosted runners no longer mount an empty `/workspace` and fail before Cargo
  can find the workspace manifest.
- Forge Desktop on Linux now grants WebKitGTK's audio-only user-media request, allowing Voice to
  record and append transcriptions while continuing to deny camera and unrelated permissions.

## [2.8.1] - 2026-07-21

This release completes the live acceptance follow-up for Forge Anywhere and the shared app, and
ships the session, Desktop, Voice, and release fixes found while using the new flow end to end.

### Changed
- **Stable session context** (#834, #841): provider payloads retain only the newest turn contract
  and one copy of standing guidance while the persisted audit history remains complete.
- **Stable host identity** (#840): direct and tunneled servers use the system hostname for display,
  preserve user renames across reconnects, and keep display identity separate from transport URLs.

### Fixed
- **Plan execution** (#832, #833): approval restores the exact prior permission mode, commits tasks
  only after Build, and binds controls to the matching live decision instead of stale questions.
- **Agent completion** (#835, #836, #842, #843, #844, #845): orchestration no longer calls a
  nonexistent skill-list operation; failed verification cannot be cleared by unrelated reads;
  tool-only bridge turns continue without blank replies; final answers remain singular; and
  recoverable Codex OAuth continuation failures retry automatically.
- **Desktop status and delivery** (#837, #838, #839): Settings reads the installed bundle version,
  Fleet health reflects the reachable host instead of cached pairing metadata, and releases become
  Latest only after every Desktop asset and public checksum verifies.
- **Voice** (#850): portable Linux CLI/TUI builds capture through `pw-record` with an `arecord`
  fallback, while shared clients append transcriptions to the existing draft without sending.
- **Mobile and Anywhere reliability** (#824–#829, #846–#848, #853, #860, #863): Hermes secure
  randomness, account/passkey recovery, host reconciliation, connection state, and the native
  multiline composer now remain stable through real reconnect and foreground cycles.
- **Long-running subagents** (#852, #862): terminal loop guards and completed follow-up rows no
  longer leave sessions looking active after work has stopped.
- **OTA publication** (#861): the safety guard distinguishes runtime, native/config, neutral, and
  unknown mobile changes so compatible source updates publish without being blocked by docs.

## [2.8.0] - 2026-07-20

Forge Anywhere now treats recovery words as an offline disaster-recovery fallback. Routine setup
and returning-device enrollment happen in the shared app through GitHub sign-in, device approval,
and transcript-bound safety-code comparison.

### Added
- **UI-first Anywhere setup** (#820): one resumable setup and Recovery Center across web, desktop,
  iOS, and the TUI's `/anywhere` surface. It includes automatic Approval inbox discovery, explicit
  approve/deny controls, safety-code comparison, host naming and rename, connection verification,
  device revocation, recovery-health warnings, and actionable failure states.
- **Recovery Kit v2** (#811, #820): new accounts create and verify a checksum-protected 12-word
  offline kit before first-host activation. Machine-readable `.forge-recovery` import and legacy
  24-word recovery remain supported.
- **Passkey recovery** (#820): system-browser WebAuthn PRF registration and recovery use an
  ephemeral encrypted claimant channel. PRF output, recovery entropy, Account Data Keys, and
  plaintext wraps never enter service storage, URLs, or logs.
- **Account-bound enrollment and clean reset** (#820): returning devices receive restricted
  ten-minute enrollment sessions, while total loss can schedule a cancellable seven-day encrypted
  account reset without touching local Forge data.

### Changed
- **Faster pipelines** (#820): CI, mobile, and security workflows detect affected file groups and
  run only the jobs required by each change.
- **Tunnel naming** (#811): self-hosted quick tunnels use `forge serve --tunnel`; `--anywhere`
  remains a deprecated compatibility alias while Forge Anywhere names the managed product.

### Fixed
- **Returning-device enrollment** (#818): GitHub sign-in no longer falls through to an empty
  Recovery Kit prompt; enrolled devices can approve the account-bound request normally.
- **Mobile OTA reliability** (#814, #816, #817, #819): manual production dispatch publishes again,
  Metro caches are isolated per run, and worker count is capped to prevent runner OOM failures.

## [2.7.0] - 2026-07-19

A redesigned cross-platform app, the Forge Anywhere relay surface, and the production hardening of
the public notification relay — the first release to ship the rebuilt mobile, desktop, and web UIs.

### Added
- **Forge Anywhere UI** (#812): the complete relay-transport surface across mobile, desktop, and
  web — GitHub device-code sign-in, 24-word recovery phrase, host enrollment and detail (rename,
  transport preference, disable/revoke), device pairing and atomic key-rotation, encrypted storage
  and retention, remote-job queue, session hand-off and encrypted replay shares, billing and the
  public read-only share viewer at `/shares/[id]`. Built over a typed `AnywhereClient` interface
  with a seeded mock client so the managed relay backend plugs in behind the same contract; signed
  out, the app is unchanged apart from the Settings and Connect entry points. Anywhere is a
  transport plus Settings detail routes — no second dashboard, no new tab, and only three
  design-system additions (`EntitlementBadge`, `HostDot`, `SyncGlyph`).
- **Hearth redesign** (#810): the mobile, desktop, and web clients are rebuilt on one Expo +
  react-native-web + Tauri codebase with the Emberline design system — de-boxed hairline lists, the
  Fleet/session shell, command palette, telemetry and context gauges, and the full set of session
  action sheets, with responsive desktop/web layouts sharing the mobile component tree.

### Changed
- **Public APNs relay default** (#805): stock installs point at the live
  `forge.adulari.dev/relay` origin so zero-configuration notifications work out of the box.

### Fixed
- **Remote overlay delivery** (#810): background overlay loads (`/mesh`, `/usage`) now mark the
  headless frame dirty so resolved overlays broadcast to remote clients instead of appearing stuck
  loading; a finished workflow overlay no longer holds the projection on web clients
  (`crates/forge-cli/src/cli/commands/run/driver.rs`, mobile `OverlayHost`).
- **Idle bridge reaper** (#810): parked persistent Claude/Codex bridge processes are torn down
  after an idle TTL instead of lingering as orphaned trees
  (`crates/forge-provider/src/cli_provider.rs`).
- **Desktop release publication** (#806): Linux AppImage packaging bypasses host binfmt
  interception on the Arch release runner.
- **Release manifests** (#807): Homebrew, Scoop, and AUR manifests track the published checksums.

### Security
- **Public APNs relay hardening** (#808): the production relay accepts only Forge alert and Live
  Activity shapes with topic/environment/priority/body constraints and per-client, per-device, and
  atomic daily limits; stock daemons send a single generic “Open Forge” alert rather than session
  IDs, titles, or transcript content, and the server independently replaces every alert again
  before APNs. Rate-limit state keeps only per-process salted keys, never raw client IPs or device
  tokens, and the service runs loopback-bound behind a locked-down systemd unit and Cloudflare-only
  origin firewall. Direct APNs and explicitly configured private relays retain rich alerts.

## [2.6.5] - 2026-07-16

Release, remote-session, and shared-app hardening from full Serve Anywhere and Claude/Sonnet
dogfooding.

### Added
- **Project-first session creation** (#799): new sessions default to the daemon directory, remember
  recent projects, support a safe server-side directory browser and native desktop folder picker,
  and keep the Fleet rail beside chats on expanded desktop/web layouts.
- **Android distribution** (#797): native APK/AAB builds, tagged artifacts, and optional Google
  Play internal-track submission complete the shared app's Android release path.
- **Release upgrade E2E**: the release gate now installs the previous public binary in an isolated
  home, creates real history and daemon sessions, upgrades and resumes them, rolls back, and checks
  config, secret, telemetry, and SQLite integrity before a tag can ship.

### Fixed
- **Serve Anywhere correctness** (#796): workflow/subagent workspace ownership, empty-response
  recovery, ordered reconnect output, mutating-turn re-drives, saved-workflow replay, FIFO queueing,
  interruption, `/loop`, and `/goal` persistence are reliable across disconnects and restarts.
- **Claude/Sonnet bridge reliability** (#801): Forge waits for the persistent control protocol and
  MCP readiness, uses each session's real workspace and isolated store, merges partial task updates,
  bounds keyring calls, and avoids heavyweight bridge processes for optional post-turn side calls.
- **Shared app and protocol robustness** (#797-#800): transactional desktop publication, protocol-v8
  conformance, WebSocket fleet invalidation, offline FIFO reconciliation, hardened links/CSP,
  accessible navigation, stable session-picker states, valid History DOM, and deduplicated slash
  commands eliminate the latest mobile, desktop, and web regressions.
- **Package-manager publication**: Homebrew, AUR, and Scoop manifests now update together from the
  just-published release checksums; the release bot can create its synchronization PR instead of
  silently leaving placeholder hashes or stale versions behind.
- **Release archive permissions**: Linux/macOS tarballs restore Forge's executable mode after the
  artifact handoff, so manual extraction works without an extra `chmod`.
- **Headless installer behavior**: a detached `/dev/tty` no longer turns a failed desktop-app prompt
  into an implicit “yes”; non-interactive installs remain CLI-only unless `FORGE_DESKTOP=1` is set.

## [2.6.4] - 2026-07-16

Core reliability and app activation improvements across Forge TUI, desktop, and mobile.

### Added
- **Anonymous product telemetry and dashboard** (#791, #792): lightweight opt-out usage signals
  and aggregate release-download metrics make installs, active use, retention, and failures
  visible without collecting prompts, paths, file content, or personal identifiers.
- **New-session model picker** (#794): desktop and mobile now expose the discovered model catalog,
  prioritize healthy models, preserve automatic mesh routing, and retain manual IDs offline.

### Fixed
- **Mesh completion and recovery** (#783-#790): explicit turn contracts, readiness checks,
  workspace-scoped verification, cleaner bridge startup, and quota-aware routing reduce false
  completion, cross-session leakage, redundant providers, and stalled recovery.
- **Desktop activation and connectivity** (#794): the app finds Forge in Cargo, Homebrew,
  Linuxbrew, and user-local installs, bounds HTTP waits, and makes WebSocket fallback one-shot and
  recoverable instead of hanging or silently losing sends.
- **Mobile presentation and live state** (#782, #793): Live Activity fields stay synchronized and
  the iOS header uses one stable elevated surface without the ambient background wash.
- **Command surfaces** (#794): command palette hooks and Composer focus now obey React lifecycle
  rules, eliminating command-center crash and compiler-error paths.

## [2.6.3] - 2026-07-15

Mesh reliability and release-surface hardening from continued dogfooding.

### Added
- **Headless OAuth login**: Codex paste mode and MCP RFC 8628 device flow now work in
  non-interactive environments, with state/PKCE validation and linewise pasted redirects.

### Fixed
- **Mesh post-check scope**: observational verification is bound to the originating session
  workspace and preserves the mutating retry budget, preventing cross-session false positives.
- **OTA release safety**: production OTA publishing remains limited to mobile JavaScript/assets;
  native changes continue to require a separately triggered binary build.

## [2.6.2] - 2026-07-14

Feature + fix wave on top of v2.6.1, from continued dogfooding of the real builds.

### Added
- **Live Activities** (#754): working iOS Live Activity with a polished Dynamic Island and
  lock-screen presentation — live session status, model, and progress, wired through the relay
  with APNs auth.
- **Uninitialized-project banner** (#755): sessions in a project with no Forge setup (no project
  guidance file and no custom agents) now show a dismissible warning banner on TUI, desktop, and
  mobile, computed from one canonical backend heuristic. The banner offers a one-click "set it up"
  action that runs the init flow, plus an opt-in config setting to auto-initialize every
  un-initialized project.

### Fixed
- **Fleet spend + page dots** (#753): the Fleet SPEND stat now always renders a real value
  (never blank) and the page-indicator dots track the real page count/position instead of looking
  like a stuck loader.
- **Session header on native** (#753): the entire header — including the top safe-area inset —
  now sits on one continuous elevated surface with no two-tone seam on the iOS app.
- **`/goal` read-only completion** (#756): a read-only / no-op goal (e.g. "check X",
  "already satisfied") now completes in one clean response instead of looping several times
  demanding file edits. The proven anti-spiral verification gate is unchanged — a completion is
  only fast-accepted when the model explicitly explains no change was needed; false "done" claims
  are still caught, reopened, and re-driven, and unverified work is still flagged.
- **External MCP servers by default** (#757): the CLI bridge now loads external project MCP
  servers by default (`mesh.bridge_mcp_external = true`), with a bounded, non-blocking bootstrap so
  a slow or unreachable server is skipped rather than stalling startup. Fixes the regression where
  project MCP integrations (e.g. Helm) silently stopped working under a bridged session.

## [2.6.1] - 2026-07-14

Follow-up polish on the v2.6.0 overhaul, from continued user testing of the real builds.

### Fixed
- **Response navigator** (#748): the up/down chevrons now correctly jump to the previous
  (earlier) and next (later) assistant response; the "jump to latest" button no longer shows at
  the same time as the navigator pill.
- **Session header** (#748): the elevated header surface now covers the full header block —
  including the model line and the CHAT/TASKS/AGENTS/REVIEW tab strip — on mobile, matching
  desktop (previously those rows sat on the bare page background).
- **Session telemetry sheet** (#748): tidied the model-configuration / tier layout — consistent
  cards, correct alignment, no stray separators or empty gaps.
- **Context gauge for Codex OAuth** (#748, `forge-mesh`): the gpt-5.6 / gpt-5.3-codex family now
  reports its context-window size, so the telemetry context bar and the header context gauge render
  instead of "context capacity is unavailable".
- **Session actions sheet** (#748): fixed the missing left content padding and wired up action
  rows that previously did nothing on press.
- **`/goal` completion** (#749): a finished goal now ends with a single normal final response that
  summarizes what was done, instead of stacking repeated bare "GOAL COMPLETE" sentinel messages;
  completion detection was tightened and the re-drive no longer re-emits the marker.

## [2.6.0] - 2026-07-14

User-tested desktop/mobile UX overhaul. The unified app (`mobile/` — one codebase for the
desktop Tauri app and the mobile app) was tested against the real desktop build across three
rounds; every fix lands in shared components so desktop and mobile both benefit. Also fixes a
serve-side model-catalog and per-session usage-attribution gap surfaced by the app.

### Fixed
- **Desktop window chrome** (#744): the app now reserves a top safe-area / draggable region so
  page titles, the History search bar, and the session header are never clipped under the native
  window controls.
- **Session header & telemetry** (#744, #746): visible back control; the reasoning-effort row is
  selectable and swaps its picker pane in place within the telemetry sheet (no drawer-over-drawer);
  the header context gauge is legible.
- **Models & mesh health** (#744, #745): per-model tier, benchmark scores, and context-window
  length; auto-ranked and grouped by work tier; search; and a resilient fallback that never shows
  an empty catalog when the server returned models. Serve's `models` API now emits tier/benchmark/
  context fields.
- **Usage** (#744, #745, #746): combined subscription-usage percentage headline; compact, scannable
  per-provider rows (bridges included); correct per-session token accounting; and session-view
  provider attribution so side/aux calls are no longer collapsed into a generic "other" bucket.
- **Floor** (#744): live sessions render in a proper grid with a real empty state.
- **Configuration / Skills / Hooks** (#744, #745): explicit save affordance and search on
  Configuration; searchable Skills and Hooks.
- **Composer & commands** (#744, #745): `/model` and `/models` both let you pick a model; all
  recognized built-in slash commands autocomplete; the multi-line input grows before it scrolls.
- **Transcript** (#744, #745): each turn's formatted assistant response renders; a scroll-aware
  response navigator appears only when scrolled away from the latest message.
- **Sheets** (#745): bottom sheets size to their content (short content no longer fills the screen),
  scroll internally when content overflows, and open/close with a faster, crisper animation.
- **Focus rings** (#745): keyboard focus is themed to the Emberline palette instead of the default
  browser blue, while staying visible for accessibility.

## [2.5.8] - 2026-07-13

Release-hardening wave: every platform's release pipeline now builds green end to end
(CLI/TUI, desktop, and iOS), plus a unified cross-platform release note. Several breakages
here were latent from a bulk merge and only surfaced in `cargo test`, the separate
`mobile/src-tauri` workspace, or an actual release build — not in `cargo check`.

### Fixed
- **Release builds unblocked** (#716, `.github/workflows/release.yml`, `.cargo/config.toml`): removed
  a committed `.cargo/config.toml` whose absolute `target-dir` made every GitHub-hosted release and
  desktop build fail with `Permission denied` on `.cargo-build-lock`. Also forced `shell: bash` on
  the Windows desktop bundle step (POSIX `if` was run under PowerShell).
- **TUI aarch64 release** (#717, `release.yml`): install `libasound2-dev` + `pkg-config` on the
  GitHub-hosted arm leg so `alsa-sys` builds (the self-hosted x64 box already had them).
- **Desktop Linux/Windows compile** (#717, `mobile/src-tauri/src/lib.rs`): `use tauri::Manager` was
  gated `#[cfg(debug_assertions)]`, but `get_webview_window` runs in the non-macOS setup hook in
  every profile — release builds failed `E0599`. Regated to `any(not(macos), debug_assertions)`.
- **Desktop macOS compile** (#717, `mobile/src-tauri/src/lib.rs`): `install_macos_menu` returned
  `app.set_menu(menu)` (`Result<Option<Menu>>`) against a `-> Result<()>` signature (`E0308`).
- **serve router panic** (#717, `crates/forge-cli/src/serve.rs`): a duplicate `GET /api/mcp` route
  panicked axum at router build (`Overlapping method route`), failing 12 serve tests.
- **Desktop updater signing** (#719, `mobile/src-tauri/tauri.conf.json`): rotated the Tauri updater
  signing keypair so `TAURI_SIGNING_PRIVATE_KEY` / `_PASSWORD` match the committed pubkey; releases
  were failing `incorrect updater private key password`.
- **Desktop macOS + linux-arm bundling** (#720, `tauri.conf.json`, `app-desktop.yml`): added the
  `app` bundle target so macOS emits the `.app.tar.gz` updater artifact, and installed `xdg-utils`
  so the arm AppImage bundler finds `xdg-open`.
- **testflight-autogroup misfire** (#721, `.github/workflows/testflight-autogroup.yml`): made it
  `workflow_dispatch`-only. It fired on every `mobile/**` push — including desktop-only
  `mobile/src-tauri/**` edits — then hung ~45m polling for a TestFlight build a plain push never
  produces (iOS is opt-in).

### Added
- **Unified release note across TUI, desktop, and mobile** (#718, `release.yml`,
  `scripts/testflight-assign-group.mjs`, `RELEASING.md`): one `CHANGELOG.md` section is now the
  single source of truth — it composes the GitHub Release body (all-platform header + curated
  section + auto PR list) and is pushed to the TestFlight build's "What to Test" note via the ASC
  API.

### Changed
- **CLI bridge stall guard** (#715, `crates/forge-provider/src/cli_provider.rs`): the persistent
  claude bridge now backstops a wedged tool turn (idle × 6) and falls back to a fresh turn instead
  of hanging indefinitely.

## [2.5.7] - 2026-07-10

### Added
- **Quota-pace-aware subscription conservation** (#573, `crates/forge-mesh`, `crates/forge-types`,
  `crates/forge-store`): `SubscriptionQuota` now carries a per-provider `QuotaPace`;
  `effective_fraction_for` raises (never lowers) the consumed fraction to the projected
  fraction-at-reset, so the mesh starts spreading work off a subscription that is on pace to run
  out *before* it actually does. Both `conserve_decision` and the `/mesh` inspector's
  `spread_probability` use it; `forge mesh` (text + JSON) and the `/mesh` TUI overlay show a
  `→ N% at reset ⚠` projection suffix on pacing providers.
- **Remote cancel for server-queued prompts** (#574, `crates/forge-cli/src/remote.rs`, mobile,
  web PWA): new `RemoteInput::Dequeue { index, text }` WS message removes ONE prompt queued while
  a turn was running, without interrupting the active turn. The echoed text validates the tap
  against a queue that shifted underneath it (stale dequeues are dropped, same philosophy as the
  seq-checked Allow/Answer). Mobile queued chips are now tappable; the web UI gained a per-row ✕.

### Fixed
- **Mobile boot resilience** (#570, `mobile/`): top-level `ErrorBoundary` above the theme provider
  (hardcoded colors, hides the splash on catch, reset button) plus fail-open try/catch around the
  auth boot IIFE, AppLock evaluation, and font loading — a thrown boot-path error now renders a
  recoverable error screen instead of hanging on the splash forever.
- **Mobile dead/foreign session links** (#571, `mobile/`): a 10s snapshot timeout surfaces error
  empty-states in chat/tasks/agents ("session not found on this server" on 404) instead of an
  infinite spinner, gated so transient errors on live sessions never flash it; a session screen
  latches its owning server per session id and bails out (toast + back to the fleet) when the
  active server diverges mid-view.
- **`forge doctor` bridge probes** (#572, `crates/forge-cli`): live-probe failures on optional
  CLI-bridge providers are now ⚠ warnings, not ✗ failures — a healthy install exits 0 again.
- **Log noise on retries** (#572, `crates/forge-cli`): default tracing filter is `warn,genai=off`,
  killing raw HTTP body spew from provider retries; an explicit `RUST_LOG` is still fully respected.

## [2.5.6] - 2026-07-06

### Changed
- **Base system prompt sharpened** (#530, `crates/forge-core/src/lib.rs`): `FORGE_SYSTEM` now
  tells the model to solve the general case rather than hardcoding to the tests/examples at hand
  (and to flag a test or task that looks wrong or infeasible instead of routing around it), and to
  report outcomes truthfully — saying plainly when a test failed, verification was skipped, or
  something is uncertain, instead of claiming success. Two concise, model-agnostic lines applied to
  every model; the actionable result of a system-prompt-quality investigation (wholesale importing
  large leaked frontier prompts was found net-negative, and Forge's preamble already covered the rest).

## [2.5.5] - 2026-07-05

### Added
- **mcp-serve bridge compile-check** (#525): the `[shell] scoped_cargo_target` knob (v2.5.4) now
  also applies on the CLI-bridge (`mcp-serve`) path, so a bridged `--mode bypass` agent can
  `cargo check`/`build` in a scoped writable target dir — previously only the `forge run` path
  got it. `crates/forge-cli/src/mcp_serve.rs`.
- **Browse + resume explicitly-archived serve sessions** (#527): the `forge serve` past-sessions
  browser now includes explicitly-archived sessions (badged) and can resume them, not just
  daemon-restart-orphaned ones. `crates/forge-cli/src/serve.rs`, `crates/forge-store`,
  `crates/forge-cli/src/remote_assets/*`.
- **Edit truncation guard catches unbalanced brackets** (#526): the file-edit truncation guard
  now also rejects an edit that leaves a `{`/`(`/`[` unclosed that wasn't unclosed before (a
  cut-off-mid-block signature) — string/comment/lifetime-aware and regression-scoped so balanced
  edits and pre-existing imbalance don't trip it. `crates/forge-tools/src/core_tools.rs`.

## [2.5.4] - 2026-07-05

### Added
- **`forge attach <id>`** (#518, `crates/forge-cli/src/attach.rs`): a thin TUI client that
  connects to a running `forge serve` daemon over its WebSocket to watch and drive a live
  session from another terminal. Authenticates via the persisted daemon token; supports
  `--url` / `--token` / `--list` flags.
- **Serve session management: past-session browser + one-tap worktree merge/discard**
  (#519, `crates/forge-cli/src/serve.rs`, `crates/forge-cli/src/remote_assets/*`): the
  `forge serve` control page can now browse and resume persisted sessions orphaned by a
  daemon restart (`GET /api/sessions/past`), merge a worktree session's branch back into
  its base (`POST /api/sessions/{id}/merge` — 3-way from the fork point, refuses a dirty
  base, reports conflicts without altering state, stages rather than auto-commits), or
  discard it (`POST /api/sessions/{id}/discard`, confirm-gated). New
  `docs/features/embedding.md` documents driving Forge programmatically over the daemon.
- **Scoped writable cargo target dir for sandboxed/bypass builds** (#521, `crates/forge-tools`,
  `crates/forge-config`): opt-in `[shell] scoped_cargo_target` injects a scoped, writable
  `CARGO_TARGET_DIR` (folded into the Landlock writable set) for cargo commands, so a
  confined or `--mode bypass` agent can `cargo check`/`build` without loosening write
  confinement. Default-off; an explicit `CARGO_TARGET_DIR` always wins.

## [2.5.3] - 2026-07-05

### Fixed
- **Hung turns no longer leak subprocess trees** (`crates/forge-provider/src/cli_provider.rs`):
  a cancelled or timed-out turn (idle watchdog, per-turn timeout, or user interrupt) now
  SIGKILLs the CLI-bridge child's whole process group on drop, not just the direct child —
  so a grandchild (the bridge's `forge mcp-serve`, or a hung env-build subshell) can no
  longer orphan and hang a run (a real bench once hung ~3h on this).
- **`forge api` honors an explicit `model`** (`crates/forge-cli/src/api_serve.rs`,
  `crates/forge-mesh`, `crates/forge-config`): the OpenAI-compatible endpoint now dispatches
  an explicit `model` verbatim to its provider, bypassing mesh classification (completing
  the incomplete #509 fix, which catalog-gated the pin and 404'd valid live models).
  Explicit pins are single-candidate (no silent cross-model failover); unroutable/unknown/
  keyless models return a clean 4xx instead of a silent reroute; `model:"auto"` still
  mesh-routes. A shared `pin_is_dispatchable` predicate keeps the API and CLI pin paths
  from diverging.

### Added
- **Token-budget continuation guard** (`crates/forge-core`): when a turn ends under ~90%
  of budget with no real progress and the goal unverified, Forge compacts then nudges the
  model to actually do the work instead of accepting a premature "done", stopping on
  diminishing returns (3 continuations with <500 new tokens; hard ceiling 6). Progress
  detection is CLI-bridge-aware (reads the real git worktree + sink tool activity), closing
  a gap where bridged models could end a turn with an empty diff.

## [2.5.2] - 2026-07-05

### Fixed
- **NIM/vLLM streamed content loss** (vendored `genai` OpenAI streamer): vLLM-based
  backends (e.g. NVIDIA NIM serving Kimi) send `"tool_calls": []` in every content delta;
  the tool-call branch matched on "not null", found no calls, and skipped the chunk —
  silently dropping the `content` in the same delta so only the final chunk survived
  (`PONG`→`ONG`). The tool-call path now requires a non-empty array.
- **Recap floods scrollback** (`forge-core`): the Recap event is a one-line contract but
  `content` was emitted unbounded; a misbehaving trivial-tier model could dump its full
  chain-of-thought. Recap output is now clamped to the first non-empty line (240 chars).
- **`forge run "/rust"` sent the literal slash command to the model**
  (`forge-cli` run command): one-shot runs did no catalog resolution. One-shot slash
  commands now expand exactly like the interactive dispatcher (commands → prompt, skills
  need a task, `//` escapes, unknown tokens pass through verbatim).
- **Same-model retries claimed "finding another model…"** (`forge-core` + `forge-tui`):
  pinned rate-limit backoff and transient retries reused the failover event, so the UI
  contradicted the "a pin must pin" semantics. The event now distinguishes a retry;
  headless prints "retrying…" and the TUI shows "retrying model".
- **`forge doctor` echoed ollama stdout warnings as the version** (`forge-cli` local):
  `ollama --version` with no server running prints `Warning:` lines on stdout; doctor now
  extracts the version token instead of rendering them raw.

## [2.5.1] - 2026-07-05

### Fixed
- **`forge api` routing + pinning correctness** (`crates/forge-cli` api server + mesh
  routing): honor an explicitly pinned `model` instead of re-routing it through the mesh,
  pass `response_format` through to the provider, exclude task-specific models
  (translation/embedding) from mesh routing so a chat request never lands on one, return a
  `404` for an unknown pinned model id, and inject the configured provider keys when
  dispatching from `forge api`.

## [2.5.0] - 2026-07-04

### Added
- **`forge api` — an OpenAI-compatible embed endpoint (run Forge as your app's AI backend)**:
  an opt-in local HTTP server exposing `/v1/chat/completions` (plus `/v1/models` and `/health`)
  backed by Forge's model mesh. Any app that speaks the OpenAI API can swap one `base_url` and
  gain tier-based routing, cross-provider failover, subscription quota-spread, and per-response
  cost visibility instead of a single hard-wired model. `model:"auto"` routes via the mesh; a
  concrete id pins it but still fails over. Streaming uses OpenAI SSE `chat.completion.chunk`
  frames (non-streaming returns JSON); advertised `tools` pass through and `tool_calls` surface
  with the right `finish_reason`. Optional bearer auth (`--api-key` / `FORGE_API_KEY`), loopback
  bind by default. Reuses the same `DispatchProvider` + `HeuristicRouter` wiring as `forge run`,
  so routing behavior matches an interactive session. `/v1/models` advertises only usable ids
  (bare provider-prefix candidates like `claude-cli::` are filtered out) plus `auto`.
  Docs: `docs/features/embed-ai-backend.md`.

### Changed
- **CLI-bridge harness robustness — seven waves of quality and cost improvements** for headless
  runs that route to a CLI bridge (claude/codex):
  - **Wave 1 — token diet**: capped `read_file`/shell result sizes, a leaner exposed tool
    surface (names-only skill listing), right-sized batched-update ritual for `update_tasks`,
    a lattice-injection gate that skips the code-index preamble on bridge turns, and honest
    surfacing of in-band bridge errors.
  - **Wave 2 — robustness**: rate-limit backoff now retries a *pinned* model instead of failing
    over off it; strict pin semantics (an explicit pin never silently fails over to another
    model); `search` accepts a file path and searches that file; and an empty-diff completion
    nudge for headless code-change runs.
  - **Wave 3 — per-CLI slim preamble** and a resume reminder tuned per bridge.
  - **Wave 4 — quality guards**: a pristine/existing-tests-are-spec guard for headless
    code-change runs, a deadline reconciliation window before the hard bench kill, a minimal-diff
    bias in the `expect_code_change` system context, and an env-fight spend cap that nudges after
    repeated failed provisioning attempts.
  - **Wave 5 — bridge cost caps**: a per-turn bridge input-token ceiling, bridge-aware
    env/build-fight tracking, and a loosened minimal-diff bias for out-of-tree verification.
  - **Wave 6 — empty-diff nudge on the bridge path** (with tests proving both quality guards
    fire on the CLI bridge).
  - **Wave 7 — mcp-serve health retry**: a toolless CLI-bridge turn (e.g. a codex MCP-startup
    failure) is now classified as TOOLS-UNAVAILABLE and retried once before being recorded,
    with the underlying MCP-startup failure parsed into a typed error.

## [2.4.0] - 2026-07-03

### Added
- **Remote control endgame — fleet dashboard, diff + plan review cards, file/image upload,
  voice input (protocol v7)**: the daemon's session list is now a real **fleet dashboard** —
  `GET /api/sessions` reports `waiting` (blocked on a permission/question decision),
  context-window tokens and limit per session, and serves **waiting-on-decision sessions
  first** (red pulsing dot + "needs decision" badge, then busy/idle, title, cwd, ⎇ worktree,
  cost, token gauge, last activity — tap to attach). `Snapshot.diff` is a **structured diff
  card** built from the same `similar` hunks the TUI renders (no second diff engine): while
  a write permission is pending it shows exactly **what tapping Allow will touch** (per-file
  `@@` hunks with old/new line spans), and after edits land it shows the latest-turn diff —
  latest edit per path, capped at 10 files / ~40 hunk lines per file with "+N more" markers
  (full content stays host-side); denied/failed writes never show as landed. `Snapshot.plan`
  projects the `present_plan` proposal as a **plan-approval card** with Approve & build /
  Revise / Cancel buttons that answer the existing seq-checked approval question — the
  identical code path a local "Build it" choice takes (verified e2e over a real headless
  driver). `POST /<t>/api/upload` accepts **multipart file/image uploads** from the page
  (📎 button + paste-an-image + upload chips): 10 MB cap, filenames flattened to one
  sanitized component (traversal-proof), stored under the session's own
  `.forge/uploads/<id>/`, then attached to the next prompt — images as vision input, text
  files as `@path` mentions (the remote prompt path now runs the same `@path` expansion as
  local typing, closing a parity gap); non-image non-UTF-8 files are refused, and the drain
  confines attach paths to the uploads dir so the input can't read arbitrary host files.
  A 🎤 **voice input** button transcribes speech into the prompt box via the Web Speech API
  (never auto-sends; hidden where unsupported; zero dependencies, CSP-safe).
- **Actionable Web Push — approve from the lock screen, self-hosted VAPID, no relay**: the
  `forge serve` daemon now pushes a notification when a session **needs a decision**
  (permission prompt / question), finishes a turn, or fails — even with the page closed and
  the phone locked. Permission pushes carry **Allow/Deny actions** the service worker answers
  itself over the new seq-validated `POST /<t>/api/answer` (stale seq = 409 no-op, exactly
  like the WS path), so the agent is unblocked without opening the app. Fully self-hosted:
  Forge mints its own VAPID (RFC 8292) keypair (persisted 0600 in the config dir) and
  end-to-end encrypts every payload per RFC 8291 `aes128gcm` (in-tree RustCrypto
  implementation, verified byte-for-byte against the RFC's test vector) before POSTing it
  straight to the browser vendor's push endpoint — no Firebase project, no third-party relay,
  the push service only ever sees ciphertext. Debounced (no push while any WS client is
  attached), strictly best-effort (fire-and-forget, never delays the turn), dead
  subscriptions (404/410) are pruned. The sessions panel gains an enable/disable push row
  (iOS: install the PWA + trust the cert first — documented).
- **Offline input queue**: prompts typed while the connection is down are no longer lost —
  they queue locally (per server + session, surviving reloads), show as "📴 queued (offline)",
  and flush in order on reconnect; bounded at 20 with a loud "queue full — N dropped" notice.
- **`forge serve` — headless multi-session daemon with session control from the phone
  (protocol v6)**: one long-lived process hosts any number of concurrent sessions, each driven
  by a headless SessionDriver task (the same App + dispatch + turn machinery + remote-input
  handling `forge chat` uses, factored to run with no terminal — commands, pickers, overlays,
  permission prompts, `/duel`, `/loop`, queued prompts and auto-compact all work identically
  from the page). Sessions keep running with **zero clients attached** and reconnects replay
  exactly the missed frames (`?rev=` + per-session event log), beating one-session-per-process
  remotes and their idle-death timeouts. The daemon listens on a **stable origin** — a fixed
  port (`[remote] port`, default 7420) plus a daemon token persisted 0600 in the config dir
  (`forge serve --rotate-token` revokes) — so the installed PWA survives forever; the page
  gains a session list (busy dot, title, cwd, cost, last activity), a new-session form
  (cwd + optional title + **isolated git worktree** toggle — the session then runs inside the
  worktree, tool calls rooted there via the audited subagent rewrite), tap-to-attach switching,
  and archive (stops the driver, snapshots worktree edits onto its branch, hides the session —
  history and worktree are kept). New routes under the stable base: `GET|POST /api/sessions`,
  `POST /api/sessions/{id}/archive`, and `GET /api/history?session=…` (the v5 pagination,
  session-addressed); `WS /ws?session=<id>&rev=<n>`. Exposures mirror `/remote`
  (`--lan` self-signed HTTPS default, `--local`, `--anywhere` via cloudflared/ngrok).
  Snapshots gain `title` + `worktree`; schema v8 adds `session.worktree_path`,
  `session.archived` (archived sessions leave every list, nothing is deleted) and pre-creates
  the `push_subscription` table for the upcoming web-push phase. The in-chat `/remote` keeps
  working unchanged, and the same control page serves both worlds (it probes `/api/sessions`
  to detect the daemon). Verified live: two mock sessions created over HTTP and driven
  concurrently over separate WebSockets with zero cross-talk, work completing with no client
  attached, exact gapless replay on reconnect, archive leaving the survivor drivable, and the
  token/origin stable across daemon restarts.
- **Remote control: bulletproof reconnect, full scrollback, rich transcript (protocol v5)**:
  every broadcast frame now lands in a bounded per-server event log, and the WS handshake takes
  `?rev=<last seen revision>` — after any disconnect the page replays exactly the frames it
  missed (no gap, no flicker, verified live over a real drop/reconnect), falling back to ONE
  full snapshot flagged `resync` when the gap is unfillable; a token-scoped
  `GET /<token>/api/history?before=<seq>&limit=<n>` pages the session's persisted transcript
  from the store (newest first, user + assistant turns plus user-facing `ui` notes), and the
  page fetches older pages as you scroll up — unlimited scrollback instead of the 12-line live
  tail; the page renders markdown (headings, lists, inline + fenced code, links as plain text)
  with a self-contained syntax highlighter (rust/js/ts/python/go/bash/json — no CDN, CSP-safe)
  and a tap-to-copy button on every fenced block. All rendering builds DOM via
  `createElement`/`textContent` only, so transcript content can never inject markup.
- **Remote control: full command + picker parity (protocol v4)**: every slash command, picker,
  palette, and overlay is now usable from the phone/browser through one generic mechanism —
  `Snapshot.overlay` projects whichever modal surface owns the TUI keyboard (the command palette,
  every picker kind incl. `/model` pin + `/models` browse, `/mode`, `/sessions`, `/checkpoints`,
  `/assay`, `/copy` blocks, resume-mode, the **`/duel` winner pick**, `@path` mentions, the
  `/config` wizard, and `/usage`/`/mesh`/workflow as text bodies), and `RemoteInput` gained a
  keystroke channel + overlay verbs (select/nav/filter/cancel) that inject through the same key
  path local keystrokes take — so a remote commit produces identical behavior, and any future
  picker is remote-drivable by adding one projection arm. `/copy` now ships its payload in the
  snapshot so the phone can copy it to its own clipboard; the dead `/diff` chip is gone (chips:
  Stop, `/plan`, `/compact`, `/model`, `/mode`, `/help` — all functional); the embedded page was
  split into separate token-scoped assets (`remote_assets/`), letting the CSP drop
  `'unsafe-inline'` entirely; `/keys` stays host-only with an explanatory note.
- **`/pr`** — turn the session's work into a pull request in one command: the agent branches (if
  on the default branch), stages the session's changes, commits, pushes, and opens the PR — whose
  body ends with a provenance section (session id + `forge replay` pointer, models used, spend)
  so a reviewer can trace every change back to the conversation that produced it.
- **`forge tour`** — a 2-minute printed introduction to Forge's headline features and the exact
  commands to try, no API keys needed; `forge tour --demo` additionally runs one real agent turn
  offline (mock model) in a scratch directory with an isolated store, showing routing, a tool
  call, and the final answer without touching your projects or history.
- **Two-phase context pipeline + UI-only messages**: `forge_core::context_pipeline` is now the one
  seam between the session transcript and every provider request — `prune_and_inject` (mutating
  phase: old-tool-output reclaim and future context transforms) then `to_llm` (pure phase: strips
  `UiOnly` messages, then fits the rest to the model's window). Messages carry a persisted
  `visibility` tag (`llm`/`ui`, migration 0007): UI-only notes — starting with the turn-ending
  budget-stop and no-usable-model errors — survive resume, forks, and replay for the user's
  scrollback but never spend prompt tokens, never skew the context gauge, never trip
  auto-compaction, and are never paid for in compaction summaries.
- **`forge fork` + `forge tree` — counterfactual session branching**: branch any past session
  BEFORE turn N (every earlier turn held verbatim as fixed context) and re-ask that one prompt,
  optionally pinned to a different model. `--rerun` executes the forked turn immediately and
  prints the counterfactual card — the original-vs-fork replay diff, where the shared prefix is
  identical by construction so the diff IS the effect of the change. `forge tree` shows fork
  lineage; forks are ordinary sessions (resumable, replayable, diffable). Complements
  `forge replay --rerun` (whole-history re-execution): a fork changes exactly one variable.
  Migration 0006 (`session.forked_from`/`forked_at_seq`). Conversation-state only — files are
  not rewound.
- **Persistent subagents — `send_to_agent`**: follow up with a child agent spawned earlier
  instead of re-spawning and re-explaining. The child's full transcript is rebuilt from its
  persisted session (nothing held in memory, so it works across parent turns and across a
  resumed parent session), the follow-up is appended, and the same child loop runs again —
  streamed live into the activity panel like any child. Children are named at spawn (session
  title = agent name); address by name (`"scout"`, most-recent wins on duplicates) or session-id
  prefix. Available on the direct path and the CLI bridge alike; children themselves never see
  the tool, so the depth guard stays structural.
- **`forge queue` — the overnight autopilot**: queue big tasks during the day
  (`forge queue add "<task>" [--budget USD] [--mode m] [--model id]`), drain them headless
  (`forge queue run [--gate high] [--max N]`) — each task runs a full agent turn in its own
  isolated git worktree, cost-capped by killing the run at the budget, optionally assay-gated,
  and leaves a review-ready `autopilot/<slug>` branch. `forge queue report` is the morning
  digest (status, branch, cost, replay pointer); the drain fires a desktop notification when it
  finishes. No daemon — point a `forge schedule` timer at `forge queue run` for true overnight
  runs. Migration 0005 (`queue_task`, machine-local like `schedule`).
- **Mock provider**: a `mock:write` / "create a file" trigger that does one `write_file`
  round-trip — exercises diff-preview, worktree-commit, and queue-drain paths offline.
- **Shadow routing + `forge scoreboard`**: `forge queue run --shadow` re-runs each task that
  finished `done` on a free/subscription model (`FORGE_MESH__CREDIT_MODE=strict` overlay — a
  shadow never spends paid credit) in a throwaway worktree, and records whether it kept up into
  the same per-repo `duel_outcome` boosts `/duel` feeds — ambient routing learning from real
  work instead of manual arenas. `forge scoreboard` shows the ledger: wins, losses, and the
  exact boost routing applies in this repo.

### Changed
- **README demo GIFs re-recorded**: a new hero GIF (chat → tools → the animated workflow view →
  mesh routing → the autopilot digest) plus five per-feature GIFs (TUI, workflow view, mesh,
  provenance, queue) under a "See it in action" README section. Recordings are reproducible —
  `scripts/demo/record.sh` stages an isolated offline `--mock` environment (vhs tapes in
  `scripts/demo/tapes/`), and the mock provider gained an env-gated pacing knob
  (`FORGE_MOCK_STREAM_DELAY_MS`, default off) so streaming and the workflow view animate like a
  real model on camera.

### Fixed
- **Remote: questions are answerable again** — a pending `AskUserQuestion` no longer renders as a
  dead permission prompt on the phone; the option buttons + free-text answer actually show
  (remote protocol v3).
- **Remote: real LAN connect URLs** — the URL/QR/cert now carry the discovered
  outbound-interface IP instead of the unreachable `0.0.0.0` bind address; `[remote] host`
  overrides it for multi-homed/VPN machines.
- **Remote (security): permission answers carry prompt identity** — Allow/Deny and question
  answers echo a `prompt_seq`; stale or raced taps are ignored instead of approving a newer,
  possibly more dangerous prompt.
- **Remote (security): LAN TLS failure now reports "unavailable"** — no more silent cleartext
  fallback (which couldn't bind anyway) or a healthy-looking "LAN" status over a dead server.
- **Remote (security): `bore` removed from `--anywhere`** — its tunnel is plain TCP end-to-end,
  putting the access token + transcript on the public internet in cleartext; only
  cloudflared/ngrok (HTTPS) are probed now.
- **Remote: snapshots broadcast only on change** — a busy turn no longer pushes ~60 identical
  frames/s per client, and the page's Allow/Deny buttons are no longer destroyed/recreated under
  your finger (taps landed on detached nodes).
- **Remote: prompts sent while a turn runs are queued** like local typing, instead of refused.
- **Remote: notifications now fire on Android Chrome** (service-worker `showNotification`, with
  the constructor as fallback).
- **Remote: stale transcript panel** — change detection compares content, not string length.
- **Remote: hardened page headers** — `X-Frame-Options: DENY`, a same-origin CSP, and
  `Referrer-Policy: no-referrer` so the token-bearing URL can't leak via the Referer header.
- **Remote: TUI-only commands typed on the phone respond** with a notice (e.g. "/remote can only
  be toggled from the TUI") instead of being silently swallowed; `/copy` copies on the host and
  says so.
- **Remote: protocol-mismatch banner covers both directions** (older page vs. older server).
- **Remote: dead PWA installs say so** — after the session ends, the installed app shows
  "session ended — reopen /remote from the TUI" instead of an infinite "reconnecting…" spinner.
- **Worktree snapshots no longer leak session checkpoints**: the child session running inside an
  isolated worktree writes `.forge/checkpoints/…` per turn, and `commit_worktree` swept those
  into the snapshot — so subagent/duel merge-backs and queue result branches carried another
  session's checkpoint plumbing. Now excluded like the `.cargo` shim (found live in
  `forge queue run --mock` e2e).

## [2.3.0] - 2026-07-03

### Added
- **Workflow scripts** (#453–#457): a sandboxed QuickJS engine runs JavaScript orchestration
  scripts inside Forge — `agent()` spawns a mesh-routed subagent, `parallel()` fans out,
  `pipeline()` streams items through stages with no barrier, `phase()`/`log()` narrate progress,
  and `workflow()` nests saved scripts. Author, run, and list scripts with `/workflow`; saved
  scripts live in `.forge/workflows/`. Script phases render as grouped sections in the activity
  panel (#456).
- **Dedicated workflow view** (#460): a full-screen animated view of a running workflow script —
  phases, live agents, per-agent status, and cost, updating as the script executes.
- **White-hot effort** (#462): `/effort whitehot` (aliases `ultra`, `max`) pins the top of the
  effort scale — providers get their maximum reasoning setting *and* Forge auto-orchestrates any
  genuinely multi-part task through a workflow script with a final adversarial-verification
  phase. Fifth stop on the effort slider; `⚒ WHITE-HOT` statusline badge; routing inflates the
  minimum context-window requirement 2× so small-window models don't get picked for deep work.
- **`forge blame`** (#468): per-line AI provenance. Trace any line of a file back to the session,
  model, and turn that wrote it (`forge blame src/lib.rs`), and with `--line N` get a why-card —
  the originating user prompt, the assistant's reasoning, and a `forge replay` pointer for the
  full turn. Latest-edit-wins matching over the recorded tool-call history; no git required.
- **`/duel`** (#469): the model arena. Race up to 3 distinct-provider models on the same task in
  parallel isolated worktrees, compare them on a diffstat/tests/duration/cost card, pick a winner
  whose diff merges into the working tree — and the mesh records the outcome per repo, softly
  boosting models that win here in future routing.
- **`forge schedule`** (#470): recurring headless runs on a native OS timer — the cron analog to
  `/loop`/`/goal`. `forge schedule add "<task>" --every 30m | --at 09:00 | --cron "<expr>"`
  (plus `--mode`/`--model` per tick), `forge schedule list`, `forge schedule remove <id-prefix>`.
  Installs a real systemd `--user` timer on Linux, a launchd agent on macOS, or a Task Scheduler
  task on Windows; removing a schedule uninstalls the timer. A failed install rolls the schedule
  back out of the store.
- **`/uncompact`** (#471): undo a `/compact`. Restores the full pre-compaction transcript —
  reactivates the soft-deleted messages and drops the summary in one transaction — and reloads
  the live session from it. A no-op (with a note) when the session was never compacted.
- **Statusline** (#451): `repo_name` widget, shell-backed `custom` modules (run a command on a
  refresh interval, spawned off the render loop), and `extra_rows` for any number of additional
  statusline rows.

### Fixed
- **Workflow engine hardening** (#458, #461, #463): runaway scripts can no longer hang or OOM
  Forge — an interrupt handler bounds synchronous JS slices (30s), the heap is capped at 256 MiB,
  JS→Rust value conversion is depth-capped (cyclic values abort cleanly instead of overflowing
  the stack), and nested `workflow()` calls are capped; a script that returns nothing now yields
  a digest of its agents' work instead of the string "null"; plus 3 TUI bugs found in live e2e
  verification of workflow scripts.
- **Subagent lifecycle** (#464): cancelling a turn (Esc) no longer leaks still-running child
  agents — children abort with the parent; a panicked child no longer counts toward "all
  children succeeded"; a whitespace-only child answer no longer counts as an answer; and a
  depth-guard env-var race at spawn is gone (set once at process init).
- **Worktree isolation data loss** (#469): child edits made in an isolated worktree were never
  committed, so merge-back diffed an empty range and reported a "clean" merge while silently
  dropping all of the child's work. Children now commit their edits before merge-back, on both
  the subagent and duel paths.
- **MCP agent mode** (#465): the `default` permission mode of the exposed MCP agent silently
  auto-allowed *everything*, including external-side-effect tools. It now denies by default.
- **Workflow view** (#466): interrupting a workflow no longer wedges the view in its "active"
  state forever; the view resets on interrupt and at the next turn start.
- **Activity panel** (#452): starting a new batch no longer wipes the previous finished batch
  from the panel.
- **Statusline** (#449): genuinely-free models are now distinguished from models whose cost is
  merely untracked.
- **Mock provider** (#459): `read_file` completion checks now consider file content, not just
  the call having happened.
- **CI** (#448, #467): dropped a redundant push-to-main workflow trigger; documented ignores for
  the quick-xml DoS advisory pair (RUSTSEC-2026-0194/0195) — no upstream fix exists yet and both
  dependents are on latest.

## [2.2.0] - 2026-07-01

### Added
- **Mesh routing**: `Router::route`/`route_hinted` now take a `ProjectContext`, so routing
  decisions can account for whether Forge is self-hosting the current project.
- **Mesh inspector**: the `/mesh` overlay's candidate list now supports real cursor navigation
  (↑/↓ moves an independent cursor with its own auto-scrolling viewport) instead of a static,
  non-scrolling list.
- **Ctrl+K**: skipping a model mid-turn now retries the *same* turn on the next model in mesh
  routing order instead of aborting it, and remembers every model skipped so far in that prompt's
  retry chain so it can't cycle back to one already skipped.

### Fixed
- **Mesh routing**: Claude context-window figures corrected — Opus and Sonnet (current CLI-bridge
  versions) are now reported as 1M tokens instead of a stale 200k; Haiku remains 200k.
- **Mesh inspector**: candidate `usable` flags (both the TUI overlay and the `forge mesh`/`models`
  CLI output) now match `decide()`'s real filter (credit-mode + context-fit, not just raw
  availability), so a model shown as usable is always actually routable.
- **TUI**: a large block of global hotkeys no longer fires underneath an open modal (palette,
  usage overlay, mesh inspector, `@`-picker, model picker) — they now correctly capture all input
  while open.
- **TUI**: `/quit` (and remote prompt-as-command quit) no longer deadlocks on a pending
  permission/question reply channel.
- **TUI**: two render-loop paths (remote-snapshot publish, usage-overlay periodic refresh) held a
  blocking lock on the session mutex every frame, which could freeze the entire render loop
  — including Esc/Ctrl-C — for a turn's full duration; both now use a non-blocking `try_lock`.
- **TUI**: tier up/down, the effort slider/cycle, and model pinning silently stopped working mid-turn
  because the session lock is held for a turn's entire duration (not just during a permission
  prompt) — they now read/update a render-loop-local mirror of the relevant state instead of
  requiring the session lock for their core compute step.
- **TUI**: Ctrl+K's model-exclusion bench is now applied inside the same lock acquisition as the
  retried turn itself, closing a race that could let it skip and immediately re-route to the same
  model.
- **TUI**: the slash-command palette no longer flashes open/closed while typing arguments after the
  command name (a second, unfiltered token-detection call site was missing the cursor-position
  filter the first one already had).

## [2.1.0] - 2026-07-01

### Added
- **Remote control**: full mobile control parity — installable PWA (manifest + service worker),
  live task/subagent/queued-prompt panels, tappable `AskUserQuestion` options, wire protocol
  versioning, default-on `[remote] auto` startup config, and a 256 KiB inbound-frame cap.

### Changed
- **Packaging**: crates renamed from `adforge-*` to `forge-agent-*` (binary install is now
  `cargo install forge-agent`); commit co-author trailers now attribute to `forge@adulari.dev`.
- **License**: switched from MIT to AGPL-3.0-only (Homebrew formula corrected to match).

### Fixed
- **Mesh routing**: niche/stale 70B+ models no longer outrank Opus on the Complex tier;
  `score_for()` no longer cross-matches an unrated new model version against an old one;
  `minimax-m3` size misclassification and incomplete Ollama size-tag coverage fixed; action-verb
  matching now respects word boundaries (`"port"`/`"test"` no longer false-match inside
  "report"/"latest"); version-digit accumulation capped against overflow; strict-mode fallback
  rationale now respects credit-mode.
- **MCP**: `forge mcp agent` no longer injects itself as a self-MCP server (the fork-bomb root
  cause), including explicit self-MCP entries previously persisted into config; sessions left
  empty by that bug are pruned from the store; post-handshake discovery is now timeout-bounded,
  an SSE task leak is fixed, and reconnects are serialized.
- **CLI**: symlink-following and path-traversal escapes closed in skill install (`import`,
  `marketplace`); the remote-control pairing token is now CSPRNG-derived instead of time/pid-derived.
- **Core**: undo/restore failures are surfaced instead of silently swallowed; a failing
  `spawn_agents` child no longer orphans the rest of the batch; four independently-drifted
  `truncate` helpers deduped into one char-safe implementation.
- **Lattice (index)**: path-boundary scope check (a sibling directory sharing a name prefix no
  longer false-matches); per-file update/prune errors are logged instead of aborting or being
  silently swallowed; the embedder client no longer panics on init failure; definition-enclosure
  lookup is O(n) instead of O(n²).
- **LSP**: `initialize` is timeout-bounded, message size is capped, URIs are percent-encoded, and a
  resource leak is plugged.
- **Config**: the `/config` editor's field tables match the renamed budget-cap keys; a malformed
  `[mesh]`/`[keybinds]` section now self-heals instead of silently dropping the write.
- **Store**: soft-deleted sessions are no longer treated as empty (a data-loss fix); tool-arg and
  memory growth are capped; `LIKE`-pattern metacharacters are escaped.
- **Security hardening**: `search`/`glob`/`list_dir` tools confined to the workspace, IP-literal
  SSRF blocked, and large file reads capped.
- **TUI**: the slash/`@` token scanner no longer panics on multi-byte UTF-8.
- **Types**: `PermissionMode::from_label` accepts the canonical `"accept-edits"` key; stale
  temper-mode doc comments corrected.
- **Provider**: four CLI-bridge isolation/cleanup bugs closed.
- **Skills**: bracket-wrapped LLM-generated descriptions are now quoted so assembled `SKILL.md`
  frontmatter can't silently misparse them as a YAML list.
- **xtasks**: `bench_lattice` no longer pollutes results with failed reps, no longer panics on
  non-UTF-8 paths, and drops a dead parameter.
- **Distribution**: Homebrew formula updated to a real released version with real checksums (was
  pinned to a stale version with placeholder zero hashes); a Homebrew auto-merge deadlock and an
  Assay-injection issue in CI are fixed.
- **Docs**: 25 instances of stale crate names, versions, and design-vs-implementation drift
  corrected across feature docs, architecture docs, and root docs.

## [2.0.0] - 2026-06-30

The "ready for the world" release. A full robustness, UX, and ecosystem pass synthesized from
three deep audits (see `docs/ROADMAP-v2.md` / `docs/ROADMAP-v2-DELIVERED.md` for the 1:1
item→PR mapping). Every change shipped CI-green across Linux, macOS, and Windows.

### Packaging
- **Crates published as `adforge-*`** (the `forge-*` names are taken on crates.io by unrelated
  projects). `cargo install adforge` installs the `forge` binary. Lib names are preserved, so
  there are no source-level changes. curl / Homebrew / AUR / Scoop installs are unchanged.

### Added
- **Robustness**: non-PTY shell no longer hangs on a backgrounded process; malformed `retry-after`
  bodies can't panic the failover path; truncated streams are retried instead of returned as
  complete; `web_fetch` is byte-capped (no OOM); store hardening (IMMEDIATE txns, busy-retry,
  `UNIQUE(session_id, seq)`, `PRAGMA user_version` migrations, retention + VACUUM); file-tool
  workspace confinement; explicit per-child checkpoint env (no process-global `set_var`).
- **UX**: onboarding reach for `forge run`/`nl`; `/keys` + discoverability cues; inline-stream
  markdown rendering; red error banner; `adforge`-aware help; configurable keybinds + interactive
  configurator + mid-turn `skip_model` / `tier_up` / `tier_down` / `/reload`; customizable
  statusline; +10 polish fixes.
- **Ecosystem**: runtime custom OpenAI-compatible endpoints (LM Studio/vLLM/llama.cpp); Azure,
  Bedrock, Vertex, Together, Fireworks, Perplexity providers; MCP OAuth Dynamic Client
  Registration, sampling/roots, `tools/list_changed`, HTTP server transport, SSE client transport;
  Claude-Code-compatible hooks (with enforced `stop`/`subagent_stop`) + `forge import` of
  permissions/hooks/MCP; `forge run --output-format stream-json`; skills marketplace + `update` +
  version pinning; crates.io readiness, Homebrew Linux-ARM, AUR, Scoop, shell completions, man page.
- **Quality gates**: `cargo audit` + `cargo deny` CI (caught and fixed two real CVEs — quinn-proto
  RUSTSEC-2026-0185 and an anyhow unsoundness), Dependabot, CODEOWNERS, release-profile PR build.

### Fixed
- Three real cross-platform production bugs surfaced by the new tri-platform CI: Windows
  `path_to_uri` emitted malformed `file://` URIs; the lattice deleted-file prune keyed on a
  non-canonical path so macOS/Windows users kept phantom symbols.

### Changed
- Unified `run`/`chat` resume + TUI conventions; one shared `--scope` enum across subcommands
  (back-compat `--project` preserved); 9 major dependency upgrades (tree-sitter ×5, toml 1.1,
  pulldown-cmark 0.13, clap_mangen 0.3, sha2 0.11).

## [1.8.4] - 2026-06-30

### Added
- **Linux aarch64 (ARM64) prebuilt binary.** Release now ships `forge-aarch64-unknown-linux-gnu.tar.gz`
  built on a native `ubuntu-24.04-arm` runner. The one-line installer (`curl … | sh`) now
  auto-selects the ARM64 binary on Linux ARM hosts. (`install.sh`,
  `.github/workflows/release.yml`)

## [1.8.3] - 2026-06-29

### Added
- **CLI bridge context windows.** `claude-cli` (opus=1M, sonnet/haiku=200k), `codex-cli` GPT
  models, and `agy-cli` Gemini models now have accurate context windows stored at session start,
  derived from the same native API data already fetched for their underlying providers.
  (`crates/forge-cli/src/context_windows.rs`)
- **Context bar updates after every tool step.** The context-window gauge now refreshes on every
  tool execution, not just at turn end — zero API cost (local token estimate).
  (`crates/forge-core/src/lib.rs`)

### Fixed
- **MCP server startup messages no longer corrupt the TUI input box.** `TokioChildProcess::new()`
  routes through a builder that defaults stderr to `Stdio::inherit()`, silently overriding any
  `.stderr(Stdio::null())` set before spawning. Now using the builder API explicitly so the GitHub
  MCP server's "running on stdio" line (and any other startup text) is suppressed.
  (`crates/forge-mcp/src/transport.rs`)
- **Context windows now populated for all providers.** Anthropic's `/v1/models` (`context_window`
  field) and Gemini's `/v1beta/models` (`inputTokenLimit`) are fetched natively. OpenRouter is
  always fetched (keyless) and cross-mapped to native namespaces: `openai::`, `xai::`,
  `deepseek::`, `mistral::`, and `nvidia::`. NVIDIA NIM and other custom providers whose model
  listing doesn't include context info now get windows via an OR basename lookup (e.g.
  `nvidia::meta/llama-3.1-405b-instruct` is matched to OR's `meta-llama/llama-3.1-405b-instruct`
  by model basename). (`crates/forge-cli/src/context_windows.rs`)
- **Token gauge hides stale ↑0 ↓0 for CLI bridge sessions.** Bridge models don't report API
  token usage; the turn gauge now shows only the timer (`⧖ 1.2s`) rather than misleading zero
  counts. (`crates/forge-tui/src/app.rs`)
- **Session Σ gauge hidden on first turn.** When `turn_base_in=0`, both gauges showed identical
  values. Σ gauge is now suppressed whenever it equals the turn delta.
  (`crates/forge-tui/src/app.rs`)
- **`forge migrate push` no longer fails when `forge` isn't on remote PATH.** Falls back to
  `~/.local/bin/forge` then `~/.cargo/bin/forge` before erroring.
  (`crates/forge-cli/src/cli/commands/migrate.rs`)

## [1.8.2] - 2026-06-29

### Added
- **Context-window-aware mesh routing.** The router now filters models whose stored context
  window is smaller than the current transcript token count, preventing routes to models that
  would overflow mid-turn. Fail-open: models with no stored window pass through and the
  conservative 32k floor still guards transcript trimming. (`crates/forge-mesh/src/lib.rs`,
  `crates/forge-mesh/src/pricing.rs`)
- **Effort multiplier for context-window selection.** HIGH effort inflates the minimum context
  requirement by 1.5×; XHIGH/Max by 2×, so large-output turns prefer models with sufficient
  headroom for extended thinking chains. (`crates/forge-mesh/src/lib.rs`)
- **Dynamic context-window fetch for Groq + all custom providers.** Context windows are now
  fetched from Groq's `/openai/v1/models` and every configured custom OpenAI-compatible
  endpoint (NVIDIA NIM, Cerebras, SambaNova, …) at session start and persisted to the store.
  All hardcoded family heuristics stripped from `context_limit()` — only CLI bridges remain
  (no queryable API). (`crates/forge-cli/src/context_windows.rs`,
  `crates/forge-mesh/src/pricing.rs`)
- **Context-window badge in model pickers.** `/model` pin picker and `/models` provider
  drill-in now show "128k ctx", "32k ctx" etc. per model row, read from the persisted store.
  (`crates/forge-cli/src/cli/commands/run/pickers.rs`)

## [1.8.1] - 2026-06-29

### Added
- **429 key-retry before mesh failover.** When a provider has ≥2 API keys configured, a
  rate-limit error now retries once with the next key (via the round-robin pool's `AtomicUsize`)
  before escalating to mesh failover. Single-key providers are unaffected.
  (`crates/forge-provider/src/genai_provider.rs`)
- **Interactive model-pin picker (`/model`).** Bare `/model` opens an animated ranked picker
  (mesh auto first, then subscription → frontier → paid → free). `/model <partial>` opens it
  pre-filtered; `/model provider::model` pins directly; `/model` bare in active-pin state clears
  the pin and restores mesh auto-routing.
  (`crates/forge-cli/src/cli/commands/run/pickers.rs`, `crates/forge-tui/src/app.rs`)
- **Animated effort slider.** `/effort` (bare) opens an interactive slider above the input bar
  with per-level animation: HIGH pulses orange→gold→red, XHIGH cycles a 12-colour rainbow with
  sparkle characters. (`crates/forge-tui/src/app.rs`)

### Fixed
- **MCP stdio stderr leak into TUI.** `TokioChildProcess::new` internally defaults to
  `stderr=inherit`, overriding any `Command::stderr(null())` we set. Switched to
  `TokioChildProcess::builder().stderr(null()).spawn()` so child startup text (e.g. "GitHub MCP
  Server running on stdio") no longer appears in the raw-mode TUI.
  (`crates/forge-mcp/src/transport.rs`)
- **MCP double-announce on startup.** The initial `announce_mcp()` ("reconnecting" placeholder)
  is now suppressed in TUI mode; a background task waits for `connect_active()` to complete via
  a `tokio::sync::watch` channel and then emits the final connected/failed state only once.
  (`crates/forge-mcp/src/lib.rs`, `crates/forge-cli/src/cli/commands/run.rs`)

## [1.8.0] - 2026-06-29

### Added
- **Auto-detect lint/test commands from project structure.** `forge_autofix` now inspects
  `package.json`, `Cargo.toml`, `pyproject.toml`, etc. to determine the correct check command
  instead of falling back to a hard-coded default. (`crates/forge-core/src/lib.rs`)
- **Tool-failure loop guard + doom-loop detection.** Repeated identical tool-call failures trigger
  a hard stop with a user-readable error instead of looping indefinitely. Doom-loop heuristic
  detects N consecutive identical error payloads and breaks out. (`crates/forge-core/src/lib.rs`)
- **TUI live observer mode for MCP-driven sessions.** When Forge is driven by an external MCP
  client, the TUI can attach as a read-only observer: streaming progress, tool calls, and model
  output in real time without interfering with the running session.
  (`crates/forge-tui/src/app.rs`, `crates/forge-cli/src/cli/commands/run.rs`)
- **`forge_assay` MCP tool.** New MCP tool that runs a bounded static analysis pass over a
  source bundle and returns structured findings; exposes Forge's assay engine to external
  MCP clients. (`crates/forge-cli/src/mcp_agent.rs`)

### Changed
- **SQLite WAL mode + pragmas.** Enabled WAL journal mode, `synchronous=NORMAL`, and
  `busy_timeout=5000` on the session store for faster concurrent writes.
  (`crates/forge-store/src/lib.rs`)
- **Transaction batching for bulk inserts.** Multi-message DB writes now execute inside a single
  transaction, cutting fsync overhead by up to 10× on turn-end flushes.
  (`crates/forge-store/src/lib.rs`)
- **`prepare_cached` on hot write paths.** `add_message_full`, `record_routing`, and
  `record_tool_call` now use `prepare_cached` instead of re-compiling SQL on every call.
  (`crates/forge-store/src/lib.rs`)
- **Background update check in TUI mode.** The once-daily GitHub version probe is now spawned
  off the startup path and delivers its notice via the TUI warning channel instead of blocking
  before the interface appears. (`crates/forge-cli/src/update_check.rs`)
- **Deduplicated config + catalog loads at startup.** `forge_config::load()` and
  `forge_skills::Catalog::load()` are each called once at TUI startup; the TUI now reuses the
  session's already-loaded `Arc<Catalog>` instead of constructing a second copy.
  (`crates/forge-cli/src/cli/commands/run.rs`)
- **Async codex bridge rollout.** Codex CLI-bridge turns now use async streaming, matching the
  claude bridge, for lower latency and better cancellation.
  (`crates/forge-provider/src/cli_provider.rs`)

### Fixed
- **Full-codebase security + stability sweep.** Four real bugs fixed: unvalidated shell-injection
  vector in tool argument handling, integer overflow in token accounting, unchecked slice index
  in transcript rewind, and a race in the MCP session lock during interrupt.
  (`crates/forge-core/src/lib.rs`, `crates/forge-tools/src/shell.rs`)
- **`/undo` on turns with autofix injections.** When `cargo check` triggered a synthetic
  `Role::User` message mid-turn, `undo()` found the injected message instead of the real prompt,
  leaving the snapshot range empty. Fixed by using a `-1` sentinel for "no turn yet" and
  recomputing the transcript index correctly. (`crates/forge-core/src/lib.rs`)

## [1.7.0] - 2026-06-28

### Added
- **`forge mcp agent` — Forge-as-MCP-server.** Expose a persistent Forge session over stdio MCP so
  any MCP-aware agent (Claude Code, another Forge instance) can drive it as a stateful coding agent.
  Four tools: `forge_chat` (send prompt, stream progress notifications), `forge_status` (inspect
  session), `forge_set_mode` (switch permission mode at runtime), `forge_interrupt` (abort an
  in-flight turn mid-flight via `tokio::sync::Notify` without touching the session lock).
  (`crates/forge-cli/src/mcp_agent.rs`)
- **`LoopOutcome` — explicit turn stop reason.** `run_turn` / `run_turn_with` now return
  `Result<LoopOutcome, CoreError>` instead of `Result<String, CoreError>`. `LoopOutcome` carries
  `text: String` and `stop_reason: StopReason` (FinalAnswer / MaxSteps / BudgetExhausted /
  Interrupted) so callers know why the turn ended without re-parsing warning text. Implements
  `Deref<Target=str>`, `Display`, and `PartialEq<str/&str/String>` so existing test assertions
  compile unchanged. (`crates/forge-types/src/lib.rs`, `crates/forge-core/src/lib.rs`)
- **TUI statusline stop_reason indicator.** When a turn ends with `MaxSteps`, the statusline shows
  an amber `⚠ step limit — send 'continue'`; `BudgetExhausted` shows a red `✕ budget cap`.
  `PresenterEvent::Done` now carries `stop_reason` so the App stores and renders it.
  (`crates/forge-tui/src/app.rs`, `crates/forge-tui/src/lib.rs`)
- **`forge_interrupt` MCP tool.** Abort the in-flight `forge_chat` turn at its next await point via
  `tokio::sync::Notify`; the session state is preserved and a partial result returned. The handler
  never holds the session lock, so it runs concurrently with the ongoing turn.
  (`crates/forge-cli/src/mcp_agent.rs`)
- **`remember` virtual tool on MCP bridge path.** `mcp_serve.rs` now advertises and handles the
  `remember` tool so bridge models (claude-cli, codex-cli) can write cross-session memories
  mid-turn, at parity with the direct path. (`crates/forge-cli/src/mcp_serve.rs`)
- **Embeddings default-on** with free Gemini `text-embedding-004` backend (falls back to OpenAI
  then ollama then no-op). Fixed `#[serde(default)]` on `EmbeddingsConfig.enabled` that silently
  kept embeddings off when the config section was partially specified.
  (`crates/forge-config/src/lib.rs`)

### Docs
- **README synced to the v1.6.x wave:** documented built-in auto-memory (capture/recall, `/remember`,
  `/memories`, `forge memory` CLI — with an honest "differentiated design, not benchmarked-best" note),
  the `/effort` mesh-wide slider, reset-aware rate-limit handling, multi-key rotation, and
  `credit_mode=strict`; refreshed the stale Action `version:` example to v1.6.1.

## [1.6.1] - 2026-06-28

### Added
- **Semantic memory recall is now live end-to-end.** The v1.6.0 engine is wired into the turn loop:
  when an embedder is configured (`[lattice.embeddings]`), end-of-turn capture stores each fact's
  embedding (`add_memory_with_embedding`) and start-of-session recall ranks memories by cosine
  similarity to the prompt (`recall_semantic`) instead of keyword overlap. Falls back to keyword
  recall transparently when no embedder is available — so it's a pure upgrade, never a regression.
  Implemented as a free `embed_one(&EmbeddingsConfig, …)` function (not a `&self` method) so the
  `.await` doesn't hold a `&Session` borrow — the fix for the non-`Send` turn future that blocked the
  v1.6.0 wiring.

## [1.6.0] - 2026-06-28

### Added
- **Semantic memory recall engine** (store layer). The auto-memory store now supports embedding-based
  recall: `add_memory_with_embedding` persists a fact's f32 vector in the (already-reserved)
  `embedding` column, and `recall_semantic` ranks a scope's memories by **cosine similarity** to a
  query embedding (memories with no/empty/mismatched embedding sort last), with little-endian f32 ↔
  blob helpers. Fully unit-tested (round-trip, cosine ranking, no-embedding fallback). *(Built by
  Forge itself via the mesh — and the v1.5.1 truncation guard proved itself live: the same task that
  corrupted the file last time landed as 175 purely-additive lines this time.)*

  The end-of-turn capture + start-of-session recall still use keyword overlap; wiring this semantic
  engine into them (best-effort, behind the existing embeddings config, keyword fallback) is the next
  increment — it needs a `Send`-safe embed path, since calling the embedder directly in the turn loop
  makes the turn future non-`Send`.

## [1.5.1] - 2026-06-28

### Fixed
- **`edit_file` now rejects a truncated replacement instead of corrupting the file.** When a model's
  `new` argument is cut off mid-string (its output got truncated), the edit used to apply and leave
  the file with an unterminated string literal / unbalanced delimiters. `apply_edit` now detects a
  likely-truncated replacement (an odd number of unescaped `"`) up front and returns an actionable
  error — "looks truncated … make a smaller edit, or use write_file" — before touching the file.
  New `looks_truncated` helper + tests. *(Built by Forge itself via the mesh — the deeper fix for
  the edit-tool truncation that previously corrupted large edits during dogfooding; pairs with the
  v1.4.1 error-message improvements.)*

## [1.5.0] - 2026-06-28

### Added
- **`/remember` and `/memories` in-session commands.** `/remember <text>` saves a durable memory for
  the current project on the spot (explicit curation alongside the automatic end-of-turn capture);
  `/memories` lists what Forge knows about the project. The start-of-session recall now also prints a
  one-line "💭 recalled N memories from past sessions" note so it's visible. Builds on the v1.4.0
  auto-memory system. *(Built by Forge itself via the mesh — dogfooding.)*

## [1.4.1] - 2026-06-28

### Improved
- **`edit_file` failures are now self-correcting instead of churning.** When `old` doesn't match
  after the whitespace + block-anchor fallbacks, the error now tells the model the file may have
  changed and to **re-read the exact lines with `read_file` and copy them verbatim**; if `old` is
  large (>40 lines) it suggests **splitting into smaller edits or using `write_file`** for a new
  file. A whitespace-insensitive *ambiguous* match (several near-matches) is now reported distinctly
  ("matches N places — add surrounding context") instead of looking like a plain miss. New
  `flex_match_count` + 3 tests. *(Built by Forge itself via the mesh — the first of the
  dogfooding-friction fixes that make Forge better at building everything.)*

## [1.4.0] - 2026-06-28

### Added
- **Built-in auto-memory — Forge now remembers durable facts across sessions, per project.** A new
  `memory` table in the store holds typed facts (preference | decision | fact | reference) scoped to
  each project (or `global`). At the end of a turn Forge makes one cheap trivial-tier call to extract
  0–3 durable facts (preferences, project decisions/conventions, key constraints — not transient task
  detail) and stores them; repeated facts auto-dedup (Jaccard) and bump salience instead of piling
  up. At the start of a session it **recalls only the few most relevant** memories (keyword overlap
  with the prompt, then salience + recency) and injects them into context — the edge over a
  dump-everything memory file. On by default (`[mesh] auto_memory`).
  - New `forge memory` command: list (no subcommand), `add <text> [--kind]`, `search <query>`,
    `rm <id>`, `clear`; `--global` for the cross-project scope.
  - Store API: `add_memory` / `list_memories` / `recall_memories` / `search_memories` /
    `delete_memory` / `clear_memories`, fully unit-tested (dedup, relevance ranking, scope
    isolation). Capture/recall are gated, best-effort, and never derail a turn.

  *(The schema + initial store layer were drafted by Forge itself via the mesh — dogfooding; Claude
  Code completed the store methods, CLI, capture/recall wiring, and tests after Forge hit edit-tool
  limits on the large files.)*

## [1.3.1] - 2026-06-28

### Fixed (mesh robustness — surfaced by dogfooding)
- **Empty-response models now fail over instead of dead-ending the turn.** A model that streams an
  empty final chunk (e.g. some NIM models like kimi-k2.6) was nudged twice and then the turn
  *stopped* — even with healthy fallback models (incl. the reliable subscription bridge) untried.
  It now benches the empty model and fails over down the chain, so the turn reaches a working model.
- **402 "Payment Required" via the streaming path is now treated as permanent (Capability).** A
  paid model surfaced through streaming (e.g. SambaNova: "A payment method is required to use
  `<model>`") was misclassified as a transient outage — retried twice and benched on a short
  cooldown instead of excluded. Added payment-required markers to the capability classifier so such
  a model is excluded immediately and failover moves on cleanly.

## [1.3.0] - 2026-06-28

### Added
- **The `/effort` slider now steers the whole mesh, not just the provider's reasoning param.** Effort
  is threaded from `Session.pinned_effort` into the routing decision (Router trait → `decide` →
  `ranked_seeded`), benchmark-driven:
  - **Medium / unset** — routing is byte-identical to before (no change).
  - **High / xHigh** — biases ranking toward the higher Artificial-Analysis-benchmarked model, but
    ONLY when the score gap is meaningful (≥1.0); equal/unknown scores keep the normal order.
  - **Low** — keeps the better model on a real quality gap, and prefers the cheaper/faster one only
    on a near-tie.
  Provider reasoning-effort is still sent, but only escalated for models that actually benefit
  (`model_benefits_from_effort`: o-series/gpt-5/thinking/reasoning/DeepSeek-R1). The active effort is
  shown as a colored, per-tier label in the TUI chat (`PresenterEvent::Effort`). New mesh tests pin
  all three behaviors. *(Implemented by Forge itself via the mesh — dogfooding.)*

## [1.2.0] - 2026-06-28

### Added
- **In-turn wait-for-reset on rate limits.** When the best-ranked model is rate-limited with a SHORT
  reset (per-minute free tiers — NIM/Groq/Gemini), Forge now waits out the reset and retries the SAME
  model instead of immediately degrading to a lower-ranked (or paid) one. Bounded by a per-turn wait
  budget (2) and a cap on the reset length (75s) — a long/hourly/daily quota still falls through to
  normal failover, so the turn never blocks indefinitely. Completes the rate-limit handling from
  v1.1.2 (short cooldown) and v1.1.1 (strict-credit).

### Changed
- **Non-chat models are excluded from the routable catalog.** Image/video generation (imagen, veo,
  lyria), audio/TTS (voxtral, orpheus, whisper), embeddings, reranking, OCR, and moderation/safety
  classifiers can't serve `/chat/completions`, so they only churned failover and showed as a
  heuristic "—". They're now filtered from EVERY discovery source (genai list, OpenRouter, the custom
  `/v1/models` listers) via `forge_config::is_non_chat_model`. Multimodal CHAT models (…-vision,
  …-vl, flash) are kept.
- **The benchmark cache is now global** (`~/.local/share/forge/benchmarks.json`) instead of
  project-local (`.forge/benchmarks.json`), so Artificial Analysis scores — which are model-wide —
  are shared across every project and refreshed once, not re-fetched per repo. The legacy project
  file is still read as a fallback so existing scores aren't lost on upgrade.

## [1.1.2] - 2026-06-28

### Fixed
- **Rate-limited free models come back in ~1 min, not 5.** The default bench cooldown when a 429
  carried no `Retry-After` was 300s, so a per-minute free tier (NVIDIA NIM, Groq, Gemini RPM) got
  parked for five minutes — long enough that routing abandoned all of that provider's high-ranked
  free models and degraded to a weaker (or paid) one. Default cooldown is now **60s**; providers
  that send `Retry-After` still use their exact value. (Pairs with the v1.1.1 strict-credit fix so
  failover can't fall onto paid models in the meantime.)
- **Transient errors retry the same model before failing over.** A one-off 5xx / dropped stream /
  network blip used to bench the model and switch immediately; it now retries the SAME model up to
  twice with backoff first (these usually succeed on a second attempt), only failing over if it
  keeps failing. Rate-limits (respect the cooldown) and permanent incapabilities (no tool support /
  402) still fail over at once — no pointless retries.

## [1.1.1] - 2026-06-28

### Fixed
- **`credit_mode = "strict"` now actually keeps paid models out of routing and failover.** It was
  wired only to the output-token cap, so its documented "free + subscription only" promise was never
  enforced — a free pick that failed over (or a declined smaller-model fallback) could land on a
  PAID model (e.g. `openrouter::google/gemini-2.5-pro`) and silently spend credit. Strict now drops
  every paid/metered model (including priced "free-tier" models that could bill once their quota is
  gone) from the auto-route candidate set and the whole failover chain; only free + subscription
  models remain. An explicit `--model` pin still overrides (it bypasses the auto path), so a
  deliberate paid choice works. Normal/Frugal are unchanged.

## [1.1.0] - 2026-06-28

### Fixed
- **The "working" spinner now stops the instant the response is done — it no longer waits on the
  end-of-turn recap.** The recap (a separate trivial-tier summary call) was awaited inside the turn,
  so the spinner kept ticking and the input/session stayed locked until it finished. The recap now
  runs on a detached task (via a clonable presenter sink) after the turn returns: the spinner stops
  and the next prompt is ready immediately, and the recap line streams in a moment later. Headless /
  non-interactive runs keep the inline behaviour.
- **The release's Homebrew-formula PR now actually opens.** `release.yml` granted only
  `contents: write`, so the `gh pr create` in the formula-update step was denied (it pushed the
  `brew/v<version>` branch but couldn't open the PR — masked by `continue-on-error`). Added
  `pull-requests: write` so future releases self-open the brew bump PR.

### Added
- **Multiple API keys per provider, with round-robin rotation.** Every key-based provider (all except
  the CLI bridges) can hold several keys — run `forge auth <provider>` again to stack another. The
  provider client round-robins across them per request, multiplying a free tier's per-key rate limit
  and landing a 429-retry on the next key. New `forge auth` flags: `--list` (masked count),
  `--replace` (overwrite), and the default now APPENDS instead of overwriting. Keys also come from
  the env as `VAR="k1,k2"` or numbered `VAR_2`/`VAR_3`/…. Rotation engages only at ≥2 keys, so
  single-key (and cache-sensitive paid) providers are unchanged. See
  [docs/features/free-models.md](docs/features/free-models.md#multiple-keys-per-provider-rotation).
- **🆓 Recommended free providers** — a new README section (linked from the top nav) tiering every
  free-tier provider with best models, rate limits, direct signup links, and the exact `forge auth`
  command.
- **NVIDIA NIM, SambaNova, and Mistral providers — free frontier models in the mesh.** Three new
  OpenAI-compatible providers wired through a generalized custom-endpoint resolver: `nvidia::`
  (DeepSeek-R1, Llama-3.1-405B, Nemotron-70B — free dev tier), `sambanova::` (DeepSeek-V3.1,
  Llama-3.3-70B, Llama-4 Maverick), `mistral::` (Mistral Large 3, Codestral). Plus `cohere::` via
  genai's native adapter (Command A 218B). Authenticate with `forge auth <provider>`; models are
  seeded into discovery (these endpoints can't be model-listed live) and route + fail over like any
  other free provider. Their **full live catalog is listed** via the provider's OpenAI `/v1/models`
  endpoint (`list_custom_models`) — generic over the registry, so NVIDIA NIM surfaces 100+ models,
  not a hand-seeded few; the `seed_models` list is now only a fallback when the live call fails.
  Embedding / reranking ids are filtered out (they can't serve chat).
- **One-row provider extensibility.** New `CUSTOM_OPENAI_PROVIDERS` registry in `forge-config` is the
  single source of truth (namespace, endpoint, key env var, free flag, seed models) for every
  OpenAI-compatible provider genai lacks an SDK adapter for. Adding a provider is a single struct
  literal — it wires auth, env injection, mesh discovery, cost-tier routing, the free/paid flag, and
  cross-provider failover end-to-end. The Cerebras integration was migrated onto it. See
  [docs/features/free-models.md](docs/features/free-models.md#adding-an-openai-compatible-provider).

## [1.0.0] - 2026-06-28

**Forge 1.0.** A model-agnostic AI coding agent for the terminal whose harness is *measurably* better
than the raw vendor CLIs: on SWE-bench Lite, same `sonnet` model, **Forge fixes 6/10 vs the raw
`claude` CLI's 4/10 at ~21% lower tokens per fix** — strictly dominating, re-confirmed on this build.

### Docs (v1.0 marketing + currency pass)
- **README rebuilt for v1.0:** new hero (proven harness + model freedom), a "same model, better
  results" benchmark section, a head-to-head comparison vs Claude Code / Codex / Cursor / OpenCode /
  Aider, a demo-GIF slot (`docs/assets/demo.tape` + recording guide), refreshed docs links, and the
  stale `v0.1.0` Action example bumped.
- **Docs currency sweep:** `known-issues.md` store-connection-pool backlog marked RESOLVED (#308);
  `roadmap.md` "Road to v1.0.0" updated (must-haves done, post-1.0 Next); benchmark + persistent-transport
  claims already corrected in `results.md` / `why-forge-is-a-better-harness.md` (#311).

### The road here (v0.4.44 → 1.0.0)
Robustness + correctness from 7 verify-first bug-hunts (28+ real bugs); the persistent
subscription-bridge transport; `forge-store` connection pool; the two final TUI bugs; Homebrew
auto-update; and the benchmark re-confirmation. Honest scope unchanged: Forge resolves more and is
cheaper per resolve; total-token efficiency is parity (not a win) — stated plainly in the proof doc.

## [0.4.71] - 2026-06-28

### Fixed (release — Homebrew auto-update now works on protected main)
- **The release's Homebrew-formula update opens a PR instead of pushing to `main`.** The v0.4.70
  release proved the direct-push approach silently no-ops on a branch-protected `main` (the push is
  rejected, and `continue-on-error` masked it as "success"). The step now pushes a `brew/v<version>`
  branch and opens an auto-merge PR — which respects branch protection. Formula brought current to
  **v0.4.70**.

## [0.4.70] - 2026-06-28

### Docs (benchmark re-confirmation on the current build + stale-claim sweep)
- **Re-confirmed the headline SWE-bench result on the current build (v0.4.65).** The standing
  comparison was from v0.4.39 (~26 releases old); re-ran the same setup (10 SWE-bench Lite instances,
  `claude sonnet`, same model both arms, loop-gated config, official `swebench` evaluator):
  **Forge 6/10 vs raw claude-cli 4/10**, Forge **strictly dominating** (every CLI solve is also
  Forge's, +2 Forge-only, 0 CLI-only) at **~21% lower tokens/resolve** (2.83M vs 3.57M). N=10 is
  small and `requests-2317` hung on both arms (excluded fairly) — it confirms no regression, not a
  fresh large-sample proof. Written up in `docs/benchmarks/results.md` + `why-forge-is-a-better-harness.md`.
- **Corrected a stale claim** in `why-forge-is-a-better-harness.md` §3: the persistent stream-json
  transport was described as "not a shipped result" — it **shipped in v0.4.63**. Now stated
  accurately (ships the ~0.88s/turn spawn-cost saving; tokens unchanged; the deeper "Forge drives the
  inner loop" variant is still open).

## [0.4.69] - 2026-06-28

### Fixed (release — Homebrew formula)
- **The Homebrew formula auto-updates per release.** `homebrew/forge.rb` was pinned at **v0.4.4** —
  `brew` users had been getting a months-old binary because the "update version + sha256 by hand
  after the tag" step never happened. `release.yml` now runs `scripts/update-brew-formula.sh` after
  publishing, reading the **just-built** `checksums.txt` (so there's no race against assets that
  don't exist yet) and committing the bumped formula to `main` (best-effort — a protected branch or
  push hiccup never fails the release; the script is also runnable by hand). Formula brought current
  to v0.4.65 in this PR.

## [0.4.68] - 2026-06-27

### Tests (harness quality)
- **Pinned that `mesh.self_review` still fires when enabled.** The self-review pass is off by default
  (it regressed when on-by-default) but must stay wired; added `self_review_runs_after_an_edit_turn_when_enabled`
  so the gated feature can't silently rot. (Also confirmed the autofix iteration-cap is already
  test-pinned by `autofix_iteration_cap_halts_the_self_heal_loop` — the v1.0 plan's "TODO" for it was stale.)

## [0.4.67] - 2026-06-27

### Changed (store — connection pool)
- **`forge-store` moved from a single `Mutex<Connection>` to an `r2d2` connection pool**, so the TUI
  run loop, subagents, and the lattice indexer no longer serialize every read behind one lock —
  WAL-backed file DBs now serve concurrent reads from separate pooled connections. Writes still
  serialize (SQLite's one-writer rule) but wait on the existing `busy_timeout` instead of failing.
  - To avoid `r2d2_sqlite` (which pins an older `rusqlite`/`libsqlite3-sys` and would link a second
    bundled SQLite), the pool uses a hand-rolled `ManageConnection` over our `rusqlite` 0.40 that
    applies the per-connection pragmas on every open.
  - In-memory stores (tests) pin to a single never-recycled connection — each `:memory:` open is a
    fresh empty DB, so recycling would silently lose data. All 35 store tests pass, plus a new
    `pool_handles_concurrent_threads_on_a_file_db` (8 threads × 20 writes land with no
    "database is locked"). The public `Store` API is unchanged.

## [0.4.66] - 2026-06-27

### Fixed (TUI — the two deferred bug-hunt-6 bugs)
- **Inline activity viewer re-enables follow at the tail.** `Down`/`j`/`space`/`PageDown` scrolled but
  never re-armed auto-follow, so returning to the bottom left new activity un-tailed until you pressed
  `End`/`G`. The render path now records the viewer's scroll geometry `(wrapped_len, body_h)`; a
  downward scroll that reaches the last page clamps and re-arms follow (matching the full-screen
  browser). Test: `viewer_down_at_tail_reenables_follow`.
- **Text selection spans the committed/streaming boundary.** `selection_text` only read the committed
  wrap cache, so a copy that ran into the live streaming reply was cut off at the boundary. It now
  pulls rows from the streaming edge too (the rendered transcript is `wrap_cache` + `stream_cache`).
  Test: `selection_spans_committed_and_streaming_rows`.

## [0.4.65] - 2026-06-27

### Changed (disk hygiene)
- **`[profile.dev] debug = "line-tables-only"`.** Full debuginfo made each debug/test binary
  ~480 MB; cargo writes a fresh one per build-hash and never GCs the old ones, so a long session of
  `cargo test` runs accumulated hundreds of GB in `target/debug/deps`. Line-tables-only keeps
  file:line in backtraces/panics at ~⅓ the size (measured: forge-core test binary 484 MB → 163 MB).
- **Subagent worktrees share the parent `target/`.** `WorktreeGuard::create` now writes
  `<worktree>/.cargo/config.toml` pointing `build.target-dir` at the parent repo's `target/`, so a
  build inside an isolated child worktree reuses the main build cache instead of compiling its own
  multi-GB copy — which both wasted disk/time and leaked if the process was killed before the guard's
  `Drop`. Best-effort (falls back to a private target on write failure); the untracked config is
  never merged back (`git diff HEAD..branch` drives merge-back). Test extended.

## [0.4.64] - 2026-06-27

### Docs
- **Persistent bridge transport scope documented** (`docs/features/persistent-bridge-transport.md`):
  why P1 is claude-only. Probed all three CLIs — **agy** 1.0.12 has no streaming-input mode (only
  one-shot `--print`); **codex** 0.141 `exec-server --listen stdio` speaks JSON-RPC but is an
  unimplemented STUB (`initialize` works, but every turn method returns `-32601 "exec-server stub
  does not implement … yet"`), so a persistent codex transport is blocked upstream, not on Forge.
  Recorded the same on the `CliProvider::persistent` field so it reads as deliberate.

## [0.4.63] - 2026-06-27

### Added (P1 — persistent bridge transport)
- **Long-lived claude `--input-format stream-json` transport.** The claude bridge now keeps ONE
  process alive across turns/re-drives and writes each turn's delta to its stdin, instead of
  re-spawning (and re-`--resume`-ing) the CLI every turn. Removes the per-turn process-spawn +
  session-reload cost that one-shot paid on every re-drive. On by default for claude;
  `FORGE_PERSISTENT_BRIDGE=0` (or `CliProvider::with_persistent(false)`) opts out.
  - **Safety:** falls back to the one-shot path whenever the live session can't be established
    *before* any turn output ran (spawn failure, first-turn stdin-write failure, immediate exit with
    no tool executed), so a tool can never double-execute. Once a turn has started, errors propagate
    (retryable) rather than re-running.
  - **Correctness:** respawns on model change, transcript shrink (compaction), and a
    `FORGE_CHECKPOINT_SEQ` change (a new user turn) — so re-drives *within* a turn reuse the process
    (the win) while bridge-edit `/undo` snapshots stay turn-accurate.
  - **Proven:** deterministic test `persistent_transport_reuses_one_process_across_turns` (a 2nd turn
    served by the same process answers "reply 2"; a fresh spawn would answer "reply 1"), live e2e
    against real claude (recalls a codeword across two turns on one process), and a measured fixed
    overhead of **≈0.88s spawn→init per one-shot turn** that persistent now pays once per session
    instead of once per re-drive. Honest scope: model inference dominates total turn time, so this is
    a real per-re-drive saving that compounds, not a headline multiplier.

## [0.4.62] - 2026-06-27

### Fixed (bug-hunt batch 7 — provider + store + tools)
- **A nameless `<parameter>` no longer drops a whole recovered tool call.** `tool_recovery::parse_span`
  used `attr_value(after, "name")?`, so a `<parameter>` with no `name` aborted the entire `<invoke>` —
  the recovered call vanished and the bridge phantom-succeeds (the exact failure prose-recovery exists
  to prevent). It now skips the malformed tag and continues, matching `parse_parameter_tags`' contract.
  Test: `nameless_parameter_tag_does_not_drop_the_whole_call`.
- **Session token / step counts exclude soft-deleted messages.** `session_tokens` and
  `session_step_count` joined `usage`→`message` on `session_id` only, so undone/compacted (`active=0`)
  turns still inflated the live TUI token counter and the Lattice "steps" benchmark metric. Added
  `AND m.active = 1`, consistent with every other session-scoped query. Test:
  `session_tokens_and_step_count_exclude_deactivated_messages`.
- **`strip_ansi` no longer leaks a byte from 3-byte charset-designation escapes.** `ESC ( B` / `ESC ) 0`
  (emitted by ncurses/box-drawing tools — tree, pagers, top) are 3 bytes; only 2 were consumed, leaking
  the final byte into model-facing output. The final byte is dropped when the post-`ESC` byte is an
  intermediate `0x20..=0x2f`; 2-byte escapes (`ESC c`, `ESC =`) are unaffected. Test:
  `strip_ansi_drops_three_byte_charset_sequences`.

(A 4th candidate — `watch.rs should_reindex` treating `./`-prefixed paths as dot-dirs — was triaged
real-but-lower-certainty and deferred.)

## [0.4.61] - 2026-06-27

### Fixed (bug-hunt batch 6 — TUI)
- **Input box no longer hides the cursor on wide glyphs.** `input_text_rows` estimated wrapped rows
  from `chars().count()`, but ratatui wraps on terminal cells. A CJK/emoji glyph is 2 cells but 1
  char, so a line of wide glyphs under-counted its rows and the cursor dropped below the visible
  input box. Now counts `unicode_width::UnicodeWidthStr::width`. Test:
  `input_text_rows_counts_cell_width_not_chars`.
- **Full-screen transcript browser re-follows at the visual bottom, not a page past it.** The browse
  loop clamped scroll to `wrapped_len - 1` and re-enabled auto-follow only there, but `transcript_lines`
  renders a `body_h = height - 3` window and clamps display scroll to `wrapped_len - body_h`. After
  scrolling up, returning to the tail left follow OFF for a full extra page (the bottom already showed,
  but new content wouldn't tail). The loop now mirrors the render's `body_h` math for both the clamp
  and the re-follow threshold.

Two further bug-hunt-6 TUI items (inline-viewer follow re-enable; selection across the
committed-cache/streaming-tail boundary) need wider plumbing and are deferred to a follow-up rather
than rushed.

## [0.4.60] - 2026-06-27

### Fixed (bug-hunt batch 5 — provider + store)
- **Claude `overloaded` errors now trigger failover.** The in-band error classifier (v0.4.52) caught
  rate-limit/auth but NOT `overloaded`/server errors, so a transient overload surfaced as a
  non-retryable `Request` and the mesh wouldn't bench or fail over. Now mapped to `Unavailable`
  (retryable). Test: `in_band_overloaded_is_retryable_for_failover`.
- **Session message counts no longer include soft-deleted rows.** `message_count` and the
  `list_sessions` count subquery omitted `AND active = 1`, so undone/compacted messages inflated the
  counts in the session picker / `forge sessions`. Test extended:
  `deactivate_excludes_messages_from_load_but_keeps_earlier_ones`.
- **`load_findings` is actually ranked now.** The query had no `ORDER BY` despite the doc promising
  (severity, confidence) ordering, so the assay UI showed the least-important finding first. Added the
  ordering.
- **`parse_secs` reads retry-after hints from pretty-printed JSON.** A newline/tab between key and
  value (`"retryDelay":
  "37s"`) aborted the parse; it now skips all whitespace.

## [0.4.59] - 2026-06-27

### Fixed (bug-hunt batch 4 — subagent routing)
- **Subagent provider concurrency cap could be bypassed after a failover.** `orchestrate` routed each
  child to size/acquire its per-provider permit, then `run_subagent` routed AGAIN internally — and if a
  concurrent child benched the first model in between, the second route returned a different provider,
  so the child held provider A's permit while running every call against provider B (silently
  exceeding B's cap, the guard that stops a burst from draining one subscription). Each child is now
  routed ONCE and the `RoutingDecision` is threaded into `run_subagent`.

## [0.4.58] - 2026-06-27

### Fixed (bug-hunt batch 4 — worktree, subagent, assay, compaction)
- **Subagent worktree merge-back silently dropped file deletions and renames.** `merge_worktree_back`
  used `--diff-filter=ACM`, so a write-capable child that deleted or renamed a file produced no patch
  entry and the change vanished with a clean-looking merge. Now `ACDMR`. Test:
  `merge_worktree_back_applies_a_deletion`.
- **A subagent that hit the step cap reported empty SUCCESS.** It returned `{final_text:"", ok:true}`,
  so the parent assembled a blank `[agent N]` block and proceeded as if the child finished. Now
  `ok=false` with a clear "hit the N-step limit" message.
- **`assay --scope diff` silently dropped STAGED changes.** It ran `git diff` (working tree vs index),
  so a fully-staged change looked like "no uncommitted changes". Now `git diff HEAD`.
- **Compaction summary couldn't see tool calls.** Tool-call assistant messages (empty content) rendered
  as blank lines, so the summarizer never knew which files/commands a turn touched. Tool calls (name +
  args) are now included in the summary input.

## [0.4.57] - 2026-06-27

### Fixed (bug-hunt batch 4 — rewind/seq model)
- **`/checkpoints` picker rewound to the wrong turn after compaction** (regression from v0.4.51).
  v0.4.51 made `rewind_to` take a transcript INDEX (to fix the `/undo` data-loss), but the checkpoint
  picker passes a DB SEQ — so after compaction (where index and seq diverge) it double-offset and
  rewound to the wrong turn or no-op'd. `rewind_to` now takes a DB seq (the stable identity both
  `/undo` and the picker use); `undo()` maps its transcript index to a seq. Test:
  `checkpoint_rewind_by_db_seq_after_compaction_targets_the_right_turn`.
- **`reload_full_context` set `self.seq` to the loaded row count** (the same class of bug v0.4.51 fixed
  in `resume`, but this spot was missed). `load_all_messages` includes soft-deleted rows from prior
  rewinds, so the count exceeded `MAX(seq)+1`, reusing seqs / corrupting the rewind offset. Now uses
  `next_seq_for_session`.

## [0.4.56] - 2026-06-27

### Fixed (bug-hunt batch 3 — config)
- **A stdio MCP server's extra secret env vars were silently dropped.** `server_from_json` keyed the
  captured secret by the server NAME and `McpAuth` has one keyring slot per server (`mcp:<name>`), so
  a stdio server declaring two secret env vars had the first silently overwritten by the second
  (last-wins). Now the first is kept deterministically and any extra is flagged loudly in the import
  notes — and never dropped into plain env (which would expose it). Multiple secrets need separate
  servers. Test: `stdio_server_with_two_secret_envs_keeps_one_and_flags_the_extra`.

## [0.4.55] - 2026-06-27

### Fixed (bug-hunt batch 3 — index cross-repo contamination)
- **Lattice PageRank + repo-map were not scoped to the project.** The store is global (one DB across
  every project), but `recompute_pagerank` and the repo-map ran UNSCOPED queries
  (`lattice_node_ids_and_names`, `lattice_ref_edges`, `lattice_nodes_ranked`) that scanned every
  project's rows. So a reference to `foo` in projectA distributed PageRank to all `foo` nodes across
  ALL projects, and projectB's map could list projectA's symbols. All three queries now take a
  `repo_root` and `JOIN lattice_file … WHERE f.repo_root = ?` — matching the already-scoped impact/ref
  queries. Test: `map_and_pagerank_are_scoped_to_their_own_repo_root`.

## [0.4.54] - 2026-06-27

### Fixed (bug-hunt batch 3 — mcp, index, skills)
- **MCP lazy reconnect was permanently unreachable after a mid-session drop.** `classify_call_error`
  marked the server `Reconnecting` but never cleared the dead peer, so `peer_for` kept returning the
  stale handle and the `reconnect()` path never ran — every later call failed. The peer is now cleared.
- **Dotfile source files were never refreshed by the watcher.** `should_reindex` applied the
  skip-directory test to the FILENAME component, so `.eslintrc.js` / a hidden `.foo.rs` (which the
  initial `update()` walk DOES index) were excluded. Only directory components are skip-tested now.
- **Frontmatter split leaked a BOM into the prompt body.** `split()` stripped the BOM into `s` but
  returned the raw (BOM-prefixed) string on the no-fence paths, so U+FEFF reached the model. Fixed.

## [0.4.53] - 2026-06-27

### Fixed (bug-hunt batch 3 — forge-tools)
- **`read_file` panicked on an inverted line range.** `start_line` > `end_line` (e.g. 8/3) sliced
  `lines[7..3]` and crashed the whole turn on untrusted tool input. Start is now clamped to `end` (an
  inverted range yields an empty result). Test: `read_file_inverted_line_range_does_not_panic`.
- **`strip_ansi` leaked OSC escape payloads into model output.** A non-CSI escape dropped only one
  char, so `ESC]0;title BEL` (window titles) and `ESC]8;;url ST` (hyperlinks) — emitted constantly by
  color-enabled programs in PTY mode — leaked their content. OSC sequences are now consumed to their
  BEL/ST terminator. Test: `strip_ansi_consumes_whole_osc_sequence`.

## [0.4.52] - 2026-06-27

### Fixed (bug-hunt batch 2 — verified logic bugs in mesh + provider)
- **In-band CLI rate-limit errors now trigger failover.** A subscription that hit its quota mid-turn
  emitted an in-band rate-limit error wrapped as a non-retryable `Request`, so the mesh surfaced a hard
  failure instead of benching the model and failing over to a fallback. Now classified `RateLimited`/
  `Auth` (retryable). Test: `in_band_rate_limit_is_retryable_for_failover`.
- **`record_session(None)` no longer preserves a stale session id.** A fresh-transcript bridge turn that
  produced no session handle left the prior id in place, so the NEXT turn `--resume`d the PRIOR session
  and skipped the current turn's context. `None` now clears it.
- **`/mesh` explain uses the routed tier, not the classified one.** `ranked_rows`/`spread_probability`
  ran before `decide()` could downshift the tier, so the explanation showed the routed pick ranked among
  the wrong tier's rows with the wrong conservation probability.
- **Cross-tier fallback rationale reports the real reason** (no usable key / benched / quota exhausted)
  instead of always "no usable key".

## [0.4.51] - 2026-06-27

### Fixed
- **P0 data loss: `/undo` after a compacted resume wiped pre-compaction history**
  (`crates/forge-core/src/lib.rs`, `crates/forge-store/src/lib.rs`). After compaction, a resumed
  session's in-memory transcript is just the active tail (+ a synthetic summary), but the DB seqs
  start high. `self.seq` was set to the loaded message COUNT (e.g. 7) instead of `MAX(seq)+1` (e.g.
  16), and `rewind_to` used the transcript INDEX directly as the DB seq — so undoing the next turn ran
  `deactivate_messages_from(low_index)` and soft-deleted the surviving pre-compaction messages. Fixed
  with `Store::next_seq_for_session` (MAX+1) on resume and an index→seq offset in `rewind_to` (0 when
  not compacted, so no behavior change for normal sessions). Test:
  `undo_after_compacted_resume_does_not_wipe_survivors`.
- **Doom-loop nudge was dropped on the concurrent read-only batch path.** The "change approach" nudge
  is queued in `pending_hints`, but only the serial tool path drained them — so a model looping on a
  concurrent batch was halted "after a nudge" it never actually received. The concurrent path now
  drains the hints too. Test: `concurrent_batch_doom_nudge_is_delivered_to_the_model`.

### Found by
A verify-first multi-agent bug-hunt over the core run-loop, mesh, store, and provider paths.

## [0.4.50] - 2026-06-27

### Fixed (diagnostics — clearer harness output, from a verified UX/observability audit)
- **Direct-model continue-nudge exhaustion is no longer silent.** A model that narrates forever with a
  task still open got nudged a bounded number of times, then the turn ended with NO warning (the
  bridge path always warned). Now surfaces a "giving up — send `continue` to resume" warning. Test
  `direct_continue_nudge_exhaustion_warns_when_giving_up`.
- **Oscillation guard says "alternating", not "repeated".** An A,B,A,B oscillation emitted the same
  "repeated the same tool call" message as a true A,A,A repeat; now distinguished.
- **Bridge stdin-write failure surfaces its cause.** A failed prompt write (child died before reading)
  was logged but not shown; the stall message now appends the real cause instead of reading as an
  unexplained 300s timeout.
- **Setup hint no longer printed twice** when a bridge CLI exits non-zero with empty stderr. Test
  extended to assert no duplication.
- **Plain-mode slash-command hint fixed** — said "use `forge chat`" when the user was already in
  `forge chat`; now "run `forge chat` (without --plain)".

## [0.4.49] - 2026-06-27

### Added (tests)
- `autofix_iteration_cap_halts_the_self_heal_loop` — pins the last untested run-loop backstop: the
  lint/test self-heal loop stops at `max_iterations` when checks never pass (drives a real turn that
  edits a file to arm autofix, with a lint command that always fails). With this, **every reliability
  guard in the run-loop now has a deterministic test**; the conformance table in `results.md` §2 gains
  the row.

## [0.4.48] - 2026-06-27

### Docs
- **`docs/harness/why-forge-is-a-better-harness.md`** — the honest, test-backed case that Forge's
  harness beats the loop inside the raw CLI it wraps: a failure-mode table where every row cites a
  deterministic conformance test, the fair-accounted N=20 SWE-bench resolve numbers, the routing/
  failover/permission features the raw CLIs lack, and a plain statement of where Forge does NOT win
  (raw token efficiency). Linked from the README. No new claims — every row maps to an existing test
  or measured result.

## [0.4.47] - 2026-06-27

### Added
- **Bridge stream-resilience tests** (`crates/forge-provider/src/cli_provider.rs`):
  `truncated_stream_line_is_skipped_not_fatal` (a corrupt NDJSON line between valid lines is skipped,
  not fatal) and `orphan_tool_result_without_started_does_not_panic_or_phantom` (a `tool_result` with
  no preceding `tool_use` neither panics nor synthesizes a phantom call).

### Docs
- **forge-lsp public API documented** — module doc + `///` on `LspRegistry`, `from_config`,
  `diagnostics_for`, `lang_from_ext`, `repo_root`, `which` (v1.0 surface polish).

## [0.4.46] - 2026-06-27

### Fixed
- **Concurrent read-only batch now feeds the failure-loop guard** (`crates/forge-core/src/lib.rs`).
  A batch of ≥2 read-only calls ran concurrently but its results never reached `failure_counts`, so a
  model issuing two `read_file`s with different missing paths every step evaded BOTH the identical-call
  doom-loop (signature changes) and the failure-loop (concurrent path untracked) — burning the
  step/token budget to the cap. `run_readonly_batch` now returns per-call failure classifications and
  the caller folds them in, exactly like the serial path.

### Added (tests — proving existing guards)
- `step_cap_halts_a_runaway_turn` pins the primary infinite-loop backstop (`max_steps`).
- `concurrent_batch_failure_loop_is_caught` proves the fix above.
- `completion_gate_covers_its_four_outcomes` — pure decision table for the completion authority.
- `docs/benchmarks/results.md` §2 conformance table extended with every new guard (oscillation, bridge
  prose-recovery + double-exec guard, concurrent-batch failure-loop, step cap, completion-gate,
  malformed-`<parameter>` panic-safety), each row backed by a deterministic test.


## [0.4.45] - 2026-06-27

### Fixed
- **P0 panic: malformed `<parameter>` tag crashed the whole turn** (`crates/forge-provider/src/tool_recovery.rs`).
  When a model emitted a `<parameter>` open tag missing its `>`, the first `>` landed inside the closing
  `</parameter>`, making `gt > val_end` so the byte-range slice (`after[gt..val_end]`) panicked — on
  untrusted model output, with no failover. This was reachable on the bridge after v0.4.44 routed prose
  recovery there. Both slice sites (`parse_invoke_span`, `parse_parameter_tags`) now use guarded
  `.get(gt..val_end)` and stop parsing params on a malformed tag. The fuzz corpus
  (`recovery_never_panics_on_adversarial_input`) gained `<parameter>` fragments that reproduce it, plus
  a direct test `malformed_parameter_tag_does_not_panic`.
- **P1 panic: resume mutex poisoning bricked every later turn** (`crates/forge-provider/src/cli_provider.rs`).
  `self.resume.lock().unwrap()` panicked permanently once the mutex was poisoned by any prior panic. Now
  poison-tolerant (`unwrap_or_else(PoisonError::into_inner)`) — a poisoned lock degrades to a fresh,
  full-transcript turn instead of a sticky brick.
- **P1: prose-recovery could double-execute a tool the CLI already ran.** Recovery now only fires when the
  turn streamed ZERO native tool events (`tool_names` empty) — the pure prose-fallback case. If the CLI
  executed a tool natively, a tool-shaped fragment in the final text is treated as prose, not re-executed
  (guards against double-running a destructive `shell`/write). Test:
  `prose_recovery_skipped_when_cli_ran_a_native_tool`.

## [0.4.44] - 2026-06-27

### Fixed
- **Bridge prose-tool-call recovery — stops a 553× spiral** (`crates/forge-provider/src/cli_provider.rs`).
  A bridged claude/codex model sometimes writes a tool call as TEXT
  (`<function_calls><invoke name="mcp__forge__read_file">…`) instead of a native `tool_use`. The CLI
  doesn't execute text, so the call landed in the final content, ran nowhere, and the model — seeing no
  result — repeated it until the turn died (observed live: **553 unexecuted `<function_calls>` on a
  single SWE-bench instance**, contaminating a measurement run). The bridge now runs
  `recover_text_tool_calls` on its output (the same recovery the direct/genai path already had), so the
  run-loop executes the recovered call and re-drives with a real result. Only fires on actual tool-call
  markup; native calls the CLI already ran stream as events (not text), so there's no double-execution.
  Test: `recovers_prose_tool_call_the_bridge_did_not_execute`.
- **Reverted the v0.4.41 "Exploring efficiently" harness-preamble nudge.** Pushing the bridge model to
  batch reads via `read_file paths[]` / `search context:N` measurably increased how often it emitted
  those calls as *prose* (the trigger for the spiral above), and its benefit was unproven (round-trips
  fell ~21% but total tokens were flat at N=2). The batch capabilities themselves remain available for
  native use; Forge just no longer steers the model toward them. With the nudge gone AND prose-recovery
  in place AND the v0.4.42 oscillation guard as a backstop, the bridge is robust to this failure mode.

## [0.4.43] - 2026-06-27

### Changed
- **CI now runs `cargo clippy --locked`**, so a `Cargo.lock` out of sync with `Cargo.toml` fails a PR
  instead of slipping through to the release (`.github/workflows/ci.yml`). The lock had silently
  drifted since v0.4.37 — the bump step ran `cargo update -p forge`, a no-op (the root crate is
  forge-cli) — and only the release build's `--locked` caught it, failing v0.4.42's first release in
  14s. `--locked` on the clippy job catches that drift on every PR from now on.

## [0.4.42] - 2026-06-27

### Fixed
- **Doom-loop oscillation guard** (`crates/forge-core/src/lib.rs`). The consecutive doom-loop guard
  missed an `A,B,A,B,…` ping-pong (every step differs from the one before, so its repeat counter kept
  resetting), and the failure-loop missed it too (an interleaved *successful* call clears the per-tool
  failure streak). So a model alternating a failing/empty call with a trivial successful one — observed
  live: an empty `shell({})` alternating with `ls -la` after a mid-run failover to a local model — ran
  to the step cap / timeout instead of halting. Added a sliding-window oscillation count: a signature
  recurring ≥ threshold times within the last 6 steps trips the same two-stage nudge-then-halt.
  Conformance test `doom_loop_halts_a_model_oscillating_between_two_calls`.
- **Recover `<function=…>` tool calls that carry `<parameter>` sub-tags** (`crates/forge-provider/src/
  tool_recovery.rs`). Some local models (observed: ollama qwen3-coder on failover) emit a mixed format
  — a Llama-style `<function=NAME>` tag whose body is not JSON but Anthropic-style `<parameter …>`
  sub-tags. Recovery extracted the name but returned empty args → an empty no-op call that looped.
  Now parses the sub-tags as a fallback (both `<parameter=key>` and `<parameter name="key">`). Both
  bugs were found during the v0.4.41 batch-tool measurement.

## [0.4.41] - 2026-06-27

### Added
- **`search` gains optional `context` lines (grep -C)** (`crates/forge-tools/src/core_tools.rs`).
  `context: N` prints N lines around each match (match lines as `path:lineno:`, context as
  `path:lineno-`, `--` between hunks; adjacent hits merge into one hunk), so a search result is often
  enough to understand a hit WITHOUT a follow-up `read_file`. Bounded by a 64 KiB output cap.
- **Harness preamble nudges the bridge model to batch exploration** (`crates/forge-provider/src/
  cli_provider.rs`). An "Exploring efficiently" clause tells the bridged claude/codex to read several
  files in one `read_file` `paths` call and to pass `search context:N` instead of search-then-read.
  Without this the new batch affordances went unused; with it the model adopts them immediately.

### Measured (small, honest)
- A 2-instance clean A/B (same instances, same model, old vs new binary; a third instance was
  discarded after a mid-run rate-limit failover to a local model corrupted it) shows the nudge drives
  **100% of searches to use `context`** (vs 0% before) and **~21% fewer tool round-trips** (23 → 18) —
  a direct hit on the structural per-step-MCP-latency gap. **Tokens were flat** (617k → 619k): the
  context lines cost about what the saved round-trips would have, so this is a **latency win, not a
  token-efficiency win** at this sample. Reported straight; N=2 is a mechanism check, not a proof.

## [0.4.40] - 2026-06-26

### Added
- **Batch `read_file` — read several files in one tool call** (`crates/forge-tools/src/core_tools.rs`).
  `read_file` now accepts an optional `paths` array in addition to single `path`; the files come back
  in one response under `===== <path> =====` headers. This directly attacks the harness's structural
  cost on the bridge — each MCP tool round-trip re-processes the growing context, and the explore phase
  was the round-trip-heaviest (500+ tool calls on the worst SWE-bench instances). Batching the read
  phase collapses N round-trips into one. A missing/unreadable file in a batch becomes an inline
  `[error: …]` block (partial context still helps) rather than failing the whole call; per-file
  (64 KiB) and total (256 KiB) caps bound context, with remaining files noted, not silently dropped.
  Single-path behavior (incl. `start_line`/`end_line`) is unchanged.

## [0.4.39] - 2026-06-26

### Changed
- **SWE-bench firming run extended to N=20** (`docs/benchmarks/results.md`). The N=10 Forge-on-bridge
  vs claude-cli result was re-measured on 20 instances (the original 10 + 10 fresh Lite instances),
  same model, same official evaluator. The win **holds and gets cleaner**: Forge loop-gated resolves
  **11/20 vs claude-cli's 9/20** at **~11% lower cost per resolve** (1.20M vs 1.35M tokens/resolve),
  and total tokens fall to **1.08×** (near parity, down from N=10's 1.39×). Reported straight,
  including the honest caveat that the *new* 10 instances tied 5/5 — the +2 net edge comes from the
  first batch, so the resolve advantage is real but modest.

## [0.4.38] - 2026-06-26

### Added
- **Conformance tests for the opt-in loop-gated completeness re-drive.** Two deterministic run-loop
  tests with a scripted bridge provider (runs a read-only tool, then yields) lock the behavior shipped
  in 0.4.37: the completeness re-drive fires **exactly once** when `mesh.verify_completeness` is on (the
  one-shot guard prevents a loop), and **never** when it's off (the default path is unchanged)
  (`crates/forge-core/src/lib.rs`).

## [0.4.37] - 2026-06-26

### Changed
- **Completeness verification is now loop-gated — Forge beats the raw CLI on resolve *and* cost-per-
  resolve.** `mesh.verify_completeness` previously appended an always-on clause to the harness preamble
  (the model carried completeness pressure through the whole turn). It now fires **once** at turn-end
  from the core run-loop: the model works the turn normally, then does a single bounded `git diff`
  review against the request's requirements. Measured (N=10 SWE-bench Lite, same model): holds the
  **6/10 resolve win over claude-cli's 4/10** at **5.53M tokens** — the cost premium fell 3× → 1.85× →
  **1.39×** across the three forms. Headline: **922k tokens per resolve vs claude-cli's 993k (~7%
  cheaper) while solving 50% more bugs** — Forge-on-bridge is now genuinely better on both axes
  (still opt-in, default off; total tokens 1.39× because it does more total work). Removes the preamble
  clause + the temporary A/B env seam (`crates/forge-core/src/lib.rs`,
  `crates/forge-provider/src/cli_provider.rs`).

## [0.4.36] - 2026-06-26

### Changed
- **`mesh.verify_completeness` now uses a bounded one-pass review — same resolve win, 39% cheaper.**
  v0.4.35's completeness clause was open-ended ("re-verify every requirement") and cost ~3× tokens. The
  clause is now a **single bounded final-diff review** (run `git diff` once, check it against the
  request's listed requirements, make targeted fixes only — no re-exploration). Measured on the same
  N=10 SWE-bench Lite set: it **holds the full 6/10 resolve** (still beats claude-cli's 4/10) at
  **6.86M tokens vs 11.3M** — strictly better than the open-ended form it replaces. Honest remaining
  cost: ~1.85× claude-cli's total tokens (~15% higher per *resolve*) — a real solve-rate win at a
  modest premium, still default-off (`docs/benchmarks/results.md` updated with the numbers).

## [0.4.35] - 2026-06-26

### Added
- **`mesh.verify_completeness` — opt-in "max-resolve" mode that beats the raw CLI on resolve rate.**
  When on, the CLI-bridge harness preamble gains a completeness clause: re-read the request and verify
  the change against EVERY requirement before finishing. On SWE-bench Lite (N=10, same model, only the
  clause changed) this took Forge-on-bridge from **4/10 → 6/10 resolved — beating claude-cli's 4/10** —
  by catching under-scoped fixes (e.g. a flask change that handled the blueprint-name dot but missed
  the endpoint dot). The cost is honest: **~3× the tokens** (more re-reading + re-verification), so it
  is **default OFF** — a deliberate quality-for-cost trade you turn on when solve rate matters more than
  spend. Documented with the measured numbers in `docs/benchmarks/results.md`
  (`crates/forge-config`, `crates/forge-provider/src/cli_provider.rs`).

## [0.4.34] - 2026-06-26

### Added
- **`lattice.inject_body_hits` — tunable context front-loading.** Controls how many top-ranked symbol
  *bodies* get injected into a turn's prompt (default 3; previously hardcoded). Raising it front-loads
  more task-relevant code so the model reads from context instead of `search`/`read_file`-ing for it.
  Measured on SWE-bench Lite: aggressive front-loading (14 bodies) took Forge-on-bridge from ~1.9×
  *worse* tokens than the raw `claude` CLI to **rough parity** (tied 4/10 resolve, ~equal tokens, at
  N=10). Honest caveat documented in `docs/benchmarks/results.md`: on a 3-instance light-repo subset it
  looked like a 44% token win that did **not** generalize — small-N benchmarks mislead.

### Fixed
- **`forge bench swe` now bounds the in-process Forge agent by `--timeout-secs`.** Only the external
  CLI path was bounded; the Forge path ran `run_turn` unwrapped, so a non-converging run could spin for
  20+ minutes (observed 500+ tool calls). It now times out like the external agents and submits the
  partial patch (`crates/forge-cli/src/bench.rs`).

## [0.4.33] - 2026-06-26

### Fixed
- **Bridge token accounting dropped cache-read/cache-write tokens — fixed (and corrected a false
  "bridge is more efficient" claim).** The CLI bridge recorded only claude/codex's *uncached*
  `input_tokens`, discarding `cache_read_input_tokens` + `cache_creation_input_tokens`. Forge's
  `Usage.input_tokens` is defined as the *full* input the model processed (cached is a subset), so
  this undercounted input **everywhere** — the statusline token gauge, cost display, and especially
  the SWE-bench efficiency comparison, where the raw-CLI metric *did* count cache reads. That made
  Forge-through-the-bridge look ~10–150× cheaper than the raw CLI purely as a counting artifact. Now
  `cli_provider::usage_from` sums uncached + cache-read + cache-write, and `bench::parse_external_usage`
  counts cache writes too, so both sides are apples-to-apples. A fair re-measure shows the *opposite*
  of the old claim — Forge's MCP-per-turn harness currently costs **more** tokens than the native CLI
  loop; `docs/benchmarks/results.md` is corrected accordingly with the real numbers
  (`crates/forge-provider/src/cli_provider.rs`, `crates/forge-cli/src/bench.rs`).

## [0.4.32] - 2026-06-26

### Fixed
- **Watcher setup no longer leaks a thread (and watcher) per `build_session`.** 0.4.31's fire-and-
  forget watcher parked a thread holding the handle for the *process* lifetime — fine for `forge
  chat` (one session per process) but a leak when `build_session` runs repeatedly in one process:
  `forge bench swe` builds a session **per instance** (hundreds in a run), and `forge replay` too, so
  each leaked a parked thread plus the watcher's own background thread + inotify/poll resources. The
  watcher is now delivered to the session through an `mpsc` channel and **owned by the session** (the
  held `Receiver` keeps it alive; it's dropped when the session ends), and the setup thread exits
  after sending. Still fully non-blocking — no filesystem op gates startup. A test proves a watcher
  held only through the channel (never received) still reindexes (`crates/forge-core/src/lib.rs`,
  `crates/forge-cli/src/cli/commands/run.rs`, `crates/forge-index/src/watch.rs`).

## [0.4.31] - 2026-06-26

### Changed
- **File watching now WORKS on WSL2 `/mnt/*` (9p) and other remote filesystems — via polling instead
  of being disabled.** 0.4.27 stopped the hang by *skipping* the watcher on a non-native filesystem
  (with a "move the project onto the Linux filesystem" caveat). Now, instead of disabling, the watcher
  transparently switches to a **polling backend** on those filesystems (`9p`/`v9fs`, `fuse*`,
  `cifs`/`smb*`, `nfs*`): it stat-walks the project tree on a 2s timer (ordinary file ops that work
  over 9p) rather than registering recursive inotify watches (the per-entry RPCs that block
  uninterruptibly on 9p). So auto-reindex works on a Windows-drive project with **no caveat and no
  manual `forge lattice update`**. Content-comparison is on so even same-size edits are caught. Native
  filesystems keep the efficient inotify backend unchanged.
- **Watcher setup is now fully fire-and-forget**, so neither inotify registration nor the polling
  backend's synchronous initial tree scan (slow over a remote link) can gate TUI startup at all — a
  detached thread owns the watcher for the process lifetime. Removes the previous 5s setup deadline.
  The polling backend is unit-tested end-to-end (an external edit is reindexed) alongside the inotify
  one (`crates/forge-index/src/watch.rs`, `crates/forge-cli/src/cli/commands/run.rs`).

## [0.4.30] - 2026-06-26

### Changed
- **MCP servers now connect in the background instead of gating `forge chat` startup.** The interactive
  path awaited `McpManager::connect_all` before the session was built, so a slow or unreachable MCP
  server stalled TUI startup by up to the per-server connect timeout (20s default) — the same
  "a startup op blocks the UI" class as the 9p watcher hang. It now uses the non-blocking pattern
  `mcp-serve` already uses: `connecting()` marks every active server `Reconnecting` and advertises the
  MCP meta-tools immediately (so the tool surface is ready at once), then a detached task connects them
  — each flips to connected/failed in the `/mcp` panel as it resolves, and the first `mcp_call` lazily
  waits on its own server. Reuses infrastructure already covered by
  `connecting_advertises_meta_tools_without_any_network_io` (`crates/forge-cli/src/cli/commands/run.rs`).

## [0.4.29] - 2026-06-26

### Changed
- **The watch-&-reindex watcher now scopes to the project root and refuses to watch all of `$HOME`.**
  Follow-up to 0.4.27's WSL fix: the recursive watch was rooted at the raw CWD, so launching `forge
  chat` in a home directory tried to recursively `inotify` the entire `$HOME` tree — `.cargo`, cloned
  `.git` repos, caches — thousands of watches and a slow initial walk even on a native filesystem
  (the original bug report watched the user's whole home dir, pulling in `.cargo/.git`). Startup now
  resolves the watch root to the nearest enclosing project marker (`.git`/`.forge`/`AGENTS.md`,
  climbing no higher than `$HOME`) and, when that resolves to the home directory itself, skips the
  watcher with a clear line ("launched in the home directory with no project root — open a project
  folder…") instead of watching everything. A marker-less subdirectory still watches just that dir.
  Pure resolver unit-tested (project-root climb, home refusal, dotfiles-`.git`-in-`$HOME`, fail-open
  when `$HOME` is unknown) (`crates/forge-index/src/watch.rs`, `crates/forge-cli/src/cli/commands/run.rs`).

## [0.4.28] - 2026-06-26

### Security
- **Built-in secret-read denylist now covers the common non-`cat` read/exfil verbs.** The shell deny
  rules blocked `cat`/`less`/`head`/`tail`/`type`/`more` on `.env`/keys/credentials, but an agent
  could still read a secret by shelling out via the OTHER obvious verbs: text tools
  (`grep`/`egrep`/`rg`/`awk`/`sed`/`nl`/`sort`/`cut`), binary dumps/encoders for exfil
  (`xxd`/`od`/`strings`/`base64`), or `source`/`.` which executes a dotenv straight into the
  environment. Those are now denied too (defense-in-depth on top of the `read_file`/`list_dir` tool
  block, which already stops the non-shell path). Verified each is denied even under `bypass` while
  ordinary uses of those verbs on non-secret files (`grep TODO src/main.rs`, `base64 logo.png`, …)
  stay allowed (`crates/forge-config/src/lib.rs`, test in `crates/forge-core/src/permission.rs`).

## [0.4.27] - 2026-06-26

### Fixed
- **`forge chat` no longer hangs on a blank screen in a WSL2 `/mnt/*` (9p/DrvFs) directory.** At
  startup Forge sets up a recursive inotify watcher (watch-&-reindex) rooted at the working dir, and
  the registration was on the TUI-init critical path. On a 9p mount (WSL2's Windows-drive DrvFs) the
  recursive watch issues a per-entry RPC to the Windows host for the whole tree, some of which block
  uninterruptibly (`D` state in `p9_client_rpc`) — so the TUI never rendered and the process hung
  until Ctrl-C. `forge run` (no watcher) and projects on the Linux filesystem (ext4 `~`) were fine.
  Two fixes: **(1)** the watcher now detects a non-native filesystem via `/proc/self/mountinfo`
  (`9p`/`v9fs`, `fuse*`, `cifs`/`smb*`, `nfs*`) and skips the recursive watch with one clear line
  ("working dir is on a Windows drive (9p/DrvFs) — file watching disabled … move the project onto the
  Linux filesystem") instead of blocking — so it works for ALL WSL `/mnt/*` paths, not just the
  reported one; **(2)** watcher setup now runs on a detached thread with a 5s deadline, so *no*
  filesystem stall (remote mount, locked dir) can ever gate TUI startup again. Retrieval is
  unaffected when the watcher is skipped — re-run `forge lattice update` to reindex after edits
  (`crates/forge-index/src/watch.rs`, `crates/forge-cli/src/cli/commands/run.rs`).

## [0.4.26] - 2026-06-26

### Added
- **Property test locking the permission broker's security invariants.** `permission::decide` is the
  single chokepoint that gates every dangerous tool (shell, `.env`/secret reads, untrusted MCP), and
  its layered ordering is easy to break in a refactor. A deterministic seeded-LCG property test runs
  5000 random combinations of mode × side-effect × tool × args × rule-set and asserts the three
  guarantees that must never regress: (1) ANY matching Deny rule wins over any allow, in every mode;
  (2) a Builtin Deny holds even under `Bypass` (the unoverridable `.env`/secret floor); (3) with no
  matching deny, `Plan` mode denies every non-ReadOnly side effect (the hard read-only contract no
  allow rule can escape). Confirms no bypass hole exists today and guards the boundary going forward
  (`crates/forge-core/src/permission.rs`).

## [0.4.25] - 2026-06-26

### Added
- **Deterministic fuzz for `clamp_to_chars` (prompt-cap boundary contract).** The function that trims
  an over-long bridge prompt to `codex exec`'s `input_too_large` cap does raw char-index arithmetic on
  a `Vec<char>` — the exact shape that produced char-boundary panics before (v0.3.10) — and carries a
  hard contract: the result must never EXCEED `max_chars` (codex rejects the turn otherwise) and must
  stay valid UTF-8. A seeded-LCG fuzz throws 6000 random multi-byte/emoji/combining-char strings at
  random caps (biased toward the degenerate 0/1/around-marker-length region where boundary bugs live)
  and asserts: no panic, result char-count ≤ cap, and an already-fitting prompt returned unchanged.
  Completes the P0.1 fuzz triad with 0.4.23/0.4.24 (`crates/forge-provider/src/cli_provider.rs`).

## [0.4.24] - 2026-06-26

### Added
- **Deterministic adversarial fuzz for the bridge stdout parsers.** Every bridge turn streams the CLI
  subprocess's stdout line-by-line through `parse_line` (claude/codex/antigravity) and, in harness
  mode, `parse_sink_line` — UNTRUSTED input that drifts with each CLI version, where a panic crashes
  the turn mid-stream (worse than a clean failure: partial/inconsistent state). A seeded-LCG fuzz test
  throws 6000 pathological JSON-event lines (truncated/unbalanced JSON, wrong-typed fields, real event
  `type`s with missing payloads, control chars, huge repeats, unicode) at all three bridge parsers +
  the sink parser and asserts no panic + determinism on every one. No new dependency; identical corpus
  on every CI box (pairs with 0.4.23's tool-recovery fuzz) (`crates/forge-provider/src/cli_provider.rs`).

## [0.4.23] - 2026-06-26

### Added
- **Deterministic adversarial fuzz for tool-call recovery (untrusted-input hardening).**
  `recover_text_tool_calls` parses UNTRUSTED model output, and a panic there crashes the whole turn
  (it can't even fail over — the worst failure mode). A new seeded-LCG fuzz test throws 5000
  pathological strings (unbalanced braces, truncated JSON, real tool-call markers spliced mid-prose,
  control chars, deep nesting, huge repeats, unicode) at both recovery entry points and asserts: no
  panic, every recovered call has a non-empty name (no silently-undispatchable phantom call), and
  determinism. No new dependency; the corpus is identical on every CI box (P0.1 "fuzz tool-recovery")
  (`crates/forge-provider/src/tool_recovery.rs`).

## [0.4.22] - 2026-06-26

### Changed
- **`forge doctor` runs its live probes concurrently.** The two LIVE diagnostics each looped
  sequentially: `provider_reachability_checks` awaited each keyed provider's `list_models` one at a
  time (N × 8s) and `bridge_roundtrip_checks` launched each CLI bridge one at a time (3 × 30s ≈ 90s
  worst case). Both now `join_all` their independent probes — doctor pays the *slowest single* probe
  per section instead of the sum — so a multi-provider, multi-bridge environment finishes in seconds
  instead of minutes. Same checks, same severities, same stable output order (`forge-cli/doctor.rs`).

## [0.4.21] - 2026-06-26

### Changed
- **Model discovery probes every provider concurrently.** Startup queried each keyed provider's
  model list in a sequential loop, paying the *sum* of per-provider timeouts (3 keyed providers ×
  8s ≈ 24s worst case on a slow/cold network). It now `join_all`s the probes — startup pays the
  *slowest single* provider's budget (~8s). Same logging, same deterministic catalog order; the
  per-provider logic moved into a `discover_provider_models` helper.

## [0.4.20] - 2026-06-26

### Added
- **`docs/benchmarks/results.md`: recorded the verified prediction pipeline + a single-task token
  smoke.** Both `forge bench swe` agent paths (`--agent forge`, `--agent claude-code`) are verified
  end-to-end on a real instance. A single trivial task on the same model produced 793 tokens via the
  Forge bridge vs 46,124 via Claude Code's own CLI — documented with explicit caveats (overhead-
  dominated on a trivial task, NOT a resolve-rate result; the representative figure needs the full
  Docker-scored run). Confirms the comparison harness works and the direction of the efficiency thesis.

## [0.4.19] - 2026-06-26

### Fixed
- **SWE-bench predictions now capture NEW files (the harness was undercounting Forge).** `forge bench
  swe` built the `model_patch` with a plain `git diff`, which **ignores untracked files** — so a
  solution that *adds* a file (very common in SWE-bench: new modules, regression tests) produced an
  **empty patch** and was scored *unresolved* even though the agent did the work. Now it stages
  everything (`git add -A` with the same excludes) then `git diff --cached`, capturing additions,
  modifications, and deletions alike. Found by running the prediction pipeline end-to-end on a real
  instance for the first time (the agent created the file; the captured patch was empty); fixed +
  regression-tested + re-run end-to-end (now an 8-line patch) (`crates/forge-cli/src/bench.rs`).

## [0.4.18] - 2026-06-26

### Added
- **`docs/benchmarks/results.md` — the measured "proven with metrics" record.** Consolidates the
  harness's evidence: the bridge-resume efficiency result (~92% fewer prompt bytes over a 6-re-drive
  turn, with the deterministic test + live e2e to reproduce), the harness conformance matrix (each
  reliability guard + how to run it), and where the Docker-gated SWE-bench resolve-rate run plugs in.
  Separates in-repo CI proofs from the external gold-standard run.

## [0.4.17] - 2026-06-26

### Fixed
- **Transcript text now wraps on terminal cell width, not char count (wide-glyph overflow).** The
  line wrappers (`wrap_lines`, `wrap_words`) counted characters per row, but a CJK ideograph / emoji
  is 2 cells — so a line of wide glyphs over-filled each row and the renderer overflowed or truncated
  it. Both now measure `unicode-width` cells and break a wide glyph to the next row instead of
  splitting it. Pairs with 0.4.16's selection fix: wide glyphs now both render AND select correctly
  (`crates/forge-tui/src/transcript.rs`, `crates/forge-tui/src/app.rs`).

## [0.4.16] - 2026-06-26

### Fixed
- **Mouse text-selection no longer drifts on wide glyphs (CJK / emoji).** A selection records the
  screen **cell** column, but the extractor sliced the line's `[char]` using that cell offset as a
  char index — and a CJK ideograph or emoji is 2 cells but 1 char, so every boundary shifted right by
  one per wide glyph before it (and could slice past the string). Added a width-aware
  `cell_to_char_index` (walks chars summing their `unicode-width`) and convert the selection bounds
  before slicing, so the copied text matches what was highlighted (`crates/forge-tui/src/app.rs`).

## [0.4.15] - 2026-06-26

### Added
- **Test coverage for the `forge bench report` comparison logic.** The headline "Forge bridging model
  X vs X's own CLI" report (#227) had no test for its arithmetic or its honesty gates. Extracted the
  aggregation into pure `summarize_agent()` + `tok_per_success_cell()` and unit-tested them: an
  instance counts as resolved only if the official scorer put its id in `resolved_ids`; token totals
  include **only** `metrics_complete` rows (a partial capture can't understate tokens-per-success and
  flatter Forge); and tokens-per-success prints a real number **only** with eval results + ≥1 resolved
  + complete capture, else `incomplete`/`n/a`. Locks down the integrity of the proven-with-metrics
  comparison (`crates/forge-cli/src/bench.rs`).

## [0.4.14] - 2026-06-26

### Fixed
- **More HTTPS-on-a-CA-less-host panics swept.** Following 0.4.13 (forge-mcp), the remaining
  `reqwest::Client::new()`/`builder()` sites that trust the OS cert store and panic at construction
  on a bare container / minimal image are fixed: the ollama embedder (`forge-index/embed.rs` — it
  panics even though ollama is plain HTTP, since the panic is in TLS-backend setup, not at connect)
  and the three `web_fetch`/`web_search` clients (`forge-tools/web.rs`). Each crate gets a local
  bundled-CA helper seeded with `webpki-root-certs` (mirrors forge-provider/forge-mcp, which they
  can't depend on). Forge now does HTTPS everywhere without a system trust store.

## [0.4.13] - 2026-06-26

### Fixed
- **MCP HTTPS no longer panics on a CA-less host.** `forge-mcp` built its OAuth-refresh client with
  `reqwest::Client::new()` and its streamable-http transport client with a plain
  `reqwest::Client::builder()` — both trust the OS certificate store and **panic internally** on a
  bare container / minimal image that has none (the same landmine fixed for the API client in
  0.4.1/#226, which `forge-mcp` didn't share because it can't depend on `forge-provider`). Added a
  `bundled_client_builder()` seeded with Mozilla's `webpki-root-certs` and routed both the OAuth flow
  and the transport through it, so connecting to a remote (HTTP/SSE) MCP server works without a system
  trust store (`crates/forge-mcp/src/transport.rs`, `crates/forge-mcp/src/oauth.rs`).

## [0.4.12] - 2026-06-26

### Fixed
- **No more misleading "model discovery failed — check your key" warning for completion-only
  providers (Cerebras).** Cerebras has no native genai adapter, so `list_models()` can't enumerate
  it and auto-discovery logged a scary keyed-provider WARN claiming its models "won't be routable
  (check the key / network)". But Cerebras *completion* works fine via the custom service-target
  resolver — it's just config-only (no model-listing API). Added `forge_provider::is_discoverable`
  and the discovery loop now skips such providers quietly with accurate guidance (pin
  `cerebras::<model>` or add it under `[mesh.models]`) instead of the alarming, inaccurate warning.
  Verified live (`crates/forge-provider/src/genai_provider.rs`, `crates/forge-cli/src/cli/commands/models.rs`).

## [0.4.11] - 2026-06-26

### Added
- **A reproducible metric for the bridge-resume efficiency win.** claude's own token accounting hides
  the saving (it prompt-caches the repeated transcript), so a deterministic test
  (`resume_sends_dramatically_fewer_prompt_bytes_over_a_turn`) measures what Forge actually controls:
  the prompt bytes streamed to the CLI's stdin across a multi-re-drive turn. Result: **~92% fewer
  prompt bytes** (a 4000-char system preamble + accumulating assistant/tool turns over 6 re-drives:
  ~59.7 KB sent without resume vs ~4.2 KB with). This is the concrete "proven with metrics" backing
  for the 0.4.9/0.4.10 claude + codex session resume (`crates/forge-provider/src/cli_provider.rs`).

## [0.4.10] - 2026-06-26

Bridge efficiency, part 2: codex session resume.

### Added
- **codex bridge session resume** — extends the 0.4.9 claude resume to the codex (ChatGPT) bridge, so
  both major subscription bridges now send only the NEW messages per turn instead of re-streaming the
  whole transcript. codex resumes via the `exec resume <id>` SUBCOMMAND (keyed by its `thread_id`),
  which — unlike claude's `--resume` flag — **rejects `--sandbox`** on resume (the recorded session's
  sandbox is inherited), so the codex arg path is rewritten `exec …` → `exec resume <id> …` with the
  sandbox pair dropped and every other harness flag kept. The model-match gate prevents codex's
  model-change warning. Verified with unit tests (the `exec resume` rewrite + `--sandbox` drop) and an
  `#[ignore]` live e2e (`e2e_codex_resume_preserves_context_across_calls`) that drives two real `codex`
  turns and asserts the resumed turn recalls a fact from turn 1 (`crates/forge-provider/src/cli_provider.rs`).

## [0.4.9] - 2026-06-26

Bridge efficiency: claude session resume (the bridge-superiority lever).

### Added
- **CLI-bridge session resume (claude `--resume`) — Forge through the bridge now sends only the NEW
  messages per turn instead of re-rendering the whole transcript every call.** A bridge `complete()`
  is a fresh process each time and used to re-stream the entire conversation; for a multi-step turn
  that re-drives several times, that re-sends the full history on every call — the biggest source of
  bridge token waste. Now Forge captures claude's own `session_id` from its `stream-json` output and,
  on the next call, spawns `claude -p --resume <id>` streaming only the delta (a `continue` nudge or
  the new user turn). claude reloads the prior context from its own session store — fewer tokens in
  *and* a prompt-cache hit on its side. Safety: a transcript shrink (compaction) or model change
  (failover) forces a fresh session, and any failed resumed turn optimistically resets so the retry
  is fresh; claude-only (`with_session_resume(false)` is the escape hatch); codex/agy unchanged.
  Verified with unit tests + an `#[ignore]` live e2e that drives two real `claude` turns and asserts
  the resumed turn recalls a fact from turn 1 while only the delta was sent
  (`crates/forge-provider/src/cli_provider.rs`).

## [0.4.8] - 2026-06-26

More harness conformance tests — empty-response and tool-call-as-text guards.

### Added
- **End-to-end conformance tests for two more reliability guards:** the empty-response
  nudge-then-stop guard (an `EmptyResponseProvider` that returns nothing must be nudged a bounded
  number of times, then stop — never loop forever) and the tool-call-as-text honest-failure guard (a
  `ToolCallAsTextProvider` that writes `<invoke …>` markup with no structured call must be nudged to
  actually execute, then end LOUDLY rather than report a phantom success). With 0.4.6/0.4.7 this
  brings end-to-end coverage to the full guard set — verification gate (direct + bridge), doom-loop,
  failure-loop, empty-response, continue/stall, and tool-call-as-text (`crates/forge-core/src/lib.rs`).

## [0.4.7] - 2026-06-26

Harness conformance tests for the runaway-prevention guards.

### Added
- **End-to-end conformance tests for the doom-loop and failure-loop guards** — the two quota-critical
  runaway guards previously had no loop-level test. A `DoomLoopProvider` (same call every step) proves
  the identical-call guard halts loudly; a `FailureLoopProvider` (a unique non-existent path each call
  → same `NotFound` kind, differing signatures) proves the failure-by-kind guard halts where the
  doom-loop can't see it. Both assert the turn actually STOPS rather than burning the step budget
  (`crates/forge-core/src/lib.rs`). Part of the "prove the harness with deterministic mock-provider
  tests" effort that already surfaced the 0.4.6 direct-gate bug.

## [0.4.6] - 2026-06-26

Make the direct-path completion-verification gate actually work.

### Fixed
- **The direct-API completion-verification gate now genuinely detects whether the model inspected real state**, where before it could not. Two root causes: (1) `inspect_ran`/`tools_ran` were only incremented from the provider's `ToolStarted` *stream events* — which the CLI bridge emits (its tool loop runs inside one `complete()`) but a direct genai provider does **not** (it returns tool calls in the response, executed separately), so the counters were always 0 on the direct path; (2) the gate read a step-local "inspected this step?" signal, but a direct model runs tools in steps *separate* from the text "done" claim, so that signal was always false at the gate. Result: a direct model that correctly verified could be wrongly flagged `UNVERIFIED`. Now the loop counts the tools a direct model runs, and the gate measures inspection **since the verification was requested**. Backed by two end-to-end conformance tests (inspect-during-verification → accepted; never-inspect → flagged `UNVERIFIED`) (`crates/forge-core/src/lib.rs`).

## [0.4.5] - 2026-06-26

Harness reliability + import/export round-trip.

### Added
- **`forge skill export <dir>` and `forge skill import <dir>`** — a portable bundle round-trip for your commands/skills/agents (the inverse of `forge import`), so a library can be moved to another machine or shared. Reuses the import copy machinery; idempotent (existing files kept) (`crates/forge-cli/src/cli/commands/skill.rs`).
- **Structured hook directive protocol** — a `PreToolUse`/`PostToolUse` hook can emit `{action: rewrite|inject|block|allow}` on stdout. `inject` feeds model-visible context (lint output, "this file is generated") via `pending_hints` — the first way a hook can *teach* the model, not just gate it. Back-compatible with the bare-object rewrite (`crates/forge-core/src/hooks.rs`).
- **Per-provider subagent fan-out cap** — `[mesh.subagents] max_per_provider` (default 2): each child also acquires a per-provider permit, so a burst routed to one subscription/key can't drain a single quota in parallel (`crates/forge-core/src/subagent.rs`).

### Changed
- **The objective completion-verification gate now guards the direct-API path too**, not just the CLI bridge — extracted into one shared `completion_gate` authority. A direct model that marks every task Done without a real tool-grounded state check is gated identically to a bridge (`crates/forge-core/src/lib.rs`).

### Fixed
- **Command namespace is preserved when copying** (import/export): a subdir command `git/commit.md` (name `git:commit`) no longer flattens to `commit.md`, which dropped the namespace and silently clobbered same-named commands from different namespaces (`crates/forge-cli/src/cli/commands/import.rs`).

## [0.4.4] - 2026-06-25

Patch release: mesh routing now factors tool-call reliability to avoid auto-selecting tool-leaky models.

### Fixed
- **Mesh routing now factors tool-call reliability: a `capability::tool_reliability_penalty` demotes models that emit tool calls as TEXT instead of structured calls (the Gemini flash family) below comparable tool-reliable peers**, so a high-bench-but-tool-leaky model is no longer auto-selected #1 for a tool-driven turn; it stays in the fallback chain. Pairs with the tool_recovery `<function=NAME>` salvage from v0.4.3 (`crates/forge-mesh/src/capability.rs`, `catalog.rs`).

## [0.4.3] - 2026-06-25

Patch release: tool_recovery now handles the Llama/Groq `<function=NAME>{json}</function>` format.

### Fixed
- **tool_recovery now recovers the Llama/Groq `<function=NAME>{json}</function>` tool-call format** (bare and wrapped in `<tool_call>`), in addition to `<invoke>` and `<tool_call>{json}`; a groq llama-3.x model routed by the mesh had leaked this as text and stalled the turn. Normalizes `mcp__forge__`/`default_api:` prefixes and recovers even an empty body so a degenerate call can't be mistaken for a final answer (`crates/forge-provider/src/tool_recovery.rs`).

## [0.4.2] - 2026-06-25

Patch release: bounded poll mode for the shell tool so the model can wait out long external jobs.

### Added
- **`shell` tool: `poll_until_exit_zero` + `poll_interval_secs` — wait for long external jobs without blocking.** A single blocking `gh run watch` is killed at the ~120s shell cap, making it impossible to wait out a multi-minute CI run or release build. The new poll mode re-runs `command` every `poll_interval_secs` (default 5, max 60) until it exits 0 or the per-call budget elapses (capped at 100s, well under the request cap). On budget exhaustion it returns a resumable "call again to keep waiting" result instead of a killed timeout, so the model waits by calling again rather than guessing. PTY mode is incompatible and rejected. The bridge preamble is updated to use this instead of a single blocking `gh run watch` (`crates/forge-tools/src/shell.rs`, `crates/forge-provider/src/cli_provider.rs`).

## [0.4.1] - 2026-06-25

Patch release: bridge-completion reliability, HTTPS on bare systems, and a security fix for
per-tool MCP permission rules.

### Security
- **`mcp_call` now enforces per-tool permission rules on the direct path.** On the
  `forge-core` direct path, `mcp_call` forwarded calls to `invoke_mcp` after checking only
  the outer wrapper name (`"mcp_call"`). A configured allow/ask/deny rule targeting the
  actual inner tool (e.g. `deny = ["myserver__dangerous"]`) was silently ignored because
  the broker never saw the inner tool name. Fix: after the outer wrapper passes, extract the
  inner tool name and arguments from the `mcp_call` arguments and run a second
  `permission::decide` against the real tool identity — deny or ask outcomes block execution
  (`crates/forge-core/src/lib.rs`).
- **`rule_matches` now fires on `deny = "*"` for MCP tool names.** For non-shell/non-path
  tools (all MCP server tools), a wildcard pattern `"*"` was matching zero args instead of
  any args, so `deny = "*"` on an MCP tool name never fired. Without this fix the per-tool
  gate above was unusable from config even after the `invoke_mcp` fix
  (`crates/forge-core/src/permission.rs`).

### Fixed
- **A bridge turn can no longer end with the plan half-done or falsely "done."** A CLI bridge
  (claude-cli/codex) is a one-shot subprocess that runs its own loop and exits, so a long plan
  could stop partway — the bridge did a few steps (merged + tagged), exited after launching the
  async release build, and the dependent steps (brew sha, verify) never ran; forge accepted the
  exit as "done." Completion is now defined by the TASK LIST and verified, not by the subprocess
  exiting (`crates/forge-core/src/lib.rs`):
  - **Task-driven re-drive.** While tracked tasks remain unfinished, forge re-invokes the bridge to
    continue (a clean new process — what the user typing `continue` does). Bounded.
  - **Progress gate (anti-spiral).** A re-run must make real progress — start a tool or close a
    task — or the turn HALTS loudly instead of re-driving. This is the guard the earlier
    bridge-nudge lacked: a bridge that just re-narrates can't loop.
  - **Objective verification gate.** When the bridge reports every task Done, forge forces a
    tool-grounded verification turn ("prove each task is actually complete by checking real state —
    git, gh, files — and reopen anything that isn't") before the turn can end. The verification must
    run a real INSPECTION tool (not just re-mark `update_tasks`); if the model never inspects, forge
    re-prompts then ends flagged UNVERIFIED rather than reporting success. Self-reported "done" is
    never trusted on its own — which is what produced the phantom release.
  - The bridge preamble now mandates completing the whole task and WAITING for any async job it
    launches (a release build, CI) rather than treating "launched" as "done"
    (`crates/forge-provider/src/cli_provider.rs`).
  - **Invariant:** forge never reports a phantom success — incomplete work is completed+verified,
    re-driven, halted loudly, or flagged UNVERIFIED. Documented in `docs/harness/bridge-completion.md`
    with the end-to-end test method (`scripts/bridge-e2e.sh` drives real subscription bridges and
    asserts on the real filesystem + run log).
- **HTTPS no longer requires a system trust store.** `build_reqwest_client()` now seeds
  Mozilla's bundled `webpki-root-certs` as the sole trust store, bypassing
  `rustls-platform-verifier`. Forge works on bare containers and minimal CI images with no
  `ca-certificates` package installed. Applied to both the API client and model auto-discovery
  (`crates/forge-provider/src/genai_provider.rs`).
- **Failover now follows the mesh ranking exactly.** When a model failed, Forge advanced to the
  *next-ranked* model — except the failover chain was secretly re-ordered by a provider
  round-robin (`interleave_by_provider`), so the second model tried was the top model of the
  *second provider*, not the second-best-ranked model overall. The chain is now in strict rank
  order. Storm protection is preserved differently: only a rate-limit (429) lazily skips that
  provider's remaining chain entries; every other failure keeps strict rank order
  (`crates/forge-mesh/src/lib.rs`, `crates/forge-core/src/lib.rs`).
- **A model that writes a tool call as *text* no longer "succeeds" without doing anything.**
  Some providers' native adapters (notably genai's Gemini adapter) don't decode function calls
  into structured tool calls — the call leaks into the assistant's text as `<invoke …>` /
  `default_api:` markup and never executes. Two defenses: (1) a **text tool-call recovery
  pass** reconstructs and executes the call from the markup
  (`crates/forge-provider/src/tool_recovery.rs`); (2) an **honest-failure guard** detects
  un-executed tool-call text, nudges the model to call the tool, and — if it persists — fails
  loudly instead of silently accepting the narration (`crates/forge-core/src/lib.rs`).
- **"database is locked" under concurrent Forge processes.** The SQLite store set WAL mode
  but no `busy_timeout`, so a second Forge process (TUI + `mcp-serve` bridge sharing one
  global db) hit `SQLITE_BUSY` immediately. The connection now waits up to 5 s for the write
  lock (`crates/forge-store/src/lib.rs`).

## [0.4.0] - 2026-06-24

Reliability release: every fix here came out of Forge attempting (and botching) its own release,
which exposed how a routed turn could silently do nothing yet report success.

### Fixed
  *next-ranked* model — except the failover chain was secretly re-ordered by a provider round-robin
  (`interleave_by_provider`), so the second model tried was the top model of the *second provider*,
  not the second-best-ranked model overall. That is how release-critical turns ended up on a
  low-ranked free model after a higher-ranked provider's first model failed over. The chain is now
  in strict rank order. Storm protection is preserved differently: only when a model fails with a
  **rate limit** does Forge skip that provider's remaining chain entries (a 429 is usually
  provider-wide) — every other failure keeps strict rank order
  (`crates/forge-mesh/src/lib.rs`, `crates/forge-core/src/lib.rs`).
- **A model that writes a tool call as *text* no longer "succeeds" without doing anything.** Some
  providers' native adapters (notably genai's Gemini adapter on newer models) don't decode a
  model's function calls into structured tool calls — the call leaks into the assistant's text as
  `<invoke …>` / `default_api:` markup and never executes. Forge saw no tool calls, accepted the
  narration as the final answer, and reported success having merged no PR and pushed no tag. Two
  defenses now: (1) a **text tool-call recovery pass** reconstructs the call from the markup
  (`<invoke>`, `<tool_call>` JSON; `default_api:`/`mcp__forge__` namespaces normalized) and executes
  it (`crates/forge-provider/src/tool_recovery.rs`); (2) an **honest-failure guard** detects
  un-executed tool-call text a direct model emits, nudges it to actually call the tool, and — if it
  persists — fails loudly instead of silently accepting the narration (`crates/forge-core/src/lib.rs`).
- **"database is locked" under concurrent Forge processes.** The SQLite store set WAL mode but no
  `busy_timeout`, so a second Forge process (the TUI plus the `mcp-serve` bridge sharing one global
  db) hit `SQLITE_BUSY` immediately and could crash a turn mid-run. The connection now waits up to
  5s for the write lock (`crates/forge-store/src/lib.rs`).

### Changed
- Workspace version and internal dependency constraints bumped to `0.4.0`; the Homebrew formula is
  bumped in lockstep (its sha256 values are filled from `checksums.txt` after the release build).
- Added `RELEASING.md` — a fixed cut-a-release checklist (the missed Homebrew-version bump that
  shipped stale `brew` installs was a recurring symptom of having no written process).

## [0.3.10] - 2026-06-24

### Fixed
- **Submitting a prompt with multi-byte whitespace no longer panics the turn.** Pasted text often
  carries a non-breaking space (`U+00A0`) or other multi-byte Unicode whitespace. The `@file`
  expansion scanned the prompt byte-by-byte, cast each byte to a `char`, and sliced the string on
  the result — which lands mid-character for any multi-byte whitespace and panicked with
  `end byte index … is not a char boundary` (`crates/forge-cli/src/cli/commands/run.rs`), crashing
  the whole session. It now splits on Unicode whitespace (`split_whitespace`), which is UTF-8-correct.

## [0.3.9] - 2026-06-24

### Fixed
- **`/copy` now actually copies on Wayland (and over SSH).** It reported "✓ copied" but the
  clipboard stayed empty: `arboard`'s Wayland backend silently no-ops from a terminal app (it needs
  an owned window/surface a TUI doesn't have). `/copy` (and mouse-selection copy) now also emit an
  **OSC 52** escape so the *terminal* sets the clipboard — reliable on Wayland, over SSH, and in
  Windows Terminal / kitty / iTerm with no display server (tmux/screen get the passthrough form).
  `arboard` is kept for X11 / macOS / Windows-native.

### Added
- **Cross-platform real-turn E2E you can run yourself, no VM.** `scripts/e2e-docker.sh` drives a
  real `forge run` turn across Ubuntu/Debian/Fedora containers against your host ollama (builds the
  *current* code in a glibc container so you test what you're editing), and reproduces the
  no-Secret-Service condition that hung `forge chat` on WSL — asserting startup stays bounded. A new
  `e2e` GitHub Actions workflow runs the same headless real turn + the probing `forge doctor` on
  **windows-latest** / ubuntu / macOS (manual or weekly) and uploads the logs, so cross-platform
  breakage is diagnosable without owning a Windows machine. (Set repo var `E2E_MODEL` + a provider
  secret, e.g. `GROQ_API_KEY`.)

## [0.3.8] - 2026-06-24

### Added
- **`/copy [N]` — copy an assistant response to the clipboard, with a code-block picker.** `/copy`
  copies the most recent assistant response; `/copy N` copies the Nth-latest (1-based from the most
  recent — `/copy 2` is the second-to-last). `/yank` is an alias. When the response contains fenced
  code blocks, `/copy` opens an interactive picker to copy the **full response** or any **individual
  block** (shown with its language + size); ↑↓ select, **Enter** copies to the clipboard, **`w`**
  writes the selection to a timestamped file in the cwd (useful over SSH, where the clipboard can't
  reach your local machine), **Esc** cancels. A response with no code blocks copies directly.

## [0.3.7] - 2026-06-24

### Changed
- **Failover shows one animated indicator instead of a per-hop warning wall.** When the mesh fails
  over between models (e.g. through rate-limited free models), it no longer prints a
  `{model} {reason} — failing over` line to the chat for every hop. Instead a single animated
  `⠋ finding a model` indicator appears in the status bar (the model being tried shows on the
  routing line) and clears the instant real output begins — so you see *that* it's searching and
  *what* it settled on, without the scrollback spam. A genuinely exhausted failover chain still
  surfaces a clear warning.

## [0.3.6] - 2026-06-24

### Fixed
- **Architect mode no longer dispatches a keyless `groq` for its planner/editor.** With
  `architect_mode` on (it's off by default — opt-in), the plan and edit phases resolved their model
  via the first configured tier candidate, and the built-in defaults lead with
  `groq::llama-3.3-70b-versatile`. On a box with no groq key every turn dispatched groq: the plan
  phase wasted a failover hop (`architect plan failing over to claude-cli::sonnet`) and the edit
  phase — which runs with mid-turn failover disabled — hard-failed (`turn failed: no API key
  configured for provider 'groq'`). The planner/editor now pick the first candidate whose provider
  has a key (keyless bridges / ollama qualify), so they land on e.g. `claude-cli` / `gemini`, never
  a keyless default. (A different path than the v0.3.5 last-resort leak — same symptom.)
- **Built-in default model lists no longer lead with `groq`.** Every tier's default candidate list
  started with `groq::llama-3.3-70b-versatile`, a free model that needs a key many users don't have
  — so any code path taking "the first candidate" landed on groq and failed. The defaults now lead
  with a keyless/bridge option (ollama / `claude-cli`) and keep groq last. Routing is unchanged
  (the cost-ranker picks the cheapest *usable* model regardless of list order); this only hardens
  the first-candidate fallback paths. A regression test asserts a config that omits `architect_mode`
  keeps it `false`.
- **`forge doctor` no longer false-warns about `TERM` on Windows.** `TERM` is a Unix concept and is
  normally unset on Windows (crossterm drives the console via the Console API regardless), so an
  interactive Windows console is simply OK. The "TUI may not render" warning now only fires on Unix,
  where an unset/`dumb` `TERM` is a real signal (e.g. WSL).

## [0.3.5] - 2026-06-24

Stability release: make Forge usable for new users on Windows / WSL / non-Arch boxes. Every fix
targets a "Forge is unusable and doesn't say why" failure reported from the field.

### Fixed
- **The mesh can no longer dispatch a provider you have no key for (the "groq for everything"
  churn).** Reported on Windows + WSL: the mesh kept trying `groq::llama-3.3-70b-versatile` despite
  no groq key — even with `--model` forced — and surfaced a raw `Resolver error`. Root cause was a
  keyless model reaching the wire via a path that wasn't key-filtered. Closed with three independent
  layers: (1) a genai "Resolver error" (adapter/auth couldn't be built — almost always a missing
  key) is now classified **permanent `Auth`** → the model is *excluded* and re-probed, not benched
  and retried forever; (2) the **last-resort fallback** (`soonest_unbenched`) now skips any provider
  with no key, so a benched keyless default can't become the pick; (3) a **pre-dispatch backstop**
  converts any keyless `active_model` into a permanent failure so failover advances to a usable
  model instead of dispatching and erroring. Keyless providers (ollama, the claude/codex bridges)
  are unaffected.
- **`forge chat` no longer hangs forever before the TUI opens (WSL).** On WSL / headless Linux an
  activatable-but-dead `org.freedesktop.secrets` made the OS-keyring call **block indefinitely** —
  and it's called per provider at startup, so the TUI never drew its first frame. The keyring is now
  probed once with an 800ms timeout and, if it doesn't answer, Forge uses its encrypted file store
  for the session (secrets stay durable). The setup menu worked because it doesn't hit that path.
- **All blocking startup steps are now time-boxed** — a command completes its preflight (or shows a
  clear error) before the TUI draws, instead of loading infinitely: model auto-discovery is capped
  at 15s (then falls back to built-in defaults + a warning), the `~/.claude` quota scan at 3s, and
  the background `claude --debug` quota probe at 20s.

### Changed
- **`forge doctor` now tests function, not just presence** — it reported "0 failures" on machines
  where Forge was unusable. It now runs two bounded live probes: a **CLI-bridge round-trip** (it
  actually launches the detected claude/codex bridge and confirms it answers — catching a bridge
  that's on PATH but can't launch, e.g. the Windows `cmd /S /C` shim case), and a **keyed-provider
  reachability** check (a `list_models` timeout means the provider won't route → the keyless-fallback
  cause). The terminal check now resolves the old `interactive (?)` (warns on an unusable `TERM`) and
  flags WSL explicitly.

## [0.3.4] - 2026-06-24

### Fixed
- **Billing safety: unpriced paid-provider models are no longer treated as "free".** A model from a
  metered API provider (OpenAI, xAI, DeepSeek, Anthropic, …) that Forge holds no bundled price for
  was classified `free` (unpriced → `$0`), so cost-aware routing would happily pick e.g.
  `gpt-5.5-pro`, `gpt-5-pro`, `o3`, or `gemini-3-pro` for a cheap tier — and **bill the user** —
  while reporting it as free. "Free" now requires positive evidence: genuinely-free providers
  (local `ollama`, free-tier `groq`/`cerebras`), an explicit `:free` variant, or a configured price
  of `0`. Everything else unpriced is **paid (unknown cost)**. **Gemini keeps its real free tier**:
  its Flash / Flash-Lite (and Gemma) models stay free, but Gemini Pro — paid-only since the free
  tier dropped it (Apr 2026) — is correctly paid. This also stops the trivial tier from routing to a
  paid model just because it had no price.
- **More non-chat models excluded from routing.** `is_routable` now also filters video (`sora`),
  realtime voice (`*realtime*`), speech-to-text (`*transcribe*`), and legacy base-completion models
  (`babbage`, `davinci`) — these leaked into the routable set from providers' full model lists (on
  top of the image/TTS/embedding/deep-research/moderation models already excluded). They remain
  visible in `forge models`, just never routed to.

## [0.3.3] - 2026-06-24

### Fixed
A multi-agent audit of the workspace surfaced these (verified) bugs, fixed here:

- **Windows: CLI bridges and MCP servers failed to launch when a path contained a space.** A `.cmd`
  shim was run as `cmd /C "<path>"`, but `cmd` strips the first/last quote of its `/C` string, so a
  quoted path broke the moment a second quoted token (an argument with a space — e.g. an
  `--mcp-config` path under `C:\Users\First Last\…`) appeared. Now launched via `cmd /S /C` with the
  whole command line individually quoted, so spaces in the path **and** in any argument survive.
  Applies to both the claude/codex bridges and stdio MCP servers. This could manifest as a bridge
  that's installed and detected yet keeps benching itself (and the mesh then routing elsewhere).
- **Bridge failures hid the CLI's own error.** A stalled or crashed bridge now includes the CLI's
  `stderr` tail in the error (so a benched bridge shows *why* it failed), and a prompt-to-stdin write
  failure is logged instead of silently dropped (which could leave the CLI waiting for EOF).
- **A configured provider whose model discovery failed vanished silently.** A keyed provider that
  errors or times out during discovery is now logged at `warn` (not `debug`), and keyed providers get
  a more forgiving discovery budget (8s vs 4s) so a slow/cold connection — e.g. OpenRouter's large
  model list — doesn't drop it and force a fallback to the built-in defaults.
- **`@path` completion was dead on Windows** outside a git repo: the fallback shelled out to Unix
  `find` (a different, incompatible tool on Windows). Replaced with a portable `std::fs` walk.
- **Architect mode reused the wrong failover chain.** When the editor model differed from the routed
  model, an editor-model failure failed over using the *routed* model's fallbacks; it now runs
  without cross-model failover (matching the self-review / autofix re-runs).
- **An empty keyring entry counted as an API key**, producing a cryptic provider 401 instead of the
  actionable "no key — run `forge auth`" message. `api_key()` now requires a non-empty value, like
  `has_api_key()`.
- **A server retry-after of the form `.5s` (leading decimal point) was ignored**, discarding the
  server's cooldown hint; the parser now accepts a leading dot.

## [0.3.2] - 2026-06-24

### Fixed
- The mesh no longer spins on a keyless provider (e.g. "keeps trying groq for everything" with no
  groq key). When routing can find no usable model — because auto-discovery came up empty, or the
  user's keys are for providers not in the built-in defaults — it previously fell back to the first
  default candidate (groq) anyway and called it, auth-failing every turn. The turn now stops with an
  actionable diagnostic instead: it names the keyless provider, lists the providers you *do* have a
  key for, and points at `forge auth` / `forge models` / `/model <id>` / `ollama serve`.
- OpenRouter is now recognised when its API key is set under the conventional `OPENROUTER_API_KEY`
  name, not only Forge/genai's canonical `OPEN_ROUTER_API_KEY`. A user who exported the standard
  name was silently treated as keyless, so the mesh skipped OpenRouter discovery and fell back to
  the groq defaults. Both names are accepted, and the conventional one is copied into the canonical
  variable the provider client authenticates with.

## [0.3.1] - 2026-06-24

### Fixed
- Failover churn on a bad/missing API key: an auth failure (HTTP 401/403) is now treated as
  **permanent** — the model is excluded from routing (24h + automatic re-probe) instead of being
  short-benched and re-tried at the top of *every* turn. A keyless or misconfigured provider no
  longer adds a failover hop + warning to each turn; it recovers automatically once the key is fixed.
- One-line turn recap no longer invents success on a stalled turn. A turn that gave up with no
  output (empty-response exhaustion / failover exhaustion) is no longer summarized — previously the
  trivial-tier summarizer leaned on the *request* and reported e.g. "Fixed the bug…" for a turn that
  did nothing. The recap prompt was also hardened to describe only what the response actually shows,
  and to say so plainly when a turn stalled, errored, or only planned.

### Changed
- The `curl … | sh` / `irm … | iex` installer can be re-run any time to update or reinstall on any
  platform **without touching your config, sessions, or API keys** — it only ever writes the binary
  and updates `PATH`. A re-run now detects the previous version and confirms your settings are
  preserved (`forge 0.3.1 -> … (was 0.3.0; your config and sessions are preserved)`).
  `scripts/test-installer-config-safe.sh` asserts a seeded config survives two installs.

## [0.3.0] - 2026-06-24

### Added
- Turn timer + token counter in the statusline (like Claude Code / Codex). While a turn runs, the
  spinner shows `⟳ working` and row 2 shows a single `⧖ 12s ↑in ↓out` segment for this turn — elapsed
  time ticking live, the per-turn token deltas filling in, both frozen at the final totals when the
  turn ends. The context gauge sits next, then the session running totals as `Σ ↑in ↓out` (clipped
  first if the row is narrow, so the gauge always stays visible).
- Mouse text selection in full-screen mode, no Shift needed. Forge now does its own click-drag
  selection (highlighted in place) and copies the text to the clipboard on release — so the wheel
  still scrolls AND plain drag selects, where kitty/most terminals otherwise force Shift+drag once an
  app reports the mouse. Drag-motion is only reported while a button is held, so there's no hover
  overhead. Disable all mouse reporting with `[tui] mouse_capture = false`.
- Floating "↓ Jump to bottom · Ctrl+End" bar in the full-screen transcript: appears only while
  scrolled up off the tail; click it or press Ctrl+End to jump back to the live tail and resume
  following.
- `scripts/tui-drive.sh` — drive `forge chat` in a real full-screen TUI inside tmux, send a
  scripted key sequence, and capture/assert on the rendered screen (the alt-screen grid, which a raw
  PTY byte-stream can't show). The `--mock` provider is now plan- and task-aware, so `/plan …`
  renders a real plan card + seeds the task panel and a task-tracking prompt renders the sticky task
  panel — reproducing the full-screen panels offline (no API/bridge). Sample scripts under
  `scripts/tui-scripts/`.
- Interactive plan mode: `/plan` now has the agent call a `present_plan` tool that renders a
  bordered, animated **plan card** (title + numbered steps + notes) instead of loose prose. You
  approve it interactively — approving switches to Auto-edit and **auto-builds** the plan, typing
  changes revises it, cancelling keeps you in planning. The plan's steps seed the live task list so
  build progress is visible, and every draft/revision is saved to `.forge/plans/`. Works on direct
  API models and the claude/codex bridges alike.
- Windows PowerShell installer (`install.ps1`): `irm …/install.ps1 | iex` downloads the x86-64
  release binary (SHA-256 verified), installs `forge.exe` to `%LOCALAPPDATA%\Programs\forge`, and
  adds it to the user `PATH`. `install.sh` now points Windows users to it.
- Anchored-block fuzzy tier for `edit_file` / `multi_edit`: when an `old` block's interior was
  paraphrased but its first/last lines match, the unique span between those anchors is replaced —
  guarded by uniqueness + a disproportionate-match rejection so it can't silently eat the wrong
  region.
- `docs/harness/competitive-analysis.md` — recon of competing coding-agent harnesses and a
  prioritized backlog of techniques to port.
- `forge doctor` — diagnose your setup (providers/keys, CLI bridges, Ollama, MCP, config, git,
  terminal) with actionable fixes.
- `forge update` — self-update to the latest release. A standalone binary install (curl/zip) is
  downloaded and swapped in place; Homebrew/cargo installs print the correct upgrade command instead
  of clobbering a package-managed file. `forge update --check` reports without changing anything.
- Startup update check now offers to update: on an interactive terminal it prompts "update now?" and
  runs `forge update` on yes; otherwise it prints the notice (disable with `[update] check = false`
  or `FORGE_NO_UPDATE_CHECK=1`). Never prompts in headless runs, pipes, or the `mcp-serve` bridge.
- Community infrastructure: `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, issue
  templates, and this changelog.

### Fixed
- The agent loop no longer ends a turn mid-task when a (often weaker/free) **direct** model narrates
  its next step without calling the tool, or signs off with tasks still open. If the tracked task
  list has unfinished items when such a model stops with plain text, Forge nudges it to continue (up
  to 4×) instead of accepting the stall as the final answer. A **CLI-bridge** turn (claude/codex) is
  exempt and treated as terminal — the bridge runs its own internal tool loop and returns the whole
  turn as one text response, so nudging it only re-runs the entire bridge in a confused state (it
  starts narrating tool calls as text and spirals). The doom-loop guard also fires a "change
  approach" nudge *before* it hard-stops, so a single repeated call no longer kills an otherwise-
  recoverable turn; it only halts if the model keeps repeating after the nudge.
- The CLI bridge (claude/codex) now gates on Forge's **live** temper, not the stale on-disk config
  mode — so switching Plan→Auto-edit (e.g. approving a plan) actually lets the bridged model write.
  `forge mcp-serve` runs as a fresh process per turn and loaded `permission_mode` from the config
  file, blind to runtime switches: after the user moved to Auto-edit, the bridge still denied
  `mcp__forge__write_file` with "denied by Forge permission policy", which the model surfaced as "I
  have no permissions" even though the UI said Auto-edit. The parent now exports its current temper
  (`FORGE_PERMISSION_MODE`) into the per-turn checkpoint env and forwards it to the bridge child,
  which honors it over the config. Verified live (codex 0.141): `plan` denies the write, `accept-edits`
  performs it. This also closes the opposite hole — writes are no longer silently allowed *during*
  Plan mode when the config happened to be `accept-edits`.
- Statusline no longer shows a duplicate turn timer (the elapsed time appeared on both the spinner
  and the `⧖` segment) and no longer pushes the context gauge off-screen — the gauge is now ordered
  ahead of the session totals so a narrow row clips the totals, not the gauge.
- Streaming replies now render as markdown live, not as one raw unwrapped blob. The in-flight reply
  edge was dumped into a single span (embedded newlines, headings, lists and code fences all
  collapsed into one wrapped paragraph) and only re-rendered as markdown once the turn finished. It's
  now markdown-rendered on every update — matching the finalized block — and memoized on content
  length so re-parsing doesn't re-introduce the long-conversation lag.
- The claude CLI bridge no longer gives up on shell commands ("can't run that / tool channel broken")
  and fails to commit or open a PR. Told to "commit and push" without a named tool, claude reached
  for its (harness-disabled) native `Bash`, emitted the call as text, and hallucinated interactive-
  shell output (login banner, prompt) — then spiralled, unable to read its own results. The harness
  preamble now states plainly that native tools are disabled, that EVERY shell command goes through
  `mcp__forge__shell` (a clean non-interactive `sh -c`), that garbled/empty output means re-verify
  rather than "channel broken," and that it must never claim it cannot run a command, commit, or open
  a PR. Verified live (claude 2.1.186): a vague "commit and push" now routes through `mcp__forge__shell`
  and lands the commit.
- The codex bridge no longer stalls a plan build claiming it has "no write permissions." codex's
  own shell is sandboxed read-only **by design** (every write is meant to go through Forge's gated
  `mcp__forge__*` tools, which run outside that sandbox), but codex would run `test -w .`, see the
  read-only sandbox + `approval_policy=never`, and quit without ever trying the MCP write tools. The
  harness preamble now states plainly that the read-only shell is expected, that file changes go
  through `mcp__forge__write_file`/`edit_file`/… , and that it must never probe writability or refuse
  a build over it. Verified live (codex 0.141): an implementation turn now calls `write_file` instead
  of bailing.
- `forge lattice impact` output now flags that it is **name-based**: it appends how many definitions
  share the queried name and warns that same-named symbols in unrelated modules/crates are included,
  so a hit must be confirmed before it's treated as a cross-module blocker. A planning agent (notably
  on the codex bridge) was taking impact's same-name collisions at face value — `impact run` reports
  21 definitions across the workspace — and hard-stopping a refactor plan on phantom external
  references instead of presenting it.
- The `present_plan` plan card now word-wraps long step titles, details, and notes to the frame width
  instead of overflowing the right border and mangling the box. Each wrapped continuation line is
  indented under its step and padded so every row meets the border at the same column.
- `forge lattice` now honors `.gitignore`/`.ignore` (via ripgrep's `ignore` walker) instead of a raw
  directory walk, so it no longer indexes gitignored trees — most importantly Forge's own
  `.forge/bench/repos/<astropy|django>/…` SWE-bench clones and `target/`. Those swamped `impact`/
  `query` with hundreds of name-collision hits from unrelated code (e.g. `impact Command` → "323
  sites" across django/astropy), which is why a `/plan` refactor on the codex bridge kept *stopping*:
  the agent dutifully halted on the prompt's "stop if anything outside the crate references this"
  rule when impact reported that vendored noise.
- Lattice queries (`impact`/`query`/`path`) are now scoped to the current repo root. The store is
  global (one DB across every project and bench clone), so an unscoped name match cross-contaminated
  across repos; impact for the project no longer returns symbols from a different checkout.
- `forge lattice update` prunes orphan roots from the global index: a root whose directory is gone
  (deleted scratch clone), or one nested under the root being indexed (stale sub-root, e.g. an old
  bench clone) — reclaiming a badly bloated index (here 101k → 3k nodes).
- Mouse-selection copy no longer spams the chat or breaks the TUI. It's silent now (no `📋 copied`
  scrollback note), and reuses one long-lived clipboard handle — creating one per copy made arboard's
  X11 backend relinquish the selection immediately ("clipboard dropped") and write to the terminal,
  which corrupted the full-screen layout.
- **The plan card and task list now actually appear on the codex bridge.** codex hands its stdio
  MCP servers a *curated* environment (only `PATH`/`HOME`/`LANG`/… survive — verified live), so the
  `FORGE_SUBAGENT_SINK` and checkpoint-context env that the parent sets for `forge mcp-serve` never
  reached it. The served `present_plan`/`update_tasks` wrote to a dead sink, so the parent TUI never
  got the events — the model *did* call the tools (visible in the live stream), nothing rendered.
  Forge now injects that env explicitly into the MCP config (`-c mcp_servers.forge.env.*` for codex,
  the `env` object in claude's `--mcp-config`). Same gap silently broke `/undo` snapshots of
  codex-made edits (the checkpoint env was stripped too); both are fixed. Verified end-to-end: a
  live codex turn's `update_tasks` + `present_plan` now round-trip to the parent sink.
- Full-screen chat is no longer laggy on long conversations. The transcript was re-wrapped
  character-by-character in full every frame (~60×/sec while streaming), which is O(transcript) and
  showed up as input/scroll lag once the log grew. The wrap is now memoized (re-wrapped only when the
  log or width changes) and each frame clones just the visible window, not the whole transcript.
- Full-screen mouse wheel scroll and native text selection now BOTH work. The wheel-scroll support
  used crossterm's full mouse capture, which turns on motion tracking and disables the terminal's
  native click-drag selection. Forge now enables only **minimal** mouse reporting (button + wheel,
  no motion tracking), so the wheel scrolls the transcript while drag-to-select keeps working.
  Configurable via `[tui] mouse_capture` (default on); set false to disable mouse reporting entirely
  and scroll with PgUp/PgDn/Home/End.
- Plan mode now works on CLI bridges: the harness tool-preamble names `mcp__forge__present_plan`
  (and notes that hosts like codex load MCP tools lazily and won't pre-list them). codex 0.141 only
  surfaces a subset of MCP tools up front, so a bridged model told to "present a plan" couldn't find
  `present_plan` and fell back to its read-only shell — the plan never rendered. Verified end-to-end
  against codex 0.141: a bare-name "present a plan" instruction now resolves and calls the tool.
- `forge lattice update` now PRUNES files that vanished from the walk — deleted files and, crucially,
  ones now under a skipped/nested-git/vendored tree. Previously the nested-git skip stopped *new*
  indexing but left already-indexed vendored symbols (e.g. a SWE-bench `django/` clone) in the graph,
  so `impact`/`query` stayed swamped with unrelated hits and the store ballooned. Re-running update
  purges them (symbols/edges/refs cascade-deleted).
- The CLI bridge (`forge mcp-serve`) now uses the SAME global session store as the parent instead of
  a relative `.forge/forge.db`. The divergent path created spurious empty sessions and meant a bridge
  turn's `update_tasks` was written to a different database than the parent's post-turn reload read —
  so bridge task updates didn't round-trip.
- Chat input is more responsive: the event loop now paces adaptively, looping back quickly while you
  type or navigate the palette/picker/approve prompts instead of always waiting a full frame.
- Context gauge no longer reads a bogus, ever-climbing percentage (e.g. "337% — auto-compact
  imminent") on a subscription CLI bridge. `claude-cli`/`codex-cli` report *cumulative* internal
  token usage across their own tool loop, not the size of the request Forge sent, so the gauge now
  uses Forge's transcript estimate for bridge turns and the provider's real input count for direct
  API models. This also stops the phantom auto-compact banner (no real compaction was ever needed).
- The sticky task list (`update_tasks`) and spawned-subagent panels now appear during an interactive
  CLI-bridge turn. The out-of-band event sink that carries those updates from `forge mcp-serve` was
  only created in benchmark/harness mode, so a normal `forge chat` on `claude-cli`/`codex-cli` showed
  no live tasks at all; it is now created and tailed for every bridge turn. A post-turn store reload
  also no longer blanks a task list that was already shown.
- `forge lattice` no longer indexes nested git repositories (vendored deps, submodules, or scratch
  clones such as SWE-bench workdirs under the project root). Their symbols were swamping `impact`/
  `query` with unrelated hits (a generic `Command` matching across a cloned `django/` tree); the
  walker now skips any subdirectory that is its own git repo.
- `forge bench swe` no longer fails with `os error 2` when `--workdir` is relative (the default):
  the per-instance clone was double-nesting the path. The workdir is now absolutized first.
- Local Ollama models that emit Hermes/Qwen-style `<tool_call>` XML (e.g. `qwen3-coder`) are now
  driven correctly: `ollama::` is routed through Ollama's OpenAI-compatible `/v1` endpoint, which
  parses those into structured tool calls instead of leaking them as text and dead-ending the turn.
- **Windows**: the Claude Code / Codex CLI bridges and stdio MCP servers now launch correctly.
  Detection and spawning are PATH-resolved with the `.exe`/`.cmd`/`.bat` suffixes, and `.cmd`/`.bat`
  shims (how npm installs `claude`, `codex`, `npx`, and node-based MCP servers like `caveman-shrink`)
  are run through `cmd /C`. Previously these failed to detect or spawn ("program not found").

## [0.2.0] - 2026-06-23

### Added
- Full-screen (alternate-screen) TUI by default — scrollable transcript, pinned panels, mouse-wheel
  scroll; `--inline` (or `[tui] fullscreen = false`) keeps the classic inline-scrollback mode.
- Unified in-loop activity viewer (Ctrl+O): main chat + subagents + assay critics in one navigable
  full-screen view.
- Dynamic `/config` editor — grouped sections, friendly labels, type-appropriate controls (toggle /
  cycle / typed input), API-key management (keyring), per-setting help, default/modified/source,
  reset-to-default, fuzzy search, user/project scope.
- `forge local` — local LLMs via Ollama: hardware detection, live model discovery, ranking by
  Artificial Analysis benchmark scores (multi-family catalog offline), install/start/status, an
  animated picker, and opt-in autostart.
- `forge setup` — guided first-run wizard (providers/plans + optional local-LLM step); auto-runs on
  first launch. `forge init` aliases it.
- Per-turn AI recap; persisted TUI view state restored on resume.

### Changed
- `forge models --probe` rechecks only benched models by default (cheap); `--all` re-pings every
  model. Capability-exclusion window shortened to 24h.

### Fixed
- Activity-viewer crash and scrollback corruption in full-screen mode; assay per-critic model/cost
  detail; macOS release checksum.

## [0.1.0] - 2026-06-18

Initial public release: Model Mesh routing, multi-provider support, cost/budget caps, the
inline TUI, session persistence + checkpoints, permission broker, subagents, Assay analysis,
Lattice code intelligence, MCP client, web tools, hooks, skills/commands, and more.

[Unreleased]: https://github.com/Adulari/forge/compare/v2.26.0...HEAD
[2.26.0]: https://github.com/Adulari/forge/compare/v2.25.2...v2.26.0
[2.25.2]: https://github.com/Adulari/forge/compare/v2.25.1...v2.25.2
[2.25.1]: https://github.com/Adulari/forge/compare/v2.25.0...v2.25.1
[2.25.0]: https://github.com/Adulari/forge/compare/v2.24.1...v2.25.0
[2.24.1]: https://github.com/Adulari/forge/compare/v2.24.0...v2.24.1
[2.24.0]: https://github.com/Adulari/forge/compare/v2.23.6...v2.24.0
[2.23.6]: https://github.com/Adulari/forge/compare/v2.23.5...v2.23.6
[2.23.5]: https://github.com/Adulari/forge/compare/v2.23.4...v2.23.5
[2.23.4]: https://github.com/Adulari/forge/compare/v2.23.3...v2.23.4
[2.23.3]: https://github.com/Adulari/forge/compare/v2.23.2...v2.23.3
[2.23.2]: https://github.com/Adulari/forge/compare/v2.23.1...v2.23.2
[2.23.1]: https://github.com/Adulari/forge/compare/v2.23.0...v2.23.1
[2.23.0]: https://github.com/Adulari/forge/compare/v2.22.0...v2.23.0
[2.22.0]: https://github.com/Adulari/forge/compare/v2.21.0...v2.22.0
[2.21.0]: https://github.com/Adulari/forge/compare/v2.20.0...v2.21.0
[2.20.0]: https://github.com/Adulari/forge/compare/v2.19.0...v2.20.0
[2.19.0]: https://github.com/Adulari/forge/compare/v2.18.0...v2.19.0
[2.18.0]: https://github.com/Adulari/forge/compare/v2.17.2...v2.18.0
[2.17.2]: https://github.com/Adulari/forge/compare/v2.17.1...v2.17.2
[2.17.1]: https://github.com/Adulari/forge/compare/v2.17.0...v2.17.1
[2.17.0]: https://github.com/Adulari/forge/compare/v2.16.2...v2.17.0
[2.16.2]: https://github.com/Adulari/forge/compare/v2.16.1...v2.16.2
[2.16.1]: https://github.com/Adulari/forge/compare/v2.16.0...v2.16.1
[2.16.0]: https://github.com/Adulari/forge/compare/v2.15.0...v2.16.0
[2.15.0]: https://github.com/Adulari/forge/compare/v2.14.1...v2.15.0
[2.14.1]: https://github.com/Adulari/forge/compare/v2.13.9...v2.14.1
[2.13.9]: https://github.com/Adulari/forge/compare/v2.13.8...v2.13.9
[2.13.8]: https://github.com/Adulari/forge/compare/v2.13.7...v2.13.8
[2.13.7]: https://github.com/Adulari/forge/compare/v2.13.6...v2.13.7
[2.13.6]: https://github.com/Adulari/forge/compare/v2.13.5...v2.13.6
[2.13.5]: https://github.com/Adulari/forge/compare/v2.13.4...v2.13.5
[2.13.4]: https://github.com/Adulari/forge/compare/v2.13.3...v2.13.4
[2.13.3]: https://github.com/Adulari/forge/compare/v2.13.2...v2.13.3
[2.13.2]: https://github.com/Adulari/forge/compare/v2.13.1...v2.13.2
[2.13.1]: https://github.com/Adulari/forge/compare/v2.13.0...v2.13.1
[2.13.0]: https://github.com/Adulari/forge/compare/v2.12.1...v2.13.0
[2.12.1]: https://github.com/Adulari/forge/compare/v2.12.0...v2.12.1
[2.12.0]: https://github.com/Adulari/forge/compare/v2.11.0...v2.12.0
[2.11.0]: https://github.com/Adulari/forge/compare/v2.10.2...v2.11.0
[2.10.2]: https://github.com/Adulari/forge/compare/v2.10.1...v2.10.2
[2.10.1]: https://github.com/Adulari/forge/compare/v2.9.1...v2.10.1
[2.10.0]: https://github.com/Adulari/forge/compare/v2.9.1...v2.10.0
[2.9.1]: https://github.com/Adulari/forge/compare/v2.9.0...v2.9.1
[2.9.0]: https://github.com/Adulari/forge/compare/v2.8.5...v2.9.0
[2.8.5]: https://github.com/Adulari/forge/compare/v2.8.4...v2.8.5
[2.8.4]: https://github.com/Adulari/forge/compare/v2.8.3...v2.8.4
[2.8.3]: https://github.com/Adulari/forge/compare/v2.8.2...v2.8.3
[2.8.2]: https://github.com/Adulari/forge/compare/v2.8.1...v2.8.2
[2.8.1]: https://github.com/Adulari/forge/compare/v2.8.0...v2.8.1
[2.8.0]: https://github.com/Adulari/forge/compare/v2.7.0...v2.8.0
[2.7.0]: https://github.com/Adulari/forge/compare/v2.6.5...v2.7.0
[2.6.5]: https://github.com/Adulari/forge/compare/v2.6.4...v2.6.5
[2.6.4]: https://github.com/Adulari/forge/compare/v2.6.3...v2.6.4
[2.6.3]: https://github.com/Adulari/forge/compare/v2.6.2...v2.6.3
[2.5.8]: https://github.com/Adulari/forge/compare/v2.5.7...v2.5.8
[2.5.7]: https://github.com/Adulari/forge/compare/v2.5.6...v2.5.7
[2.5.6]: https://github.com/Adulari/forge/compare/v2.5.5...v2.5.6
[2.5.5]: https://github.com/Adulari/forge/compare/v2.5.4...v2.5.5
[2.5.4]: https://github.com/Adulari/forge/compare/v2.5.3...v2.5.4
[2.5.3]: https://github.com/Adulari/forge/compare/v2.5.2...v2.5.3
[2.5.2]: https://github.com/Adulari/forge/compare/v2.5.1...v2.5.2
[2.5.1]: https://github.com/Adulari/forge/compare/v2.5.0...v2.5.1
[2.5.0]: https://github.com/Adulari/forge/compare/v2.4.0...v2.5.0
[2.4.0]: https://github.com/Adulari/forge/compare/v2.3.0...v2.4.0
[2.3.0]: https://github.com/Adulari/forge/compare/v2.2.0...v2.3.0
[2.2.0]: https://github.com/Adulari/forge/compare/v2.1.0...v2.2.0
[2.1.0]: https://github.com/Adulari/forge/compare/v2.0.0...v2.1.0
[1.8.1]: https://github.com/Adulari/forge/compare/v1.8.0...v1.8.1
[1.8.0]: https://github.com/Adulari/forge/compare/v1.7.0...v1.8.0
[1.7.0]: https://github.com/Adulari/forge/compare/v1.6.1...v1.7.0
[0.4.4]: https://github.com/Adulari/forge/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/Adulari/forge/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/Adulari/forge/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/Adulari/forge/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/Adulari/forge/compare/v0.3.10...v0.4.0
[0.3.10]: https://github.com/Adulari/forge/compare/v0.3.9...v0.3.10
[0.3.9]: https://github.com/Adulari/forge/compare/v0.3.8...v0.3.9
[0.3.8]: https://github.com/Adulari/forge/compare/v0.3.7...v0.3.8
[0.3.7]: https://github.com/Adulari/forge/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/Adulari/forge/compare/v0.3.5...v0.3.6
[0.3.5]: https://github.com/Adulari/forge/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/Adulari/forge/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/Adulari/forge/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/Adulari/forge/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/Adulari/forge/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/Adulari/forge/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Adulari/forge/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Adulari/forge/releases/tag/v0.1.0
