export type DesktopPerformanceSnapshot = {
  startupToInteractiveMs: number | null;
  interactiveMilestone: "first-frame-after-hydration";
  frameSamples: number;
  droppedFrames: number;
  estimatedRefreshRateHz: number | null;
  frameTimeP50Ms: number | null;
  frameTimeP95Ms: number | null;
  frameTimeMaxMs: number | null;
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
}

const startedAt = typeof performance !== "undefined" ? performance.now() : null;
const composerSamples: number[] = [];
const composerImeSamples: number[] = [];
const frameIntervals: number[] = [];
let startupToInteractiveMs: number | null = null;
let frameSamples = 0;
let droppedFrames = 0;
let longTaskCount = 0;
let longTaskTotalMs = 0;
let longestTaskMs = 0;
let frameHandle: number | null = null;
let lastFrameAt: number | null = null;
let longTaskObserver: PerformanceObserver | null = null;

function percentile(values: number[], percentileValue: number): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * percentileValue))];
}

function sampleFrame(now: number): void {
  if (lastFrameAt != null) {
    const interval = now - lastFrameAt;
    frameIntervals.push(interval);
    frameSamples += 1;
    const expected = percentile(frameIntervals, 0.5) ?? 16.67;
    if (interval > expected * 1.5) droppedFrames += Math.max(1, Math.round(interval / expected) - 1);
  }
  lastFrameAt = now;
  frameHandle = requestAnimationFrame(sampleFrame);
}

export function startDesktopPerformanceMonitor(): void {
  if (typeof window === "undefined" || typeof requestAnimationFrame !== "function") return;
  if (frameHandle == null) frameHandle = requestAnimationFrame(sampleFrame);
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

export function markDesktopInteractive(): void {
  if (startupToInteractiveMs == null && startedAt != null && typeof performance !== "undefined") {
    startupToInteractiveMs = performance.now() - startedAt;
  }
}

export function recordComposerInput(): void {
  if (typeof performance === "undefined") return;
  const inputAt = performance.now();
  requestAnimationFrame(() => {
    requestAnimationFrame(() => composerSamples.push(performance.now() - inputAt));
  });
}

export function recordComposerImeCommit(): void {
  if (typeof performance === "undefined") return;
  const commitAt = performance.now();
  requestAnimationFrame(() => {
    requestAnimationFrame(() => composerImeSamples.push(performance.now() - commitAt));
  });
}

export function getDesktopPerformanceSnapshot(): DesktopPerformanceSnapshot {
  const refreshInterval = percentile(frameIntervals, 0.5);
  return {
    startupToInteractiveMs,
    interactiveMilestone: "first-frame-after-hydration",
    frameSamples,
    droppedFrames,
    estimatedRefreshRateHz: refreshInterval && refreshInterval > 0 ? 1000 / refreshInterval : null,
    frameTimeP50Ms: percentile(frameIntervals, 0.5),
    frameTimeP95Ms: percentile(frameIntervals, 0.95),
    frameTimeMaxMs: percentile(frameIntervals, 1),
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
  };
}

export async function dumpDesktopPerformanceSnapshot(): Promise<void> {
  if (typeof window === "undefined" || process.env.EXPO_PUBLIC_PERF_FIXTURE !== "1") return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("perf_dump", { snapshot: JSON.stringify(getDesktopPerformanceSnapshot()) });
}
export function resetDesktopPerformanceSamples(): void {
  composerSamples.length = 0;
  composerImeSamples.length = 0;
  frameIntervals.length = 0;
  frameSamples = 0;
  droppedFrames = 0;
  longTaskCount = 0;
  longTaskTotalMs = 0;
  longestTaskMs = 0;
}
