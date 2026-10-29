// Stored notification preferences (categories, filter, sound, hide-text). The rules that read
// them live in `notifyPrefsCore.ts`; this is the AsyncStorage-backed store and its hook.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

import { DEFAULT_NOTIFY_PREFS, NOTIFY_PREFS_KEY, parseNotifyPrefs, type NotifyPrefs } from "./notifyPrefsCore";

export interface LoadedNotifyPrefs extends NotifyPrefs {
  loaded: boolean;
}

let state: LoadedNotifyPrefs = { loaded: false, ...DEFAULT_NOTIFY_PREFS };
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

function replaceState(next: LoadedNotifyPrefs): void {
  state = next;
  for (const listener of listeners) listener();
}

export function ensureNotifyPrefsLoaded(): Promise<void> {
  if (state.loaded) return Promise.resolve();
  hydration ??= AsyncStorage.getItem(NOTIFY_PREFS_KEY)
    .then((raw) => replaceState({ loaded: true, ...parseNotifyPrefs(raw) }))
    .catch(() => replaceState({ loaded: true, ...DEFAULT_NOTIFY_PREFS }));
  return hydration;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureNotifyPrefsLoaded();
  return () => listeners.delete(listener);
}

function snapshot(): LoadedNotifyPrefs {
  return state;
}

export function useNotifyPrefs(): LoadedNotifyPrefs {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export async function getNotifyPrefs(): Promise<NotifyPrefs> {
  await ensureNotifyPrefsLoaded();
  return state;
}

export async function setNotifyPrefs(patch: Partial<NotifyPrefs>): Promise<void> {
  await ensureNotifyPrefsLoaded();
  const previous = state;
  const next: LoadedNotifyPrefs = { ...state, ...patch, loaded: true };
  replaceState(next);
  const { loaded: _loaded, ...stored } = next;
  try {
    await AsyncStorage.setItem(NOTIFY_PREFS_KEY, JSON.stringify(stored));
  } catch (error) {
    if (state === next) replaceState(previous);
    throw error;
  }
}
