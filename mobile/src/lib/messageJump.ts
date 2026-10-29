// A palette message hit opens its session scrolled to the matching row. The palette lives at the
// app root and the session screen mounts later (or is already mounted and just switches), so the
// request is parked here and the session's transcript consumes it once the row is loaded.
interface Jump {
  sessionId: string;
  seq: number;
}

let pending: Jump | null = null;
const listeners = new Set<() => void>();

function notify(): void {
  listeners.forEach((listener) => listener());
}

export function requestMessageJump(sessionId: string, seq: number): void {
  pending = { sessionId, seq };
  notify();
}

export function peekMessageJump(sessionId: string): number | null {
  return pending && pending.sessionId === sessionId ? pending.seq : null;
}

export function clearMessageJump(sessionId: string): void {
  if (pending?.sessionId !== sessionId) return;
  pending = null;
  notify();
}

export function subscribeMessageJump(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Index of the message row for `seq` in the (newest-first) timeline, or -1 while it is not loaded. */
export function findJumpIndex(items: readonly { id: string }[], seq: number): number {
  const id = `h${seq}`;
  return items.findIndex((item) => item.id === id);
}

/** Older pages to pull while hunting for the row before giving up. */
export const MAX_JUMP_PAGES = 40;
