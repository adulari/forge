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

The prior fixture and AppImage captures are **unverified**: the exact exported bundle and its build-time environment were not recorded, and the mixed constant-folded gates prove those artifacts cannot be attributed safely. This includes the reported AppImage `907 ms` launch-to-window-map figure and all fixture phase figures. The prior "normal route" measurements were specifically the unpaired `/connect` screen, not an
empty shell. They are superseded and must not be compared with connected app figures.


Measured packaging artifacts from the prior display-free release build:

- `Forge.AppDir` install footprint: **12 MB** (`du -sh`; uncompressed AppDir)
- bundled release executable: **11,678,824 bytes** (`stat`)
- compressed `.AppImage`: **pending** — the available cached linuxdeploy plugin is not executable in this environment (`linuxdeploy` exits 127 while invoking it; the cached plugin file is not a valid SquashFS/AppImage). The uncompressed AppDir and executable sizes above are the reproducible packaging artifacts.

Display-dependent measurements:

- cold/warm startup to interactive: historical entries below are **JS-clock-relative**, not
process-start-relative. The native-prefix-corrected headline is recorded in the verified section
below.
- composer application-event-to-paint, including IME composition: **pending** — requires a display; deferred at user's request. Synthetic application events would not measure end-to-end input and are not substituted.
- 10,000-row transcript scroll, dropped frames, and long tasks: **pending** — requires a display; deferred at user's request.
- streaming-turn main-thread long tasks: **pending** — requires a display; deferred at user's request.
- steady, peak, fixture, and long-session process-tree RSS: **pending** — requires a display; deferred at user's request.

No latency figure is presented as input-to-present. No display refresh-rate claim is made in this deferred capture.

## Real-display capture attempt (2026-08-03)

A real-display capture was attempted with `DISPLAY=:1`, `GDK_BACKEND=x11`, `WEBKIT_DISABLE_DMABUF_RENDERER=1`, and no `WAYLAND_DISPLAY`. Hyprland reported HDMI-A-1 at 1920×1080/60 Hz, eDP-1 at 60.012 Hz scale 2, and DP-2 at 143.99899 Hz. Despite the active runtime rule, the launched Forge windows mapped to DP-2 (monitor index 2), so the attempt was stopped without driving input or moving any window. No user window was moved or modified; the launched Forge processes and Expo server were terminated afterward.

The instrumentation endpoint received no valid application snapshot from the attempted runs; the only line was a connectivity probe. Consequently there is **no measured startup, composer, IME, scroll, streaming, or workload RSS value from this attempt**. All display-dependent figures remain **pending — capture did not produce an app measurement; no number is inferred**. This is not a valid HDMI-A-1 measurement and must not be used as one.

A release `npx tauri build --bundles appimage` attempt again produced the uncompressed AppDir but no compressed AppImage within the bounded run. The cached linuxdeploy/plugin path remains invalid as previously documented; compressed artifact size and packaged first-run startup remain pending.

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

## Connected app-surface capture attempt (invalidated)

The earlier attempted capture used a bundle whose provenance is now unverified and emitted no
snapshot. It is superseded by the clean-export/runtime-gate fix below; no connected numbers are
claimed from it.

## Verified paired connected app-surface baseline (clean artifact, 2026-08-04)

The clean release binary was launched with runtime `FORGE_PERF_SEED_SERVER=1`, which reads the
local daemon state and seeds the exact `forge.servers` / `forge.activeServerId` SecureStore records.
The live daemon was available at `127.0.0.1:7420` with one session. No credential is recorded in
this ledger or emitted into the perf JSON.

The 1280×800 raw frame events show the key timing precisely: the `intervalMs: 241` frame ends at
`atMs: 481`, while interactive is marked at **483 ms**. This is a single first-frame-after-
hydration stall, after which the capture returns to a steady approximately 16 ms cadence. The
previous one-shot result is not used as the headline.


## Paired connected app-surface attribution sweep

At least seven fresh paired launches were run per size against the clean artifact. No launch was
discarded; every run emitted a perf JSON. Medians and observed min–max ranges are:

The previous paired sweep values are retained but explicitly labeled **JS-clock-relative**:

| Logical capture size | JS-clock startup median (range) | Max-frame median (range) |
| --- | ---: | ---: |
| 640×480 | **614 ms** (402–659) | **172 ms** (95–230) |
| 1280×800 | **399 ms** (383–418) | **133 ms** (98–143) |
| 1890×1114 | **411 ms** (378–448) | **118 ms** (105–160) |

