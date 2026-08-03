# Retained desktop performance baseline

This baseline covers the retained Expo + react-native-web + Tauri client in `mobile/`. It excludes `desktop-native/`.

## Run identity

- Source commit for the instrumented app: `6dece827b4ee4dd449dfcd6717f2ec7d30962649`
- Capture commit (harness/docs): `4a1f80a5`
- Machine: x86_64 Linux workstation
- OS/kernel: Arch Linux, `7.1.3-arch2-2`
- Display strategy: display-dependent capture is deferred at the user's request. No display or compositor was queried in this update.
- Build: `npm ci`; `npx expo export -p web`; `npx tauri build --bundles appimage`

## Baseline capture status

The existing harness and export are verified. All display-dependent values below are pending: **requires a display; deferred at user's request**. No GUI process was launched and no compositor state was queried.

Measured packaging artifacts from the prior display-free release build:

- `Forge.AppDir` install footprint: **12 MB** (`du -sh`; uncompressed AppDir)
- bundled release executable: **11,678,824 bytes** (`stat`)
- compressed `.AppImage`: **pending** — the available cached linuxdeploy plugin is not executable in this environment (`linuxdeploy` exits 127 while invoking it; the cached plugin file is not a valid SquashFS/AppImage). The uncompressed AppDir and executable sizes above are the reproducible packaging artifacts.

Display-dependent measurements:

- cold/warm startup to interactive: **pending** — requires a display; deferred at user's request.
- composer application-event-to-paint, including IME composition: **pending** — requires a display; deferred at user's request. Synthetic application events would not measure end-to-end input and are not substituted.
- 10,000-row transcript scroll, dropped frames, and long tasks: **pending** — requires a display; deferred at user's request.
- streaming-turn main-thread long tasks: **pending** — requires a display; deferred at user's request.
- steady, peak, fixture, and long-session process-tree RSS: **pending** — requires a display; deferred at user's request.

No latency figure is presented as input-to-present. No display refresh-rate claim is made in this deferred capture.

## Real-display capture attempt (2026-08-03)

A real-display capture was attempted on HDMI-A-1/eDP-1 preference, but the retained Tauri development/release windows mapped on Hyprland monitor index 2 (DP-2) despite the requested placement override. No user window was moved or modified; the launched windows were terminated afterward. The instrumentation endpoint received no application snapshot beyond a connectivity probe, and the Diagnostics/fixture route could not be driven without interfering with the user's active workspace. Therefore every display-dependent figure remains **pending — capture did not produce an app measurement; no number is inferred**. The AppImage build produced the uncompressed AppDir but no compressed artifact before the bounded build command stopped; the existing cached linuxdeploy tool remains invalid as documented above.


The earlier 10-second idle capture reported **785,520 KiB** total RSS. Its process table attributes that total as follows:

| Process | RSS KiB | Share of total | Method |
| --- | ---: | ---: | --- |
| Tauri root `forge-desktop` | 318,028 | 40.5% | `ps` process-tree sample |
| `bwrap` helper | 1,988 | 0.3% | same sample |
| nested `bwrap` helper | 1,212 | 0.2% | same sample |
| `glycin-svg` helper | 19,148 | 2.4% | same sample |
| `WebKitNetworkProcess` | 153,356 | 19.5% | same sample |
| `WebKitWebProcess` | 291,788 | 37.2% | same sample |
| **Total** | **785,520** | **100%** | sum of listed processes |

This attribution is historical and workload-specific to that idle capture. Fixture, streaming, and long-session per-process values are **pending — requires a display; deferred at user's request**.

## Reproducible fixtures

`mobile/src/app/perf-fixture.tsx` is a deterministic debug-only route (`/perf-fixture`, available only in `__DEV__`). It renders exactly 10,000 stable rows through the retained `FlatList` settings and emits a local mock stream of 600 state updates at 50 ms intervals. The mock stream simulates token-arrival cadence and rendering pressure only; it does not simulate network, daemon, model, or authentication work. Display-dependent execution is deferred at the user's request. IME samples are tagged from browser composition commits and are unavailable on native surfaces that do not expose that event.

The interactive startup milestone is **the first animation frame after hydration completes**: `RootNavigator` waits for `AuthProvider.isLoading` to become false, schedules one `requestAnimationFrame`, then calls `markDesktopInteractive`. This excludes OS process creation and WebView/module loading before the collector starts, but includes the app's hydration gate and first post-hydration frame. Cold runs must clear the WebView profile/cache and terminate all Forge processes; warm runs retain the profile/cache and relaunch without clearing it.

- **Measured pure-logic benchmark (Node/Vitest, no display):** `groupByPhase` over 10,000 rows and 100 phases: **0.2746 ms/op** across 20 iterations; later reruns under variable host load measured **1.0950**, **0.4383**, and **0.3683 ms/op**. This does not establish rendered list cost; the hypothesis is **disproven for this pure grouping workload**.
- **Measured pure-logic benchmark (Node/Vitest, no display):** `parseReasoning` over a 560 kB streamed message: **0.0629 ms/op** across 20 iterations; later reruns under variable host load measured **0.1805**, **0.0718**, **0.0845 ms/op**. This does not establish Markdown/render cost; the hypothesis is **disproven for parsing alone**.
- **Measured pure-logic benchmark (Node/Vitest, no display):** `highlightTokens` over a 420 kB TypeScript block: **14.8464 ms/op** across 10 iterations; later reruns measured **13.7722**, **17.3641**, **18.4768**, and **17.5500 ms/op** under variable host load. A keyword-set cache attempt was re-measured at **17.3641 ms/op**, worse than the 14.8464 ms/op reference by **2.5177 ms/op (17.0%)**, so it was reverted and no optimization was retained. This is a measured pure-tokenization cost, but not a rendered text/layout cost.

The benchmark output is emitted by `mobile/src/performance/render.bench.test.ts`; it reports elapsed milliseconds per operation rather than pass/fail assertions.
- **HYPOTHESIS:** `mobile/src/app/session/[id]/index.tsx:500-760` rebuilds timeline items and dispatches `renderItem` work when streaming state changes; this may allocate or reconcile per streamed update. Unmeasured without rendering; no optimization applied.
- **HYPOTHESIS:** `mobile/src/components/chat/MessageRow.tsx:108-185` calls `parseReasoning` and renders `Markdown` while constructing each message row; large transcripts may repeat parsing/render allocation. Parsing alone was measured cheap above; Markdown/render allocation remains unmeasured without rendering.
- **HYPOTHESIS:** `mobile/src/app/perf-fixture.tsx:48-72` creates a 10,000-item data array and renders text rows through `FlatList`; grouping/windowing and text layout remain unmeasured without rendering.

## Phase 2 status

Phase 2 assessment remains intentionally blocked. The historical idle total is attributed above, but fixture, streaming, and long-session attribution is unavailable without display execution. No optimization is recorded because changing source against the idle total alone would be an unmeasured optimization. The next valid Phase 2 candidate must be selected from a completed workload-specific capture and re-measured with the same harness.
