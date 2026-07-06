// T5.1 — global desktop/web navigation shortcuts: Cmd/Ctrl+1..4 jump to the four tab
// routes (Fleet/Inbox/History/Settings), Cmd/Ctrl+N opens New Session. Built on the same
// `useHotkey` registry T4.2 wired ⌘K through; native's `useHotkeys.ts` twin makes every
// call here a no-op there, so this hook needs no platform branching of its own. Mounted
// once at the app root (HANDOFF in src/app/_layout.tsx).
import { router } from "expo-router";

import { useHotkey } from "./useHotkeys";

const TAB_ROUTES = ["/", "/inbox", "/history", "/settings"] as const;

export function useGlobalShortcuts(): void {
  useHotkey("1", () => router.push(TAB_ROUTES[0]), { meta: true });
  useHotkey("2", () => router.push(TAB_ROUTES[1]), { meta: true });
  useHotkey("3", () => router.push(TAB_ROUTES[2]), { meta: true });
  useHotkey("4", () => router.push(TAB_ROUTES[3]), { meta: true });
  useHotkey("n", () => router.push("/new-session"), { meta: true });
}
