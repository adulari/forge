// Web: a small ⌘/Ctrl+<key> hotkey registry backed by ONE window `keydown` listener
// (not one per hook instance). T4.2 only wires ⌘K (the palette) through this; T5.1 adds
// ⌘1..4/⌘N/⌘Enter on top of the same `useHotkey` primitive — do not build those here.
import { useEffect } from "react";

import { normalizeShortcutKey } from "./bindings";

export type HotkeyHandler = () => void;

interface HotkeyEntry {
  key: string;
  meta: boolean;
  alt: boolean;
  shift: boolean;
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
    const alt = e.altKey;
    const shift = e.shiftKey;
    const key = normalizeShortcutKey(e.key);
    if (!key) return;
    for (const entry of registry) {
      if (entry.meta !== meta || entry.alt !== alt || entry.shift !== shift || entry.key !== key) continue;
      // A Mod/Alt combo always fires even while a text field is focused (that's the
      // point of global app shortcuts); an unmodified or Shift-only key never does.
      if (!meta && !alt && isTypingTarget(e.target)) continue;
      e.preventDefault();
      entry.handler();
    }
  });
}

/**
 * Registers a ⌘/Ctrl+<key> (or ⌥/Alt+<key>) combo for as long as the calling component is
 * mounted. `alt: true` exists because ⌘/Ctrl+1..9 is a hard OS/browser-chrome-level tab
 * switcher on every major browser — it never reaches page JS, so preventDefault can't
 * intercept it. Digit shortcuts must use `alt` instead of `meta` to actually fire.
 */
export function useHotkey(
  key: string,
  handler: HotkeyHandler,
  options?: { meta?: boolean; alt?: boolean; shift?: boolean },
): void {
  const meta = options?.meta ?? (options?.alt || options?.shift ? false : true);
  const alt = options?.alt ?? false;
  const shift = options?.shift ?? false;
  useEffect(() => {
    ensureListener();
    const normalizedKey = normalizeShortcutKey(key);
    if (!normalizedKey) return;
    const entry: HotkeyEntry = { key: normalizedKey, meta, alt, shift, handler };
    registry.add(entry);
    return () => {
      registry.delete(entry);
    };
  }, [key, meta, alt, shift, handler]);
}
