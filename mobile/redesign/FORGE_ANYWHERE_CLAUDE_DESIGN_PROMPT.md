# Claude Design brief: Forge Anywhere app extension

> **Existing Forge app project only.** Run this brief in the separate Claude Design project that
> produced the already-implemented Forge app redesign.
>
> Do not create another application, app shell, dashboard, navigation model, session interface,
> design system, or set of generic mobile/desktop screens. Forge Anywhere is a feature and
> transport extension inside the existing Forge app. The public marketing page belongs to the
> Forge website project and is explicitly out of scope here.

## Objective

Extend the existing Forge app’s completed Emberline design with Forge Anywhere. Design only the
new or changed Anywhere states, routes, and integration points. Reuse the existing app shell,
Fleet, Inbox, History, Settings, session shell, Chat, Tasks, Agents, Review, command palette,
responsive rail/tabs, and design-system components.

The same app must continue to serve:

- iOS.
- Android.
- Responsive web app/PWA.
- macOS desktop through the existing Tauri shell.
- Windows desktop through the existing Tauri shell.
- Linux desktop through the existing Tauri shell.

There is one Forge app. `Direct` and `Anywhere` are transport choices inside it.

## Authoritative references

Treat the current implementation and these documents in the Forge repository as binding:

- `mobile/redesign/DESIGN_SYSTEM.md` — Emberline tokens, components, motion, accessibility, and
  responsive rules.
- `mobile/redesign/DESIGN_ELEVATION.md` — thermal identity, de-boxing, and instrument-grade type.
- `mobile/redesign/FEATURES.md` — final app information architecture and capability map.
- `mobile/redesign/ARCHITECTURE.md` — shared Expo/Tauri architecture and transport boundaries.
- `mobile/src/theme/` — implemented semantic tokens, typography, breakpoints, and motion.
- `mobile/src/components/ds/` — implemented reusable component library.
- `mobile/src/app/` — implemented route structure.
- `mobile/src/app/anywhere/` — existing functional Anywhere routes that need to be reconciled with
  the approved app design rather than replaced by a parallel experience.

Where this brief conflicts with a generic app convention, the existing Forge app wins. Do not
invent new colors, spacing, radii, typography, motion curves, icons, navigation, or component
primitives.

## Absolute anti-duplication rules

- Do not create an “Anywhere app.”
- Do not create a second home, fleet dashboard, history browser, session viewer, settings shell,
  login shell, tab bar, sidebar, or command palette.
- Do not add a permanent `Anywhere` bottom tab or desktop rail destination.
- Do not duplicate Direct-connected sessions into a separate Anywhere session list.
- Do not fork mobile, web, and desktop into separate products. Use the existing responsive system.
- Do not redesign the existing session timeline, composer, tasks, agents, review, permission cards,
  or overlays.
- Do not create a second design system or website-derived app styling.
- Do not add team, organization, role, shared-fleet, or enterprise administration UI.
- Do not design marketing pages, pricing landing pages, or legal documents in this app project.
- Do not use mock product capabilities or mock daemon data.

If an Anywhere task can be completed by adding a state or action to an existing Forge surface,
extend that surface instead of creating a new route.

## Existing app identity: Emberline

The app is a control surface for a fleet of AI coding agents. Its character is “precision metal,
live ember”: calm graphite or warm paper, developer-native density, restrained type hierarchy, and
a scarce ember accent for things that are alive or need a human.

Binding principles:

1. Ember is scarce.
2. Ink hierarchy is preferred over boxes.
3. Motion maps to real system events.
4. Developer-native texture uses monospace, tabular numerals, status dots, diffs, and precise
   metadata.
5. Dark and light themes are both first-class.
6. Compact layouts use bottom tabs; expanded layouts use the existing MasterDetail rail.
7. Waiting-on-you remains the strongest cross-app signal.

Use existing semantic tokens from `mobile/src/theme/tokens.ts`. Do not introduce hex values in
design specifications or implementation notes.

Use existing type tokens: `display`, `title`, `heading`, `body`, `bodyBold`, `sub`, `meta`,
`section`, `code`, and `codeSmall`.

Use the existing component inventory and variants, including `Button`, `IconButton`, `Input`,
`PromptComposer`, `Chip`, `Segmented`, `SearchField`, `StatusDot`, `Badge`, `ContextGauge`,
`CostMetric`, `KeyValueRow`, `Screen`, `Card`, `ListRow`, `BoundedList`, `Sheet`, `Toast`,
`Banner`, `EmptyState`, `Skeleton`, and `ConfirmDialog`.

