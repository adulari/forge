// A bridge between the session socket, which notices a host has stopped answering, and the auth
// provider, which owns the stored server list and knows how to find a moved daemon. The socket
// only announces the need; every reconnecting session announces it at once, so requests inside
// the cooldown collapse into one.
type Listener = () => void;

const COOLDOWN_MS = 10_000;
const listeners = new Set<Listener>();
let lastRequestAt = Number.NEGATIVE_INFINITY;

export function onRediscoverRequested(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function requestHostRediscovery(now: number = Date.now()): boolean {
  if (now - lastRequestAt < COOLDOWN_MS) return false;
  lastRequestAt = now;
  for (const listener of [...listeners]) listener();
  return true;
}

export function resetHostDiscoveryForTests(): void {
  lastRequestAt = Number.NEGATIVE_INFINITY;
  listeners.clear();
}
