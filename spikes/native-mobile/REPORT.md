# Native mobile spike: SwiftUI / Compose over a Rust (UniFFI) core vs the current Expo RN app

Branch `spike/native-mobile` (not for merge), code in `spikes/native-mobile/`. Raw numbers: `raw/final-speed.json`, `raw/final-reset.json` (and `raw/*naive*`, an earlier run of the first native version). Screenshots: `side-by-side-{fleet,session,streaming}.png` (+ the individual `native-*` / `rn-*`).

## Recommendation

**Do not rewrite the whole app natively now. The measured gains are real but they are on cheap screens; the cost is 20-28 weeks and the loss of OTA plus the web/desktop code sharing.** Two cheaper moves capture most of the benefit:

1. Move the protocol/state/crypto logic (`lib/ws`, `api`, reconciler, streaming accumulation, markdown parsing, Anywhere crypto/replay) into a Rust core now. The spike shows that works end to end (710 lines, 4 tests, bindings generated for Kotlin and Swift, Swift side verified on Linux). The core can be consumed by Tauri today and by RN via a native module, or by native UIs later.
2. If you want native feel, go native for the one surface where it matters most and RN is weakest, the streaming session screen (RN used 110-117% of a core while streaming vs 15-18% native), as a native view embedded in the RN shell, and keep everything else (settings, IDE, onboarding, Anywhere) in RN with OTA.

Revisit a full rewrite if: RN streaming cost cannot be fixed inside RN (try the same per-block lazy-item splitting and update coalescing first, the native spike gained from it), or OTA is no longer important, or web+desktop are dropped or moved to a shared Rust UI.

## Metrics (same emulator, same fixture data)

Emulator: AVD `spike_bench`, Android 13 (API 33, google_apis_playstore x86_64), Pixel 6 profile, 4 cores / 4 GB, **host GPU (RTX 4090, verified `GLES: ... NVIDIA` in SurfaceFlinger, 0 swiftshader maps)**, headless `-no-window`, own AVD home and port 5584, dark mode, 60 Hz. Both apps are release builds installed on it, both x86_64 only. Medians of 7 cold starts; other numbers single runs per condition. Two ART conditions were run (`reset` = as-installed JIT, `speed` = AOT compiled); they agree, so the table gives `speed` and notes the other where it differs.

| Metric | Native spike | Current RN app | Ratio |
|---|---|---|---|
| Cold start to fleet, `am start -W` TotalTime (median, range) | 297 ms (245-356) | 951 ms (912-1076) | 3.2x |
| Cold start straight into the 1000-msg session (RN via `forge://session/<id>`) | 305 ms (254-367) | 914 ms (826-1188) | 3.0x |
| Launch to first daemon request (fleet) | 330 ms | 928 ms | 2.8x |
| PSS, fleet idle (10 s settle) | 39.9 MB | 280 MB | 7.0x |
| PSS, session open (after 10 s) | 48.1 MB | 239.6 MB | 5.0x |
| PSS, after scrolling ~50 screens of the transcript | 56.1 MB | 334 MB | 6.0x |
| Native heap / Java heap after scroll | 18.5 / 8.7 MB | 119.9 / 27.0 MB | |
| Scroll (40 down + 10 up fast flings): frames, p50 / p90 / p99 frame ms | 1286, 16 / 16 / 16 | 1322, 17 / 18 / 19 | |
| Scroll janky % (gfxinfo) | **3.0%** (39 frames, all "slow issue draw") | **0.08%** (1 frame) | RN better |
| Streaming 50 tok/s for 14.5 s: CPU, % of one core (idle baseline 0.5 vs 2.9) | **17.6%** | **117%** | 6.7x |
| Streaming frame ms p50 / p90 / p99, janky % | 16 / 16 / 16, 20.9% | 16 / 31 / 65, 42.8% | native smoother tail |
| PSS growth during one 600-token stream | +1.6 MB | +80 MB (reset run: +134 MB) | |
| Idle CPU, % of one core | 0.4-0.5 | 2.9-3.0 | |
| APK size, x86_64 only, release | **4.6 MB** (3.3 MB core .so, 2.0 MB dex) | **65.7 MB** (28.4 MB .so, 22.9 MB compressed dex, 6.9 MB Hermes bundle) | 14x |
| Core `.so` arm64 vs x86_64 | 2.9 vs 3.2 MiB | n/a | |
| History load, 1000 rows (Rust paging + markdown parse), logcat | ~107 ms | not instrumented | |