New Anywhere-specific components may only compose those primitives. If a missing reusable variant
is genuinely required, document the smallest addition to the existing `ds/` component rather than
creating a parallel component.

## Product contract

Forge Anywhere is the optional paid, end-to-end encrypted companion to free, open-source Forge.

Positioning:

> Leave your desk without leaving your Forge session.

Forge, local history, loopback/LAN access, direct pairing, and user-managed
`forge serve --anywhere` tunnels remain free.

Anywhere adds:

- Managed encrypted relay and stable host identity.
- Up to three active hosts.
- Unlimited personal controller devices.
- 5 GB encrypted sync/history.
- Remote jobs.
- Generic push notifications with no workspace content.
- Safe workspace handoff.
- Expiring encrypted replay shares.

Commercial facts:

- EUR 10/month or EUR 79/year through Paddle.
- Annual is selected by default where a plan choice is needed.
- 14-day no-card trial begins when the first host connects.
- One trial per GitHub account.
- V1 is personal-only.

Local Forge must remain visibly usable when Anywhere is disabled, unreachable, expired,
suspended, over quota, or unpaid.

## Existing information architecture to extend

Preserve these primary destinations:

- Fleet.
- Inbox.
- History.
- Settings.
- Existing session shell with Chat, Tasks, Agents, and Review.

Integrate Anywhere as follows:

- `Connect`: add Direct/Anywhere connection paths without changing the Direct flow.
- `Fleet`: show sessions and enrolled hosts together only where host context is useful; do not
  create a second fleet home.
- `History`: show synchronized/offline availability in the existing history experience.
- `Settings`: use one `Forge Anywhere` entry for account, entitlement, hosts, devices, storage,
  notifications, and billing management.
- `Session shell`: show selected host/transport and eligible handoff/share actions without changing
  the existing session structure.
- `New Session`: allow selection of an enrolled host and remote-job behavior within the existing
  creation flow.
- `Command palette`: add context-appropriate host, handoff, and Anywhere management actions where
  useful; do not create another command surface.

Management subroutes may remain under the existing `/anywhere` route group, but they are detail
screens reached from Settings, Connect, Fleet, or a session—not a second top-level app.

Current implemented management routes to design/reconcile:

- `/anywhere` — account bootstrap/status and management hub.
- `/anywhere/hosts`.
- `/anywhere/devices`.
- `/anywhere/pair`.
- `/anywhere/history` only where it adds sync diagnostics; ordinary browsing belongs in existing
  `/history`.
- `/anywhere/jobs` only where a dedicated queue is necessary; job creation should integrate with
  existing New Session.
- `/anywhere/notifications`.
- `/anywhere/storage`.
- `/anywhere/billing`.
- `/anywhere/handoff` only for management/recovery; initiating a handoff should start in the
  relevant session.
- Existing `/shares/[id]` public replay retrieval.
- Existing `/session/[id]/replay` replay view.

## Integration map

### Connect

Extend the existing Connect screen with two legible paths:

- Direct: current URL/token, QR, LAN, and daemon behavior remains unchanged.
- Anywhere: GitHub sign-in, encrypted account enrollment, and host fleet connection.

Do not present these as separate products. Explain that Direct is ideal for local/LAN/user-managed
access and Anywhere is the optional managed encrypted transport.

Cover:

- Direct remains the current default where already paired.
- Existing direct server plus optional Anywhere enrollment.
- No host enrolled.
- Anywhere service offline while Direct remains available.
- Account requires recovery.
- Entitlement blocks new managed work while local/direct access remains available.

### Fleet

Preserve the existing session-first Fleet. Add only the host context needed to answer:

- Which host runs this session?
- Is it reachable Direct, Anywhere, or both?
- Is the host online, busy, stale, or offline?
- Can a new session or remote job be started there?

Do not add a competing host-card dashboard above or beside the session fleet. A compact host
selector/filter, contextual status, or Settings detail route is preferable.

### History

Preserve the existing history list/search/resume behavior. Add:

- Synced/offline-available indicator.
- Last successful sync.
- Stale/retrying/read-only/over-quota states.
- Conflict-copy indication where a divergent portable file record is preserved.
- Host provenance only where it aids understanding.

Do not create a separate “cloud history” browser.

### Session shell

Extend the existing session header/status strip and overflow actions with:

- Host name.
- `Direct` or `Anywhere` transport.
- Reconnection/degraded status.
- Handoff eligibility at an idle checkpoint.
- Create/revoke encrypted replay share.
- Large encrypted transfer progress when relevant.

