import AsyncStorage from "@react-native-async-storage/async-storage";

const STORAGE_KEY = "forge.previewRecents.v1";
export const RECENT_LIMIT = 8;

/** Newest first, no duplicates (a trailing slash is the same address), capped. */
export function pushRecent(list: readonly string[], url: string, limit = RECENT_LIMIT): string[] {
  const key = (value: string) => value.replace(/\/$/, "");
  return [url, ...list.filter((entry) => key(entry) !== key(url))].slice(0, limit);
}

export function parseRecents(raw: string | null): string[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((entry): entry is string => typeof entry === "string").slice(0, RECENT_LIMIT);
  } catch {
    return [];
  }
}

export async function loadRecents(): Promise<string[]> {
  try {
    return parseRecents(await AsyncStorage.getItem(STORAGE_KEY));
  } catch {
    return [];
  }
}

export async function rememberRecent(url: string): Promise<string[]> {
  const next = pushRecent(await loadRecents(), url);
  try {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // A failed write only loses the history entry.
  }
  return next;
}

export async function forgetRecents(): Promise<void> {
  try {
    await AsyncStorage.removeItem(STORAGE_KEY);
  } catch {
    // Nothing stored to remove.
  }
}

/** `localhost:5173` for an address row; the whole string when it is not a URL. */
export function recentLabel(url: string): string {
  try {
    const parsed = new URL(url);
    const path = parsed.pathname === "/" ? "" : parsed.pathname;
    return `${parsed.host}${path}${parsed.search}`;
  } catch {
    return url;
  }
}

const LOOPBACK = new Set(["localhost", "127.0.0.1", "[::1]", "::1"]);

/** The web client can only frame loopback servers; arbitrary sites stay out of the app. */
export function isLoopbackUrl(url: string): boolean {
  try {
    return LOOPBACK.has(new URL(url).hostname.toLowerCase());
  } catch {
    return false;
  }
}
