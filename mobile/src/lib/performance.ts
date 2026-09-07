import { isTauri } from "./platform";

const startedAt = typeof performance !== "undefined" ? performance.now() : null;
let moduleEvalEndMs: number | null = null;
let nativeTimeline: Record<string, number | null> | null = null;
let nativeModuleEvalMs: number | null = null;
void (isTauri ? import("@tauri-apps/api/core").then(({ invoke }) => invoke<number>("perf_native_now")) : Promise.resolve(null))
  .then((elapsed) => { nativeModuleEvalMs = elapsed; })
  .catch(() => undefined);

export type DesktopPerformanceSnapshot = {
  startupToInteractiveMs: number | null;
  interactiveMilestone: "first-frame-after-hydration";
  frameSamples: number;
  droppedFrames: number;
  estimatedRefreshRateHz: number | null;
  frameTimeP50Ms: number | null;
  frameTimeP95Ms: number | null;
  frameTimeMaxMs: number | null;
  nativeTimeline: Record<string, number | null> | null;
  processStartToFirstPaintMs: number | null;
  hydration: {
    moduleEvalEndMs: number | null;
    reactMountStartMs: number | null;
    reactMountEndMs: number | null;
    firstDataResolveMs: number | null;
    firstPaintMs: number | null;
  };
  phaseStartAtMs: number | null;
  firstWorkloadAtMs: number | null;
  longTaskCount: number;
  longTaskTotalMs: number;
  longestTaskMs: number;
  composerInputSamples: number;
  composerInputToPaintP50Ms: number | null;
  composerInputToPaintP95Ms: number | null;
  composerInputToPaintMaxMs: number | null;
  composerImeSamples: number;
  composerImeToPaintP50Ms: number | null;
  composerImeToPaintP95Ms: number | null;
  composerImeToPaintMaxMs: number | null;
  composerInputPaintSamples: number[];
  composerImePaintSamples: number[];
  composerInputEvents: {
    eventAtMs: number;
    paintAtMs: number;
    latencyMs: number;
  }[];
  composerImeEvents: {
    eventAtMs: number;
    paintAtMs: number;
    latencyMs: number;
  }[];
};

// Bounded ring buffer of frame intervals — fixed-size Float64Array with a wrapping write
// cursor, so `push` is O(1) regardless of how long the monitor has been running (unlike the
// old unbounded array + per-frame sort, which grew to tens of thousands of entries and cost
// tens of milliseconds per frame after 30 minutes on screen).
class RingBuffer {
  private readonly buf: Float64Array;
  private writeIndex = 0;
  private count = 0;

  constructor(private readonly capacity: number) {
    this.buf = new Float64Array(capacity);
  }

  push(value: number): void {
    this.buf[this.writeIndex] = value;
    this.writeIndex = (this.writeIndex + 1) % this.capacity;
    this.count = Math.min(this.count + 1, this.capacity);
  }

  toArray(): number[] {
    if (this.count < this.capacity) return Array.from(this.buf.subarray(0, this.count));
    const out: number[] = new Array(this.capacity);
    for (let i = 0; i < this.capacity; i++) out[i] = this.buf[(this.writeIndex + i) % this.capacity];
    return out;
  }

  clear(): void {
    this.buf.fill(0);
    this.writeIndex = 0;
    this.count = 0;
  }

  get length(): number {
    return this.count;
  }
}

/** Cap a "newest N" sample/event array in place — cheap since composer events fire at most a
 * few times per second (user typing), never per animation frame. */
function pushBounded<T>(arr: T[], value: T, cap: number): void {
  arr.push(value);
  if (arr.length > cap) arr.splice(0, arr.length - cap);
}

const FRAME_WINDOW = 1024;
const COMPOSER_WINDOW = 512;
// Smoothing factor for the running "expected interval" estimate used for dropped-frame
// detection. Small alpha = slow to react to real refresh-rate changes but immune to a single
// frame hitch skewing the baseline; recomputed with O(1) work per frame (no sort).
const EXPECTED_INTERVAL_ALPHA = 0.05;
const DEFAULT_EXPECTED_INTERVAL_MS = 16.67;