`reset` condition differs only in noise (native cold fleet 302 ms vs RN 984 ms; scroll janky 3.2% vs 0.08%; stream CPU 15% vs 110%, janky 21% vs 34%).

Build times (this machine, no daemon):

| | Native spike | RN app |
|---|---|---|
| Dependencies | cargo deps + core first build, 2 ABIs + host + bindgen: 75 s; gradle deps cached | `npm@10 ci` 46 s |
| Native project generation | none (checked in) | `expo prebuild` 4 s |
| Release APK | gradle clean build 68 s; Rust core one ABI with LTO from touch 81 s; incremental Rust debug 10 s | gradle release 3 m 13 s (x86_64 only) |
| Hand-written lines for the two screens | Rust 710 + Kotlin ~410 (+ Swift ~410, not built) | see below |

Lines of code, RN equivalent (non-test, from the RN tree): `app/(tabs)/index.tsx` 376; `app/session/[id]/*` 2,896; `components/chat` 6,447; `components/fleet` 1,254; `components/session` 6,874; `theme` 1,381; `lib/ws.ts` 653; `lib/api.ts` 1,818; markdown stack 1,336. About 20k lines for the screens the spike covers at roughly 5% of their feature surface. **Not like for like**: the RN screens include tool rows, diffs, plans, queue, attachments, syntax highlighting, tabs, project chips, Respond/Peek actions, a tab bar, and so on. Compare the screenshots.

## Methodology

- **Data**: `fixture/server.py` is a deterministic stand-in for `forge serve` with the real v10 wire shape (`/api/sessions`, `/api/history` with `before`/`include_tools`, `/ws?session&rev` full snapshots with `streaming` tail + `streaming_offset`). 24 sessions (3 needs-you, 3 busy, 2 starred, 2 done), one with a 1,000-message transcript of markdown with headings, lists, links, inline code, quotes and code blocks. A GET `/bench/stream?tokens=600` makes it stream ~50 tokens/s (20 ms per full-snapshot frame) to connected clients; `forge serve --mock` (isolated HOME, port 39183) was only used to prove the Rust core against the real daemon (fleet, history, WS stream, send prompt, turn-end refresh all worked). The binary used was the already-built `~/.cache/forge-target/release/forge` (2.27.0), not a fresh build from the worktree.
- Both apps are driven identically by `scripts/bench_android.py`: force-stop both, wait 2.5 s, `am start -W`, `dumpsys meminfo` after a 10 s settle, `dumpsys gfxinfo <pkg> reset` then 40 down + 10 up `input swipe` flings (1200 px, 140 ms, 0.25 s apart) and read the gfxinfo summary, `/proc/<pid>/stat` utime+stime over the stream window.
- RN: Expo release build (Hermes bytecode, minified, not debuggable), debug-keystore signed, x86_64 only. The generated manifest needed `usesCleartextTraffic=true` to talk http to the fixture (untracked, gitignored `mobile/android`). Paired via the "paste a link" flow, session opened by the `forge://session/<id>` deep link. `mobile/` tracked files untouched.
- Native: R8 minified + resource shrinking, same signing, x86_64 only.

## Caveats (read before quoting numbers)

- **Emulator, one image, one machine, shared host load.** x86 emulation on a 4090 laptop is not a phone. Relative numbers are more trustworthy than absolute ones; real devices (especially mid-range ARM) will widen the cold-start and memory gaps, and jank behaves differently.
- **gfxinfo "janky" on this emulator mostly counts "slow issue draw commands"**, which is partly an emulator GPU-pipe artefact. It flatters RN on scroll (RN 0.08% vs native 3%) while the frame-time percentiles show the two as equal (16-19 ms p99). I would not conclude either list scrolls better. Percentiles are 1 ms-bucketed histograms from `dumpsys gfxinfo`, not per-frame traces.
- **The spike is not feature-equivalent.** No syntax highlighting, no tool/diff rows, no tabs, plain system fonts instead of the app's, no pull-to-load of older history (it loads all 1,000 rows up front, RN pages), no persistence, no accessibility pass beyond content descriptions, no error UI. RN's memory and startup include far more code and the whole Expo/Hermes runtime, which is the point, but a native app with the same features would grow (probably 2-4x in lines; memory and start would grow much less).
- RN paging means it renders fewer rows than native at any time; RN PSS grows while scrolling because pages load.
- Native was tuned once (blocks as separate lazy items, equality-skipping per block): scroll janky 4.0% -> 3.0%, streaming 26% -> 21%. RN was not tuned for this run.
- Cold start is `TotalTime` (first frame) plus time-to-first-data-request; there is no equal "content fully drawn" marker across the two. RN's first frame can be a splash, so the real RN time-to-content is at least the data-request figure.
- The streaming fixture sends full snapshots every 20 ms per client; real daemons send fewer when the model is slow. Fixture markdown differs from real model output.
- ART state: first native cold start after install was ~500 ms (JIT), settling to ~300 ms. Play installs would ship baseline profiles; this spike did not add one.
- A first round of numbers was taken with a fixture bug that corrupted the streamed markdown (unterminated fence); it was fixed and everything re-run; only the post-fix numbers are in the table (`raw/*naive*` is the pre-fix data, kept for transparency).

