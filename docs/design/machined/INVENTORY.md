# Machined Design → Implementation Inventory

Source files (both read in full):
- `docs/design/machined/Forge Machined - Desktop.dc.html` (1092 lines) — 37 frames
- `docs/design/machined/Forge Machined - Mobile.dc.html` (668 lines) — 25 frames

App is a single unified codebase (Expo RN + react-native-web + Tauri v2) under `mobile/src/`. "Maps to" paths below are relative to `mobile/src/` unless stated. Line ranges are `data-screen-label` span boundaries in the `.dc.html` files — read only that slice when implementing a frame.

---

## Shared patterns (recur across many frames)

- **Icon rail** (desktop only, Desktop L140-155, 203-211): 48px-wide collapsed sidebar, icons centered, 30×30px hit targets, 4px gaps, hairline divider `rgba(244,244,246,.09)` between nav icons and session icons. No current file — new desktop-only chrome; would live alongside `components/DesktopWindowChrome.tsx`.
- **Tab-underline strip** (session header, Desktop L80,160,557 / Mobile L71-72,194): row of tab labels (Chat/Tasks N/Agents N/Review/Replay), active tab has `border-bottom:2px solid #FF8A3D`, others `color:#9A9AA6`, font-size 11.5-12.5px. No dedicated component found in `components/session/`; `SessionHeader.tsx` is the closest existing file.
- **Usage rings**: circular SVG progress arcs, `stroke-width:2` (rail, r=6.5) or `4` (settings, r=18), track `rgba(244,244,246,.08-.1)`, fill color varies by pace (`#9A9AA6` neutral / `#D9A94E` warn / `#F4F4F6` in settings bars). Sidebar mini-rings have no counterpart component; `components/ds/ContextGauge.tsx` is the closest existing primitive (single-metric gauge, not multi-provider ring row).
- **Quiet tool rows** (mono action log, Desktop L96-99,173-176 / Mobile L78-81): `Run`/`Edit` label in `#5F5F6B` mono, filename in default text, `+N` in `#5FB97D`, `−N` in `#E5605C`, no border/background — plain flow. Maps to chat transcript rendering; no exact existing component (`components/chat/SystemOutput.tsx` and `CodeBlock.tsx` are adjacent).
- **Plan panel**: bordered card, header row "Plan" + `n/m` counter + chevron, checklist rows with `✓` (done, `#5F5F6B` strikethrough), pulsing dot (in-progress, `#FF8A3D`), `○` (queued, `#33333B`). Maps directly to `components/session/PlanSheet.tsx` + `components/review/PlanCard.tsx`.
- **Composer**: bordered card (`radius:4`, `border:1px solid rgba(244,244,246,.09-.12)`), placeholder text row, then a control row (model chip, effort/mode chip, attach icons, mic/voice icon on mobile) ending in a 22-30px orange circular send button (`#FF8A3D` bg, `#1A0E04` glyph). Maps to `components/chat/Composer.tsx` (30.4K, already the largest chat component) and `VoiceRecordingPill.tsx`.
- **Permission card**: `PERMISSION · <timer>` label in `#E5605C` uppercase mono, question text, optional mono diff snippet block (`#0B0B0E` bg, red/green diff lines), Allow (outlined, bold) / Deny (muted) buttons, 44px tall on mobile for hit-target. Maps to `components/cards/PermissionCard.tsx`.
- **Status dots**: 5-6px circle — `#5FB97D` ok/online, `#FF8A3D` active/forging, `#E5605C` needs-you/danger, `#D9A94E` warn/degraded, `#33333B` stale/idle-none. Maps to `components/ds/StatusDot.tsx`.
- **Worktree badge**: `⑂ <name>` glyph+text in `#FF8A3D` mono, appears next to session titles wherever a git worktree is in use. No dedicated component; embedded inline in session header markup today.
- **Host/session list row**: desktop rows are 28px tall, dense; mobile rows are 44-50px for touch. Same information (dot, title, meta line in mono `#5F5F6B`) at two density tiers — implies the same data model rendered by density-aware row components, e.g. `components/fleet/SessionCard.tsx` (mobile) vs. a new dense variant for desktop.
- **Toggle switch**: pill track 26-30×15-18px, `#FF8A3D` when on / `#33333B` when off, circular knob. Maps to `components/ds/Switch.tsx`.
- **Segmented control** (READ/ASK/EDIT/FULL, LIGHT/DARK/SYSTEM, etc.): bordered pill container, 2px internal padding, active segment gets `rgba(244,244,246,.09)` fill. Maps to `components/ds/Segmented.tsx`.

---

## Design tokens observed

**Backgrounds:** `#050506` (outer `<body>` only) · `#09090B` (app/window bg) · `#0D0D11` (sidebar/rail/title bar panel) · `#0E0E12` and `#101015` (cards — used near-interchangeably) · `#0B0B0E` (terminal dock, tab-bar footer, darkest card variant — **not in the documented card set**) · `#000` (Dynamic Island native chrome).

