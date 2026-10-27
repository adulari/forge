// Desktop onboarding phase 0 (before any daemon exists): bridges the Tauri commands in
// src-tauri/src/onboarding.rs. Off Tauri these resolve to "nothing here" so callers need no
// isTauri guard of their own.
import type { DetectedServeState } from "./desktopServe";
import { isTauri } from "./platform";

export const INSTALL_PROGRESS_EVENT = "onboarding://install-progress";

export type InstallPhase =
  | "resolve"
  | "download"
  | "verify"
  | "install"
  | "service-install"
  | "service-start"
  | "wait"
  | "done"
  | "failed";

export interface InstallProgress {
  phase: InstallPhase;
  fraction: number | null;
  message: string;
}

export interface LocalServiceFacts {
  supported: boolean;
  installed: boolean;
  running: boolean;
}

/** Mirrors `LocalStatus` in onboarding.rs. */
export interface LocalStatus {
  forge_binary: string | null;
  forge_version: string | null;
  serve: DetectedServeState | null;
  service: LocalServiceFacts;
  platform: { os: string; arch: string };
  install_dir: string;
}

export interface InstallCliResult {
  path: string;
  version: string | null;
}

export interface InstallServiceResult {
  running: boolean;
  serve: DetectedServeState | null;
}

export async function localStatus(): Promise<LocalStatus | null> {
  if (!isTauri) return null;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return await invoke<LocalStatus>("onboarding_local_status");
  } catch {
    return null;
  }
}

async function withProgress<T>(
  command: string,
  onProgress: ((progress: InstallProgress) => void) | undefined,
): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  let unlisten: (() => void) | undefined;
  if (onProgress) {
    const { listen } = await import("@tauri-apps/api/event");
    unlisten = await listen<InstallProgress>(INSTALL_PROGRESS_EVENT, (event) => onProgress(event.payload));
  }
  try {
    return await invoke<T>(command);
  } finally {
    unlisten?.();
  }
}

/** Downloads, verifies and installs the forge CLI. Rejects with a one-line message. */
export async function installForgeCli(
  onProgress?: (progress: InstallProgress) => void,
): Promise<InstallCliResult> {
  if (!isTauri) throw new Error("installing Forge is only available in the desktop app");
  return withProgress<InstallCliResult>("install_forge_cli", onProgress);
}

/** Installs and starts the background service, resolving once the daemon is up. */
export async function installForgeService(
  onProgress?: (progress: InstallProgress) => void,
): Promise<InstallServiceResult> {
  if (!isTauri) throw new Error("installing the service is only available in the desktop app");
  return withProgress<InstallServiceResult>("install_forge_service", onProgress);
}
