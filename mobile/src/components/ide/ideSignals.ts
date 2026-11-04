// Small shared state the IDE panes hand to each other without a provider: the why-trace toggle,
// "jump to turn", "reveal this line", unsaved files, and what the last checks implicated. Each is
// a tiny external store, so a write re-renders only the pane that reads it.
import { useSyncExternalStore } from "react";

interface Store<T> {
  get: () => T;
  set: (next: T) => void;
  use: () => T;
}

function store<T>(initial: T): Store<T> {
  let value = initial;
  const listeners = new Set<() => void>();
  const get = () => value;
  return {
    get,
    set: (next) => {
      if (Object.is(next, value)) return;
      value = next;
      listeners.forEach((listener) => listener());
    },
    use: () =>
      useSyncExternalStore(
        (listener) => {
          listeners.add(listener);
          return () => listeners.delete(listener);
        },
        get,
        get,
      ),
  };
}

// --- why-trace toggle (remembered per viewer; storage can be blocked) ---
const TRACE_KEY = "forge.ideTrace.v1";
function readTrace(): boolean {
  try {
    return typeof localStorage !== "undefined" && localStorage.getItem(TRACE_KEY) === "1";
  } catch {
    return false;
  }
}
const traceStore = store<boolean>(readTrace());
export const useWhyTraceOn = traceStore.use;
export function setWhyTrace(on: boolean): void {
  traceStore.set(on);
  try {
    localStorage.setItem(TRACE_KEY, on ? "1" : "0");
  } catch {
    // Storage blocked: the toggle still works for this run.
  }
}
export const whyTraceOn = traceStore.get;

// --- jump to a turn in the agent timeline ---
export interface TurnFocus {
  sessionId: string;
  turn: number;
  path: string | null;
  nonce: number;
}
const turnStore = store<TurnFocus | null>(null);
export const useTurnFocus = turnStore.use;
let turnNonce = 0;
export function focusTurn(sessionId: string, turn: number, path: string | null): void {
  turnNonce += 1;
  turnStore.set({ sessionId, turn, path, nonce: turnNonce });
}

// --- reveal a line in the editor ---
export interface Reveal {
  sessionId: string;
  path: string;
  line: number;
  nonce: number;
}
const revealStore = store<Reveal | null>(null);
export const useReveal = revealStore.use;
export const currentReveal = revealStore.get;
let revealNonce = 0;
export function revealLine(sessionId: string, path: string, line: number): void {
  revealNonce += 1;
  revealStore.set({ sessionId, path, line, nonce: revealNonce });
}
let focusNonce = 0;
let focusPending = false;
const focusEditorStore = store<number>(0);
export const useEditorFocusRequest = focusEditorStore.use;
/** Ask for the editor to take keyboard focus, even if it has not mounted yet. */
export function requestEditorFocus(): void {
  focusNonce += 1;
  focusPending = true;
  focusEditorStore.set(focusNonce);
}
/** The editor calls this once it can focus; true means a request was waiting. */
export function consumeEditorFocus(): boolean {
  const pending = focusPending;
  focusPending = false;
  return pending;
}

// --- unsaved files ---
const dirtyStore = store<ReadonlySet<string>>(new Set());
export const useDirtyFiles = dirtyStore.use;
export const dirtyFiles = dirtyStore.get;
export const dirtyKey = (sessionId: string, path: string) => `${sessionId}\0${path}`;
export function setFileDirty(sessionId: string, path: string, dirty: boolean): void {
  const key = dirtyKey(sessionId, path);
  const current = dirtyStore.get();
  if (current.has(key) === dirty) return;
  const next = new Set(current);
  if (dirty) next.add(key);
  else next.delete(key);
  dirtyStore.set(next);
}

// --- files named by failing checks, per session ---
export type FailingByAction = Readonly<Record<string, readonly string[]>>;
const failingStore = store<Readonly<Record<string, FailingByAction>>>({});
export const useFailingChecks = failingStore.use;
export function setActionFailing(sessionId: string, action: string, paths: readonly string[]): void {
  const current = failingStore.get();
  const forSession = current[sessionId] ?? {};
  const had = forSession[action];
  if (paths.length === 0 && !had) return;
  const nextSession: Record<string, readonly string[]> = { ...forSession };
  if (paths.length === 0) delete nextSession[action];
  else nextSession[action] = paths;
  failingStore.set({ ...current, [sessionId]: nextSession });
}

/** Every path any failing check of `sessionId` names. */
export function failingPathsOf(map: Readonly<Record<string, FailingByAction>>, sessionId: string): ReadonlySet<string> {
  const out = new Set<string>();
  for (const paths of Object.values(map[sessionId] ?? {})) for (const path of paths) out.add(path);
  return out;
}

// --- which bottom-panel tab is showing ---
export type BottomTab = "terminal" | "checks" | "shells";
const bottomTabStore = store<BottomTab>("terminal");
export const useBottomTab = bottomTabStore.use;
export const setBottomTab = bottomTabStore.set;
