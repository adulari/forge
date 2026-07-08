// Feeds the iOS Home Screen widget (mobile/targets/widget) via @bacons/apple-targets'
// `ExtensionStorage` — an NSUserDefaults-backed App Group the widget extension reads directly
// (see targets/widget/ForgeSharedData.swift). `ExtensionStorage` itself no-ops safely when the
// native module isn't linked (Android, web, or an iOS build taken before `expo prebuild` wove
// the widget target in) — see node_modules/@bacons/apple-targets/build/ExtensionStorage.js —
// so this only needs its own `isIOS` gate to avoid touching a real app group on other platforms.
import { ExtensionStorage } from "@bacons/apple-targets";

import type { SessionRow } from "./api";
import { isIOS } from "./platform";

const APP_GROUP = "group.dev.adulari.forge";
const SESSIONS_KEY = "sessions";
const MAX_SESSIONS = 4;

// Field names must match targets/widget/ForgeSharedData.swift's `ForgeSessionSnapshot` exactly —
// a hand-kept-in-sync wire contract, same caveat as the Live Activity content-state.
interface WidgetSessionSnapshot {
  id: string;
  title: string;
  busy: boolean;
  waiting: boolean;
  cost_usd: number;
}

/** Sync the fleet list to the widget's shared storage and ask it to redraw. Waiting sessions
 * sort first, since the widget only ever shows a handful of rows. */
export function syncWidgetSessions(sessions: SessionRow[]): void {
  if (!isIOS) return;

  const sorted = [...sessions].sort((a, b) => Number(b.waiting) - Number(a.waiting));
  const snapshot: WidgetSessionSnapshot[] = sorted.slice(0, MAX_SESSIONS).map((s) => ({
    id: s.id,
    title: s.title,
    busy: s.busy,
    waiting: s.waiting,
    cost_usd: s.cost_usd,
  }));

  const storage = new ExtensionStorage(APP_GROUP);
  storage.set(SESSIONS_KEY, JSON.stringify(snapshot));
  ExtensionStorage.reloadWidget();
}
