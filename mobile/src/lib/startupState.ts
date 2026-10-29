// Whether the app has reached its first interactive frame. The crash boundary and the startup
// watchdog sit above every provider, so they read this module-level flag instead of any context.
let complete = false;
const listeners = new Set<() => void>();

export function isStartupComplete(): boolean {
  return complete;
}

export function markStartupComplete(): void {
  if (complete) return;
  complete = true;
  for (const listener of listeners) listener();
}

export function subscribeStartup(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function resetStartupForTest(): void {
  complete = false;
  listeners.clear();
}
