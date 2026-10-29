// Context-aware Find. One bar (components/find/FindBar) and one controller, many surfaces: each
// surface that can be searched registers a target, and Mod+F opens the bar on the one that has
// the user's attention, falling back through a fixed order. The terminal registers the same way
// (`id: "terminal"`) once its scrollback can be searched; until then Mod+F never resolves to it.
import { useSyncExternalStore } from "react";

export type FindTargetId = "chat" | "diff" | "editor" | "terminal";

export interface FindResult {
  /** Hits found. */
  count: number;
  /** Active hit, 0-based, or -1 when there is none. */
  index: number;
  /** More hits exist than `count` reports. */
  capped?: boolean;
  /** A short note beside the count, e.g. the active hit's file or line. */
  detail?: string;
}

export interface FindTarget {
  id: FindTargetId;
  label: string;
  /** Searches and activates the first hit. */
  search(query: string, caseSensitive: boolean): Promise<FindResult> | FindResult;
  /** Moves to the next (`1`) or previous (`-1`) hit, wrapping. */
  step(direction: 1 | -1): Promise<FindResult> | FindResult;
  /** Drops the target's highlights. */
  clear(): void;
}

export interface FindState {
  open: boolean;
  targetId: FindTargetId | null;
  query: string;
  caseSensitive: boolean;
  result: FindResult;
  searching: boolean;
  /** Bumps on every request so an already-open bar takes focus again. */
  focusNonce: number;
}

const NO_RESULT: FindResult = { count: 0, index: -1 };
const SEARCH_DEBOUNCE_MS = 140;
const FALLBACK_ORDER: readonly FindTargetId[] = ["chat", "diff", "editor", "terminal"];

/** The target Mod+F opens: the focused surface when it can be searched, else the first
 * registered in the fixed order, else none. */
export function resolveFindTarget(
  registered: readonly FindTargetId[],
  focused: FindTargetId | null,
): FindTargetId | null {
  if (focused && registered.includes(focused)) return focused;
  return FALLBACK_ORDER.find((id) => registered.includes(id)) ?? null;
}

const targets = new Map<FindTargetId, FindTarget>();
const listeners = new Set<() => void>();
let focused: FindTargetId | null = null;
let timer: ReturnType<typeof setTimeout> | null = null;
let generation = 0;
let state: FindState = {
  open: false,
  targetId: null,
  query: "",
  caseSensitive: false,
  result: NO_RESULT,
  searching: false,
  focusNonce: 0,
};
let availability = 0;

function emit(next: Partial<FindState>): void {
  state = { ...state, ...next };
  listeners.forEach((listener) => listener());
}

function activeTarget(): FindTarget | null {
  return state.targetId ? (targets.get(state.targetId) ?? null) : null;
}

async function runSearch(): Promise<void> {
  const target = activeTarget();
  const mine = ++generation;
  if (!target || state.query === "") {
    target?.clear();
    emit({ result: NO_RESULT, searching: false });
    return;
  }
  emit({ searching: true });
  try {
    const result = await target.search(state.query, state.caseSensitive);
    if (mine === generation) emit({ result, searching: false });
  } catch {
    if (mine === generation) emit({ result: NO_RESULT, searching: false });
  }
}

function schedule(): void {
  if (timer) clearTimeout(timer);
  timer = setTimeout(() => {
    timer = null;
    void runSearch();
  }, SEARCH_DEBOUNCE_MS);
}

export const findController = {
  getState: (): FindState => state,
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },

  /** Makes `target` searchable; returns the unregister function. */
  register(target: FindTarget): () => void {
    targets.set(target.id, target);
    availability += 1;
    listeners.forEach((listener) => listener());
    return () => {
      if (targets.get(target.id) !== target) return;
      targets.delete(target.id);
      availability += 1;
      if (focused === target.id) focused = null;
      if (state.targetId === target.id) {
        if (timer) clearTimeout(timer);
        generation += 1;
        emit({ open: false, targetId: null, result: NO_RESULT, searching: false });
      } else {
        listeners.forEach((listener) => listener());
      }
    };
  },

  hasTargets: (): boolean => targets.size > 0,
  availabilityVersion: (): number => availability,

  /** The surface the user last touched. */
  markFocus(id: FindTargetId): void {
    focused = id;
  },

  /** Mod+F. Opens the bar on the resolved target, or refocuses it when already open there. */
  request(): boolean {
    const id = resolveFindTarget([...targets.keys()], focused);
    if (!id) return false;
    if (state.open && state.targetId !== id) activeTarget()?.clear();
    const switched = state.targetId !== id;
    emit({ open: true, targetId: id, focusNonce: state.focusNonce + 1, ...(switched ? { result: NO_RESULT } : {}) });
    if (state.query !== "" && (switched || state.result.count === 0)) schedule();
    return true;
  },

  setQuery(query: string): void {
    emit({ query });
    schedule();
  },

  toggleCase(): void {
    emit({ caseSensitive: !state.caseSensitive });
    schedule();
  },

  async step(direction: 1 | -1): Promise<void> {
    const target = activeTarget();
    if (!target || state.result.count === 0) return;
    const mine = ++generation;
    try {
      const result = await target.step(direction);
      if (mine === generation) emit({ result });
    } catch {
      // The target's rows moved under the step; the next search settles it.
    }
  },

  close(): void {
    if (timer) clearTimeout(timer);
    generation += 1;
    activeTarget()?.clear();
    emit({ open: false, searching: false });
  },
};

export function useFindState(): FindState {
  return useSyncExternalStore(findController.subscribe, findController.getState, findController.getState);
}

/** True while any surface can be searched. Mod+F is claimed only then, so the browser's own
 * find keeps working on a page with nothing of ours to search. */
export function useFindAvailable(): boolean {
  return useSyncExternalStore(
    findController.subscribe,
    findController.hasTargets,
    () => false,
  );
}

/** Spread onto a surface's root so a pointer press or focus inside it makes it the Find target.
 * react-native-web forwards onPointerDown and onFocus to the DOM but drops their Capture variants. */
export function findFocusProps(id: FindTargetId): object {
  const mark = () => findController.markFocus(id);
  return { onPointerDown: mark, onFocus: mark };
}
