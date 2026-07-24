// Desktop shell shortcuts (Machined wave 2), layered on the same `useHotkey` registry
// T5.1 wired ⌘K/⌘1..4/⌘N through — named wrappers so call sites in shell/ and
// CommandPalette read as intent, matching `usePaletteHotkey`'s convention. Each is a
// thin `useHotkey` call, so it's a no-op on native for free (useHotkey's own platform
// twin handles that) — no branching needed here.
import { useHotkey } from "./useHotkeys";

/** ⌘\ — collapse/expand the desktop sidebar to the icon rail. */
export function useSidebarCollapseHotkey(onToggle: () => void): void {
  useHotkey("\\", onToggle, { meta: true });
}

/** ⌘U — toggle the right-hand usage dock. */
export function useUsageDockHotkey(onToggle: () => void): void {
  useHotkey("u", onToggle, { meta: true });
}

/** ⌥Space — open the in-window quick composer. */
export function useQuickComposerHotkey(onOpen: () => void): void {
  useHotkey(" ", onOpen, { alt: true });
}

/** ⌘P — open the command palette straight into thread-search mode. */
export function useThreadSearchHotkey(onOpen: () => void): void {
  useHotkey("p", onOpen, { meta: true });
}
