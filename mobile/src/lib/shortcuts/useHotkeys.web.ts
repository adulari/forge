// Web: a small ⌘/Ctrl+<key> hotkey registry backed by ONE window `keydown` listener
// (not one per hook instance). T4.2 only wires ⌘K (the palette) through this; T5.1 adds
// ⌘1..4/⌘N/⌘Enter on top of the same `useHotkey` primitive — do not build those here.
import { useEffect } from "react";

export type HotkeyHandler = () => void;

interface HotkeyEntry {
  key: string;
  meta: boolean;
  handler: HotkeyHandler;
}

const registry = new Set<HotkeyEntry>();
let listenerAttached = false;

function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || target.isContentEditable;
}

function ensureListener(): void {
  if (listenerAttached || typeof window === "undefined") return;
  listenerAttached = true;
  window.addEventListener("keydown", (e: KeyboardEvent) => {
    const meta = e.metaKey || e.ctrlKey;
    const key = e.key.toLowerCase();
    for (const entry of registry) {
      if (entry.meta !== meta || entry.key !== key) continue;
      // A meta-combo (⌘K) always fires even while a text field is focused (that's the
      // whole point of a global palette shortcut); a bare key never does.
      if (!meta && isTypingTarget(e.target)) continue;
      e.preventDefault();
      entry.handler();
    }
  });
}

/** Registers a ⌘/Ctrl+<key> combo for as long as the calling component is mounted. */
export function useHotkey(key: string, handler: HotkeyHandler, options?: { meta?: boolean }): void {
  const meta = options?.meta ?? true;
  useEffect(() => {
    ensureListener();
    const entry: HotkeyEntry = { key: key.toLowerCase(), meta, handler };
    registry.add(entry);
    return () => {
      registry.delete(entry);
    };
  }, [key, meta, handler]);
}

/** ⌘K / Ctrl+K opens the command palette (DESIGN_SYSTEM.md §6 CommandPalette). */
export function usePaletteHotkey(onOpen: HotkeyHandler): void {
  useHotkey("k", onOpen, { meta: true });
}
