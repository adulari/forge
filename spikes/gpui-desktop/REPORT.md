# GPUI desktop spike vs current Forge Desktop (Tauri v2 + Expo/react-native-web)

Branch `spike/gpui-desktop` (not for merge), code in `spikes/gpui-desktop/` (own cargo workspace and Cargo.lock, main lockfile untouched).
Baseline = `mobile/` at origin/main `dce0aa55b` (Forge Desktop 2.27.0), release build (`tauri build --no-bundle`).
Screenshots: this directory (`side-by-side-*.png`, plus per-app shots). Harness: `spikes/gpui-desktop/harness/`.

## Recommendation

**Do not port Forge Desktop to GPUI now. Keep the spike as a reference and re-evaluate if a concrete pain (WebKitGTK memory, Linux scroll jank) becomes the top desktop complaint.**

What the spike shows: GPUI is real and fast to *build screens in* (the two reference screens took ~760 lines of Rust, because `gpui-component` ships a virtualized diff view with tree-sitter highlighting, a chat message scroller, markdown rendering and a text area). Cold start (~0.32 s vs ~1.0 s) and memory (~0.35 GB vs ~1.0-1.1 GB) are 3x better, and composer keystroke-to-paint is ~2x lower. Scroll-frame data is inconclusive for GPUI (see caveats). Against that: the port would fork desktop from the mobile/web code base (the app is one codebase, six targets), drop OTA updates, and put Forge on a pre-1.0 UI stack whose API moves weekly and whose published crates are re-packaged Zed snapshots (`gpui-pre 0.3.11`). The remaining 90% of the UI (settings, onboarding, Anywhere pairing, palette, terminal, the full IDE) is the real cost, not the two screens.

