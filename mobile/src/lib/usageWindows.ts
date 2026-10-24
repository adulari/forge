// Shared shaping for the usage surfaces (Settings > Usage and the desktop usage dock): which
// subscription windows a provider has, what to call them, and how token usage and quota windows
// merge into one block per provider. Pure, so both screens tell the same story.
import type { UsageProvider, UsageQuota } from "./api";

export type QuotaKind = UsageQuota["kind"];

/** A quota window as rendered. `seen: false` marks a window the provider is known to have but the
 * daemon has not observed yet; its numbers are null and the UI says "unknown". */
export interface DisplayQuota {
  windowKind: string;
  status: string;
  fraction: number | null;
  resetsAt: number | null;
  updatedAt?: number;
  pacing?: UsageQuota["pacing"];
  seen: boolean;
}

export interface ProviderBlock {
  provider: string;
  kind: QuotaKind;
  usage: UsageProvider | null;
  quotas: DisplayQuota[];
}

// Windows each subscription is known to meter. Claude's `weekly_fable` is the per-model weekly
// meter (Claude Code's "Fable only" row); Codex has no per-model window on a ChatGPT plan.
const EXPECTED_WINDOWS: Record<string, readonly string[]> = {
  "claude-cli": ["five_hour", "weekly", "weekly_fable"],
  "codex-cli": ["five_hour", "weekly"],
  "codex-oauth": ["five_hour", "weekly"],
};

const QUOTA_TWIN: Record<string, string> = { "codex-cli": "codex-oauth", "codex-oauth": "codex-cli" };

/** One name per shared subscription account, for surfaces that show a single figure per plan. */
export const quotaAccount = (provider: string) => (provider === "codex-cli" ? "codex-oauth" : provider);

const WINDOW_ORDER = ["five_hour", "weekly", "weekly_fable", "weekly_opus", "weekly_sonnet", "monthly"];

const LONG_LABEL: Record<string, string> = {
  five_hour: "5-hour session",
  weekly: "Weekly · all models",
  weekly_fable: "Weekly · Fable only",
  weekly_opus: "Weekly · Opus only",
  weekly_sonnet: "Weekly · Sonnet only",
  monthly: "Monthly",
  secondary: "Secondary",
};

const SHORT_LABEL: Record<string, string> = {
  five_hour: "5h",
  weekly: "week",
  weekly_fable: "fable",
  weekly_opus: "opus",
  weekly_sonnet: "sonnet",
  monthly: "month",
  secondary: "2nd",
};

export const windowLabel = (windowKind: string) => LONG_LABEL[windowKind] ?? windowKind;
export const windowShortLabel = (windowKind: string) => SHORT_LABEL[windowKind] ?? windowKind;

/** A weekly meter for one model family (Fable/Opus/Sonnet only). It gates that family, not the
 * whole plan, so it never drives a combined "how full is my plan" figure. Mirrors
 * `forge_types::is_model_scoped_window`. */
export const isModelScopedWindow = (windowKind: string) => windowKind.startsWith("weekly_");

/** Subscriptions (CLI bridges and OAuth plans) bill nothing per call. */
export const isSubscription = (kind: string) => kind !== "api";
export const kindLabel = (kind: string) => (isSubscription(kind) ? "subscription" : "api");

export function priceLabel(block: Pick<ProviderBlock, "kind" | "usage">): string {
  if (isSubscription(block.kind)) return "Included";
  const cost = block.usage?.costUsd ?? 0;
  return cost > 0 ? `$${cost.toFixed(2)}` : "No price";
}

const windowRank = (windowKind: string) => {
  const index = WINDOW_ORDER.indexOf(windowKind);
  return index === -1 ? WINDOW_ORDER.length : index;
};

