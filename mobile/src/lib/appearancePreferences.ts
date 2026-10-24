import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

const STORAGE_KEY = "forge.appearance.v1";

export interface StoredAppearance {
  wrapCodeBlocks: boolean;
  /** Fold each run of tool calls into one summary row (the TUI's `[tui] fold_tool_groups`). */
  foldToolGroups: boolean;
}

export interface AppearancePreferences extends StoredAppearance {
  loaded: boolean;
}

const STORED_DEFAULTS: StoredAppearance = {
  wrapCodeBlocks: false,
  foldToolGroups: true,
};

let state: AppearancePreferences = { loaded: false, ...STORED_DEFAULTS };
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

function replaceState(next: AppearancePreferences): void {
  state = next;
  emit();
}

export function parseAppearancePreferences(raw: string | null): StoredAppearance {
  if (!raw) return { ...STORED_DEFAULTS };
  try {
    const parsed = JSON.parse(raw) as { wrapCodeBlocks?: unknown; foldToolGroups?: unknown };
    return {
      wrapCodeBlocks: parsed?.wrapCodeBlocks === true,
      foldToolGroups: parsed?.foldToolGroups !== false,
    };
  } catch {
    return { ...STORED_DEFAULTS };
  }
}

async function ensureLoaded(): Promise<void> {
  if (state.loaded) return;
  if (hydration) return hydration;
  hydration = AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => replaceState({ loaded: true, ...parseAppearancePreferences(raw) }))
    .catch(() => replaceState({ loaded: true, ...STORED_DEFAULTS }));
  return hydration;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureLoaded();
  return () => listeners.delete(listener);
}

function snapshot(): AppearancePreferences {
  return state;
}

export function useAppearancePreferences(): AppearancePreferences {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

async function update<K extends keyof StoredAppearance>(key: K, value: StoredAppearance[K]): Promise<void> {
  await ensureLoaded();
  const previous = state;
  const next: AppearancePreferences = { ...state, loaded: true, [key]: value };
  replaceState(next);
  const stored: StoredAppearance = { wrapCodeBlocks: next.wrapCodeBlocks, foldToolGroups: next.foldToolGroups };
  try {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(stored));
  } catch (error) {
    if (state[key] === value) replaceState(previous);
    throw error;
  }
}

export function setWrapCodeBlocks(wrapCodeBlocks: boolean): Promise<void> {
  return update("wrapCodeBlocks", wrapCodeBlocks);
}

export function setFoldToolGroups(foldToolGroups: boolean): Promise<void> {
  return update("foldToolGroups", foldToolGroups);
}
