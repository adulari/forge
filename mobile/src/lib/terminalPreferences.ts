import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

const STORAGE_KEY = "forge.terminal.v1";

export const TERMINAL_FONT_SIZES = [11, 12, 13, 14, 16] as const;
export const TERMINAL_SCROLL_SPEEDS = [0.5, 1, 2, 3] as const;

export interface StoredTerminalPreferences {
  fontSize: number;
  /** Multiplier on the wheel distance inside a terminal (web and desktop). */
  scrollSpeed: number;
  /** Ask before closing a terminal tab whose shell is still running. */
  confirmClose: boolean;
}

export interface TerminalPreferences extends StoredTerminalPreferences {
  loaded: boolean;
}

export const TERMINAL_DEFAULTS: StoredTerminalPreferences = {
  fontSize: 11,
  scrollSpeed: 1,
  confirmClose: true,
};

let state: TerminalPreferences = { loaded: false, ...TERMINAL_DEFAULTS };
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

function replaceState(next: TerminalPreferences): void {
  state = next;
  for (const listener of listeners) listener();
}

function oneOf<T extends number>(value: unknown, allowed: readonly T[], fallback: T): T {
  return allowed.find((candidate) => candidate === value) ?? fallback;
}

export function parseTerminalPreferences(raw: string | null): StoredTerminalPreferences {
  if (!raw) return { ...TERMINAL_DEFAULTS };
  try {
    const parsed = JSON.parse(raw) as Partial<Record<keyof StoredTerminalPreferences, unknown>> | null;
    return {
      fontSize: oneOf(parsed?.fontSize, TERMINAL_FONT_SIZES, TERMINAL_DEFAULTS.fontSize),
      scrollSpeed: oneOf(parsed?.scrollSpeed, TERMINAL_SCROLL_SPEEDS, TERMINAL_DEFAULTS.scrollSpeed),
      confirmClose: parsed?.confirmClose !== false,
    };
  } catch {
    return { ...TERMINAL_DEFAULTS };
  }
}

/** Line height that keeps a monospace row readable at any of the offered sizes. */
export function terminalLineHeight(fontSize: number): number {
  return Math.round(fontSize * 1.45);
}

async function ensureLoaded(): Promise<void> {
  if (state.loaded) return;
  if (hydration) return hydration;
  hydration = AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => replaceState({ loaded: true, ...parseTerminalPreferences(raw) }))
    .catch(() => replaceState({ loaded: true, ...TERMINAL_DEFAULTS }));
  return hydration;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureLoaded();
  return () => listeners.delete(listener);
}

function snapshot(): TerminalPreferences {
  return state;
}

export function useTerminalPreferences(): TerminalPreferences {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

/** The current value without subscribing, for event handlers. */
export function getTerminalPreferences(): TerminalPreferences {
  return state;
}

async function update<K extends keyof StoredTerminalPreferences>(
  key: K,
  value: StoredTerminalPreferences[K],
): Promise<void> {
  await ensureLoaded();
  const previous = state;
  const next: TerminalPreferences = { ...state, loaded: true, [key]: value };
  replaceState(next);
  const stored: StoredTerminalPreferences = {
    fontSize: next.fontSize,
    scrollSpeed: next.scrollSpeed,
    confirmClose: next.confirmClose,
  };
  try {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(stored));
  } catch (error) {
    if (state[key] === value) replaceState(previous);
    throw error;
  }
}

export const setTerminalFontSize = (value: number) => update("fontSize", value);
export const setTerminalScrollSpeed = (value: number) => update("scrollSpeed", value);
export const setTerminalConfirmClose = (value: boolean) => update("confirmClose", value);
