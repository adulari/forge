import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

const STORAGE_KEY = "forge.desktopShell.v1";

export const USAGE_REFRESH_DEFAULT_MINUTES = 5;
export const USAGE_REFRESH_MIN_MINUTES = 2;
export const USAGE_REFRESH_MAX_MINUTES = 60;

export interface StoredDesktopShellPrefs {
  /** OS notifications while Forge is in the background. On unless switched off. */
  notifyWhenBackground: boolean;
  /** How often the account-wide usage figures refresh while the window is open. */
  usageRefreshMinutes: number;
}

export interface DesktopShellPrefs extends StoredDesktopShellPrefs {
  loaded: boolean;
}

const DEFAULTS: StoredDesktopShellPrefs = {
  notifyWhenBackground: true,
  usageRefreshMinutes: USAGE_REFRESH_DEFAULT_MINUTES,
};

export function clampUsageRefreshMinutes(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return USAGE_REFRESH_DEFAULT_MINUTES;
  return Math.min(USAGE_REFRESH_MAX_MINUTES, Math.max(USAGE_REFRESH_MIN_MINUTES, Math.round(value)));
}

export function parseDesktopShellPrefs(raw: string | null): StoredDesktopShellPrefs {
  if (!raw) return { ...DEFAULTS };
  try {
    const parsed = JSON.parse(raw) as { notifyWhenBackground?: unknown; usageRefreshMinutes?: unknown } | null;
    return {
      notifyWhenBackground: parsed?.notifyWhenBackground !== false,
      usageRefreshMinutes: clampUsageRefreshMinutes(parsed?.usageRefreshMinutes ?? USAGE_REFRESH_DEFAULT_MINUTES),
    };
  } catch {
    return { ...DEFAULTS };
  }
}

/** Per-session usage keeps its live 30 s cadence; only the account-wide view follows the setting. */
export function usageRefetchInterval(sessionId: string | null | undefined, minutes: number): number {
  return sessionId ? 30_000 : clampUsageRefreshMinutes(minutes) * 60_000;
}

let state: DesktopShellPrefs = { loaded: false, ...DEFAULTS };
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

function replaceState(next: DesktopShellPrefs): void {
  state = next;
  for (const listener of listeners) listener();
}

export function ensureDesktopShellPrefsLoaded(): Promise<void> {
  if (state.loaded) return Promise.resolve();
  hydration ??= AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => replaceState({ loaded: true, ...parseDesktopShellPrefs(raw) }))
    .catch(() => replaceState({ loaded: true, ...DEFAULTS }));
  return hydration;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureDesktopShellPrefsLoaded();
  return () => listeners.delete(listener);
}

function snapshot(): DesktopShellPrefs {
  return state;
}

export function useDesktopShellPrefs(): DesktopShellPrefs {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export async function getDesktopShellPrefs(): Promise<DesktopShellPrefs> {
  await ensureDesktopShellPrefsLoaded();
  return state;
}

async function update(patch: Partial<StoredDesktopShellPrefs>): Promise<void> {
  await ensureDesktopShellPrefsLoaded();
  const previous = state;
  const next: DesktopShellPrefs = { ...state, ...patch, loaded: true };
  next.usageRefreshMinutes = clampUsageRefreshMinutes(next.usageRefreshMinutes);
  replaceState(next);
  const stored: StoredDesktopShellPrefs = {
    notifyWhenBackground: next.notifyWhenBackground,
    usageRefreshMinutes: next.usageRefreshMinutes,
  };
  try {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(stored));
  } catch (error) {
    if (state === next) replaceState(previous);
    throw error;
  }
}

export function setNotifyWhenBackground(notifyWhenBackground: boolean): Promise<void> {
  return update({ notifyWhenBackground });
}

export function setUsageRefreshMinutes(usageRefreshMinutes: number): Promise<void> {
  return update({ usageRefreshMinutes });
}