## iOS status

Code only; **not compiled on macOS and not built in Xcode Cloud.**

- Written: SwiftUI app (`ios/ForgeSpike/*.swift`, ~410 lines, same two screens), XcodeGen `ios/project.yml`, `scripts/build-ios-core.sh` (aarch64-apple-ios, aarch64-apple-ios-sim, x86_64-apple-ios static libs, lipo, Swift bindings, `xcodebuild -create-xcframework`).
- Verified on Linux with the real Swift 6.4 toolchain: the UniFFI-generated Swift bindings compile and run against the live fixture (`ios/LinuxSmoke/main.swift`: fleet, 1000-row history, markdown parse, stream updates, send prompt, turn-end items all work, async and callback interface included). The SwiftUI layer itself could not be type-checked here (no SwiftUI on Linux), so expect a few compile fixes on first build.
- Xcode Cloud: deliberately not attempted. Needing (1) a committed `.xcodeproj` (XcodeGen is not installed here and a hand-written pbxproj is error-prone), (2) a `ci_post_clone.sh` that installs Rust and runs the XCFramework script on the runner, (3) a new workflow created through the ASC API (the product `Forge APP` exists; Apple enforces `branchStartCondition` even for manual runs, and the memory notes show 409s and the PATCH dance), and (4) iterating against uncompiled SwiftUI at 10-15 min per cloud build on the owner's compute hours. That is more than the one-hour, low-risk bar. What it would take: a Mac (or one cloud run) to do `brew install xcodegen && scripts/build-ios-core.sh && (cd ios && xcodegen) && xcodebuild -scheme ForgeSpike -sdk iphonesimulator build`, then fix compile errors; roughly half a day including a simulator measurement pass.

## Assessment beyond the two screens

### Rust core vs per-platform UI

Hand-written lines in the spike (generated UniFFI bindings excluded: 3.3k lines Kotlin, similar Swift):

| Part | Lines |
|---|---|
| Rust core (`core/src`: HTTP+WS client, snapshot/streaming accumulation, history paging, fleet ordering, markdown to blocks, 4 unit tests) | 710 |
| Kotlin / Compose (fleet, session, markdown renderer, theme, view-model) | ~410 |
| Swift / SwiftUI (same screens, not compiled here) | ~410 |

So for this slice about 60% of the hand-written code is shared and the UI is ~410 lines per platform. That ratio flatters the core, because these two screens are protocol-heavy and UI-light. Screens like the IDE review and Settings are mostly UI, and the share would drop towards 30-40%.

What the current app keeps in TypeScript that a native rewrite would have to move or duplicate (non-test TS/TSX under `mobile/src`, 110.7k lines): `components/` 56.8k, `lib/` 35.9k (of which `lib/anywhere*` 6.9k: pairing, crypto, passkeys, sequence/replay state, jobs, push), `app/` 16.7k, `theme/` 1.4k. The protocol/state-machine half of `lib/` (ws, api, reconcilers, streamingText, markdown parsing, fleet rows) would move into the Rust core and is reusable. `components/` would be rewritten twice (SwiftUI and Compose).

### Remaining screens: porting effort (rough, solo dev + agents)

Sizes are today's RN source for the area; "effort" is my estimate of calendar time for one human steering AI agents, per platform unless noted. These are estimates, not measurements.

