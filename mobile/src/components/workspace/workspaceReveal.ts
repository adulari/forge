import { useSyncExternalStore } from "react";

export interface RevealRequest {
  sessionId: string;
  path: string;
  nonce: number;
}

let current: RevealRequest | null = null;
let nonce = 0;
const handled = new Map<string, number>();
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** "Reveal in tree": the file browser expands to `path` and highlights it. */
export function requestReveal(sessionId: string, path: string): void {
  nonce += 1;
  current = { sessionId, path, nonce };
  listeners.forEach((listener) => listener());
}

/** A request the browser has not applied yet: one already handled must not re-apply when the
 * browser remounts (a tab switch), or it would re-expand folders the person has since closed. */
export function pendingReveal(request: RevealRequest | null, sessionId: string): RevealRequest | null {
  if (!request || request.sessionId !== sessionId) return null;
  return handled.get(sessionId) === request.nonce ? null : request;
}

export function markRevealHandled(request: RevealRequest): void {
  handled.set(request.sessionId, request.nonce);
}

export function useRevealRequest(sessionId: string): RevealRequest | null {
  const request = useSyncExternalStore(subscribe, () => current, () => current);
  return pendingReveal(request, sessionId);
}

/** Every directory above `path`, outermost first ("a/b/c.ts" -> ["a", "a/b"]). */
export function ancestorDirectories(path: string): string[] {
  const parts = path.split("/").filter(Boolean);
  parts.pop();
  return parts.map((_, index) => parts.slice(0, index + 1).join("/"));
}
