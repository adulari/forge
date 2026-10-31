import { useEffect, useMemo, useState } from "react";

export function msUntilNextLocalDay(now: Date): number {
  const next = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 0, 0, 1);
  return Math.max(1000, next.getTime() - now.getTime());
}

/** The current local day as a Date that moves forward: a timer fires just after local midnight, and
 * `refreshKey` (the query's fresh rows) re-reads the clock too, since timers pause while a tab or
 * a phone sleeps. A `useMemo(() => new Date(), [])` froze it at mount, so an app left open overnight
 * dropped the new day's usage from every total. */
export function useToday(refreshKey?: unknown): Date {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const timer = setTimeout(() => setTick((n) => n + 1), msUntilNextLocalDay(new Date()));
    return () => clearTimeout(timer);
  }, [tick]);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- tick and refreshKey are the triggers
  return useMemo(() => new Date(), [tick, refreshKey]);
}
