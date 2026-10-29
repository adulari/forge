export const BACKOFF_MS = [500, 1000, 2000, 4000, 8000, 15000];
export const JITTER = 0.25;
export const REDISCOVER_EVERY_FAILURES = 3;

/** Delay before reconnect attempt `attempt` (0-based), spread ±25% so every socket a restarted
 * daemon dropped does not come back in the same instant. */
export function backoffDelay(attempt: number, random: () => number = Math.random): number {
  const base = BACKOFF_MS[Math.min(Math.max(0, attempt), BACKOFF_MS.length - 1)];
  const spread = 1 + (random() * 2 - 1) * JITTER;
  return Math.round(base * spread);
}

/** A stored address that keeps failing may simply have moved (a tunnel hostname changes on every
 * daemon start), so every third consecutive failure is the moment to look for the host again. */
export function shouldRediscover(failedAttempts: number): boolean {
  return failedAttempts > 0 && failedAttempts % REDISCOVER_EVERY_FAILURES === 0;
}
