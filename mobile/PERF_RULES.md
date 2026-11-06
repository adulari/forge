# Performance rules for `mobile/`

The rules the shared Expo app (desktop in Tauri, web, iOS, Android) follows to stay smooth on a
phone while a reply streams. Each one names the code that already does it, so a new screen can copy
the pattern instead of rediscovering the problem. Baselines and how they were measured live in
`docs/performance/desktop-baseline.md`; the capture tools are in `scripts/perf/`.

## Rendering

1. **Commit one state per paint.** The session socket can deliver about 33 snapshots a second
   while a reply streams. Anything that turns a wire frame into React state goes through
   `lib/frameCoalescer.ts` (newest value wins, one commit per animation frame) as `ws.ts` does. The
   streamed text is rebuilt from every frame before coalescing, so no text is lost.
   The frame coalescer is not enough on its own: a snapshot that differs only in the streamed
   text must not reach React state at all. `lib/sessionFrameSink.ts` keeps the previous snapshot
   object for such a frame (`lib/snapshotStable.ts`) and sends the reply text to `lib/streamStore.ts`
   instead, paced to 20 commits a second (`lib/streamPacer.ts`). Only `StreamingRow` reads it
   (`useStreamText`), so a token re-renders one row, not the session screen, the header and every
   visible message. A screen that needs the reply text subscribes to the store; it never reads
   `snapshot.streaming`, which is always empty on the committed snapshot.
2. **Lists are `BoundedList`.** Stable keys, a designed empty state, pagination through
   `onEndReached`. Never `.map()` an unbounded array inside a `ScrollView`.
3. **Rows are `memo`, and their props are primitives or stable references.** Build per-row
   handlers with `useCallback` over ids, not inline closures over the whole item (see
   `ProviderRow` in `app/usage.tsx`).
4. **Derive in `useMemo`, not in render.** Merging, grouping and sorting a list belongs behind a
   `useMemo` keyed on the query data (`buildProviderBlocks` callers).
5. **Keep high-frequency state local.** A value that changes 10 times a second (microphone
   level, scroll offset, a typing indicator) lives in the smallest component that draws it, as
   `VoiceRecordingPill` does, never in a screen or a context that wraps the transcript.
6. **Do not call `Date.now()` during render.** Time-dependent text takes its clock from the data
   it describes (`dataUpdatedAt`) or from a ticking hook such as `RelativeTime`.

## Network and storage

7. **One socket per session, reconnect with jitter.** `lib/wsBackoff.ts` spreads retries by
   +-25% so a restarted daemon is not hit by every client in the same instant.
8. **Cap what you keep.** Queues and logs have a maximum length (`pushOutbox.ts` keeps 20
   requests), and a persisted value that fails to parse reads as empty instead of throwing
   (`serverStoreJournal.ts`).
9. **Storage failures never break rendering.** Every `localStorage`, `AsyncStorage` and
   `SecureStore` read and write is wrapped, and the screen works without it.

## Animation and layout

10. **Animate transform and opacity only,** on the UI thread through Reanimated, and respect
    `useReducedMotion`.
11. **Avoid layout work on scroll.** No `onLayout` that sets state on every scroll tick; measure
    once and on resize.
12. **Heavy modules load on demand.** Code only some replies need (KaTeX for math, the Tauri
    plugins) is a dynamic `import()` so it stays out of the startup bundle.

13. **Parse what changed, not the whole reply.** A streamed reply grows at its end;
    `Markdown incremental` (`lib/markdownIncremental.ts`) keeps the blocks before the last blank
    line outside a code fence and parses only the open part, and finished blocks keep their element.

## Checking a change

- A change to the streaming path is also measured on a phone-class target:
  `scripts/perf/android/fixture_server.py` streams a 600-token reply at 50 tok/s over the daemon's
  wire shape and `scripts/perf/android/bench_android.py` records CPU per thread, frame percentiles
  and memory from a release APK. Interleave the runs of the two builds; emulator CPU moves with
  host load.
- A change to the transcript, composer or socket path gets measured, not guessed: build a clean
  web export (`node scripts/export-web-clean.mjs`), then drive the scenarios in
  `scripts/perf/` against the desktop build and compare runs with `scripts/perf/compare.py`.
- Check at phone width as well as desktop width; a layout that fits only wide is a regression.