**Text:** `#F4F4F6` (primary) · `#9A9AA6` (secondary) · `#5F5F6B` (tertiary/mono labels) · `#DCDCE2` (a 4th tone used for body/assistant-message copy — **not in the documented 3-tone text set**).

**Accent/semantic:** `#FF8A3D` ember accent (state-only, per doc) · `#FFB27A` hover (CSS reset only, never in a frame) · `#5FB97D` ok · `#D9A94E` warn · `#E5605C` danger · `#1A0E04` (text-on-ember, for filled buttons/badges).

**Undocumented neutrals:** `#33333B` and `#232329` — inactive/stale dots and macOS traffic-light placeholders. Not in the 3-4 documented radius/color set but used consistently as a 4th/5th neutral step.

**Borders:** hairlines `rgba(244,244,246,.06)` through `.16)`, mostly `.07-.12`; permission/warn cards use tinted borders `rgba(229,96,92,.3-.35)` (danger) and `rgba(217,169,78,.3-.35)` (warn) instead of the neutral hairline.

**Radius:** `2px` (chip inner fill, tiny badges) · `3px` and `4px` (documented default — most cards/buttons/inputs) · `5-6px` (overlay/menu containers) · `7px` (native iOS Live Activity buttons — **larger than doc**) · `8-9px` (native notification, toggle pill) · `14/17/22px` (native iOS Live Activity card, Dynamic Island — **deliberately native-scale, well outside the 3-4px doc range**).

**Font sizes:** mono technical labels floor at `8.5-9.5px` (desktop) / `10.5-11px` (mobile "Air" pass, per its stated 10.5-11px mono floor); body text `11-13px` desktop, `12.5-14px` mobile; headers `13-22px`. Both files consistently use `Geist` (UI) and `Geist Mono` (technical/status/diff text).

**Row/hit-target heights:** desktop list rows `27-28px` (dense); mobile rows `44-50px` (documented ≥44px hit-target rule in Mobile file header); native Live Activity buttons `40px`.

**Light theme (Mobile `M Fleet Light`, L629-661) — an entirely separate palette not in the dark-mode doc:** bg `#F5F4F1` · footer `#EFEDE8` · text `#1C1B19` · secondary `#8A867D`/`#6E6A61` · accent `#D96A1E` · danger `#C44A42` · ok `#4C8A60` · borders `rgba(0,0,0,.1-.25)`. This is a full parallel token set, not a simple invert.

---

## Desktop frames (`Forge Machined - Desktop.dc.html`)

### 01 · Shell — main · rail · split + docks