const composerSamples: number[] = [];
const composerImeSamples: number[] = [];
const composerInputEvents: {
  eventAtMs: number;
  paintAtMs: number;
  latencyMs: number;
}[] = [];
const composerImeEvents: {
  eventAtMs: number;
  paintAtMs: number;
  latencyMs: number;
}[] = [];
const frameIntervals = new RingBuffer(FRAME_WINDOW);
let expectedIntervalMs = DEFAULT_EXPECTED_INTERVAL_MS;
let startupToInteractiveMs: number | null = null;
let phaseStartAtMs: number | null = null;
let firstWorkloadAtMs: number | null = null;
let frameSamples = 0;
let droppedFrames = 0;
let longTaskCount = 0;
let longTaskTotalMs = 0;
let longestTaskMs = 0;
let frameHandle: number | null = null;
let lastFrameAt: number | null = null;
let firstDataResolveMs: number | null = null;
let firstPaintMs: number | null = null;
let reactMountStartMs: number | null = null;
let reactMountEndMs: number | null = null;
let longTaskObserver: PerformanceObserver | null = null;
moduleEvalEndMs = startedAt == null ? null : performance.now() - startedAt;

function percentile(values: number[], percentileValue: number): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[
    Math.min(sorted.length - 1, Math.floor(sorted.length * percentileValue))
  ];
}

function sampleFrame(now: number): void {
  if (lastFrameAt != null) {
    const interval = now - lastFrameAt;
    frameIntervals.push(interval);
    frameSamples += 1;
    if (interval > expectedIntervalMs * 1.5)
      droppedFrames += Math.max(1, Math.round(interval / expectedIntervalMs) - 1);
    // Cheap O(1) running estimate rather than a per-frame median — only nudged by frames
    // that look "normal" so one dropped frame doesn't drag the baseline toward it.
    if (interval <= expectedIntervalMs * 1.5)
      expectedIntervalMs += (interval - expectedIntervalMs) * EXPECTED_INTERVAL_ALPHA;
  }
  lastFrameAt = now;
  frameHandle = requestAnimationFrame(sampleFrame);
}

export function startDesktopPerformanceMonitor(): void {
  if (isTauri) {
    void import("@tauri-apps/api/core")
      .then(({ invoke }) => invoke<Record<string, number | null>>("perf_native_timeline"))
      .then((timeline) => { nativeTimeline = timeline; })
      .catch(() => undefined);
  }
  if (typeof requestAnimationFrame !== "function") return;
  if (frameHandle == null) {
    lastFrameAt = null;
    frameHandle = requestAnimationFrame(sampleFrame);
  }
  if (longTaskObserver == null && typeof PerformanceObserver !== "undefined") {
    try {
      longTaskObserver = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          const duration = entry.duration;
          longTaskCount += 1;
          longTaskTotalMs += duration;
          longestTaskMs = Math.max(longestTaskMs, duration);
        }
      });
      longTaskObserver.observe({ type: "longtask", buffered: true });
    } catch {
      longTaskObserver = null;
      // Long-task entries are not implemented by every WebView.
    }
  }
}

/** Stops the always-on rAF sampler and long-task observer. Safe to call when not running, and
 * `startDesktopPerformanceMonitor` is safe to call again afterwards (matches its own
 * `frameHandle == null` idempotency guard). */
export function stopDesktopPerformanceMonitor(): void {
  if (frameHandle != null && typeof cancelAnimationFrame === "function") {
    cancelAnimationFrame(frameHandle);
  }
  frameHandle = null;
  lastFrameAt = null;
  if (longTaskObserver != null) {
    longTaskObserver.disconnect();
    longTaskObserver = null;
  }
}

export function markReactMountStart(): void {
  if (reactMountStartMs == null && startedAt != null)
    reactMountStartMs = performance.now() - startedAt;
}
export function markReactMountEnd(): void {
  if (reactMountEndMs == null && startedAt != null)
    reactMountEndMs = performance.now() - startedAt;
}
export function markFirstDataResolve(): void {
  if (firstDataResolveMs == null && startedAt != null)
    firstDataResolveMs = performance.now() - startedAt;
}
export function markFirstPaint(): void {
  if (firstPaintMs == null && startedAt != null)
    firstPaintMs = performance.now() - startedAt;
}

export function markPerformancePhaseStart(): void {
  if (
    phaseStartAtMs == null &&
    startedAt != null &&
    typeof performance !== "undefined"
  )
    phaseStartAtMs = performance.now() - startedAt;
}

export function markFirstWorkloadEvent(): void {
  if (
    firstWorkloadAtMs == null &&
    startedAt != null &&
    typeof performance !== "undefined"
  )
    firstWorkloadAtMs = performance.now() - startedAt;
}
export function markDesktopInteractive(): void {
  if (
    startupToInteractiveMs == null &&
    startedAt != null &&
    typeof performance !== "undefined"
  ) {
    startupToInteractiveMs = performance.now() - startedAt;
  }
}

