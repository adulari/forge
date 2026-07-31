import { type AppShortcutAction, useShortcutBinding } from "./bindings";
import { type HotkeyHandler, useHotkey } from "./useHotkeys";

/**
 * Registers one persisted desktop/web app action. Native mobile keeps the same
 * call graph but its platform-specific `useHotkey` twin is intentionally inert.
 */
export function useAppShortcut(action: AppShortcutAction, handler: HotkeyHandler): void {
  const binding = useShortcutBinding(action);
  useHotkey(binding.key, handler, {
    meta: binding.mod,
    alt: binding.alt,
    shift: binding.shift,
  });
}

/** Opens the command palette using its current user binding. */
export function usePaletteHotkey(onOpen: HotkeyHandler): void {
  useAppShortcut("command.palette", onOpen);
}