#### D Main — 1440×900 · L28-129
**Maps to:** NEW (desktop shell) — wraps `components/chat/Composer.tsx`, `components/cards/PermissionCard.tsx`, `components/session/PlanSheet.tsx`, a dense session-list row; closest existing chrome file is `components/DesktopWindowChrome.tsx`.
- 3-pane: 36px macOS-style title bar → 232px expanded sidebar (search ⌘P, +New ⌘N, host-grouped session groups, Forge Anywhere host list, usage-ring row, What's New/Schedules/Settings/Hide-sidebar footer) → chat column (760px centered).
- Sidebar sessions grouped by project name (VECTRA-BOT, HELM, ADULARI-SITE), then a separate FORGE ANYWHERE host group with online/busy/stale dots.
- Chat column stacks: user message → "Thought for Ns" collapse → assistant text → quiet tool-run log → permission card → plan panel (pinned via `margin-top:auto`) → composer.
- Structural difference from mobile: session list + chat live side-by-side in one window; no tab-bar navigation.

#### D Rail + Terminal — 1440×900 · L133-186
**Maps to:** NEW — collapsed 48px icon rail (no current file) + bottom terminal dock (no current file); goal-loop chat banner maps to `components/session/GoalBanner.tsx` + `LoopSheet.tsx`.
- Icon rail replaces the 232px sidebar: nav icons, session icons (with needs-you/active dot badges), then usage rings + settings/update icons stacked at bottom.
- Goal-loop banner: `GOAL · LOOPING` label, quoted goal text, iteration counter + cost, inline "Stop".
- Terminal dock: 190px fixed-height pane below chat, mono green/amber ok/running text, blinking cursor block.
- Session header shows `goal · iter 3` instead of the usual cost/context readout — goal-mode is a distinct header state.

### 02 · Split panes + usage dock · git review dock

#### D Split + Usage — 1440×900 · L196-260
**Maps to:** NEW (split-pane layout) — panes reuse the same chat/permission/composer components as D Main; right-hand Usage dock (296px) maps to `mobile/src/app/usage.tsx` content re-flowed as a docked panel, no dock-container component exists yet.
- Two session panes side-by-side, each with its own 38px mini-header (dot, title, project, ⇄/✓/× icons) and its own composer.
- Usage dock: per-provider cards (Anthropic Max 20x, OpenAI Pro, Google API key, Forge Cloud) each with session/weekly bars + pace dot (`● ≈75% by reset`) + a dashed "+ Connect provider" card.
- Usage dock is drag-to-reorder and has its own refresh/close icons — a persistent side dock, not a modal/sheet.

#### D Git Review — 1440×900 · L265-308
**Maps to:** Partially `components/review/DiffCard.tsx`/`DiffLines.tsx` (diff rendering) and `PlanCard.tsx` (commit-message-adjacent); the staged/unstaged file list + split-diff browser + commit box is NEW — no dedicated git-review route exists in the current app.
- Full-window dock: 264px file list (STAGED/UNSTAGED groups, M/A status letters, +/− counts) → main pane with per-file split diff (old|new columns, line numbers, red/green highlighted changed lines) → generated commit message box + "Commit N files" button + branch target (`⑂ wt-a1b2 → develop`).
- Diff view toggles `split`/`unified` per file (top-right of file header).
- Entered via a "Return to app" affordance, implying it replaces the whole window content rather than living in a side panel.

### 03 · Overlays — ⌘K · ⌘P · quick composer · forge a task · what's new · schedules

#### D Palette (⌘K) — 660px · L318-331
**Maps to:** `components/overlay/CommandPalette.tsx` (26.9K — already the largest overlay component, likely already covers this).
- Search input + esc hint, SESSIONS section (dot + title + status/meta), ACTIONS section (icon + label + shortcut, `/plan`, `/compact` slash commands, "Hand off workspace…").
- Footer: `↑↓ navigate · ↵ select` left, tab-bar equivalent breadcrumb right (`Fleet · Inbox • · History · Settings`).

#### D Thread Search (⌘P) — 560px · L336-343
**Maps to:** Likely a mode of `components/overlay/CommandPalette.tsx` or `OverlayPanel.tsx`; no separate search-only component confirmed.
- Query text with cursor caret rendered inline (`relay|`), match count top-right.
- Results show highlighted match term in `#FF8A3D` within title and snippet, project+age mono meta.

#### D Quick Composer (⌥Space) — 640px · L348-358
**Maps to:** NEW — global systemwide overlay composer, no current equivalent (current `Composer.tsx` is in-session only).
- Single-line growing text with caret, then a control row: project chip, mode chip (Automatic), permission-tier chip (ASK), worktree chip, `↵ forge · ⇧↵ open full` hint, send button.
- Distinguishing feature: designed to float over any application, not just Forge's own window.

#### D Forge a Task (⌘N) — 640px · L363-379
**Maps to:** `mobile/src/app/new-session.tsx` (19.7K) — desktop modal variant of the same flow.
- Freeform prompt textarea, helper caption, project/mode/host chip row, READ/ASK/EDIT/FULL segmented permission tier, isolated-worktree toggle (default on, labeled "recommended").
- Footer changes copy conditionally: offline host turns primary button into "Queue remote job".

#### D What's New — 520px · L384-390
**Maps to:** NEW — no current file found for a changelog/release-notes surface.
- Simple list: version + NEW/date badge, 1-2 line description per release, most recent expanded.

#### D Schedules — 640px · L395-403
**Maps to:** NEW — no current file found for recurring/cron-style session scheduling.
- Row per schedule: status dot (`#5FB97D` active / `#33333B` paused), name, workflow+cadence+host mono meta, next-run time.
- Footer note clarifies scheduled runs land in Fleet as ordinary sessions, and route through the offline-host remote-job queue.

### 04 · Core views — fleet · inbox · history · floor · connect

#### D Fleet — 1100×760 · L413-440
**Maps to:** `mobile/src/app/(tabs)/index.tsx` + `components/fleet/SessionCard.tsx`, `FleetWatcher.tsx` — desktop is a centered 800px single-column re-flow of the same list, not tab-barred.
- Header stat line (`1 needs you · 1 forging · $2.27 today`) + host filter chips (All/MacBook Pro/atlas/forge-mini) + inline "Describe a task" composer entry point above the list.
- Needs-you card gets a tinted danger border and inline Respond/Peek actions; other cards are plain rows.
- Footer: "N hosts online · N stale — Hosts" link, absent on mobile.

#### D Inbox — 1100×760 · L445-462
**Maps to:** `mobile/src/app/(tabs)/inbox.tsx` — desktop re-flow, same permission-card-only content model.
- Single centered card list (decisions only), same PERMISSION card pattern as Fleet/session chat, plus an explicit "Open session" action (desktop-only third action next to Allow/Deny).
- Empty state: checkmark glyph + "That's everything — nothing else needs you."

#### D History — 1100×760 · L467-502
**Maps to:** `mobile/src/app/(tabs)/history.tsx` — desktop re-flow.
- Search field + All/Active/Archived segmented filter, TODAY/THIS WEEK date-grouped sections.
- Sync-state variants per row: `✓ synced · offline ok`, `↑ syncing N records`, `⑂ conflict copy kept` (tinted warn border), `◌ offline — device-encrypted`, `archived`.

#### D Floor — 1100×760 · L506-522
**Maps to:** `mobile/src/app/(tabs)/floor.tsx` + `components/floor/FloorTile.tsx`.
- Two (or N) equal-width live-tail columns separated by 1px gaps, each showing title+LIVE badge, assistant text, mono tool-output block, and inline permission card when needed, footer meta line.
- Desktop-only structural difference: tiles are literal side-by-side columns (`display:flex` with 1px gaps) vs. mobile's vertically stacked cards.

#### D Connect — 1100×760 · L527-546
**Maps to:** `mobile/src/app/connect.tsx` (20.8K) — desktop variant, centered 460px column instead of full-bleed mobile screen.
- 3-step numbered instructions, connect-URL input, "found on this machine" auto-discovery card (desktop-only convenience, LAN daemon detection), GitHub sign-in link for Forge Anywhere.

### 05 · Session tabs — tasks · agents · review · replay

#### D Session Tasks — 760px · L556-564
**Maps to:** `components/fleet/TaskRow.tsx` + session task list (no single "Tasks tab" container file confirmed — likely composed ad hoc inside `session/[id]`).
- Flat list, 32px rows, done (strikethrough+✓)/in-progress (highlighted row bg + pulsing dot + assignee mono tag)/queued (`○` + "queued" tag) states.

#### D Session Agents — 760px · L569-586
**Maps to:** `components/session/SubagentsPanel.tsx`, `SubagentStrip.tsx`, `AgentRow.tsx`.
- Card per subagent: name, model+cost mono tag, live mono status line (e.g. compiling output); needs-permission state gets Allow/Deny inline; completed state shows diff stat + cost, muted border.

#### D Session Review — 760px · L591-604
**Maps to:** `components/session/PlanSheet.tsx` + `components/review/PlanCard.tsx`.
- Numbered plan steps with per-step file path mono caption, a warn-bordered callout ("Touches the default upstream…"), Approve/Revise/Cancel action row, inline revise-text input below.

#### D Session Replay — 760px · L609-618
**Maps to:** NEW — no replay/scrubber file found in `components/session/` or `app/`; closest adjacent concept is `mobile/src/app/session-tree.tsx` (checkpoints, not a message-by-message replay).
- Chronological log rows (`sys`/`you`/`forge`/`tool`, fixed-width mono timestamp column) + a scrub bar with draggable thumb and `mm:ss/mm:ss` counter.

### 06 · Forge systems — workflows · subagents · mesh · assay · tree · checkpoints · memory · lattice · hooks · skills · duel · effort

#### D Workflow Run — 700px · L628-644
**Maps to:** `components/workflow/PhaseTimeline.tsx`, `ProgressHeader.tsx`, `AgentDrill.tsx`, `PipelineLane.tsx`.
- Overall progress bar + phase list (done/active/queued), active phase expands into a nested mono agent list (running/failed-with-Retry/queued), Pause queue / Abort run footer actions.

#### D Workflow Library — 700px · L649-666
**Maps to:** `components/workflow/WorkflowResult.tsx`, `StructuredOutput.tsx`; no dedicated top-level `workflows` route found in the `app/` listing — likely needs a new screen or lives nested under a session tab.
- Workflow definition cards (name, description, typed args chips, "Run workflow" button, recent-run history strip) + a separate expandable JSON result card with per-phase duration breakdown.

#### D Mesh Explain — 700px · L671-684
**Maps to:** No dedicated "why this model" component found; closest existing file is `mobile/src/app/models.tsx`.
- Picked-model card (name, COMPLEX/SUBSCRIPTION pill badges, reasoning sentence, coding/iq/ctx mono stats) + ranked candidate list with per-candidate benched/quota/threshold reasons.

#### D Assay — 700px · L689-701
**Maps to:** `components/session/AssaySheet.tsx`, `AssayView.tsx` (26.2K).
- Voter status chips (`✓ security · sonnet-5`, running `●`), findings list with severity pill (HIGH/MED, filled background), confirmation ratio (`3/3` voters), file:line mono citation.

#### D Tree Checkpoints — 700px · L706-719
**Maps to:** `mobile/src/app/session-tree.tsx` (16.5K) + `components/session/ForkSheet.tsx`, `CheckpointSheet.tsx`.
- Vertical tree: main line (current, `#FF8A3D` dot) with indented fork branches (`border-left` connector line), each fork row has Open/Diff/Merge back actions. Checkpoints list below with Restore/Diff actions and auto-vs-manual distinction.

#### D Memory Lattice — 700px · L724-738
**Maps to:** `components/session/MemorySheet.tsx`, `LatticeSheet.tsx`.
- Memory rows tagged USER/PROJECT/FEEDBACK with recall-count mono badge and inline Edit/Forget; Lattice card shows a function signature, PUB visibility tag, reference/caller counts, and a caller list with file:line + function name.

#### D Hooks Skills — 700px · L743-756
**Maps to:** `mobile/src/app/hooks.tsx` (7.9K), `skills.tsx` (12.1K).
- Hook rows: event-type tag (POST-EDIT/PRE-CMD/SESSION-END), name, matcher/command mono caption, last-run status (`ok`/`blocked`/`disabled`) right-aligned. Skill rows: name, BUILTIN/PROJECT source tag + invocation hint, one-line description.

#### D Duel Effort — 700px · L761-781
**Maps to:** `components/session/DuelSheet.tsx`, `DuelView.tsx` (13.8K), `EffortPicker.tsx` (11.4K).
- Duel: two side-by-side model-answer cards (name, cost/time, answer text, "Pick this answer") + scoreboard win-rate line. Effort: DEFAULT/LOW/MED/HIGH/XHIGH/WHITEHOT segmented control, WHITEHOT rendered in accent color as a distinct/expensive tier, per-tier cost caption below.

### 07 · Settings — full window + all panes

#### D Settings General — 1100×720 · L791-827
**Maps to:** `mobile/src/app/(tabs)/settings.tsx` (32.3K) content, restructured as a master-detail desktop shell.
- 196px nav rail (General/Providers & usage/Forge Anywhere w/ TRIAL badge/Models & mesh/Plans/MCP servers/Configuration/Skills/Hooks/Session tree) + detail pane. Maps structurally to `components/ds/MasterDetail.tsx` (1.9K, exists — likely the intended container).
- General pane: SERVERS list, APPEARANCE segmented (Light/Dark/System), version footer.

#### D Settings Usage — 860px · L832-848
**Maps to:** `mobile/src/app/usage.tsx` (10.7K).
- Big combined-usage ring + token breakdown header card, then per-provider cards with dual progress bars (5h window + week) and an OAuth/API badge; dashed "+ Connect provider" affordance.

#### D Settings Models — 860px · L853-864
**Maps to:** `mobile/src/app/models.tsx` (7.2K).
- Search-models input, tier-grouped (COMPLEX/STANDARD/TRIVIAL) rows: status dot, mono model name, provider+tier mono caption, IQ/code/ctx stat caption, ready/benched status.

#### D Settings Plans MCP Config — 860px · L869-891
**Maps to:** `mobile/src/app/(tabs)/plans.tsx` (15.7K), `mcp.tsx` (3.6K), `configuration.tsx` (8.9K) — three panes stacked into one desktop scroll region; on mobile these are 3 separate screens (see M Config MCP Plans below).
- Plans: proposal cards w/ inline Approve/Revise. MCP: server rows w/ transport type, secret-ref count, enable toggle. Configuration: EVERYWHERE/PROJECT scope segmented, per-setting row with dotted-path mono caption showing scope provenance (`mesh.auto_failover · default`).

### 08 · Native — menu bar · context menus · tray · notifications · dock · about

#### D Menus — 900px · L901-955
**Maps to:** NEW — native Tauri/macOS menu bar, no RN component; would be implemented via Tauri's native menu API, not `components/`.
- Full macOS menu bar strip (Forge/File/Session/View/Go/Window/Help) with Session menu open (New Session ⌘N, Quick Composer ⌥Space, Search ⌘P, Approve Waiting Decision ⌘⏎, Interrupt ⌘., Fork, Checkpoint ⌘S, Hand Off, Share Replay, Archive) and View menu (Sidebar ⌘\, Split Pane ⌘D, Terminal ⌘J, Usage ⌘U, Notes, Git Review ⌘G, Appearance submenu) both shown, plus session-row and dock-icon right-click context menus.
- Every session-scoped power-user action (fork, checkpoint, handoff, interrupt) is exposed as both a keyboard shortcut and a menu item — a discoverability layer with no mobile equivalent.

#### D Tray Notif About — 560px · L960-988
**Maps to:** NEW — native menu-bar-extra (tray), notification, and About dialog; closest existing file is `components/DesktopWindowChrome.tsx` for app-level chrome conventions.
- Tray dropdown: mini permission card + compact session list + Open Forge/Quick composer footer. Notification: generic-when-locked copy (Anywhere pushes must not leak content). About: logo, version+build+protocol, daemon version+host, Check for updates/Acknowledgements buttons.

### 09 · Forge Anywhere — settings pane · transport + handoff · remote jobs · shares

#### D AW Settings — 860px · L998-1023
**Maps to:** `mobile/src/app/anywhere/index.tsx` (33.3K) + `billing.tsx` (9.7K), `storage.tsx` (10.3K) — desktop combines hosts/devices/storage/billing/account into one scrollable pane where mobile splits them into separate screens.
- HOSTS list (2 of 3, online/busy/stale states, version+heartbeat mono caption) · two-column DEVICES list + STORAGE/BILLING/ACCOUNT column (storage bar with retention captions, trial billing card with plan prices, account row with Export/Sign out/Delete).

#### D AW Host Detail — 560px · L1028-1040
**Maps to:** `mobile/src/app/anywhere/host/[id].tsx` (4.5K).
- Identity (SHA256 fingerprint, "unchanged by rename"), connector version+heartbeat, reachable-via transport list, sessions-on-this-host list, transport-for-new-sessions segmented (AUTO/DIRECT/ANYWHERE), Disable/Revoke footer.

#### D AW Transport Handoff — 760px · L1045-1065
**Maps to:** `mobile/src/app/anywhere/handoff.tsx` (5.9K) + `components/anywhere/HandoffSheet.tsx` (16.6K), `components/session/StatusStrip.tsx` (10.7K).
- 5 mutually-exclusive status-strip variants (reconnecting/asleep/relay-unreachable-paired/relay-unreachable-unpaired/read-only-elsewhere) shown stacked for reference — only one renders at a time in product. Handoff modal: source/destination host picker row, preflight file-count + secret-exclusion warning, progress checklist (packaged → uploaded → verifying → apply&import) as the final state machine.

#### D AW Jobs Shares — 700px · L1071-1084
**Maps to:** `mobile/src/app/anywhere/jobs.tsx` (9.3K) + `components/anywhere/ShareSheet.tsx` (9.6K).
- Remote jobs queue (running/waiting/queued-offline/failed states, Cancel/Replace/Requeue actions, "queue is management only — running jobs live in Fleet" caption) + replay share-link creation (expiry segmented, key-in-fragment privacy note, active/expired/revoked link history).

---

## Mobile frames (`Forge Machined - Mobile.dc.html`)

All frames are 390×844 unless noted; all use the "2a Air" recalibration (type floor 10.5-11px mono / 13px body, card padding +30%, hit targets ≥44px) per the file's own header note.

### 01 · Core — fleet · session · floor · inbox · history · new session

#### M Fleet — L25-64
**Maps to:** `app/(tabs)/index.tsx` + `components/fleet/SessionCard.tsx`.
- Header stat line + host filter chips, needs-you card gets danger-tinted border + Respond(44px)/Peek(92×44px) buttons, inline task-composer entry above tab bar, 4-item tab bar (Fleet/Inbox w/ badge/History/Settings).

#### M Session Chat — L66-95
**Maps to:** `app/session/[id]/` + `components/chat/Composer.tsx`, `components/cards/PermissionCard.tsx`, `components/session/SessionHeader.tsx`.
- Back chevron + status dot + title + overflow `⋯`; mono meta line (project·worktree·model·cost·ctx); 5-tab strip (Chat/Tasks/Agents/Review/Replay); message stack identical in structure to desktop but single-column full-width; composer adds a mic icon absent on desktop; home-indicator bar at bottom (native iOS chrome).

#### M Floor — L97-115
**Maps to:** `app/(tabs)/floor.tsx` + `components/floor/FloorTile.tsx`.
- Cards stacked vertically (not side-by-side columns like desktop), each with LIVE badge, mono tool-output block, inline permission card when needed, meta footer line.

#### M Inbox — L117-133
**Maps to:** `app/(tabs)/inbox.tsx`.
- Same permission-card pattern as Fleet's needs-you card; explicit empty state illustration+copy when nothing is waiting.

#### M History — L135-168
**Maps to:** `app/(tabs)/history.tsx`.
- Sync-status header caption (`synced 2m · 1.2/5 GB`), search field + All/Active segmented filter, TODAY/THIS WEEK grouping, same sync-state vocabulary as desktop (synced/syncing/conflict/offline/archived).

#### M New Session — L170-186
**Maps to:** `app/new-session.tsx` (19.7K).
- Full-screen modal: freeform prompt textarea w/ blinking caret, helper line, Project/Host/Model rows (each a 48px tappable row with trailing chevron-style hint), READ/ASK/EDIT/FULL segmented, isolated-worktree toggle, warn-tinted offline-host callout that changes CTA copy to "Queue remote job", bottom-pinned CTA button.

### 02 · Session tabs + features

#### M Session Tasks Agents — L193-206
**Maps to:** `components/fleet/TaskRow.tsx`, `components/session/AgentRow.tsx`, `SubagentsPanel.tsx`.
- Tasks list (done/in-progress-highlighted-row/queued) stacked above an Agents list (running/needs-permission-inline-Allow-Deny/completed) in one scroll view — mobile combines what desktop splits into separate Tasks/Agents tabs.

#### M Session Review Replay — L208-226
**Maps to:** `components/session/PlanSheet.tsx` (Review) + NEW for Replay (no scrubber component found, same gap as desktop).
- Plan card (numbered steps, warn callout, Approve/Revise buttons) stacked above a Replay log (timestamped sys/you/forge/tool rows + scrub bar) — again two desktop tabs combined into one mobile scroll.

#### M Workflows — L228-253
**Maps to:** `components/workflow/PhaseTimeline.tsx`, `ProgressHeader.tsx`, `WorkflowResult.tsx`; no dedicated top-level route found.
- Workflow library card (args chips, Run button, run-history strip) directly followed by the active run's phase breakdown (progress bar, per-phase agent list with Retry action) and a separate completed-run JSON result card.

#### M Mesh Effort Duel — L255-280
**Maps to:** No "why this model" file found (same gap as desktop); `components/session/EffortPicker.tsx`; `DuelSheet.tsx`/`DuelView.tsx` (13.8K).
- "Why this model" card (COMPLEX/SUBSCRIPTION badges, reasoning line, coding/iq stats) + ranked candidate list → Effort segmented (AUTO/LOW/MED/HIGH/XHIGH/WHITEHOT, WHITEHOT in accent) → Duel (two stacked answer cards, not side-by-side like desktop, + scoreboard line).
- Structural difference from desktop: Duel cards stack vertically instead of two-column, since 390px width can't fit them side-by-side.

#### M Tree Checkpoints Goal — L282-299
**Maps to:** `app/session-tree.tsx` (16.5K) + `components/session/ForkSheet.tsx`, `CheckpointSheet.tsx`, `GoalBanner.tsx`, `LoopSheet.tsx`.
- Session tree (main line + indented forks w/ Open/Merge back), Checkpoints list (manual/auto, Restore/Diff), Goal loop card (quoted goal, iteration history, Stop button) — three desktop panels combined into one mobile scroll.

#### M Memory Lattice Hooks Skills Assay — L301-325
**Maps to:** `components/session/MemorySheet.tsx`, `LatticeSheet.tsx`; `app/hooks.tsx`, `skills.tsx`; `components/session/AssaySheet.tsx`.
- Five distinct desktop panels (Memory, Lattice, Hooks, Skills, Assay) stacked into a single long mobile scroll view — the largest instance of mobile-collapsing-multiple-desktop-panes-into-one-screen in this file.

### 03 · Settings stack

#### M Settings — L332-355
**Maps to:** `app/(tabs)/settings.tsx` (32.3K).
- SERVERS list, FORGE section (Forge Anywhere w/ TRIAL badge, Usage, Models & mesh, Plans, MCP servers, Configuration, Skills, Session tree — each a 50px row w/ trailing mono value + chevron), APPEARANCE segmented, version footer.

#### M Usage Models — L357-382
**Maps to:** `app/usage.tsx` (10.7K), `app/models.tsx` (7.2K).
- Big combined-usage ring card, per-provider dual-bar cards (5h + week), then Models & Mesh tier-grouped list (COMPLEX/STANDARD) with status dot + IQ/code stats — two desktop panes stacked into one mobile screen.

#### M Config MCP Plans — L384-401
**Maps to:** `app/configuration.tsx` (8.9K), `mcp.tsx` (3.6K), `app/(tabs)/plans.tsx` (15.7K).
- EVERYWHERE/PROJECT segmented config rows (toggle/segmented/text value + dotted-path mono caption), MCP server rows (name, transport+secret-ref caption, enable toggle), Plans awaiting-review card (Approve/Revise) — three separate desktop panes stacked here too.

#### M Connect — L403-424
**Maps to:** `app/connect.tsx` (20.8K) + `components/pairing/QRScan.native.tsx`/`.web.tsx`.
- Full-screen first-run: logo+tagline, 3-step instructions, "Scan QR code" primary CTA (camera stays off until tapped — explicit privacy note), connect-URL fallback text, GitHub sign-in link for Anywhere.

### 04 · Forge Anywhere — hub · connect · sign-in · phrase · hosts · pairing · transport · handoff

#### M AW Hub — L431-446
**Maps to:** `app/anywhere/index.tsx` (33.3K).
- TRIAL badge in header, account+sync mono caption, list (Hosts/Devices/Remote jobs/Notifications/Encrypted storage/Billing/Account rows, each 50px w/ trailing mono value+chevron), privacy footnote about sealed envelopes.

#### M AW Connect Signin — L448-472
**Maps to:** `app/connect.tsx` (Direct card) + `app/anywhere/sign-in.tsx` (161B — currently a stub, not yet implemented).
- DIRECT card (LAN/tunnel, free, connected-status caption) + ANYWHERE card (managed relay pitch, "Sign in with GitHub" CTA, pricing caption) + relay-unreachable-but-Direct-still-works warning row, then GitHub device-code flow (code display, Copy/waiting-countdown, new-vs-returning-account explainer, expired/denied states).
- `sign-in.tsx` being a 161B stub is a concrete gap: this frame's device-code UI has no real implementation yet.

#### M AW Phrase FirstHost — L474-494
**Maps to:** `app/anywhere/recovery-phrase.tsx` (13.3K) + `first-host.tsx` (6.3K).
- 24-word recovery phrase grid (masked after word ~2, "shown once" warning, no-cloud-backup caption), 2-word confirmation inputs, "I wrote it down" CTA; then first-host connect card (CLI command snippet, waiting-for-host state, "Use Direct meanwhile" / "Pair another device" fallbacks).

#### M AW Hosts Detail Pair — L496-522
**Maps to:** `app/anywhere/hosts.tsx` (160B stub) + `host/[id].tsx` (4.5K) + `pair.tsx` (168B stub), `passkey.tsx` (3.3K).
- Hosts list (online/busy/stale/revoked-strikethrough states, 3-active-max caption) + host detail (identity fingerprint, reachable-via, transport segmented) + approve-new-device pairing card (device+fingerprint+grants summary, Approve/Reject, expired/used/wrong-account state captions).
- `hosts.tsx` and `pair.tsx` being stub files (160-168B) is a concrete gap versus this frame's fairly detailed content.

#### M AW Transport Handoff — L524-547
**Maps to:** `app/anywhere/handoff.tsx` (5.9K) + `components/anywhere/HandoffSheet.tsx` (16.6K), `components/session/StatusStrip.tsx`.
- Same 5+1 status-strip variants as desktop (reconnecting/asleep/relay-unreachable×2/read-only/plan-read-only-billing) shown stacked for reference, then the handoff flow (source/destination picker, preflight capsule size + secret-exclusion note, CTA, then a 4-line progress readout: packaged→uploaded→verifying→apply&import).

#### M AW Jobs Shares Devices — L549-570
**Maps to:** `app/anywhere/jobs.tsx` (9.3K) + `components/anywhere/ShareSheet.tsx` (9.6K) + `app/anywhere/devices.tsx` (162B stub).
- Remote jobs queue/history segmented, per-job state rows (running/waiting+Cancel+Replace/failed+Requeue); Share Replay (expiry segmented, link+Copy+Revoke, "relay never sees the key" note); Devices list (this-device tag, last-seen mono, Pair device + Revoke-with-key-rotation explainer).
- `devices.tsx` being a 162B stub is a concrete gap versus this frame's device-list detail.

#### M AW Billing Storage Account — L572-603
**Maps to:** `app/anywhere/billing.tsx` (9.7K), `storage.tsx` (10.3K), `account.tsx` (162B stub).
- Billing (trial badge, yearly-vs-monthly price cards, Paddle-checkout CTA, full ENTITLEMENT LIFECYCLE state table: NOT STARTED/ACTIVE/GRACE·7D/READ-ONLY/SUSPENDED); Storage (usage bar + retention-schedule mono caption); Account (Sign out/Export/Delete rows, recovery-via-phrase-or-paired-device explainer).
- `account.tsx` being a 162B stub is a concrete gap versus this frame's account-management detail; the entitlement lifecycle table is the single densest piece of business logic in either design file and has no corresponding state machine visibly implemented yet.

### 05 · Native iOS — live activity · dynamic island · notifications · widget · light theme

#### M Native Surfaces — L610-627
**Maps to:** NEW — native iOS Live Activity/Dynamic Island/widget extensions are Swift/WidgetKit targets, out of the React Native component tree entirely; no `mobile/src/` file can implement these directly (would need a native module + widget extension target).
- Lock-screen Live Activity (permission card at 7px radius, Allow/Deny/Open triple action), Dynamic Island compact (icon+pace text) and expanded (full permission mini-card, pill-shaped Allow/Deny), generic locked push notification (content deliberately hidden for Anywhere-routed pushes), home-screen widget (170×170, Fleet glance w/ 3 session dots + needs-you footer).
- Radii here (7/14/17/22px) are a deliberate native-platform exception to the documented 3-4px radius — flagged in the tokens section above.

#### M Fleet Light — 390×844 · L629-661
**Maps to:** `app/(tabs)/index.tsx` (light-theme variant, not a separate file).
- Identical structure/content to M Fleet but on the full light palette documented in the tokens section (bg `#F5F4F1`, accent `#D96A1E`, etc.) — confirms light theme is a token swap, not a layout change.

---

## Gaps summary (frames with no current implementation file)

New desktop-only chrome (native Tauri/OS surfaces, no RN equivalent expected): D Rail+Terminal icon rail, D Quick Composer, D What's New, D Schedules, D Git Review dock, D Menus, D Tray Notif About.

Missing on both platforms (a real feature gap, not just desktop chrome): session **Replay** scrubber (D Session Replay / M Session Review Replay), **Mesh Explain** "why this model" panel (D Mesh Explain / M Mesh Effort Duel), and a top-level **Workflows** library route (D Workflow Library / M Workflows — components exist under `components/workflow/` but no confirmed `app/` route).

Stub files (<200B) that are placeholders for fairly detailed designed screens: `app/anywhere/sign-in.tsx`, `hosts.tsx`, `pair.tsx`, `devices.tsx`, `account.tsx`, `_layout.tsx`.
