// Disk and memory pressure on the machine a Forge daemon runs on (`pressure` in
// `GET /api/identity`, and on `POST /api/sessions` when a worktree was created while space was
// low). Pure helpers only: the screens decide where the line and the Reclaim offer appear.

export type PressureLevel = "ok" | "warn" | "floor";

export type PressureCause = "disk" | "memory";

export interface HostPressure {
  level: PressureLevel;
  /** What is short; absent from a daemon that predates it (treated as disk). */
  cause?: PressureCause | null;
  /** One sentence naming the tightest disk and/or low memory; null while the host is fine. */
  summary?: string | null;
}

/** What a create-session answer or refusal says about the disk a new worktree would land on. */
export interface WorktreeGate {
  level: PressureLevel;
  summary?: string | null;
  reclaimable_bytes?: number | null;
}

export interface ReclaimOutcome {
  freed_bytes: number;
  removed: number;
  pruned: number;
  failures: string[];
}

export function isPressured(pressure: HostPressure | null | undefined): pressure is HostPressure {
  return pressure?.level === "warn" || pressure?.level === "floor";
}

/** Reclaim frees disk; memory-only pressure gets a plain line and no destructive action. */
export function offersReclaim(pressure: HostPressure | null | undefined): pressure is HostPressure {
  return isPressured(pressure) && pressure.cause !== "memory";
}

/** Dismissal is remembered per level and cause, so a worse or different problem shows again. */
export function dismissKey(pressure: HostPressure): string {
  return `${pressure.level}:${pressure.cause ?? "disk"}`;
}

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number): string {
  let value = Math.max(0, bytes);
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${UNITS[unit]}`;
}

/** The banner sentence: what is wrong, and what that means for starting work. */
export function pressureMessage(pressure: HostPressure): string {
  const why = pressure.summary ?? "this host is low on disk or memory";
  if (pressure.cause === "memory") return `Host memory is low. ${why}`;
  return pressure.level === "floor"
    ? `New worktree sessions are paused. ${why}`
    : `Host running low. ${why}`;
}

/** Reclaim runs on the host; only a direct connection can reach it (the relay does not carry it). */
export function canReclaimFrom(baseUrl: string | null | undefined): boolean {
  return !!baseUrl && !baseUrl.startsWith("fany://");
}

export const RELAY_RECLAIM_HINT = "Run `forge worktree reclaim --artifacts` on the host to free space.";

interface ErrorLike {
  status?: number;
  message?: string;
  body?: unknown;
}

/** The refusal a create-session call gets under the hard floor, or null for any other error. */
export function floorRefusal(error: unknown): { message: string; gate: WorktreeGate } | null {
  const err = error as ErrorLike | null;
  const body = err?.body as { code?: unknown; pressure?: WorktreeGate } | undefined;
  if (!body || body.code !== "host_disk_floor") return null;
  return {
    message: err?.message ?? "not enough disk space for a new worktree",
    gate: { ...body.pressure, level: "floor" },
  };
}

/** The line shown after a worktree session was created with the disk under its warn threshold. */
export function createWarning(gate: WorktreeGate | null | undefined): string | null {
  if (!gate || gate.level === "ok") return null;
  const offer =
    gate.reclaimable_bytes && gate.reclaimable_bytes > 0
      ? ` ${formatBytes(gate.reclaimable_bytes)} can be reclaimed.`
      : "";
  return `Session started, but ${gate.summary ?? "the disk is low"}.${offer}`;
}

export function reclaimLabel(gate: WorktreeGate | null | undefined): string {
  return gate?.reclaimable_bytes ? `Reclaim ${formatBytes(gate.reclaimable_bytes)}` : "Reclaim";
}

/** The confirmation body: what is wrong, what Reclaim removes and (when known) how much it frees. */
export function reclaimConfirmMessage(summary: string | null | undefined, reclaimableBytes: number | null | undefined): string {
  const amount =
    reclaimableBytes == null
      ? "How much it frees is not known yet."
      : reclaimableBytes > 0
        ? `About ${formatBytes(reclaimableBytes)} can be freed.`
        : "Nothing is provably safe to remove right now.";
  return `${summary ? `${summary}. ` : ""}Removes worktrees that are merged, clean, pushed and not in use, and stale build directories, across every project on the host. Anything else is kept. ${amount}`;
}

export function reclaimResultMessage(out: ReclaimOutcome): string {
  const parts = [`${out.removed} worktree${out.removed === 1 ? "" : "s"} removed`, `${out.pruned} build dir${out.pruned === 1 ? "" : "s"} pruned`];
  const failed = out.failures.length > 0 ? `, ${out.failures.length} failed` : "";
  return `Reclaimed ${formatBytes(out.freed_bytes)}: ${parts.join(", ")}${failed}.`;
}