The startup ranges overlap across 1280×800 and 1890×1114, and max-frame ranges overlap across
all sizes. The 640×480 startup range overlaps 1280×800 as well. Therefore raster cost is **not
demonstrated** by this sweep; n=1 claims are withdrawn.



The instrumentation decomposition is now recorded in each snapshot under `hydration`: module
evaluation end, React mount start/end, first `/api/sessions` resolve, and the first post-hydration
paint. These marks separate JavaScript mount, daemon data, and paint timing rather than inferring
the cause from the aggregate frame interval.

## Lucide bundle optimization investigation

The clean source-map export measured the Lucide barrel at approximately **585,934 generated bytes**
inside a **5,589,476-byte** entry bundle (10.5%), the largest attributed contributor. The proposed
reversible Babel rewrite was tested against `lucide-react-native` 1.23.0.

Metro does not permit the deep imports in a production export: although it falls back to files with
warnings for some paths, the export fails on `lucide-react-native/dist/esm/icons/settings2.mjs`
because that subpath is absent from the package `exports` map. The package exposes only `.` and
`./icons`; the per-icon files are not public subpaths. The transform was removed and no source
imports were changed.

Consequently no before/after startup comparison or icon-rendering claim is made. The optimization
is blocked by the package export boundary and requires an upstream export-map change or a maintained
local wrapper/package; this task did not bypass the resolver.


The native `Instant` captured at the top of `main()` is now aligned with the JS snapshot. The
honest headline is **median process-start → first post-hydration paint: 995 ms**, observed range
**926–1,249 ms** across seven paired 1280×800 launches. This supersedes the JS-clock-relative
**399 ms** headline; that figure remains in the ledger as JS-relative and omits the native prefix.

The current per-stage native medians are:

| Mark | Median | Range |
| --- | ---: | ---: |
| Tauri builder start | 0.012 ms | 0.010–0.016 |
| Notification plugin | 0.062 ms | 0.059–0.087 |
| Opener plugin | 0.067 ms | 0.065–0.092 |
| HTTP plugin | 0.069 ms | 0.065–0.092 |
| Dialog plugin | 0.071 ms | 0.068–0.096 |
| WebSocket plugin | 0.072 ms | 0.068–0.097 |
| Updater plugin registration | 0.072 ms | 0.069–0.097 |
| Process plugin | 0.074 ms | 0.071–0.100 |
| Builder `.build()` finished | 50 ms | 45–120 |
| Window available/created | 318 ms | 256–544 |
| WebView navigation start | 414 ms | 354–678 |
| DOM/content-loaded | 582 ms | 508–850 |

The plugin registrations are not the source of the 305 ms-class delay: each is sub-millisecond.
The updater check also cannot be blocking before window creation. `checkDesktopUpdate()` runs in
`RootLayout`'s React effect, after the WebView has loaded; the updater plugin registration itself
is ~0.072 ms and performs no network check. The remaining pre-window interval is Tauri/GTK runtime
and window/WebView construction between builder completion and the window mark (roughly **268 ms
median** in this series). The post-window segments are navigation startup and bundle load/parse.

The seven-run sweep was repeated with the hydration marks. Offsets are from the performance
module's process-start clock; gaps are consecutive mark differences:

| Size | Module eval end | Mount start | Mount end | First sessions resolve | First post-hydration paint | Consecutive gaps (ms) |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 640×480 | 0 | 238 | 290 | 421 | 373 | 238, 52, 131, **-48** |
| 1280×800 | 0 | 231 | 259 | 416 | 355 | 231, 28, 157, **-61** |
| 1890×1114 | 0 | 204 | 232 | 403 | 344 | 204, 28, 171, **-59** |

The negative final gap is decisive: first post-hydration paint occurs before the first sessions
response, not after it. The largest positive post-mount interval is mount-end → sessions resolve
(131–171 ms), but it does not block first paint. The data therefore do **not** support the
hypothesis that the app waits for `/api/sessions` before painting. The mount work itself is short
(28–52 ms), with no long tasks, and the remaining pre-mount delay is outside these marks. Raster
cost is still not demonstrated by the overlapping size ranges.

The module-evaluation mark is necessarily zero because the instrumentation module establishes the
clock and records its own end at module evaluation. It is a process-start anchor, not an
independent JS-evaluation duration.


