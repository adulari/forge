import type { AccountSyncCategory, AccountSyncStatus } from "./api";

function formatRelativeTime(fromMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.round((nowMs - fromMs) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.round(hours / 24)}d`;
}

export type AccountSyncProblem = "old-daemon" | "unreachable" | "error";

/** A 404 from a reachable daemon means it predates account sync. */
export function classifyAccountSyncError(error: unknown): { kind: AccountSyncProblem; message: string } {
  if (error instanceof Error && error.name === "ApiError" && typeof (error as { status?: unknown }).status === "number") {
    const { status } = error as Error & { status: number };
    if (status === 404) return { kind: "old-daemon", message: "Update Forge on this machine to use sync." };
    if (status === 0) return { kind: "unreachable", message: error.message };
    return { kind: "error", message: error.message };
  }
  return { kind: "error", message: error instanceof Error ? error.message : "Sync status could not be read." };
}

export function ageLabel(atMs: number | null, nowMs = Date.now()): string {
  if (atMs == null) return "never";
  return `${formatRelativeTime(atMs, nowMs)} ago`;
}

export interface KeysToggleContext {
  /** The daemon is reached over Forge Anywhere rather than directly. */
  viaAnywhere: boolean;
  /** iOS or Android: phones never receive secrets. */
  isPhone: boolean;
}

/** Why a category's switch is locked, or null when it can be flipped from here. */
export function toggleLockReason(category: AccountSyncCategory, context: KeysToggleContext): string | null {
  if (!category.toggle_requires_direct) return null;
  if (context.isPhone || context.viaAnywhere) return "Only on the computer itself";
  return null;
}

/** Turning a direct-only category (key sync) on asks first, like the CLI and the terminal menu. */
export function needsKeysConfirmation(category: AccountSyncCategory, enabled: boolean): boolean {
  return enabled && category.toggle_requires_direct;
}

export const KEYS_CONFIRM_TITLE = "Turn on key sync?";
export const KEYS_CONFIRM_MESSAGE =
  "Your provider and search API keys and static MCP secrets will be uploaded and delivered to every device on this account that also turns key sync on.\n\n" +
  "- Sealed with a second vault key, on top of end-to-end encryption.\n" +
  "- OAuth sign-ins (Codex, xAI, MCP) never sync; other devices get a sign-in prompt instead.\n" +
  "- A revoked device keeps what it already downloaded, so rotate keys at the provider after revoking one.\n" +
  "- Secret values are never shown on screen.";

export interface CategoryStatusLine {
  text: string;
  tone: "neutral" | "warn" | "success";
}

export function categoryStatusLine(category: AccountSyncCategory, nowMs = Date.now()): CategoryStatusLine {
  if (!category.enabled) return { text: "Off on this machine", tone: "neutral" };
  const parts = [`${category.items} ${category.items === 1 ? "item" : "items"}`];
  if (category.pending > 0) parts.push(`${category.pending} pending`);
  if (category.remote_only != null && category.remote_only > 0) parts.push(`${category.remote_only} to apply`);
  parts.push(`synced ${ageLabel(category.last_sync_at, nowMs)}`);
  const attention = category.conflicts.length > 0 || category.skipped.length > 0;
  return { text: parts.join(" · "), tone: category.conflicts.length > 0 ? "warn" : attention ? "neutral" : "success" };
}

/** Hub/Settings row meta: "Off", "3 on · synced 2m ago", "1 conflict". */
export function syncHubMeta(status: AccountSyncStatus | undefined, nowMs = Date.now()): string {
  if (!status) return "—";
  if (!status.signed_in) return "Sign in to enable";
  const on = status.categories.filter((category) => category.enabled);
  const conflicts = status.categories.reduce((sum, category) => sum + category.conflicts.length, 0);
  if (conflicts > 0) return `${conflicts} ${conflicts === 1 ? "conflict" : "conflicts"}`;
  if (status.last_error && on.length > 0) return "Needs attention";
  if (on.length === 0) return "Off";
  return `${on.length} on · synced ${ageLabel(status.last_sync_at, nowMs)}`;
}

export function syncNeedsAttention(status: AccountSyncStatus | undefined): boolean {
  if (!status || !status.signed_in) return false;
  return status.categories.some((category) => category.conflicts.length > 0) || (status.last_error != null && status.categories.some((c) => c.enabled));
}

/** Replace one category's switch in a status snapshot (used for optimistic display). */
export function withCategoryEnabled(status: AccountSyncStatus, id: string, enabled: boolean): AccountSyncStatus {
  return { ...status, categories: status.categories.map((category) => (category.id === id ? { ...category, enabled } : category)) };
}

export type SyncPillTone = "muted" | "success" | "neutral" | "warn" | "danger";

export interface SyncPillModel {
  label: string;
  tone: SyncPillTone;
  /** Full sentence for the tooltip and screen readers. */
  description: string;
}

/** The title-bar pill: null (hidden) unless this machine is signed in to Anywhere. */
export function syncPillModel(status: AccountSyncStatus | undefined, nowMs = Date.now()): SyncPillModel | null {
  if (!status || !status.signed_in) return null;
  const enabled = status.categories.filter((category) => category.enabled);
  const conflicts = enabled.reduce((sum, category) => sum + category.conflicts.length, 0);
  const pending = enabled.reduce((sum, category) => sum + category.pending, 0);
  const synced = `Last sync ${ageLabel(status.last_sync_at, nowMs)}`;
  if (status.last_error && enabled.length > 0) {
    return { label: "Sync error", tone: "danger", description: `Sync error: ${status.last_error}` };
  }
  if (conflicts > 0) {
    return { label: `${conflicts} ${conflicts === 1 ? "conflict" : "conflicts"}`, tone: "warn", description: `${conflicts} sync conflicts to review. ${synced}` };
  }
  if (status.sign_ins.length > 0) {
    return { label: "Sign in needed", tone: "warn", description: `Sign in to ${status.sign_ins.map((s) => s.label).join(", ")} on this machine` };
  }
  if (enabled.length === 0) return { label: "Sync off", tone: "muted", description: "Sync across machines is off on this machine" };
  if (pending > 0) {
    return { label: `${pending} pending`, tone: "neutral", description: `${pending} local changes waiting to sync. ${synced}` };
  }
  return { label: "Synced", tone: "success", description: `All enabled categories are in sync. ${synced}` };
}
