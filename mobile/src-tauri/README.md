# Forge desktop shell (Tauri v2)

Thin wrapper around the static web export (ARCHITECTURE.md §6). No custom Rust commands in
v1 — all daemon communication happens in the webview via the JS transport seam
(`../src/lib/transport/index.ts`).

## What's registered

- `tauri-plugin-notification` — native notifications, called from `../src/lib/notify.ts`
  when `isTauri` (feature-detected alongside the web `Notification` API).
- `tauri-plugin-opener` — external links open in the system browser.
- `tauri-plugin-http` — backs `@tauri-apps/plugin-http`'s `fetch`, used by the transport
  seam for `http:` daemon URLs (immune to the WebView's mixed-content blocking of plain
  `http://` requests that a plain `https://`-origin WebView page would otherwise refuse).
- `tauri-plugin-websocket` — backs the WS fallback adapter in the transport seam, used only
  if the native `WebSocket` fails to open a plain `ws://` connection.

A basic app menu (About, Reload, Quit, standard Edit menu for copy/paste on macOS) is built
in `src/lib.rs`'s `.setup()` hook.

## Building this (NOT done in this session)

This was built on a headless box with no display server and no system WebView — `tauri
dev`/`tauri build` need a real display (or at minimum a working system webview: WebKitGTK on
Linux, WebView2 on Windows, WKWebView on macOS) to actually launch the shell. What WAS
verified here, on this machine:

- `cargo check` in this directory compiles cleanly against the pinned plugin versions
  (tauri 2.11, tauri-plugin-http/websocket/notification/opener 2.x) and validates
  `tauri.conf.json` + `capabilities/default.json` (ACL/permission identifiers) via
  `tauri-build`'s codegen — a bad permission name or malformed config fails this step.
- `npx tauri icon ../assets/icon.png -o icons` generated real `icon.icns`/`icon.ico`/PNG set
  (not placeholders) from the app's actual icon.
- `npx tsc --noEmit` (from `mobile/`) is clean with the new transport/notify code.
- `npx expo export -p web` still produces `dist/` — the exact `frontendDist` this shell
  wraps — and an unminified debug export confirms all three Tauri plugin dynamic imports
  (`@tauri-apps/plugin-http`, `-websocket`, `-notification`) resolve correctly and are
  behind runtime `isTauri` checks, never touched on plain web/native bundles.

Actually running `npm run tauri:dev` / `npm run tauri:build` and confirming the window
opens, pairs with a `--local` daemon over plain http, and streams a session is deferred to
a dev machine or the B6 CI matrix (`.github/workflows/app-desktop.yml`, T6.3) per
BUILD_ORDER.md's T5.2 "Done" line — that check needs a real 3-OS runner, not this box.

## Forward path (not built, per ARCHITECTURE §6.4)

`transport`/`auth` seams are already shaped for a later Rust command that discovers/starts
`forge serve` locally and hands the app a ready-made baseUrl. Tray icon + global shortcut is
the second follow-up. Neither is in scope for v1.
