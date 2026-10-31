// One owner for everything the desktop workbench remembers: the dock lanes (right tabs, bottom
// terminal), the pane grid (several sessions on one screen) and every size the user dragged.
// Persisted together under `forge.workbench.v2`, migrating the old `forge.splitPanes` entry.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { usePathname } from "expo-router";
import React, { createContext, useCallback, useContext, useEffect, useMemo, useReducer, useRef, useState } from "react";

import { routedSessionId } from "../shell/activeSession";
import { openSession as openRoutedSession } from "../../lib/nav";
import { useSessionActivationOverride } from "../../lib/sessionPrewarm";
import { useSessions } from "../../lib/queries";
import {
  LEGACY_SPLIT_KEY,
  WORKBENCH_STORAGE_KEY,
  addPane,
  cycleFocus,
  focusPane,
  initialLayout,
  openFromPane,
  withSecondary,
  migrateLegacySplit,
  openInFocused,
  openInNewPane,
  openInPane,
  parseLayout,
  promoteFirstSecondary,
  removePane,
  serializeLayout,
  swapSecondary,
  swapWithPrimary,
  toggleMaximize,
  type PaneLayoutState,
  type PaneSlot,
  type PaneWeights,
  type PresetName,
} from "./layoutModel";
import { adjacentSession, recentAt, touchRecent } from "./sessionRecency";
import {
  INITIAL_WORKBENCH_STATE,
  createWorkbenchSurface,
  parseWorkbenchState,
  restorableWorkbenchState,
  workbenchReducer,
  type WorkbenchPlacement,
  type WorkbenchState,
  type WorkbenchSurfaceInput,
} from "./model";

export type OpenMode = "focused" | "new" | { pane: number; tap?: boolean };

export interface WorkbenchController {
  state: WorkbenchState;
  openSurface: (input: WorkbenchSurfaceInput) => void;
  toggleSurface: (input: WorkbenchSurfaceInput) => void;
  activateSurface: (placement: WorkbenchPlacement, id: string) => void;
  closeSurface: (placement: WorkbenchPlacement, id: string) => void;
  hidePlacement: (placement: WorkbenchPlacement) => void;
  reorderSurface: (placement: WorkbenchPlacement, from: number, to: number) => void;

  layout: PaneLayoutState;
  /** The routed session — pane 0. */
  primaryId: string | null;
  /** `[primary, ...secondary]`. */
  paneIds: PaneSlot[];
  /** Session the docks and pane shortcuts act on: the focused pane's. */
  focusedSessionId: string | null;
  addPane: (slot?: PaneSlot) => void;
  closePane: (index: number) => void;
  focusPane: (index: number) => void;
  cycleFocus: (direction: 1 | -1) => void;
  toggleMaximize: (index?: number) => void;
  swapWithPrimary: (index: number) => void;
  swapPanes: (a: number, b: number) => void;
  openSession: (id: string, mode?: OpenMode) => void;
  /** Open the next/previous session of the list in the focused pane, wrapping. */
  stepSession: (delta: 1 | -1) => void;
  /** Ctrl+Tab: each press goes one further back through recently used sessions until the modifier is released. */
  cycleRecent: () => void;
  setPaneWeights: (key: keyof PaneWeights, weights: number[]) => void;
  setRightWidth: (width: number | null) => void;
  setBottomHeight: (height: number | null) => void;
  applyPreset: (preset: PresetName) => void;
  /** Back to one pane, no docks, default sizes. */
  resetLayout: () => void;
}

const WorkbenchContext = createContext<WorkbenchController | null>(null);

type LayoutAction =
  | { type: "set"; layout: PaneLayoutState }
  | { type: "update"; update: (layout: PaneLayoutState) => PaneLayoutState };

function layoutReducer(state: PaneLayoutState, action: LayoutAction): PaneLayoutState {
  return action.type === "set" ? action.layout : action.update(state);
}

const PERSIST_DELAY_MS = 250;
const DUPLICATE_GRACE_MS = 800;

