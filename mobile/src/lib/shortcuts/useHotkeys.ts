// Native no-op twin of useHotkeys.web.ts — hardware ⌘K isn't a native input path
// (BUILD_ORDER T4.2: native opens the palette via an IconButton affordance instead).
// Kept symbol-compatible so callers never branch on platform themselves.
export type HotkeyHandler = () => void;

export function useHotkey(
  _key: string,
  _handler: HotkeyHandler,
  _options?: { meta?: boolean; alt?: boolean },
): void {
  // no-op on native
}

export function usePaletteHotkey(_onOpen: HotkeyHandler): void {
  // no-op on native
}
