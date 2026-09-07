import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  getDesktopPerformanceSnapshot,
  recordComposerInput,
  resetDesktopPerformanceSamples,
  startDesktopPerformanceMonitor,
  stopDesktopPerformanceMonitor,
} from "./performance";

vi.mock("./platform", () => ({ isTauri: false }));

// Minimal deterministic rAF harness: `flushFrame` drains whatever callbacks are currently
// queued (advancing the stubbed clock by `deltaMs` first), and any rAF calls made from within
// those callbacks land in a fresh queue for the *next* flush — matching real rAF's one-frame-
// later scheduling semantics closely enough for this module's single-callback chains.
type QueueEntry = { handle: number; cb: FrameRequestCallback };
let frameQueue: QueueEntry[] = [];
let handleCounter = 0;
let currentTime = 0;
let canceled = new Set<number>();

function flushFrame(deltaMs: number): void {
  currentTime += deltaMs;
  const due = frameQueue;
  frameQueue = [];
  for (const { handle, cb } of due) {
    if (canceled.has(handle)) continue;
    cb(currentTime);
  }
}

beforeEach(() => {
  frameQueue = [];
  handleCounter = 0;
  currentTime = 0;
  canceled = new Set();
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback): number => {
    const handle = ++handleCounter;
    frameQueue.push({ handle, cb });
    return handle;
  });
  vi.stubGlobal("cancelAnimationFrame", (handle: number): void => {
    canceled.add(handle);
  });
});

afterEach(() => {
  stopDesktopPerformanceMonitor();
  resetDesktopPerformanceSamples();
  vi.unstubAllGlobals();
});

describe("desktop performance monitor", () => {
  it("keeps the frame-interval ring buffer bounded instead of growing (and sorting) forever", () => {
    startDesktopPerformanceMonitor();
    flushFrame(0); // first callback only establishes lastFrameAt — no interval yet
    flushFrame(99_999); // one huge stall, pushed as the very first ring-buffer entry
    for (let i = 0; i < 4998; i++) flushFrame(16.67);

    const snapshot = getDesktopPerformanceSnapshot();
    expect(snapshot.frameSamples).toBe(4999); // cumulative counter — never trimmed
    expect(snapshot.droppedFrames).toBeGreaterThan(0);
    // If the ring buffer weren't bounded, the 99999ms stall would still be sitting in the
    // window used for frameTimeMaxMs after 5000 frames. Bounded to the newest ~1024, it must
    // have aged out by now, leaving only the ~16.67ms frames.
    expect(snapshot.frameTimeMaxMs).toBeLessThan(100);
  });

  it("counts a 3x-normal interval as a dropped frame", () => {
    startDesktopPerformanceMonitor();
    flushFrame(0);
    flushFrame(16.67); // establishes the ~16.67ms baseline, not itself dropped
    const before = getDesktopPerformanceSnapshot().droppedFrames;

    flushFrame(16.67 * 3);
    const after = getDesktopPerformanceSnapshot();
    expect(after.frameSamples).toBe(2);
    expect(after.droppedFrames).toBeGreaterThan(before);
  });

  it("caps composer sample/event arrays at the newest window instead of growing forever", () => {
    for (let i = 0; i < 600; i++) recordComposerInput();
    flushFrame(0); // first level of each double-rAF chain
    flushFrame(0); // second level — this is where samples actually land

    const snapshot = getDesktopPerformanceSnapshot();
    expect(snapshot.composerInputSamples).toBe(512);
    expect(snapshot.composerInputPaintSamples).toHaveLength(512);
    expect(snapshot.composerInputEvents).toHaveLength(512);
  });

  it("stop cancels the rAF loop, and start is idempotent and safe to resume after stop", () => {
    startDesktopPerformanceMonitor();
    startDesktopPerformanceMonitor(); // idempotent — must not schedule a second chain
    expect(frameQueue).toHaveLength(1);

    flushFrame(0);
    flushFrame(16.67);
    expect(getDesktopPerformanceSnapshot().frameSamples).toBe(1);

    stopDesktopPerformanceMonitor();
    flushFrame(16.67); // the frame queued right before stop must be cancelled, not sampled
    expect(getDesktopPerformanceSnapshot().frameSamples).toBe(1);

    startDesktopPerformanceMonitor(); // resuming after stop must work
    flushFrame(0);
    flushFrame(16.67);
    expect(getDesktopPerformanceSnapshot().frameSamples).toBe(2);
  });
});