export function buildProviderBlocks(usage: readonly UsageProvider[] | undefined, quota: readonly UsageQuota[] | undefined): ProviderBlock[] {
  const blocks = new Map<string, ProviderBlock>();
  const keyOf = (provider: string, kind: string) => `${kind}:${provider}`;
  for (const row of usage ?? []) {
    blocks.set(keyOf(row.provider, row.kind), { provider: row.provider, kind: row.kind, usage: row, quotas: [] });
  }
  for (const row of quota ?? []) {
    const key = keyOf(row.provider, row.kind);
    const block = blocks.get(key) ?? { provider: row.provider, kind: row.kind, usage: null, quotas: [] };
    block.quotas.push({
      windowKind: row.windowKind,
      status: row.status,
      fraction: row.fraction,
      resetsAt: row.resetsAt,
      updatedAt: row.updatedAt,
      pacing: row.pacing,
      seen: true,
    });
    blocks.set(key, block);
  }
  for (const block of blocks.values()) {
    for (const windowKind of EXPECTED_WINDOWS[block.provider] ?? []) {
      if (block.quotas.some((q) => q.windowKind === windowKind)) continue;
      block.quotas.push({ windowKind, status: "unknown", fraction: null, resetsAt: null, seen: false });
    }
    block.quotas.sort((a, b) => windowRank(a.windowKind) - windowRank(b.windowKind));
  }
  // codex-cli and codex-oauth meter the same ChatGPT account and the daemon reports the windows
  // under both names. A surface with no usage of its own adds nothing but a duplicate block.
  const byProvider = new Map([...blocks.values()].map((block) => [block.provider, block]));
  for (const [key, block] of blocks) {
    const twin = byProvider.get(QUOTA_TWIN[block.provider] ?? "");
    if (block.usage != null || !twin) continue;
    // Keep the twin that has usage; with neither, keep the OAuth name.
    if (twin.usage != null || block.provider === "codex-cli") blocks.delete(key);
  }
  // Metered spend first (it is real money), then subscriptions with live windows, then the rest.
  return [...blocks.values()].sort(
    (a, b) =>
      (b.kind === "api" ? b.usage?.costUsd ?? 0 : 0) - (a.kind === "api" ? a.usage?.costUsd ?? 0 : 0) ||
      Number(isSubscription(b.kind)) - Number(isSubscription(a.kind)) ||
      a.provider.localeCompare(b.provider),
  );
}

export function resetLabel(resetsAt: number | null): string | null {
  if (resetsAt == null) return null;
  return `resets ${new Intl.DateTimeFormat(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" }).format(new Date(resetsAt * 1000))}`;
}

export function formatDurationShort(totalSec: number): string {
  const sec = Math.max(0, Math.round(totalSec));
  const days = Math.floor(sec / 86400);
  const hours = Math.floor((sec % 86400) / 3600);
  const minutes = Math.floor((sec % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${Math.max(1, minutes)}m`;
}

/** Largest unit only ("3h", "2d"), so the timing line fits the narrow dock. */
function formatAgeCoarse(totalSec: number): string {
  const sec = Math.max(0, Math.round(totalSec));
  if (sec >= 86400) return `${Math.floor(sec / 86400)}d`;
  if (sec >= 3600) return `${Math.floor(sec / 3600)}h`;
  return `${Math.max(1, Math.floor(sec / 60))}m`;
}

/** "resets Tue 09:00 · seen 3h ago" — when the window rolls over and how old the reading is. */
export function quotaTiming(quota: DisplayQuota, nowSec: number): string {
  return [resetLabel(quota.resetsAt), quota.updatedAt == null ? null : `seen ${formatAgeCoarse(nowSec - quota.updatedAt)} ago`]
    .filter(Boolean)
    .join(" · ");
}

/** "42% · resets Tue 09:00 · seen 3h ago", or "unknown · not seen yet". Never a number the daemon
 * did not report. */
export function quotaSummary(quota: DisplayQuota, nowSec: number): string {
  if (!quota.seen) return "unknown · not seen yet";
  const percent = quota.fraction == null ? "unknown" : `${Math.round(quota.fraction * 100)}%`;
  return [percent, quotaTiming(quota, nowSec)].filter(Boolean).join(" · ");
}
