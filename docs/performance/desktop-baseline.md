# Retained desktop performance baseline

This baseline covers the retained Expo + react-native-web + Tauri client in `mobile/`. It excludes `desktop-native/`.

## Run identity

- Source commit for the instrumented app: `6dece827b4ee4dd449dfcd6717f2ec7d30962649`
- Capture commit (harness/docs): `4a1f80a5`
- Machine: x86_64 Linux workstation
- OS/kernel: Arch Linux, `7.1.3-arch2-2`
- Display strategy: real Hyprland display via `DISPLAY=:1`, `WAYLAND_DISPLAY=wayland-1`; monitor placement and refresh are recorded below from `hyprctl monitors -j` for every launch capture.
- Build: `npm ci`; `npx expo export -p web`; `npx tauri build --bundles appimage`

## Baseline capture status

The harness and export are verified. Real-display launch captures were run on **DP-2**, Hyprland monitor index 2, 1920×1080 at **143.99899 Hz**, scale 1.0. The window landed on the requested headline monitor in all three captures. The helper measured a mapped Tauri window, not application interactivity; the Diagnostics surface was not reached through an automated route/input fixture, so those values remain pending rather than being inferred.

Measured bundle artifacts from the release build:

- `Forge.AppDir` install footprint: **12 MB** (`du -sh`; uncompressed AppDir)
- bundled release executable: **11,678,824 bytes** (`stat`)
- compressed `.AppImage`: **pending** — Tauri built the release executable and AppDir but `linuxdeploy` failed before producing the final AppImage.

Measured launch and process-tree captures on DP-2:

- cold process launch to mapped window: **446.778 ms**, `scripts/perf/capture-tauri-process.sh ... cold`; this is process-start → Hyprland client mapping, not interactive readiness.
- warm process launch to mapped window: **431.435 ms**, same helper after a prior launch; this is not a cache-controlled warm-start protocol and remains a mapped-window figure only.
- idle process-tree RSS snapshot: **785,520 KiB (~767.1 MiB)** at the 10-second capture; root, bwrap/glycin-svg helper, WebKitNetworkProcess, and WebKitWebProcess included.
- peak process-tree RSS during the 10-second idle sample: **785,520 KiB (~767.1 MiB)**; no user session or streaming turn was exercised.
- RSS after a long session: **pending** — no authenticated/connected long-session fixture was used, so there is no honest long-session measurement.

Still pending:

- cold/warm **interactive** startup: requires opening the app's Diagnostics route and defining the interactive milestone; mapped-window timing above is not substituted.
- composer input-to-paint, including IME composition: requires controlled real-display input automation and a composition-capable IME fixture.
- 10,000-row transcript scroll, dropped frames, and long tasks: requires a deterministic 10k-row fixture in the retained app; no fixture was available and no source estimate is reported.
- streaming-turn main-thread long tasks: requires a connected streaming turn fixture; the 10-second idle sample is not substituted.

No latency figure above is presented as input-to-present. The only display-dependent launch captures are explicitly tied to DP-2 at 143.99899 Hz. The harness records the full Hyprland monitor/client JSON before each capture, and its process cleanup only terminates the process it launched.