The remote-v8 session protocol and all existing Chat/Tasks/Agents/Review behavior remain unchanged.

### Settings

Add one Forge Anywhere settings entry showing a concise status summary. Its detail routes cover:

- Account and entitlement.
- Hosts.
- Devices and pairing.
- Notifications.
- Encrypted storage.
- Billing.
- Export/delete/logout.

Do not move existing Direct servers, appearance, app lock, usage, diagnostics, or app information
into an Anywhere-only settings hierarchy.

## Flow 1: GitHub sign-in and encryption bootstrap

Design within the existing Connect/Settings shell:

1. Start GitHub Device Flow.
2. Pending state with one-time code and external GitHub action.
3. Expired, denied, network-failed, retry, and success states.
4. Returning-account recovery path.
5. New-account explanation before key generation.
6. One-time 24-word recovery phrase display.
7. Sampled-word confirmation.
8. Failure, restart, and abandon paths.
9. First-host instructions using `forge anywhere enable --name NAME`.
10. Waiting state that says the trial has not started until the first host connects.

Do not expose realistic recovery words in shared design thumbnails. Use numbered redacted
placeholders. Do not offer cloud backup, analytics, screenshot encouragement, or persistence for
the phrase. “Shown once” and “Forge support cannot recover it” must be explicit and calm.

## Flow 2: Pair another device

Extend the app’s existing QR/paste pairing patterns:

- New device presents a ten-minute public-key challenge.
- Authorized device scans or pastes it.
- Review shows account, device name/type, fingerprint, and expiry.
- Approval is explicit; scanning alone does not grant access.
- Success explains that the account data key was wrapped to the new device.

Cover expired, already-used, malformed, wrong-account, offline, camera-denied, paste fallback,
rejected, and success states.

## Flow 3: Hosts and transport

Design the host detail/management route and lightweight integration states:

- Maximum three active hosts.
- Stable host name and identity.
- Last heartbeat and connector version.
- Online idle, online busy, connecting, stale, offline, disabled, revoked, incompatible, relay
  unavailable, and entitlement blocked.
- Existing sessions and current session ownership.
- Add-host CLI instructions.
- Fourth-host limit with a route to disable/revoke another host.
- Safe disable/revoke that never implies local Forge is removed.

Where both Direct and Anywhere are possible, provide a compact existing-style transport choice.
Do not duplicate the session or lose its current navigation state when transport changes.

## Flow 4: Live remote control and reconnect

Use the existing session UI. Design only its added status/action variants:

- Anywhere connected.
- Direct connected.
- Reconnecting.
- Host sleeping/offline.
- Relay unavailable with Direct fallback.
- Duplicate controller.
- Session ended.
- Entitlement changes while viewing.
- Large encrypted relay blob transfer.

Do not reimplement or visually reinterpret remote-v8 frames.

## Flow 5: Encrypted sync and offline history

Design states for the existing cache/history/settings surfaces:

- Initial sync.
- Incremental sync.
- Current.
- Paused.
- Retrying.
- Offline with device-encrypted cached data.
- Read-only entitlement.
- Over quota while download/delete remain available.
- Key-epoch update required.
- Tombstone/deletion propagation.
- File conflict copy created without overwriting either version.

Eligible records include sessions, messages, checkpoints, tool calls, routing decisions, usage,
compactions, memories, user settings, commands, skills, agents, workflows, and portable file
records.

Never imply syncing provider credentials, keyring contents, embeddings/indexes, push secrets, host
schedules, queue internals, caches, build output, checkpoint scratch files, pending uploads, or
arbitrary repository content.

## Flow 6: Remote jobs

Integrate job creation into existing New Session where possible:

- Choose an enrolled host.
- Optional working directory and session title.
- Queue encrypted request.
- Show queued locally, uploaded, waiting for host, claimed, running, completed, failed, canceled,
  expired, and entitlement-blocked states.
- Explain in detail view that path/title content is encrypted while routing metadata remains
  visible.

A dedicated queue route may exist for history/management, but must not duplicate active sessions
or become another Fleet.

## Flow 7: Generic notifications

Extend existing notification settings:

- Opt-in and platform permission.
- Enabled, denied, disabled, token-refreshing, and relay-unavailable states.
- Generic lock-screen copy: “Open Forge to view an update.”
- Opening an alert refreshes the existing app and routes to the relevant state after decryption.

Never show prompts, commands, filenames, repository names, diffs, or transcript content in a push
mockup.

## Flow 8: Workspace handoff