export function recordComposerInput(): void {
  if (typeof performance === "undefined") return;
  const relativeNow = () =>
    startedAt == null || typeof performance === "undefined"
      ? 0
      : performance.now() - startedAt;
  const inputAt = relativeNow();
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      const paintAt = relativeNow();
      const latencyMs = paintAt - inputAt;
      pushBounded(composerSamples, latencyMs, COMPOSER_WINDOW);
      pushBounded(composerInputEvents, {
        eventAtMs: inputAt,
        paintAtMs: paintAt,
        latencyMs,
      }, COMPOSER_WINDOW);
    });
  });
}

export function recordCompositorKey(): void {
  if (typeof performance === "undefined") return;
  const eventAt = startedAt == null ? 0 : performance.now() - startedAt;
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      const paintAt =
        startedAt == null ? eventAt : performance.now() - startedAt;
      const latencyMs = paintAt - eventAt;
      pushBounded(composerSamples, latencyMs, COMPOSER_WINDOW);
      pushBounded(composerInputEvents, {
        eventAtMs: eventAt,
        paintAtMs: paintAt,
        latencyMs,
      }, COMPOSER_WINDOW);
    });
  });
}
export function recordComposerImeCommit(): void {
  if (typeof performance === "undefined") return;
  const relativeNow = () =>
    startedAt == null ? 0 : performance.now() - startedAt;
  const commitAt = relativeNow();
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      const paintAt = relativeNow();
      const latencyMs = paintAt - commitAt;
      pushBounded(composerImeSamples, latencyMs, COMPOSER_WINDOW);
      pushBounded(composerImeEvents, {
        eventAtMs: commitAt,
        paintAtMs: paintAt,
        latencyMs,
      }, COMPOSER_WINDOW);
    });
  });
}

export function getDesktopPerformanceSnapshot(): DesktopPerformanceSnapshot {
  const frameWindow = frameIntervals.toArray();
  const refreshInterval = percentile(frameWindow, 0.5);
  return {
    startupToInteractiveMs,
    interactiveMilestone: "first-frame-after-hydration",
    frameSamples,
    droppedFrames,
    estimatedRefreshRateHz:
      refreshInterval && refreshInterval > 0 ? 1000 / refreshInterval : null,
    frameTimeP50Ms: percentile(frameWindow, 0.5),
    frameTimeP95Ms: percentile(frameWindow, 0.95),
    frameTimeMaxMs: percentile(frameWindow, 1),
    nativeTimeline,
    processStartToFirstPaintMs: nativeModuleEvalMs == null || firstPaintMs == null ? null : nativeModuleEvalMs + firstPaintMs,
    hydration: {
      moduleEvalEndMs,
      reactMountStartMs,
      reactMountEndMs,
      firstDataResolveMs,
      firstPaintMs,
    },
    phaseStartAtMs,
    firstWorkloadAtMs,
    longTaskCount,
    longTaskTotalMs,
    longestTaskMs,
    composerInputSamples: composerSamples.length,
    composerInputToPaintP50Ms: percentile(composerSamples, 0.5),
    composerInputToPaintP95Ms: percentile(composerSamples, 0.95),
    composerInputToPaintMaxMs: percentile(composerSamples, 1),
    composerImeSamples: composerImeSamples.length,
    composerImeToPaintP50Ms: percentile(composerImeSamples, 0.5),
    composerImeToPaintP95Ms: percentile(composerImeSamples, 0.95),
    composerImeToPaintMaxMs: percentile(composerImeSamples, 1),
    composerInputPaintSamples: [...composerSamples],
    composerImePaintSamples: [...composerImeSamples],
    composerInputEvents: [...composerInputEvents],
    composerImeEvents: [...composerImeEvents],
  };
}

export async function dumpDesktopPerformanceSnapshot(): Promise<void> {
  if (typeof window === "undefined" || !isTauri) return;
  const { invoke } = await import("@tauri-apps/api/core");
  nativeTimeline = await invoke<Record<string, number | null>>("perf_native_timeline");
  await invoke("perf_dump", { snapshot: JSON.stringify(getDesktopPerformanceSnapshot()) });
}
export function resetDesktopPerformanceSamples(): void {
  composerSamples.length = 0;
  composerImeSamples.length = 0;
  composerInputEvents.length = 0;
  composerImeEvents.length = 0;
  frameIntervals.clear();
  expectedIntervalMs = DEFAULT_EXPECTED_INTERVAL_MS;
  frameSamples = 0;
  droppedFrames = 0;
  longTaskCount = 0;
  longTaskTotalMs = 0;
  firstDataResolveMs = null;
  firstPaintMs = null;
  reactMountStartMs = null;
  reactMountEndMs = null;
}
