# Mobile/desktop parity contract

The retained product surface is shared by Expo native targets, react-native-web, and the Tauri desktop shell. A platform difference is allowed only when the capability genuinely differs; a screen, state transition, validation rule, copy string, or business behavior must remain shared.

## Legitimate divergence

These capability boundaries may diverge and must be listed in `mobile/parity/inventory.json`:

- filesystem and document/image pickers
- OS notifications, push registration, haptics, and secure storage
- Tauri window controls, desktop menus, updater, and native IPC/WebSocket transport
- safe-area, keyboard, pointer, touch, and IME input handling
- platform-required WebView/browser or native implementation files

Each inventory row names the source file, platform capability, and reason. The inventory is deliberately file-scoped for the first guard: a file containing a capability branch is reviewed as a unit, and adding a new source file or platform branch still requires an explicit inventory update and review.

## Accidental divergence

The following are not legitimate merely because they are convenient:

- a screen existing on one surface but not another
- different loading, error, empty, permission, or offline states
- different API/state rules or persistence semantics
- platform-only copy, labels, defaults, or accessibility behavior
- a desktop-only feature implemented in shared UI without a capability boundary

The check is `python3 scripts/check-mobile-parity.py`. It scans production TypeScript/TSX under `mobile/src` for `Platform.OS`, Tauri markers, and platform-suffixed modules. It fails if a detected file is absent from the checked-in inventory or if an inventory row points at a missing file. CI must run it before mobile checks. Reviewers should reject rows that describe product behavior rather than a capability boundary.