Initiate handoff from the existing session’s actions. Use an existing Sheet or focused route for a
deliberate high-trust flow:

1. Confirm the session is at an idle checkpoint.
2. If a tool call is active, wait or explicitly interrupt.
3. Choose destination host; source is the current host and cannot also be destination.
4. Preflight scan and capsule summary.
5. Show blocked files as an actionable visible list; never silently drop non-secret user files.
6. Confirm encrypted capsule creation/upload.
7. Destination verifies the base commit and prepares an isolated worktree.
8. Apply/import progress.
9. Destination acknowledgement.
10. Transfer session ownership only after acknowledgement.
11. Continue in the same existing session UI on the destination.

Preflight must communicate rejection of `.git`, symlinks, devices/special files,
absolute/traversal paths, detected secrets, ignored caches/build output, files above 25 MB, and
compressed capsules above 100 MB.

Cover patch conflict, unsafe file, missing commit, destination offline, expiry, quota, session ID
remap, interrupted transfer, import failure, and acknowledgement timeout. Every pre-transfer
failure must state that the destination worktree is removed and the source remains authoritative.

## Flow 9: Encrypted replay shares

Start from the existing session/replay actions:

- Choose expiry: 24 hours, 7 days, or 30 days.
- Explain read-only scope and end-to-end encryption.
- Show creation/upload progress.
- Success provides copy link, exact expiry, and revoke.
- Manage active, expired, and revoked shares without creating a content-management dashboard.
- Public no-login retrieval uses the existing replay viewer shell in a restricted read-only mode.

Cover decrypting, ready, wrong/missing key fragment, corrupted, expired, revoked, unavailable, and
deleted states. The public viewer cannot control a live session or browse unrelated data.

## Flow 10: Devices and key rotation

Design the existing management subroute using Emberline list/detail patterns:

- Device name/type, “this device,” enrollment date, and last seen.
- Fingerprint detail on demand.
- Pair-device action.
- Lost-device revocation with strong confirmation.
- Recovery phrase verification where required, held in memory only.
- Atomic progress: revoke tokens/hosts, create new key epoch, wrap to remaining devices/recovery,
  then commit.
- Success explains that future data uses the new epoch.
- Failure clearly states whether nothing changed.

Do not hide key rotation behind a generic red Delete button.

## Flow 11: Storage and quota

Design the Settings detail route:

- Used bytes versus 5 GB, with accessible text plus meter.
- What counts toward encrypted storage.
- Empty, calculating, stale, nearly full, full, and cleanup states.
- Above quota blocks writes but allows download and deletion.
- Concise retention information for temporary blobs/capsules, superseded revisions, tombstones,
  shares, and expired subscriptions.

## Flow 12: Billing and entitlement

Use this exact state model:

- `trialing`: full access for 14 days from first host connection.
- `active`: full access through the paid period.
- `grace`: seven days after payment failure; full read access and relay continue.
- `read_only`: 30 days after trial/period/grace expiry; download, restore, delete, export, and
  billing remain, while new relay work, uploads, commands, shares, and capsules are blocked.
- `suspended`: billing, export, and deletion only until the 90-day retention deadline.

Design trial-not-started, trial countdown, annual/monthly choice with annual default, external
Paddle checkout/return, active renewal, cancel-at-period-end, payment failure, grace countdown,
read-only, suspended, retention warnings, resubscribe, delayed webhook, and checkout failure.

Cancellation remains active through the paid-through date and never affects local Forge.

## Flow 13: Account controls and recovery

Design within existing Settings/detail patterns:

- Local logout removes this device’s Anywhere tokens/keys but preserves local Forge data.
- Host disable/revoke leaves local Forge installed and usable.
- Account export preparation, progress, and expiring download.
- Account deletion scope, confirmation, idempotent progress, 24-hour live-data target, and backup
  expiry within 30 days.
- Recover on a new device with the 24-word phrase.
- Wrong phrase, checksum failure, unavailable wrapped epoch, revoked device, and unrecoverable
  terminal state.

If every device and the recovery phrase are lost, support cannot decrypt the account.

## Shared state variants

Add named variants to the existing component/state canvases rather than creating an Anywhere-only
component library.

### Entitlement

- Trial not started.
- Trialing.
- Active.
- Grace.
- Read-only.
- Suspended.
- Retention deadline approaching.

### Host

- Online idle.
- Online busy.
- Connecting.
- Stale.
- Offline.
- Disabled/revoked.
- Update required.

### Sync

