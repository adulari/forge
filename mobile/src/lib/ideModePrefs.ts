// IDE mode's per-viewer state: whether it is on, how wide its panes are, and whether the
// first-run coach marks were seen. Same shape as appearancePreferences.ts: one JSON blob,
// hydrated lazily, every storage call guarded so a blocked store leaves the in-memory value.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

const STORAGE_KEY = "forge.ideMode.v1";

export const IDE_RAIL_WIDTH = { min: 220, max: 420, default: 260 } as const;
export const IDE_AGENT_WIDTH = { min: 280, max: 520, default: 340 } as const;
export const IDE_BOTTOM_HEIGHT = { min: 120, max: 640, default: 240 } as const;

export interface StoredIdeMode {
  enabled: boolean;
  railWidth: number;
  agentWidth: number;
  bottomHeight: number;
  coachSeen: boolean;
}

export interface IdeModePreferences extends StoredIdeMode {
  loaded: boolean;
}

const STORED_DEFAULTS: StoredIdeMode = {
  enabled: false,
  railWidth: IDE_RAIL_WIDTH.default,
  agentWidth: IDE_AGENT_WIDTH.default,
  bottomHeight: IDE_BOTTOM_HEIGHT.default,
  coachSeen: false,
};

export function clampWidth(range: { min: number; max: number }, value: number): number {
  return Math.round(Math.max(range.min, Math.min(range.max, value)));
}

function sized(range: { min: number; max: number; default: number }, value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? clampWidth(range, value) : range.default;
}

export function parseIdeModePreferences(raw: string | null): StoredIdeMode {
  if (!raw) return { ...STORED_DEFAULTS };
  try {
    const parsed = JSON.parse(raw) as Partial<Record<keyof StoredIdeMode, unknown>> | null;
    return {
      enabled: parsed?.enabled === true,
      railWidth: sized(IDE_RAIL_WIDTH, parsed?.railWidth),
      agentWidth: sized(IDE_AGENT_WIDTH, parsed?.agentWidth),
      bottomHeight: sized(IDE_BOTTOM_HEIGHT, parsed?.bottomHeight),
      coachSeen: parsed?.coachSeen === true,
    };
  } catch {
    return { ...STORED_DEFAULTS };
  }
}

let state: IdeModePreferences = { loaded: false, ...STORED_DEFAULTS };
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

function replaceState(next: IdeModePreferences): void {
  state = next;
  for (const listener of listeners) listener();
}

function ensureLoaded(): Promise<void> {
  if (state.loaded) return Promise.resolve();
  if (hydration) return hydration;
  hydration = AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => {
      // A write made before hydration finished wins over what was on disk.
      if (!state.loaded) replaceState({ loaded: true, ...parseIdeModePreferences(raw) });
    })
    .catch(() => {
      if (!state.loaded) replaceState({ ...state, loaded: true });
    });
  return hydration;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureLoaded();
  return () => {
    listeners.delete(listener);
  };
}

const snapshot = () => state;

export function useIdeModePreferences(): IdeModePreferences {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export function readIdeModePreferences(): IdeModePreferences {
  return state;
}

function persist(): void {
  const { loaded: _loaded, ...stored } = state;
  void AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(stored)).catch(() => undefined);
}

function patch(change: Partial<StoredIdeMode>, save = true): void {
  replaceState({ ...state, ...change, loaded: true });
  if (save) persist();
}

export function setIdeModeEnabled(enabled: boolean): void {
  patch({ enabled });
}

export function toggleIdeMode(): void {
  patch({ enabled: !state.enabled });
}

export function setIdeRailWidth(width: number, save = true): void {
  patch({ railWidth: clampWidth(IDE_RAIL_WIDTH, width) }, save);
}

export function setIdeAgentWidth(width: number, save = true): void {
  patch({ agentWidth: clampWidth(IDE_AGENT_WIDTH, width) }, save);
}

export function setIdeBottomHeight(height: number, save = true): void {
  patch({ bottomHeight: clampWidth(IDE_BOTTOM_HEIGHT, height) }, save);
}

export function persistIdeModePreferences(): void {
  persist();
}

export function markIdeCoachSeen(): void {
  patch({ coachSeen: true });
}

/** Test seam: back to the unhydrated defaults. */
export function resetIdeModePreferencesForTests(): void {
  hydration = null;
  state = { loaded: false, ...STORED_DEFAULTS };
}