| Area | RN source today | Native effort | Notes |
|---|---|---|---|
| Session chat parity (tool rows, diffs, plan cards, task dock, queue, attachments, voice pill, find, jump) | `components/chat` 6.4k + `components/session` 6.9k | 3-4 wk | The spike covers about 5% of this surface. Syntax highlighting and math need native libs or a core-side tokenizer. |
| IDE / git / review (file tree, editor, diff review, PR, terminal) | `ide` 5.7k, `git` 3.7k, `review` 1.0k, `shell` 7.4k, `pr` 0.6k | 6-8 wk | Biggest block. Editor and terminal views are the risky parts (no Compose/SwiftUI equivalent of the web editor surface the desktop shares). |
| Settings, providers, models, MCP, hooks, plans, keybindings, diagnostics | `app/*.tsx` ~16k, `components/settings|mcp|skills|usage` | 3-4 wk | Mostly forms; schema-driven forms from the core would cut this. |
| Onboarding + pairing (QR scan, nearby hosts, cert pinning) | `onboarding` 1.6k, `pairing` 1.1k | 1-2 wk | Camera/QR and mDNS are first-class on native, easier than RN. Cert pinning is simpler natively (URLSession delegate / OkHttp CertificatePinner) than the RN path, which today only enforces it on desktop. |
| Anywhere (passkeys, E2E crypto, replay window, epoch refresh, push) | `lib/anywhere*` 6.9k + `components/anywhere` 1.0k | 4-6 wk | Crypto and sequence/replay logic should move into Rust (noble to RustCrypto): one implementation, testable on desktop CI. Passkeys use ASAuthorization / Credential Manager directly: better than RN wrappers, but two platform integrations. Highest correctness risk; needs the existing test vectors ported. |
| Voice (capture, upload, push-to-talk) | `VoiceRecordingPill` + expo-audio | 1 wk | AVAudioEngine / MediaRecorder are simpler than expo-audio. |
| Widgets, Live Activities, notification service extension | already native Swift: `mobile/targets` + `modules` 1.4k lines | 0 extra (iOS); Android widgets new | These are the one area where today's app is already native; they would just talk to the core instead of RN bridges. Android has none today. |
| Push (APNs/FCM registration, background handling) | `anywherePush*` | 1 wk | Native is the simpler path on both. |

Total: roughly 20-28 weeks of focused work to reach parity on both platforms, before polish and the long tail (the app has ~35 top-level settings-style routes, hosts, crews, plans, notes, floor, gallery...). The spike took one session for 2 screens x 1 platform measured plus 1 unbuilt.

### What would be lost

- **OTA updates.** Today JS-only changes ship via EAS Update in minutes (the repo has `eas-update.yml`, `mobile-ota-reconcile.yml`, fingerprint/runtime-version machinery, and a stack of memory notes about OTA/native boundary mistakes). A native app ships only through App Store review / Play (hours to days), and the daemon<->app protocol would need stricter back-compat since you cannot hot-fix a client. Store-side staged rollout and Play in-app updates partly compensate; there is no iOS equivalent of OTA. Note that it also removes a whole class of OTA/fingerprint mistakes.
- **Code sharing with web and Tauri desktop.** Today one `mobile/` codebase is six targets (iOS, Android, web, Tauri on Linux/macOS/Windows). A native rewrite makes iOS and Android two codebases, and web and desktop stay on RN-web/Tauri, so every UI feature is built three times (SwiftUI, Compose, RN-web) unless the desktop/web UI also changes. The Rust core would be shareable with the Tauri backend, which is a real upside, but the TS UI components are not.
- Expo ecosystem churn goes away (SDK upgrades, prebuild, npm overrides, lockfile fights), which is a cost today.

### Solo dev + AI agents: team-size implications

- Three UI stacks instead of one is the main multiplier. Agents are good at porting a screen once the design is fixed, but each port needs a visual and behavioural review on two OSes, and review is the scarce resource.
- iOS-only parts cannot be built or tested on this Linux machine at all; they need macOS (Xcode Cloud / a Mac runner) in the loop, which makes agent iteration slow (the memory notes record Xcode Cloud builds taking 10-15 min and 409s on branch rules). Android is fully local.
- The Rust core is a good fit for solo + agents: it is typed, testable on Linux CI, and one change lands on both apps. Pushing more logic there reduces the per-platform surface.
- Maintenance risk: two more native toolchains to keep green (Gradle/AGP/Compose, Xcode/Swift), UniFFI version pinning, XCFramework and `.so` build pipelines in CI.

### Platform feel and accessibility

- Measured and visible gains: ~3x faster cold start, 5-7x lower memory, and ~7x lower CPU while streaming, which translates to battery and thermals on a phone that sits open on a long-running agent session. Idle CPU is essentially zero.
- Compose/SwiftUI give system text rendering, selection, Dynamic Type / font scaling, TalkBack/VoiceOver semantics, predictive back, native context menus and haptics without bridges. The RN app already works on a11y (labels exist) but each of these is a custom component in RN.
- Not a gain: scroll smoothness was a wash on the emulator (see the table; gfxinfo jank favours RN, frame percentiles are equal); native does not automatically mean smoother, and large heterogeneous markdown rows need the same care (item splitting, caching) as in RN.
