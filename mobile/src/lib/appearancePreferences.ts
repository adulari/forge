import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

const STORAGE_KEY = "forge.appearance.v1";

export interface AppearancePreferences {
  loaded: boolean;
  wrapCodeBlocks: boolean;
}

const DEFAULTS: AppearancePreferences = {
  loaded: false,
  wrapCodeBlocks: false,
};

let state = DEFAULTS;
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

function replaceState(next: AppearancePreferences): void {
  state = next;
  emit();
}

export function parseAppearancePreferences(raw: string | null): Pick<AppearancePreferences, "wrapCodeBlocks"> {
  if (!raw) return { wrapCodeBlocks: false };
  try {
    const parsed = JSON.parse(raw) as { wrapCodeBlocks?: unknown };
    return { wrapCodeBlocks: parsed?.wrapCodeBlocks === true };
  } catch {
    return { wrapCodeBlocks: false };
  }
}

async function ensureLoaded(): Promise<void> {
  if (state.loaded) return;
  if (hydration) return hydration;
  hydration = AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => replaceState({ loaded: true, ...parseAppearancePreferences(raw) }))
    .catch(() => replaceState({ loaded: true, wrapCodeBlocks: false }));
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

export async function setWrapCodeBlocks(wrapCodeBlocks: boolean): Promise<void> {
  await ensureLoaded();
  const previous = state;
  replaceState({ loaded: true, wrapCodeBlocks });
  try {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify({ wrapCodeBlocks }));
  } catch (error) {
    if (state.wrapCodeBlocks === wrapCodeBlocks) replaceState(previous);
    throw error;
  }
}
