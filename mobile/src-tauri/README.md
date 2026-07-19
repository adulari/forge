# Forge desktop shell (Tauri v2)

Native shell around the shared Expo web export (`../dist`). The React Native Web app remains the UI
and session client; Rust provides the narrow desktop capabilities a browser cannot safely provide.

## Native surface

- `tauri-plugin-http` and `tauri-plugin-websocket` carry HTTP/WS traffic to a same-machine
  `forge serve --local` daemon without WebView mixed-content failures.
- `tauri-plugin-notification` provides native desktop notifications.
- `tauri-plugin-opener` opens external links in the system browser.
- `tauri-plugin-dialog` provides the native project-folder picker.
- `tauri-plugin-updater` plus `tauri-plugin-process` verifies signed updater artifacts and restarts
  after an accepted update.
- `serve_discovery.rs` exposes three narrow commands: detect a live daemon from
  `serve-state.json`, check whether a Forge binary is installed, and start `forge serve --local`.
  No general filesystem or shell plugin is granted to the webview.

The shell also installs native window chrome and a standard macOS app/Edit menu. Windows and Linux
use the shared React title bar.

## Development

From `mobile/`:

```bash
npm ci
npm run check
npm run tauri:dev
```

The Tauri crate has focused Rust tests for daemon discovery and executable lookup:

```bash
cd src-tauri
cargo test
cargo clippy --all-targets -- -D warnings
```

## Release builds

`.github/workflows/app-desktop.yml` runs the mobile preflight and builds five desktop targets:

- Linux x86-64 and ARM64 (`.AppImage` updater + `.deb` installer)
- macOS Apple Silicon and Intel (`.app.tar.gz` updater + `.dmg` installer)
- Windows x86-64 (NSIS `.exe` updater/installer)

The OS installers are currently unsigned. Tauri updater artifacts are signed, and `latest.json` is
published only after all five platform artifacts and signatures exist. A protected-`main` dispatch
with an existing `v*`/`app-v*` tag checks out that exact source and publishes it; a dispatch without
`release_tag` is artifact-only.

Tray/fleet-glance behavior remains a possible follow-up; local daemon discovery and startup are
already shipped.
