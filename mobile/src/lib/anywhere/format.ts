// Pure formatting helpers for Forge Anywhere — no React, no tokens import beyond types,
// unit-tested in format.test.ts. Text content mirrors mobile.dc.html "AW State Variants"
// (lines 1283-1360) so screens render the exact labels from the design comp.
import type { BadgeTone } from "../../components/ds/Badge";
import type { AnywhereAccount, AnywhereHost, SyncStatus } from "./types";

/** Translate the service's wire vocabulary into the UI state vocabulary. */
export function normalizeEntitlementState(value?: string): AnywhereAccount["entitlement"] {
  switch (value) {
    case "trialing": case "trial": return "trial";
    case "active": return "active";
    case "grace": return "grace";
    case "read_only": case "read-only": return "read-only";
    case "suspended": return "suspended";
    case "webhook_pending": case "webhook-pending": return "webhook-pending";
    case "trial_not_started": case "not-started": default: return "not-started";
  }
}

// Local copy of theme/typography.ts's formatRelativeTime — duplicated rather than
// imported because typography.ts pulls in the real `react-native` package at
// module-eval time (it uses `Platform`), and this project's vitest has no Flow/RN
// transform configured; this module must stay RN-free to stay unit-testable per its
// "pure functions" contract. Keep in sync with theme/typography.ts if that scale changes.
function formatRelativeTime(fromMs: number, nowMs: number = Date.now()): string {
  const deltaSec = Math.max(0, Math.round((nowMs - fromMs) / 1000));
  if (deltaSec < 60) return `${deltaSec}s`;
  const deltaMin = Math.round(deltaSec / 60);
  if (deltaMin < 60) return `${deltaMin}m`;
  const deltaHour = Math.round(deltaMin / 60);
  if (deltaHour < 24) return `${deltaHour}h`;
  const deltaDay = Math.round(deltaHour / 24);
  return `${deltaDay}d`;
}

/** `1.2 GB` / `18.2 MB` / `512 KB` / `900 B` — adaptive unit, 1 decimal above KB. */
export function formatBytes(bytes: number): string {
  const abs = Math.abs(bytes);
  if (abs < 1024) return `${Math.round(bytes)} B`;
  if (abs < 1024 ** 2) return `${Math.round(bytes / 1024)} KB`;
  if (abs < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

export interface EntitlementBadgeInfo {
  label: string;
  tone: BadgeTone;
}

type EntitlementBadgeInput = Pick<
  AnywhereAccount,
  "entitlement" | "trialDaysLeft" | "graceDaysLeft" | "deletesInDays"
>;

/** trial/grace -> warn, active -> success, read-only/suspended/deletes -> danger, not-started -> outline. */
export function entitlementBadge(account: EntitlementBadgeInput): EntitlementBadgeInfo {
  switch (account.entitlement) {
    case "not-started":
      return { label: "NOT STARTED", tone: "outline" };
    case "trial":
      return { label: account.trialDaysLeft != null ? `TRIAL · ${account.trialDaysLeft}D` : "TRIAL", tone: "warn" };
    case "active":
      return { label: "ACTIVE", tone: "success" };
    case "grace":
      return { label: account.graceDaysLeft != null ? `GRACE · ${account.graceDaysLeft}D` : "GRACE", tone: "warn" };
    case "read-only":
      return { label: "READ-ONLY", tone: "danger" };
    case "suspended":
      return account.deletesInDays != null
        ? { label: `DELETES IN ${account.deletesInDays}D`, tone: "danger" }
        : { label: "SUSPENDED", tone: "danger" };
    case "webhook-pending":
      return { label: "PENDING", tone: "outline" };
    default: {
      const _exhaustive: never = account.entitlement;
      return _exhaustive;
    }
  }
}

/** "online · idle" | "online · busy · 2 sessions" | "stale · last seen 26m" | ... */
export function hostStateText(host: Pick<AnywhereHost, "state">, nowMs: number = Date.now()): string {
  const { state } = host;
  switch (state.kind) {
    case "online":
      return state.activity === "busy"
        ? `online · busy · ${state.sessionCount} session${state.sessionCount === 1 ? "" : "s"}`
        : "online · idle";
    case "connecting":
      return "connecting…";
    case "stale":
      return `stale · last seen ${formatRelativeTime(state.lastSeenAt, nowMs)}`;
    case "offline":
      return `offline · last seen ${formatRelativeTime(state.lastHeartbeatAt, nowMs)}`;
    case "disabled":
      return "disabled";
    case "revoked":
      return "revoked";
    case "update-required":
      return `update required · v${state.connectorVersion}`;
    default: {
      const _exhaustive: never = state;
      return _exhaustive;
    }
  }
}

export interface SyncGlyphInfo {
  glyph: string;
  colorKey: "success" | "accent" | "warn" | "danger" | "ink3";
  text: string;
}

/** Mono glyph + status text for one sync row, matching the design's "sync glyph" legend. */
export function syncGlyph(status: SyncStatus, nowMs: number = Date.now()): SyncGlyphInfo {
  switch (status.kind) {
    case "synced":
      return {
        glyph: "✓",
        colorKey: "success",
        text: `current · synced ${formatRelativeTime(status.syncedAt, nowMs)} ago${status.offlineAvailable ? " · available offline" : ""}`,
      };
    case "uploading":
      return {
        glyph: "↑",
        colorKey: "accent",
        text: `uploading ${status.recordCount} record${status.recordCount === 1 ? "" : "s"}`,
      };
    case "downloading":
      return { glyph: "↓", colorKey: "accent", text: `downloading ${formatBytes(status.bytes)}` };
    case "offline-cache":
      return {
        glyph: "◌",
        colorKey: "ink3",
        text: `offline — device-encrypted cache from ${formatRelativeTime(status.cachedAt, nowMs)} ago`,
      };
    case "retrying":
      return {
        glyph: "↻",
        colorKey: "warn",
        text: `retrying · attempt ${status.attempt}${
          status.lastSuccessAt != null ? ` · last success ${formatRelativeTime(status.lastSuccessAt, nowMs)} ago` : ""
        }`,
      };
    case "conflict":
      return { glyph: "⑂", colorKey: "warn", text: "conflict copy preserved · both versions kept" };
    case "over-quota":
      return { glyph: "■", colorKey: "danger", text: "over quota — writes blocked · download/delete ok" };
    case "key-epoch-required":
      return { glyph: "⚿", colorKey: "warn", text: "key epoch update required" };
    case "read-only":
      return { glyph: "▢", colorKey: "danger", text: "read-only entitlement" };
    default: {
      const _exhaustive: never = status;
      return _exhaustive;
    }
  }
}
