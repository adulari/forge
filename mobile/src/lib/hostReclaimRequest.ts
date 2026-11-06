// Tiny module-level stores shared by the pressure banner and the command palette: one pending
// "reclaim?" confirmation (rendered by HostReclaimDialog) and the banner dismissal, which has to
// outlive the banner because OperationalNotice unmounts it on every session route.

import { useSyncExternalStore } from "react";

function store<T>(initial: T) {
  let value = initial;
  const listeners = new Set<() => void>();
  return {
    get: () => value,
    set(next: T) {
      if (next === value) return;
      value = next;
      listeners.forEach((l) => l());
    },
    subscribe(l: () => void) {
      listeners.add(l);
      return () => listeners.delete(l);
    },
  };
}

const requested = store(false);
const dismissed = store<string | null>(null);

export function requestHostReclaim(): void {
  requested.set(true);
}

export function clearHostReclaimRequest(): void {
  requested.set(false);
}

export function useHostReclaimRequested(): boolean {
  return useSyncExternalStore(requested.subscribe, requested.get, requested.get);
}

export function dismissHostPressure(key: string): void {
  dismissed.set(key);
}

export function useDismissedHostPressure(): string | null {
  return useSyncExternalStore(dismissed.subscribe, dismissed.get, dismissed.get);
}