- Current.
- Uploading/downloading.
- Offline cached.
- Retrying.
- Conflict copy.
- Over quota.
- Key update required.
- Read-only.

### Handoff

- Eligible.
- Waiting for checkpoint.
- Scanning.
- Blocked with actionable files.
- Packaging/uploading.
- Waiting for destination.
- Applying/importing.
- Awaiting acknowledgement.
- Complete.
- Rolled back.
- Expired.

### Application

- First-use empty.
- Loading/skeleton.
- Slow network.
- Offline.
- Partial/stale data.
- Permission denied.
- Session expired/re-authentication.
- Service unavailable while local Forge remains available.
- Destructive confirmation.
- Success with next action.

Status must pair color with text, icon, and—where useful—timestamp or next action.

## Security and privacy UX

Primary copy should answer who can read data, what the service sees, what happens after device loss,
and whether support can recover it. Algorithm names belong in technical disclosures:

- X25519 exchange.
- Ed25519 signatures.
- HKDF-SHA256 derivation.
- XChaCha20-Poly1305 payload encryption.
- Signed, replay-protected envelopes and account key epochs.

The service can observe routing identifiers, timestamps, sizes, object kind, and signatures, but
not plaintext payloads. Never use “military-grade,” “unhackable,” “zero knowledge,” or “sync
everything.”

Never expose prompts, filenames, repository names, commands, diffs, transcript content, recovery
words, private keys, QR payloads, or device tokens in notifications, analytics, shared mockups, or
diagnostic examples.

## Responsive behavior

Use the existing app breakpoints and one responsive composition:

- Compact phone approximately 390 px.
- Medium tablet/compact desktop approximately 768–1024 px.
- Expanded desktop approximately 1180–1440 px.

Compact uses the existing bottom tabs and Sheets. Expanded uses the existing MasterDetail rail,
command palette, and denser metadata. Do not create separate native/web/desktop designs with
different information architecture.

On compact layouts, prioritize the current session/host, next action, and critical state. Move
secondary protocol detail into existing disclosure/detail patterns without hiding consequences.

## Accessibility and platform behavior

Follow the existing app’s binding requirements:

- WCAG 2.2 AA on web/desktop.
- Dynamic type and screen-reader order on native.
- Visible keyboard focus on web/desktop.
- Tap targets at least 44×44.
- 200% zoom and long localized strings.
- Reduced-motion guard for every animation.
- Accessible progress/status announcements.
- Error summary plus field-level errors.
- No state conveyed by color, animation, hover, or haptics alone.
- QR pairing always has a paste/manual fallback.
- Recovery confirmation is keyboard and screen-reader operable.
- Existing keyboard shortcuts and command palette remain available on desktop.
- Existing Face ID/app lock, camera, share, clipboard, PWA, and Tauri behavior remain intact.

## Deliverables

In the existing Forge app Claude Design project, add one canvas group named
`Forge Anywhere — Emberline Extension` containing:

- A map showing exactly which existing app frames are modified and which management detail routes
  are added.
- Compact and expanded variants for every changed existing surface.
- Light and dark variants using existing semantic tokens.
- All 13 flows above, integrated into existing navigation.
- Entitlement, host, sync, handoff, offline, error, destructive, and recovery variants.
- Reusable compositions built from existing Emberline components.
- The smallest documented component variants required in `mobile/src/components/ds/`.
- Interaction, keyboard, screen-reader, responsive, reduced-motion, and platform notes.
- A migration map from current implemented `mobile/src/app/anywhere/` routes to the approved frames,
  identifying screens that should be merged into existing Connect, Fleet, History, New Session,
  Session, or Settings surfaces.
- A short explicit list of proposed routes/components to delete or consolidate to prevent duplicate
  UX.

## Acceptance checklist

Before completion, verify:

- There is still one Forge app shell.
- Fleet, Inbox, History, Settings, and the existing session shell remain primary.
- Direct and Anywhere are transport choices, not separate apps.
- Ordinary synced history appears in existing History.
- Remote job creation uses existing New Session where practical.
- Handoff and replay sharing begin from the relevant existing session.
- Anywhere account/host/device/storage/billing management is reached through existing Settings or
  contextual actions.
- No duplicate tab bar, rail, session list, timeline, composer, settings, or component library was
  created.
- All six targets use the same responsive system.
- Exact pricing, limits, entitlement lifecycle, encryption boundary, and local-Forge continuity are
  represented accurately.
- The result extends Emberline instead of restyling it.

When finished, present the changed-frame map, consolidation/deletion recommendations, and any
unresolved product questions before exporting implementation assets.