If memory/start-up is the motivation, cheaper levers exist inside the current stack (the web build also paints a 4k-line diff in ~75 ms; the weight is WebKit's three processes).

## Metrics (3 rounds each, median and range; Xvfb :87, 1440x900, window focused)

| Metric | Tauri + RN-web (current) | GPUI spike | Note |
|---|---|---|---|
| Cold start to first meaningful paint (agent list pixels, 15 launches) | 1029 ms (797-1223) | 323 ms (298-349) | page cache warm, process spawn to painted rail, black-box pixel probe |
| Idle RSS, IDE view (process tree) | 985 MiB (972-988) | 304 MiB | web = app + WebKitWeb/Network processes |
| RSS after opening 4k-line diff | 1028 MiB | 338 MiB | |
| RSS after opening 1,000-msg transcript | 1131 MiB (1113-1156) | 345 MiB | |
| RSS after scrolling transcript | 1124 MiB | 350 MiB | |
| Open diff (input to code rows painted) | 74 ms (74-77) | 104 ms (100-117) | GPUI: 22 ms HTTP fetch + 1 ms parse + frame pacing |
| Diff scroll, frame gap p50 / p95 / max | 25 / 73 / 102 ms | 32 / 32 / 32-48 ms | **GPUI value is a harness pacing floor, not render cost** (below) |
| Diff scroll, dropped frames (>25 ms gap) per ~3.4 s | 53 of 110 | 93 of 94 (pacing) | |
| Diff scroll CPU (process tree, 3.4 s burst) | ~4.0 s | ~2.1 s | GPUI figure includes driver spin in present |
| Transcript scroll, frame gap p50 / p95 / max | 19 / 46 / 61 ms | 32 / 32 / 272-304 ms | GPUI max = first-time layout of unmeasured markdown rows (2 gaps >100 ms per burst) |
| Single wheel notch to paint, diff | 31 ms (p95 34-44) | 30 ms (p95 37-43) | black-box, region hash change |
| Single wheel notch to paint, transcript | 48 ms (p95 51) | 30 ms (p95 36) | |
| Composer keystroke to paint | 50 ms p50 / 52 p95 | 26 ms p50 / 30 p95 | black-box, 40 keys, glyph-pixel increase |
| Streaming 600 tokens (web: real daemon mock turns, 20 ms/word; GPUI: synthetic 50/s) | frame p50 17, p95 27, max 73 ms; CPU 9.4 s / 14.2 s | 600 appends in 18.7 s (loop paced to ~32/s); per-append state cost 0.01 ms; CPU 11 s / 18.7 s | not identical workloads, see caveats |
| Binary size (stripped release) | 13.4 MB (+ system WebKitGTK ~ tens of MB, not bundled) | 29.8 MB fat-LTO (38 MB no-LTO) | GPUI includes tree-sitter grammars, fonts, Vulkan loaders at runtime from system |
| Clean build | 293 s Tauri (+ `npm ci` + Expo export, warm cargo registry) | 298 s release (fat LTO), 162 s no-LTO | measured with other builds idle |
| Incremental build | 114 s (Rust only), 104 s (touch a TSX; export + Rust) | 144 s (fat LTO), **2 s** (lto=off, cgu=16, spike crate only) | Rust crate is tiny; LTO dominates. Tauri has no fast dev profile equivalent here; JS dev uses Metro HMR (not measured) |
| Source lines (non-blank, non-comment) | RN equivalent ~2,700 (IDE review: AgentsList, ChangeQueue, DiffView, shell, models, hooks, git/diffModel) + ~2,400 (transcript, markdown, code block, streaming, composer, tool rows; composer alone ~1,180) | 762 total Rust (ide 227, chat 162, data 125, main 147, theme 48, bench 53) | not equivalent in features: no resize handles, tabs, review actions, mentions, voice, tool rows, Why-trace, editor |

Raw per-round JSON and logs: `/home/floris/.cache/forge-spike-gpui/results/` (not committed).

## Methodology

- Data: isolated mock daemon (`forge-bin-c` 2.26.4 `serve --local --mock`, port 7611, HOME/XDG under `~/.cache/forge-spike-gpui/daemon`, `FORGE_MOCK_STREAM_DELAY_MS=20`). Fixture repo (copy of the earlier harness repo + a 3,920-line `engine.rs` rewritten to a 5,020-line unified diff). Both apps read the same daemon: `/api/sessions`, `/api/projects/changes`, `/api/git/diff`, `/api/history` (1,000-message session of markdown, lists and rust/ts/json/bash code blocks). The user's real daemon/store/screen were never touched.
- The daemon caps a diff response at 4,000 lines (the RN app shows "886 more changed lines are not shown"); both apps therefore render the same 4,000 lines + 130 hunk headers (not 5,000). The spike repairs the last cut hunk header.
- Baseline launched via `dbus-run-session`, `GDK_BACKEND=x11`, `WEBKIT_DISABLE_DMABUF_RENDERER=1` (the existing harness recipe), WebKit inspector for DOM clicks, rAF recorder, screenshots. GPUI launched directly.
- All input is real X11 events (XTest via libXtst/ctypes). Black-box probes read pixels from Xvfb with XGetImage: cold start (first 40 bright pixels in the agent-list region), open-diff (400 bright pixels in the editor region), notch latency (region CRC change), typing (bright-pixel count increases by a glyph).
- Frame gaps: web = `requestAnimationFrame` gaps during a 120-notch wheel burst (25 ms spacing); GPUI = timestamp of each root `render()` during the same burst. CPU = utime+stime of the process tree. RSS = VmRSS summed over the tree.
- Both windows were given X input focus (`XSetInputFocus`) because there is no window manager on the private Xvfb; this changed GPUI's cadence from 48 ms to 32 ms (see caveats).

## Caveats (read before trusting a number)

1. **GPUI frame times are pacing-limited by this harness.** With a trivial scene (no diff open, continuous redraw) GPUI's X11 loop still delivers 48 ms frames unfocused and 32 ms focused, on the NVIDIA Vulkan device and also on software Lavapipe (extracted from Arch `vulkan-swrast`, llvmpipe 256-bit). So the 32 ms rows say nothing about renderer speed. Per-notch latency (30 ms = ~2 ticks) is similarly floored. A real compositor session (Wayland or X11 with a WM/vblank) is needed for real frame times; I did not do it because I may not open windows on the Hyprland display. Treat GPUI scroll/stream frame numbers as "no evidence of a problem, no evidence of a win", except the 270-300 ms transcript hitches, which are real layout cost of newly measured markdown rows.
2. **Different GPU paths.** GPUI used the RTX 4090 via Vulkan (`gpu: NVIDIA ... software=false`, X11 present without DRI3). WebKitGTK ran with the dmabuf renderer disabled (software compositing under Xvfb). On a real desktop WebKit would be GPU-composited and likely smoother than the 25/73 ms scroll figures here, so the scroll comparison favours GPUI in a way it might not in the field.
3. Web idle RSS rose from ~700 MiB (unfocused window, earlier runs) to ~985 MiB once the window had input focus; the table uses the focused runs. Treat web RSS as ~0.7-1.1 GiB depending on window state.
4. Streaming is not apples to apples: the web path is the real daemon websocket (the daemon coalesces to ~6 snapshots/s even at 50 words/s), the GPUI path is a synthetic in-process append of 600 words with markdown re-parse per tick. GPUI's append loop only reached ~32/s because timers are serviced on the pacing-limited main loop.
5. Diff colouring/markdown come from `gpui-component` (tree-sitter); RN uses its own tokenizer. Highlight quality and cost differ.
6. Cold start is process-spawn to painted rail with warm page cache; no disk-cold runs. The first GPUI launch after rebuilding was 2.2 s (cache cold).
7. Only X11 was exercised. No Wayland, macOS or Windows run.
8. The spike is read-only: no review actions, no websocket, no tabs, no resize panes, no command palette, no persistence.

## Qualitative assessment

- **Text input / IME**: `gpui-component` Textarea/Input/Editor handle selection, undo, copy/paste, soft wrap and auto-grow; typed text worked end to end. Platform IME plumbing exists in gpui-linux (XIM with preedit on X11, `zwp_text_input` on Wayland, from reading the crate source); I did not test composition, so CJK/dead-key quality on Linux is unverified. WebKit gives this for free and is mature.
- **Selection/copy**: Diff and markdown `TextView` ship their own selection layers (copy tested only by reading code, not exercised). Cross-message selection across a virtualized transcript is something browsers do natively; expect gaps.
- **Accessibility**: AccessKit is wired in gpui (accesskit_unix, window.rs) and gpui-component claims roles/labels; untested here. The web build gets screen-reader semantics from ARIA/react-native-web; a native rewrite would need an audit.
- **Theming**: straightforward. Machined tokens mapped onto the component `Theme` in 48 lines (`theme.rs`); the Geist fonts load via `include_bytes!`. Light theme and the existing token module would need a Rust port of `tokens.ts`.
- **Remaining screens**: settings (forms, many panels, MCP/skills/providers pages) is large but `gpui-component` has Form/Setting/Select/Switch; onboarding is an 11-step wizard driven by the daemon API (moderate); Anywhere pairing needs QR rendering and a relay client (moderate); command palette has `Command` component (small); **terminal** has no ready GPUI component (Zed's terminal is its own crate, not a published widget): high risk; plus ide pane docking, review actions, hunk staging, WhyTrace gutter, voice, push, deep links, auto-update, tray, single-instance (currently Tauri plugins).
- **API churn risk**: high. gpui-component 0.9.0 (Nov 2) pins `gpui-pre =0.3.11`; releases weekly (0.7.3 Oct 12, 0.8.0 Oct 19, 0.9.0 Nov 2) and its own README says any snapshot may change GPUI's API. crates.io `gpui` itself is stale (0.2.2, Oct 2025). The spike pins `gpui-kit =0.9.0` exactly; upgrades will be manual porting work every few weeks or a permanent pin.
- **Platforms**: Zed documents Linux (Vulkan required; X11 and Wayland; NVIDIA problems are triaged per-driver via `ERROR_INITIALIZATION_FAILED`/`ERROR_SURFACE_LOST_KHR` issues, no NVIDIA section), macOS (Metal) and Windows are first-class for Zed itself. Forge Desktop ships today on all three via Tauri+WebView2/WKWebView/WebKitGTK; a GPUI port needs a Vulkan-capable GPU or software fallback, and there is no WebKitGTK dependency to fight (that is a plus for Linux packaging and the Hyprland/NVIDIA dmabuf workaround).
- **Loss of shared code and OTA**: desktop today shares the full React Native tree with iOS/Android/web (one design system, one query layer, Expo OTA updates). A GPUI desktop would be a second UI implementation of every feature (the repo's "every feature gets a UI on every surface" rule means each feature ships twice more), the Rust client would need its own copy of `lib/api.ts` types and websocket/session state machine, and Expo OTA is not available (only Tauri updater / new binaries).

## Gaps

No real-display frame times, no Wayland/NVIDIA-real-driver run, no IME/AT-SPI/copy testing, no full-suite Lavapipe run (only the cadence probe), streaming workloads differ, only 3 rounds per app.
