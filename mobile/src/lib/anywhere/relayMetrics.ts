const WINDOW = 100;

export interface RelayLatencyStats {
  samples: number;
  p50Ms: number | null;
  p90Ms: number | null;
  maxMs: number | null;
}

export interface RelayMetricsSnapshot {
  roundTrip: RelayLatencyStats;
  inboundHandling: RelayLatencyStats;
}

class RollingWindow {
  private readonly values: number[] = [];
  private cursor = 0;

  add(value: number): void {
    if (this.values.length < WINDOW) this.values.push(value);
    else {
      this.values[this.cursor] = value;
      this.cursor = (this.cursor + 1) % WINDOW;
    }
  }

  stats(): RelayLatencyStats {
    if (this.values.length === 0) return { samples: 0, p50Ms: null, p90Ms: null, maxMs: null };
    const sorted = [...this.values].sort((a, b) => a - b);
    const at = (q: number) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];
    return { samples: sorted.length, p50Ms: at(0.5), p90Ms: at(0.9), maxMs: sorted[sorted.length - 1] };
  }

  clear(): void {
    this.values.length = 0;
    this.cursor = 0;
  }
}

const roundTrips = new RollingWindow();
const inbound = new RollingWindow();

export function nowMs(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

/** Bridge request sent -> matching response delivered (includes relay + host time). */
export function recordRelayRoundTrip(ms: number): void {
  roundTrips.add(ms);
}

/** Inbound envelope received -> delivered (decode, verify, decrypt, replay accept). */
export function recordInboundHandling(ms: number): void {
  inbound.add(ms);
}

export function getRelayMetrics(): RelayMetricsSnapshot {
  return { roundTrip: roundTrips.stats(), inboundHandling: inbound.stats() };
}

export type ColdStartStage = "credentialsLoaded" | "consumersMounted" | "relayOpen" | "firstFleetRows";

const coldStart = new Map<ColdStartStage, number>();

/** First mark per stage wins, so later reconnects and refreshes do not overwrite the cold start. */
export function markColdStart(stage: ColdStartStage): void {
  if (!coldStart.has(stage)) coldStart.set(stage, nowMs());
}

/** Milliseconds since credentials loaded, per reached stage. */
export function getColdStartTimings(): Partial<Record<ColdStartStage, number>> {
  const origin = coldStart.get("credentialsLoaded");
  if (origin === undefined) return {};
  const result: Partial<Record<ColdStartStage, number>> = {};
  for (const [stage, at] of coldStart) result[stage] = Math.max(0, at - origin);
  return result;
}

export function resetRelayMetrics(): void {
  roundTrips.clear();
  inbound.clear();
  coldStart.clear();
}