Perf routing and fixture activation now query separate Tauri commands. `perf_enabled` checks the
running process's `FORGE_PERF_OUT` and enables snapshot dumping; `perf_fixture_enabled` checks the
explicit runtime `FORGE_PERF_FIXTURE=1` flag and is the only condition that redirects to the
fixture. The normal connected UI can therefore be measured with `FORGE_PERF_OUT` without being
redirected.
`mobile/scripts/export-web-clean.mjs` removes `dist`, `.expo`, and Metro cache before exporting,
then asserts both runtime gates are present and the old build-time variable is absent from emitted
JS/HTML. The Tauri build invokes this script.


The seven-run paired captures and hydration decomposition above are the current connected results;
this section no longer treats them as unmeasured. The earlier `/tmp` files remain un-attributed and
are not used.

`mobile/src/app/perf-fixture.tsx` is a deterministic debug-only route (`/perf-fixture`, available only in `__DEV__`). It renders exactly 10,000 stable rows through the retained `FlatList` settings and emits a local mock stream of 600 state updates at 50 ms intervals. The mock stream simulates token-arrival cadence and rendering pressure only; it does not simulate network, daemon, model, or authentication work. Display-dependent execution is deferred at the user's request. IME samples are tagged from browser composition commits and are unavailable on native surfaces that do not expose that event.

The interactive startup milestone is **the first animation frame after hydration completes**: `RootNavigator` waits for `AuthProvider.isLoading` to become false, schedules one `requestAnimationFrame`, then calls `markDesktopInteractive`. This excludes OS process creation and WebView/module loading before the collector starts, but includes the app's hydration gate and first post-hydration frame. Cold runs must clear the WebView profile/cache and terminate all Forge processes; warm runs retain the profile/cache and relaunch without clearing it.

- **Measured pure-logic benchmark (Node/Vitest, no display):** `groupByPhase` over 10,000 rows and 100 phases: **0.2746 ms/op** across 20 iterations; later reruns under variable host load measured **1.0950**, **0.4383**, and **0.3683 ms/op**. This does not establish rendered list cost; the hypothesis is **disproven for this pure grouping workload**.
- **Measured pure-logic benchmark (Node/Vitest, no display):** `parseReasoning` over a 560 kB streamed message: **0.0629 ms/op** across 20 iterations; later reruns under variable host load measured **0.1805**, **0.0718**, **0.0845 ms/op**. This does not establish Markdown/render cost; the hypothesis is **disproven for parsing alone**.
- **Measured pure-logic benchmark (Node/Vitest, no display):** `highlightTokens` over a 420 kB TypeScript block: **14.8464 ms/op** across 10 iterations; later reruns measured **13.7722**, **17.3641**, **18.4768**, and **17.5500 ms/op** under variable host load. A keyword-set cache attempt was re-measured at **17.3641 ms/op**, worse than the 14.8464 ms/op reference by **2.5177 ms/op (17.0%)**, so it was reverted and no optimization was retained. This is a measured pure-tokenization cost, but not a rendered text/layout cost.

The benchmark output is emitted by `mobile/src/performance/render.bench.test.ts`; it reports elapsed milliseconds per operation rather than pass/fail assertions.
- **HYPOTHESIS:** `mobile/src/app/session/[id]/index.tsx:500-760` rebuilds timeline items and dispatches `renderItem` work when streaming state changes; this may allocate or reconcile per streamed update. Unmeasured without rendering; no optimization applied.
- **HYPOTHESIS:** `mobile/src/components/chat/MessageRow.tsx:108-185` calls `parseReasoning` and renders `Markdown` while constructing each message row; large transcripts may repeat parsing/render allocation. Parsing alone was measured cheap above; Markdown/render allocation remains unmeasured without rendering.
- **HYPOTHESIS:** `mobile/src/app/perf-fixture.tsx:48-72` creates a 10,000-item data array and renders text rows through `FlatList`; grouping/windowing and text layout remain unmeasured without rendering.

## Parity guard live-PR evidence

The parity guard caught undeclared platform drift on live PR #954: the desktop-only performance
fixture and instrumentation (`mobile/src/app/perf-fixture.tsx` and `mobile/src/lib/performance.ts`)
were rejected until explicitly inventoried as behavioral desktop boundaries. This is stronger
than the synthetic negative test because the guard detected a real unreviewed change in the PR.

## Phase 2 status

Phase 2 is **not measured**. The shell already paints before `/api/sessions` resolves, so a
paint-before-data change would not target the observed stall. The dominant interval is before
React mount starts; an independent native window/WebView startup boundary is required before
choosing a code change.
