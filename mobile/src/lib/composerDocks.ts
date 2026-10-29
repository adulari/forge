import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

import type { SnapshotTask } from "./ws";

export const DOCK_IDS = ["tasks", "spend"] as const;
export type DockId = (typeof DOCK_IDS)[number];
export type DockPlacement = "composer" | "panel";

export interface SessionDockState {
  /** Docks the person folded; a collapsed dock keeps its header and headline. */
  collapsed: DockId[];
  /** Docks hidden for this thread, with the content they were hidden at: a dock comes back when
   * what it shows changes (a new plan), so hiding one never hides the next one. */
  dismissed: Partial<Record<DockId, string>>;
}

export interface DockPrefs {
  placement: DockPlacement;
  order: DockId[];
  sessions: Record<string, SessionDockState>;
}

const STORAGE_KEY = "forge.composerDocks.v1";
const MAX_SESSIONS = 80;
const EMPTY_SESSION: SessionDockState = { collapsed: [], dismissed: {} };

export const DEFAULT_DOCK_PREFS: DockPrefs = { placement: "composer", order: [...DOCK_IDS], sessions: {} };

const isDockId = (value: unknown): value is DockId => DOCK_IDS.includes(value as DockId);

/** Every dock exactly once, in the stored order; anything missing or unknown is repaired. */
export function normalizeOrder(order: unknown): DockId[] {
  const seen = new Set<DockId>();
  const out: DockId[] = [];
  for (const id of Array.isArray(order) ? order : []) {
    if (isDockId(id) && !seen.has(id)) {
      seen.add(id);
      out.push(id);
    }
  }
  for (const id of DOCK_IDS) if (!seen.has(id)) out.push(id);
  return out;
}

export function parseDockPrefs(raw: string | null): DockPrefs {
  if (!raw) return { ...DEFAULT_DOCK_PREFS, order: [...DOCK_IDS], sessions: {} };
  try {
    const parsed = JSON.parse(raw) as Partial<DockPrefs> | null;
    const sessions: Record<string, SessionDockState> = {};
    for (const [id, value] of Object.entries(parsed?.sessions ?? {})) {
      if (typeof value !== "object" || value == null) continue;
      const dismissed: Partial<Record<DockId, string>> = {};
      for (const [dock, signature] of Object.entries(value.dismissed ?? {})) {
        if (isDockId(dock) && typeof signature === "string") dismissed[dock] = signature;
      }
      sessions[id] = { collapsed: (value.collapsed ?? []).filter(isDockId), dismissed };
    }
    return {
      placement: parsed?.placement === "panel" ? "panel" : "composer",
      order: normalizeOrder(parsed?.order),
      sessions,
    };
  } catch {
    return { ...DEFAULT_DOCK_PREFS, order: [...DOCK_IDS], sessions: {} };
  }
}

export function moveDock(order: readonly DockId[], id: DockId, delta: -1 | 1): DockId[] {
  const from = order.indexOf(id);
  const to = from + delta;
  if (from < 0 || to < 0 || to >= order.length) return [...order];
  const next = [...order];
  [next[from], next[to]] = [next[to], next[from]];
  return next;
}

/** What the task dock shows, for deciding whether a hidden dock should return. */
export function taskSignature(tasks: readonly SnapshotTask[]): string {
  return tasks.map((task) => task.title).join("\n");
}

export function sessionDockState(prefs: DockPrefs, sessionId: string): SessionDockState {
  return prefs.sessions[sessionId] ?? EMPTY_SESSION;
}

export function isDismissed(state: SessionDockState, id: DockId, signature: string): boolean {
  return state.dismissed[id] === signature;
}

/** Keep the newest sessions only; `touched` is the one just written. */
export function trimSessions(sessions: Record<string, SessionDockState>, touched: string): Record<string, SessionDockState> {
  const keys = Object.keys(sessions).filter((key) => key !== touched);
  const keep = keys.slice(Math.max(0, keys.length - (MAX_SESSIONS - 1)));
  const out: Record<string, SessionDockState> = {};
  for (const key of keep) out[key] = sessions[key];
  if (sessions[touched]) out[touched] = sessions[touched];
  return out;
}

let state: DockPrefs & { loaded: boolean } = { ...DEFAULT_DOCK_PREFS, loaded: false };
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

function replace(next: DockPrefs & { loaded: boolean }): void {
  state = next;
  for (const listener of listeners) listener();
}

async function ensureLoaded(): Promise<void> {
  if (state.loaded) return;
  hydration ??= AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => replace({ ...parseDockPrefs(raw), loaded: true }))
    .catch(() => replace({ ...state, loaded: true }));
  await hydration;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureLoaded();
  return () => listeners.delete(listener);
}

const snapshot = () => state;

export function useDockPrefs(): DockPrefs & { loaded: boolean } {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

async function commit(change: (prefs: DockPrefs) => DockPrefs): Promise<void> {
  await ensureLoaded();
  const next = change(state);
  replace({ ...next, loaded: true });
  try {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify({ placement: next.placement, order: next.order, sessions: next.sessions }));
  } catch {
    // The choice still holds for this run; only its persistence is lost.
  }
}

function withSession(prefs: DockPrefs, sessionId: string, edit: (current: SessionDockState) => SessionDockState): DockPrefs {
  const current = sessionDockState(prefs, sessionId);
  const sessions = { ...prefs.sessions };
  delete sessions[sessionId];
  sessions[sessionId] = edit(current);
  return { ...prefs, sessions: trimSessions(sessions, sessionId) };
}

export function setDockCollapsed(sessionId: string, id: DockId, collapsed: boolean): Promise<void> {
  return commit((prefs) =>
    withSession(prefs, sessionId, (current) => ({
      ...current,
      collapsed: collapsed ? [...current.collapsed.filter((dock) => dock !== id), id] : current.collapsed.filter((dock) => dock !== id),
    })),
  );
}

export function dismissDock(sessionId: string, id: DockId, signature: string): Promise<void> {
  return commit((prefs) => withSession(prefs, sessionId, (current) => ({ ...current, dismissed: { ...current.dismissed, [id]: signature } })));
}

export function restoreDock(sessionId: string, id: DockId): Promise<void> {
  return commit((prefs) =>
    withSession(prefs, sessionId, (current) => {
      const dismissed = { ...current.dismissed };
      delete dismissed[id];
      return { ...current, dismissed };
    }),
  );
}

export function setDockPlacement(placement: DockPlacement): Promise<void> {
  return commit((prefs) => ({ ...prefs, placement }));
}

export function reorderDock(id: DockId, delta: -1 | 1): Promise<void> {
  return commit((prefs) => ({ ...prefs, order: moveDock(prefs.order, id, delta) }));
}
