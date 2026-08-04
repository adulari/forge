# Mobile/desktop parity contract

The retained product surface is shared by Expo native targets, react-native-web, and the Tauri desktop shell. A platform difference is allowed only when the capability genuinely differs; a screen, state transition, validation rule, copy string, or business behavior must remain shared.

## Legitimate divergence

These capability boundaries may diverge and must be listed in `mobile/parity/inventory.json`:

- filesystem and document/image pickers
- OS notifications, push registration, haptics, and secure storage
- Tauri window controls, desktop menus, updater, and native IPC/WebSocket transport
- safe-area, keyboard, pointer, touch, and IME input handling
- platform-required WebView/browser or native implementation files

Each inventory row names the source file, platform, reason, and category. `capability` rows cover genuine platform APIs; `behavior` rows are reviewed more strictly because they sit near rendered state, copy, state setters, queries, or transport decisions. A behavioral row is not permission to fork product rules: it declares the narrow capability boundary that needs review. Adding a new source file or platform branch still requires an explicit inventory update and review.

The guard deliberately does not treat every `Platform.OS` occurrence as behavioral. It uses a local context window around platform markers, ignores style-only branches as `capability`, and classifies branches near JSX, state updates, queries, copy, notifications, or transport as `behavior`. Both categories must be declared; a category mismatch fails CI.

## Accidental divergence

The following are not legitimate merely because they are convenient:

- a screen existing on one surface but not another
- different loading, error, empty, permission, or offline states
- different API/state rules or persistence semantics
- platform-only copy, labels, defaults, or accessibility behavior
- a desktop-only feature implemented in shared UI without a capability boundary

The check is `python3 scripts/check-mobile-parity.py`. It scans production TypeScript/TSX under `mobile/src`, reports declared capability and behavioral boundaries, and fails if a detected file is absent, stale, or declared with the wrong category. CI must run it before mobile checks. Reviewers should reject rows that describe product behavior rather than a narrow capability boundary.