export function WorkbenchProvider({ children }: { children: React.ReactNode }) {
  const [state, dispatch] = useReducer(workbenchReducer, INITIAL_WORKBENCH_STATE);
  const [layout, dispatchLayout] = useReducer(layoutReducer, undefined, initialLayout);
  const [hydrated, setHydrated] = useState(false);
  const pathname = usePathname();
  // A rail click names the open session before the URL does (see activateSession), so pane
  // layout decisions made in that window already agree with what is on screen.
  const activationOverride = useSessionActivationOverride();
  const primaryId = activationOverride ?? routedSessionId(pathname);
  const { data: sessions } = useSessions();

  // Handlers read the freshest layout/primary without being re-created on every pane change.
  const latest = useRef({ layout, primaryId, sessions });
  useEffect(() => {
    latest.current = { layout, primaryId, sessions };
  });

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const raw = await AsyncStorage.getItem(WORKBENCH_STORAGE_KEY);
        if (raw) {
          const parsed: unknown = JSON.parse(raw);
          const record = (typeof parsed === "object" && parsed !== null ? parsed : {}) as { layout?: unknown; lanes?: unknown };
          if (!cancelled) {
            dispatchLayout({ type: "set", layout: parseLayout(record.layout) });
            dispatch({ type: "replace", state: parseWorkbenchState(record.lanes) });
          }
        } else {
          const legacy = migrateLegacySplit(await AsyncStorage.getItem(LEGACY_SPLIT_KEY));
          if (legacy && !cancelled) dispatchLayout({ type: "set", layout: legacy });
        }
      } catch {
        // A corrupt or unreadable entry just means the workbench starts from its defaults.
      }
      if (!cancelled) setHydrated(true);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!hydrated) return;
    const timer = setTimeout(() => {
      const payload = { version: 2, layout: serializeLayout(layout), lanes: restorableWorkbenchState(state) };
      void AsyncStorage.setItem(WORKBENCH_STORAGE_KEY, JSON.stringify(payload)).catch(() => undefined);
    }, PERSIST_DELAY_MS);
    return () => clearTimeout(timer);
  }, [hydrated, layout, state]);

  // A pane must never duplicate pane 0, nor point at a session the fleet no longer has (archived
  // elsewhere, or a stale id from a previous launch) — those slots fall back to the picker.
  useEffect(() => {
    if (sessions == null) return;
    const known = new Set(sessions.map((row) => row.id));
    if (!layout.secondary.some((slot) => slot != null && !known.has(slot))) return;
    dispatchLayout({
      type: "update",
      update: (current) => ({ ...current, secondary: current.secondary.map((slot) => (slot != null && !known.has(slot) ? null : slot)) }),
    });
  }, [sessions, layout.secondary]);

  // A duplicate is only pruned once it has outlasted the route change that can cause it: a swap
  // puts the old primary into a slot a beat before the router lands on the new one.
  useEffect(() => {
    const seen = new Set<string>(primaryId ? [primaryId] : []);
    const duplicated = layout.secondary.some((slot) => {
      if (slot == null) return false;
      if (seen.has(slot)) return true;
      seen.add(slot);
      return false;
    });
    if (!duplicated) return;
    const timer = setTimeout(() => {
      const current = latest.current;
      const taken = new Set<string>(current.primaryId ? [current.primaryId] : []);
      const secondary = current.layout.secondary.map((slot) => {
        if (slot == null) return slot;
        if (taken.has(slot)) return null;
        taken.add(slot);
        return slot;
      });
      dispatchLayout({ type: "update", update: (layoutNow) => ({ ...layoutNow, secondary }) });
    }, DUPLICATE_GRACE_MS);
    return () => clearTimeout(timer);
  }, [primaryId, layout.secondary]);

  const update = useCallback((fn: (layout: PaneLayoutState) => PaneLayoutState) => {
    dispatchLayout({ type: "update", update: fn });
  }, []);

  const openSurface = useCallback((input: WorkbenchSurfaceInput) => {
    dispatch({ type: "open", surface: createWorkbenchSurface(input) });
  }, []);
  const toggleSurface = useCallback((input: WorkbenchSurfaceInput) => {
    dispatch({ type: "toggle", surface: createWorkbenchSurface(input) });
  }, []);
  const activateSurface = useCallback((placement: WorkbenchPlacement, id: string) => {
    dispatch({ type: "activate", placement, id });
  }, []);
  const closeSurface = useCallback((placement: WorkbenchPlacement, id: string) => {
    dispatch({ type: "close", placement, id });
  }, []);
  const hidePlacement = useCallback((placement: WorkbenchPlacement) => {
    dispatch({ type: "hide", placement });
  }, []);
  const reorderSurface = useCallback((placement: WorkbenchPlacement, from: number, to: number) => {
    dispatch({ type: "reorder", placement, from, to });
  }, []);

  const nextUnopened = useCallback((): string | null => {
    const { layout: current, primaryId: primary, sessions: rows } = latest.current;
    const open = new Set<string>([...(primary ? [primary] : []), ...current.secondary.filter((slot): slot is string => slot != null)]);
    return rows?.find((row) => !open.has(row.id))?.id ?? null;
  }, []);

  const addPaneAction = useCallback(
    (slot?: PaneSlot) => {
      const next = slot === undefined ? nextUnopened() : slot;
      update((current) => addPane(current, next));
    },
    [nextUnopened, update],
  );

  const closePane = useCallback(
    (index: number) => {
      if (index === 0) {
        const result = promoteFirstSecondary(latest.current.layout);
        if (result.promote) openRoutedSession(result.promote);
        dispatchLayout({ type: "set", layout: result.state });
        return;
      }
      update((current) => removePane(current, index));
    },
    [update],
  );

  const focusPaneAction = useCallback((index: number) => update((current) => focusPane(current, index)), [update]);
  const cycleFocusAction = useCallback((direction: 1 | -1) => update((current) => cycleFocus(current, direction)), [update]);
  const toggleMaximizeAction = useCallback(
    (index?: number) => update((current) => toggleMaximize(current, index ?? current.focus)),
    [update],
  );

  const swapWithPrimaryAction = useCallback((index: number) => {
    const { layout: current, primaryId: primary } = latest.current;
    const result = swapWithPrimary(current, primary, index);
    if (!result.promote) return;
    dispatchLayout({ type: "set", layout: result.state });
    openRoutedSession(result.promote);
  }, []);

  const swapPanes = useCallback((a: number, b: number) => update((current) => swapSecondary(current, a, b)), [update]);

  const openSession = useCallback((id: string, mode: OpenMode = "focused") => {
    const { layout: current, primaryId: primary } = latest.current;
    if (mode === "new") {
      dispatchLayout({ type: "set", layout: openInNewPane(current, primary, id) });
      return;
    }
    if (typeof mode === "object" && mode.tap) {
      dispatchLayout({ type: "set", layout: openFromPane(current, primary, mode.pane, id) });
      return;
    }
    const result =
      mode === "focused" ? openInFocused(current, primary, id) : openInPane(current, primary, mode.pane, id);
    dispatchLayout({ type: "set", layout: result.state });
    if (result.routeTo) openRoutedSession(result.routeTo);
  }, []);

  const recent = useRef<string[]>([]);
  const cycle = useRef<{ base: string[]; index: number } | null>(null);
  useEffect(() => {
    if (primaryId) recent.current = touchRecent(recent.current, primaryId);
  }, [primaryId]);
  useEffect(() => {
    // React Native defines `window` without DOM events; only the web/desktop build has keyup.
    if (typeof window === "undefined" || typeof window.addEventListener !== "function") return;
    const release = (event: KeyboardEvent) => {
      if (event.key === "Control" || event.key === "Meta") cycle.current = null;
    };
    window.addEventListener("keyup", release);
    return () => window.removeEventListener("keyup", release);
  }, []);

  const stepSession = useCallback((delta: 1 | -1) => {
    const { sessions: rows } = latest.current;
    const focused = latest.current.layout.focus === 0
      ? latest.current.primaryId
      : (latest.current.layout.secondary[latest.current.layout.focus - 1] ?? latest.current.primaryId);
    const next = adjacentSession((rows ?? []).map((row) => row.id), focused, delta);
    if (next) openSession(next);
  }, [openSession]);

  const cycleRecent = useCallback(() => {
    const { sessions: rows } = latest.current;
    const known = new Set((rows ?? []).map((row) => row.id));
    const run = cycle.current ?? { base: recent.current, index: 0 };
    run.index += 1;
    const next = recentAt(run.base, known, run.index);
    if (!next) return;
    cycle.current = run;
    openSession(next);
  }, [openSession]);

  const setPaneWeights = useCallback(
    (key: keyof PaneWeights, weights: number[]) => update((current) => ({ ...current, weights: { ...current.weights, [key]: weights } })),
    [update],
  );
  const setRightWidth = useCallback(
    (width: number | null) => update((current) => ({ ...current, dock: { ...current.dock, right: width } })),
    [update],
  );
  const setBottomHeight = useCallback(
    (height: number | null) =>
      update((current) => ({ ...current, dock: { ...current.dock, bottom: height } })),
    [update],
  );

  const resetLayout = useCallback(() => {
    dispatchLayout({ type: "set", layout: initialLayout() });
    dispatch({ type: "replace", state: INITIAL_WORKBENCH_STATE });
  }, []);

  const applyPreset = useCallback((preset: PresetName) => {
    const { layout: current, primaryId: primary, sessions: rows } = latest.current;
    const open = new Set<string>(primary ? [primary] : []);
    const pick = (): PaneSlot => {
      const id = rows?.find((row) => !open.has(row.id))?.id ?? null;
      if (id) open.add(id);
      return id;
    };
    const keep = current.secondary.filter((slot): slot is string => slot != null && slot !== primary);
    for (const id of keep) open.add(id);
    const base: PaneLayoutState = { ...current, focus: 0, maximized: null };
    const hideDocks: WorkbenchState = {
      right: { ...INITIAL_WORKBENCH_STATE.right },
      bottom: { ...INITIAL_WORKBENCH_STATE.bottom },
    };
    if (preset === "focus") {
      dispatchLayout({ type: "set", layout: withSecondary(base, []) });
      dispatch({ type: "replace", state: hideDocks });
    } else if (preset === "side-by-side") {
      dispatchLayout({ type: "set", layout: withSecondary(base, [keep[0] ?? pick()]) });
      dispatch({ type: "replace", state: hideDocks });
    } else if (preset === "grid") {
      const secondary: PaneSlot[] = keep.slice(0, 3);
      while (secondary.length < 3) secondary.push(pick());
      dispatchLayout({ type: "set", layout: withSecondary(base, secondary) });
      dispatch({ type: "replace", state: hideDocks });
    } else {
      dispatchLayout({ type: "set", layout: withSecondary(base, []) });
      const git = createWorkbenchSurface({ kind: "git" });
      const tasks = createWorkbenchSurface({ kind: "tasks" });
      dispatch({
        type: "replace",
        state: { right: { tabs: [git, tasks], activeId: git.id, visible: true }, bottom: hideDocks.bottom },
      });
    }
  }, []);

  const focusedSessionId =
    layout.focus === 0 ? primaryId : (layout.secondary[layout.focus - 1] ?? primaryId);
  const paneIds = useMemo<PaneSlot[]>(() => [primaryId, ...layout.secondary], [primaryId, layout.secondary]);

  const value = useMemo<WorkbenchController>(
    () => ({
      state,
      openSurface,
      toggleSurface,
      activateSurface,
      closeSurface,
      hidePlacement,
      reorderSurface,
      layout,
      primaryId,
      paneIds,
      focusedSessionId,
      addPane: addPaneAction,
      closePane,
      focusPane: focusPaneAction,
      cycleFocus: cycleFocusAction,
      toggleMaximize: toggleMaximizeAction,
      swapWithPrimary: swapWithPrimaryAction,
      swapPanes,
      openSession,
      stepSession,
      cycleRecent,
      setPaneWeights,
      setRightWidth,
      setBottomHeight,
      applyPreset,
      resetLayout,
    }),
    [
      state, openSurface, toggleSurface, activateSurface, closeSurface, hidePlacement, reorderSurface,
      layout, primaryId, paneIds, focusedSessionId, addPaneAction, closePane, focusPaneAction,
      cycleFocusAction, toggleMaximizeAction, swapWithPrimaryAction, swapPanes, openSession, stepSession, cycleRecent,
      setPaneWeights, setRightWidth, setBottomHeight, applyPreset, resetLayout,
    ],
  );

  return <WorkbenchContext.Provider value={value}>{children}</WorkbenchContext.Provider>;
}

export function useOptionalWorkbench(): WorkbenchController | null {
  return useContext(WorkbenchContext);
}

export function useWorkbench(): WorkbenchController {
  const controller = useContext(WorkbenchContext);
  if (!controller) {
    throw new Error("useWorkbench must be used inside WorkbenchProvider");
  }
  return controller;
}

