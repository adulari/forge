// Two small pieces of module state the notification paths share across the provider tree: which
// sessions are on screen (published from inside the workbench, read by the watchers that sit
// above it), and a one-shot request to focus a session's composer once its screen mounts.
let visible: readonly string[] = [];
let rendered: RenderedPanes | null = null;
const renderedListeners = new Set<() => void>();
const modelPickerListeners = new Set<(sessionId: string) => void>();

/** What the pane grid actually has on screen: panes hidden by a narrow window do not count. */
export interface RenderedPanes {
  count: number;
  maximized: number | null;
}
let pendingFocus: string | null = null;
const focusListeners = new Set<() => void>();

export function setVisibleSessionIds(ids: readonly string[]): void {
  visible = ids;
}

/** Published by PaneGrid from its geometry; null while no grid is mounted (compact layouts). */
export function setRenderedPanes(next: RenderedPanes | null): void {
  if (rendered === next || (rendered && next && rendered.count === next.count && rendered.maximized === next.maximized)) return;
  rendered = next;
  for (const listener of renderedListeners) listener();
}

export function getRenderedPanes(): RenderedPanes | null {
  return rendered;
}

export function subscribeRenderedPanes(listener: () => void): () => void {
  renderedListeners.add(listener);
  return () => {
    renderedListeners.delete(listener);
  };
}

/** A pane header's model chip asks that session's composer to open its model picker. */
export function requestModelPicker(sessionId: string): void {
  for (const listener of modelPickerListeners) listener(sessionId);
}

export function onModelPickerRequest(listener: (sessionId: string) => void): () => void {
  modelPickerListeners.add(listener);
  return () => {
    modelPickerListeners.delete(listener);
  };
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
