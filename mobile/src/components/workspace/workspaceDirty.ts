import { useSyncExternalStore } from "react";

const dirty = new Set<string>();
const listeners = new Set<() => void>();
let version = 0;

const keyOf = (sessionId: string, path: string) => `${sessionId}\u0000${path}`;

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Editors report unsaved edits here so the tab strip can show a marker for a file whose editor
 * is not the one on screen. */
export function setWorkspaceDirty(sessionId: string, path: string, value: boolean): void {
  const key = keyOf(sessionId, path);
  if (value === dirty.has(key)) return;
  if (value) dirty.add(key);
  else dirty.delete(key);
  version += 1;
  listeners.forEach((listener) => listener());
}

export function isWorkspaceDirty(sessionId: string | null, path: string | null): boolean {
  return sessionId != null && path != null && dirty.has(keyOf(sessionId, path));
}

export function useWorkspaceDirty(sessionId: string | null, path: string | null): boolean {
  useSyncExternalStore(subscribe, () => version, () => version);
  return isWorkspaceDirty(sessionId, path);
}
