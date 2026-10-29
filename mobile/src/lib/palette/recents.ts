// Which palette entries the person ran lately, newest first. It feeds the "then recency" tie-break
// in ranking, and orders Commands when the query is empty.
import AsyncStorage from "@react-native-async-storage/async-storage";

const KEY = "forge.palette.recents.v1";
export const MAX_RECENTS = 24;

export function pushRecent(list: readonly string[], id: string, max = MAX_RECENTS): string[] {
  return [id, ...list.filter((entry) => entry !== id)].slice(0, max);
}

/** Larger is more recent; 0 for an entry never run. */
export function recencyOf(list: readonly string[], id: string): number {
  const index = list.indexOf(id);
  return index < 0 ? 0 : list.length - index;
}

let recents: string[] = [];
let loading: Promise<void> | null = null;
const listeners = new Set<() => void>();

export function getRecents(): readonly string[] {
  return recents;
}

export function subscribeRecents(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function loadRecents(): Promise<void> {
  loading ??= AsyncStorage.getItem(KEY)
    .then((raw) => {
      const parsed: unknown = raw ? JSON.parse(raw) : [];
      if (Array.isArray(parsed)) {
        const stored = parsed.filter((entry): entry is string => typeof entry === "string");
        recents = [...recents, ...stored.filter((entry) => !recents.includes(entry))].slice(0, MAX_RECENTS);
        listeners.forEach((listener) => listener());
      }
    })
    .catch(() => undefined);
  return loading;
}

export function rememberRecent(id: string): void {
  recents = pushRecent(recents, id);
  listeners.forEach((listener) => listener());
  AsyncStorage.setItem(KEY, JSON.stringify(recents)).catch(() => undefined);
}
