// Shared clock for relative timestamps. One interval updates all mounted labels instead of one
// timer per visible row in Fleet, Inbox, and History.
import { useEffect, useState } from "react";

const REFRESH_MS = 30_000;
const listeners = new Set<() => void>();
let interval: ReturnType<typeof setInterval> | null = null;

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (interval == null) {
    interval = setInterval(() => {
      listeners.forEach((notify) => notify());
    }, REFRESH_MS);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && interval != null) {
      clearInterval(interval);
      interval = null;
    }
  };
}

export function useRelativeClock() {
  const [, tick] = useState(0);
  useEffect(() => subscribe(() => tick((value) => value + 1)), []);
}
