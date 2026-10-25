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
  /**
   * Whether this install can replace itself. False on Linux unless Forge runs from a mounted
   * AppImage file: the updater would otherwise write an AppImage over a .deb, rpm, AUR or
   * unpacked install, so those get a link to the release page instead.
   */
  selfInstall: boolean;
  releaseUrl: string | null;
}

export const RELEASES_URL = "https://github.com/Adulari/forge/releases";

export function releasePageUrl(version: string): string {
  return `${RELEASES_URL}/tag/v${encodeURIComponent(version.replace(/^v/, ""))}`;
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
  selfInstall: true,
  releaseUrl: null,
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

async function canSelfInstall(): Promise<boolean> {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return (await invoke<boolean>("desktop_update_installable")) === true;
  } catch {
    return false;
  }
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
          selfInstall: snapshot.selfInstall,
          releaseUrl: null,
        });
        return;
      }
      const selfInstall = await canSelfInstall();
      installable = selfInstall ? update : null;
      publish({
        phase: "available",
        checkedAt,
        availableVersion: update.version,
        body: update.body ?? null,
        message: null,
        selfInstall,
        releaseUrl: releasePageUrl(update.version),
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
  if (!snapshot.selfInstall) {
    return Promise.reject(new Error("This install updates from the release page, not in place."));
  }
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

/** Opens the GitHub release for installs that cannot replace themselves. */
export async function openDesktopReleasePage(url: string | null): Promise<void> {
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url ?? RELEASES_URL);
}

export type DesktopUpdateAction = "install" | "release-page" | "installing" | "check";

/** The one action the Desktop update panel offers for this state. */
export function desktopUpdateAction(state: DesktopUpdateState): DesktopUpdateAction {
  if (state.phase === "available") return state.selfInstall ? "install" : "release-page";
  if (state.phase === "installing") return "installing";
  return "check";
}
