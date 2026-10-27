// Desktop-only client of the native OTA module (src-tauri/src/ota.rs, docs/features/desktop-ota.md).
// The shell downloads and verifies signed web-UI bundles in the background and serves the active
// one in place of the embedded UI; this file reports that the UI came up (so a broken bundle
// rolls itself back), surfaces a bundle that is waiting, and backs the Diagnostics rows.
// Phones and the plain web build have no such shell, so every entry point is a no-op there.
import { useSyncExternalStore } from "react";

import { isTauri } from "./platform";

export interface DesktopOtaBundle {
  id: string;
  /** Unix seconds of the commit the bundle was built from. */
  createdAt: number;
}

export interface DesktopOtaStatus {
  enabled: boolean;
  disabledReason: string | null;
  runtimeVersion: string;
  appVersion: string;
  embeddedBundleId: string;
  embeddedCreatedAt: number;
  active: DesktopOtaBundle | null;
  pending: DesktopOtaBundle | null;
  /** Unix seconds. */
  lastCheck: number | null;
  lastError: string | null;
  lastRollback: string | null;
  booting: boolean;
}

export const OTA_PENDING_EVENT = "forge://ota-pending";

const listeners = new Set<() => void>();
let snapshot: DesktopOtaStatus | null = null;
let dismissedPending: string | null = null;
let listening = false;

function publish(next: DesktopOtaStatus | null): void {
  snapshot = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getDesktopOtaStatus(): DesktopOtaStatus | null {
  return snapshot;
}

export function useDesktopOtaStatus(): DesktopOtaStatus | null {
  return useSyncExternalStore(subscribe, getDesktopOtaStatus, getDesktopOtaStatus);
}

async function call<T>(command: string): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command);
}

export async function refreshDesktopOtaStatus(): Promise<DesktopOtaStatus | null> {
  if (!isTauri) return null;
  try {
    publish(await call<DesktopOtaStatus>("ota_status"));
  } catch {
    // An older shell without the OTA commands: leave the rows hidden.
    publish(null);
  }
  return snapshot;
}

/** Subscribes to "a bundle is ready" from the shell. Safe to call more than once. */
export async function startDesktopOtaListener(): Promise<void> {
  if (!isTauri || listening) return;
  listening = true;
  try {
    const { listen } = await import("@tauri-apps/api/event");
    await listen(OTA_PENDING_EVENT, () => void refreshDesktopOtaStatus());
  } catch {
    listening = false;
  }
}

/**
 * Tells the shell the app shell rendered. Until it hears this, a launch that serves a downloaded
 * bundle counts as unhealthy, and the next launch rolls back to the previous bundle.
 */
export async function markDesktopOtaHealthy(): Promise<void> {
  if (!isTauri) return;
  try {
    await call<void>("ota_mark_healthy");
  } catch {
    return;
  }
  await refreshDesktopOtaStatus();
}

export async function checkDesktopOtaNow(): Promise<DesktopOtaStatus | null> {
  if (!isTauri) return null;
  publish(await call<DesktopOtaStatus>("ota_check_now"));
  return snapshot;
}

/** Switches to the pending bundle and reloads the window. */
export async function applyDesktopOta(): Promise<boolean> {
  if (!isTauri) return false;
  return call<boolean>("ota_apply_pending");
}

/** Drops every downloaded bundle and reloads into the UI built into this install. */
export async function resetDesktopOta(): Promise<void> {
  if (!isTauri) return;
  await call<void>("ota_reset");
}

export function dismissDesktopOtaBanner(): void {
  dismissedPending = snapshot?.pending?.id ?? null;
  publish(snapshot ? { ...snapshot } : null);
}

/** The pending bundle to offer, unless the user already dismissed that exact one. */
export function pendingBundleToOffer(status: DesktopOtaStatus | null): DesktopOtaBundle | null {
  if (!status?.enabled || !status.pending) return null;
  return status.pending.id === dismissedPending ? null : status.pending;
}

export function shortId(id: string): string {
  return id.slice(0, 7);
}

function formatDate(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toLocaleString();
}

export interface DesktopOtaRows {
  title: string;
  subtitle: string;
  badge: { label: string; tone: "success" | "accent" | "warn" | "neutral" };
}

/** What the Diagnostics screen shows for the desktop UI bundle. */
export function describeDesktopOta(status: DesktopOtaStatus): DesktopOtaRows {
  if (!status.enabled) {
    return {
      title: "Built-in interface",
      subtitle: `Over-the-air updates are off (${status.disabledReason ?? "disabled"}).`,
      badge: { label: "Off", tone: "neutral" },
    };
  }
  const live = status.active
    ? `Bundle ${shortId(status.active.id)} · ${formatDate(status.active.createdAt)}`
    : `Built-in ${shortId(status.embeddedBundleId)}${
        status.embeddedCreatedAt ? ` · ${formatDate(status.embeddedCreatedAt)}` : ""
      }`;
  const checked = status.lastCheck ? `Checked ${formatDate(status.lastCheck)}` : "Not checked yet";
  const detail = status.lastError ?? (status.lastRollback ? `Rolled back ${shortId(status.lastRollback)}` : checked);
  if (status.pending) {
    return {
      title: live,
      subtitle: `Bundle ${shortId(status.pending.id)} is ready · ${detail}`,
      badge: { label: "Ready", tone: "accent" },
    };
  }
  if (status.lastError) {
    return { title: live, subtitle: detail, badge: { label: "Needs attention", tone: "warn" } };
  }
  return { title: live, subtitle: detail, badge: { label: "Up to date", tone: "success" } };
}
