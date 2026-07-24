// Tab badge (W Settings BEHAVIOR, L183) — web only: prefix the browser tab title with the
// number of sessions waiting on a decision.
//
// This lives in lib/ rather than beside the Settings screen that toggles it because the root
// layout has to mount `useTabBadgeTitle()` app-wide; importing a route module from the layout
// would pull a screen into expo-router's registry from the wrong direction.
//
// The preference is persisted in AsyncStorage, which has no change notification, so the toggle
// and the title effect share a tiny module-level store. Hydration happens once, on first read.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useEffect, useSyncExternalStore } from "react";

import { isWeb } from "./platform";
import { useSessions } from "./queries";

const TAB_BADGE_KEY = "forge.tabBadge";
const TAB_BADGE_PREFIX = /^\(\d+\)\s+/;

let tabBadgeEnabled = false;
let tabBadgeHydrated = false;
const tabBadgeListeners = new Set<() => void>();
/** Captured before the first badge is ever written, so a re-render can't badge a badged title. */
let baseDocumentTitle: string | null = null;

function subscribeTabBadge(listener: () => void): () => void {
  tabBadgeListeners.add(listener);
  return () => {
    tabBadgeListeners.delete(listener);
  };
}

/** Publish a new preference value to every mounted reader. Does not persist — see `setTabBadge`. */
export function publishTabBadge(value: boolean) {
  tabBadgeEnabled = value;
  for (const listener of tabBadgeListeners) listener();
}

/** Persist the preference. The caller publishes optimistically and rolls back on failure. */
export function persistTabBadge(value: boolean): Promise<void> {
  return AsyncStorage.setItem(TAB_BADGE_KEY, value ? "true" : "false");
}

export function useTabBadgePreference(): boolean {
  const enabled = useSyncExternalStore(
    subscribeTabBadge,
    () => tabBadgeEnabled,
    () => false,
  );
  useEffect(() => {
    if (tabBadgeHydrated) return;
    tabBadgeHydrated = true;
    void AsyncStorage.getItem(TAB_BADGE_KEY).then((raw) => publishTabBadge(raw === "true"));
  }, []);
  return enabled;
}

/**
 * Keeps `document.title` in sync with the waiting-decision count while the preference is on.
 * Mounted once in the root navigator so the count keeps tracking after the user navigates away
 * from Settings. A no-op off the web.
 */
export function useTabBadgeTitle() {
  const enabled = useTabBadgePreference();
  const sessions = useSessions();
  const waiting = (sessions.data ?? []).filter((session) => session.waiting).length;

  useEffect(() => {
    if (!isWeb || typeof document === "undefined") return;
    baseDocumentTitle ??= document.title.replace(TAB_BADGE_PREFIX, "");
    const base = baseDocumentTitle;
    document.title = enabled && waiting > 0 ? `(${waiting}) ${base}` : base;
    return () => {
      document.title = base;
    };
  }, [enabled, waiting]);
}
