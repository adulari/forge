// Two small pieces of module state the notification paths share across the provider tree: which
// sessions are on screen (published from inside the workbench, read by the watchers that sit
// above it), and a one-shot request to focus a session's composer once its screen mounts.
let visible: readonly string[] = [];
let pendingFocus: string | null = null;
const focusListeners = new Set<() => void>();

export function setVisibleSessionIds(ids: readonly string[]): void {
  visible = ids;
}

export function getVisibleSessionIds(): readonly string[] {
  return visible;
}

export function requestComposerFocus(sessionId: string): void {
  pendingFocus = sessionId;
  for (const listener of focusListeners) listener();
}

/** Called when a focus request arrives while the session's screen is already mounted. */
export function onComposerFocusRequest(listener: () => void): () => void {
  focusListeners.add(listener);
  return () => {
    focusListeners.delete(listener);
  };
}

/** True once for the session a notification click asked to focus. */
export function consumeComposerFocus(sessionId: string): boolean {
  if (pendingFocus !== sessionId) return false;
  pendingFocus = null;
  return true;
}
