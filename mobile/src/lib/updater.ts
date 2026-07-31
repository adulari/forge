import { useSyncExternalStore } from "react";

import { isTauri } from "./platform";

export type DesktopUpdatePhase =
  | "idle"
  | "checking"
  | "up-to-date"
  | "available"
  | "installing"
  | "error";

export interface DesktopUpdateState {
  phase: DesktopUpdatePhase;
  checkedAt: number | null;
  availableVersion: string | null;
  body: string | null;
  message: string | null;
}

type InstallableUpdate = {
  version: string;
  body?: string | null;
  downloadAndInstall: () => Promise<void>;
};

const listeners = new Set<() => void>();
let snapshot: DesktopUpdateState = {
  phase: "idle",
  checkedAt: null,
  availableVersion: null,
  body: null,
  message: null,
};
let installable: InstallableUpdate | null = null;
let checking: Promise<void> | null = null;
let installing: Promise<void> | null = null;

function publish(next: DesktopUpdateState): void {
  snapshot = next;
  for (const listener of listeners) listener();
}

function messageFor(error: unknown, fallback: string): string {
  return error instanceof Error && error.message.trim() ? error.message : fallback;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getDesktopUpdateState(): DesktopUpdateState {
  return snapshot;
}

export function useDesktopUpdateState(): DesktopUpdateState {
  return useSyncExternalStore(subscribe, getDesktopUpdateState, getDesktopUpdateState);
}

/** Shared, de-duplicated updater check used by launch, Settings, and Diagnostics. */
export function checkDesktopUpdate(): Promise<void> {
  if (!isTauri) return Promise.resolve();
  if (checking) return checking;

  publish({
    ...snapshot,
    phase: "checking",
    message: null,
  });
  checking = (async () => {
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const update = await check();
      const checkedAt = Date.now();
      if (!update) {
        installable = null;
        publish({
          phase: "up-to-date",
          checkedAt,
          availableVersion: null,
          body: null,
          message: null,
        });
        return;
      }
      installable = update;
      publish({
        phase: "available",
        checkedAt,
        availableVersion: update.version,
        body: update.body ?? null,
        message: null,
      });
    } catch (error) {
      publish({
        ...snapshot,
        phase: "error",
        checkedAt: Date.now(),
        message: messageFor(error, "Could not check for desktop updates."),
      });
      throw error;
    } finally {
      checking = null;
    }
  })();
  return checking;
}

/** Downloads the discovered signed update, installs it, and relaunches Forge. */
export function installDesktopUpdate(): Promise<void> {
  if (installing) return installing;
  if (!installable) {
    return Promise.reject(new Error("No desktop update is ready to install."));
  }

  publish({ ...snapshot, phase: "installing", message: null });
  installing = (async () => {
    try {
      await installable?.downloadAndInstall();
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch (error) {
      publish({
        ...snapshot,
        phase: "error",
        message: messageFor(error, "Could not install the desktop update."),
      });
      throw error;
    } finally {
      installing = null;
    }
  })();
  return installing;
}
