import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

const STORAGE_KEY = "forge.usage.hidden.v1";

let hidden: readonly string[] = [];
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

export function parseHiddenProviders(raw: string | null): string[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === "string") : [];
  } catch {
    return [];
  }
}

function replace(next: readonly string[]): void {
  hidden = next;
  for (const listener of listeners) listener();
}

function ensureLoaded(): Promise<void> {
  hydration ??= AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => replace(parseHiddenProviders(raw)))
    .catch(() => undefined);
  return hydration;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureLoaded();
  return () => listeners.delete(listener);
}

export function useHiddenProviders(): readonly string[] {
  return useSyncExternalStore(subscribe, () => hidden, () => hidden);
}

export async function setProviderHidden(provider: string, hide: boolean): Promise<void> {
  await ensureLoaded();
  const next = hide ? [...new Set([...hidden, provider])] : hidden.filter((name) => name !== provider);
  replace(next);
  try {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // The choice still holds for this run; it just will not survive a restart.
  }
}
