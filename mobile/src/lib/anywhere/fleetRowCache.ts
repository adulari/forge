import AsyncStorage from "@react-native-async-storage/async-storage";

import type { SessionRow } from "../api";

const KEY_PREFIX = "forge.anywhere.fleetRows.v1.";
const WRITE_DELAY_MS = 2000;
const MAX_ROWS = 200;

export interface FleetRowStorage {
  getItem(key: string): Promise<string | null>;
  setItem(key: string, value: string): Promise<void>;
  removeItem(key: string): Promise<void>;
}

const memory = new Map<string, SessionRow[]>();
const lastWritten = new Map<string, string>();
const timers = new Map<string, ReturnType<typeof setTimeout>>();

export function relayHostIdOf(baseUrl: string | null | undefined): string | null {
  if (!baseUrl) return null;
  try {
    const url = new URL(baseUrl);
    return url.protocol === "fany:" || url.protocol === "fany-ws:" ? url.hostname : null;
  } catch {
    return null;
  }
}

/** Explicit projection of the list metadata the Fleet card renders; no transcript content. */
function project(row: SessionRow): SessionRow {
  return {
    id: row.id,
    title: row.title,
    cwd: row.cwd,
    worktree: row.worktree,
    busy: row.busy,
    waiting: row.waiting,
    cost_usd: row.cost_usd,
    context_tokens: row.context_tokens,
    context_limit: row.context_limit,
    model: row.model,
    permission_mode: row.permission_mode,
    created_at: row.created_at,
    last_activity: row.last_activity,
    ...(row.read_only !== undefined ? { read_only: row.read_only } : {}),
    ...(row.terminal !== undefined ? { terminal: row.terminal } : {}),
    ...(row.last_turn_outcome !== undefined ? { last_turn_outcome: row.last_turn_outcome } : {}),
    ...(row.last_stop_reason !== undefined ? { last_stop_reason: row.last_stop_reason } : {}),
    ...(row.provider !== undefined ? { provider: row.provider } : {}),
    ...(row.parent_id !== undefined ? { parent_id: row.parent_id } : {}),
    ...(row.diff_added !== undefined ? { diff_added: row.diff_added } : {}),
    ...(row.diff_removed !== undefined ? { diff_removed: row.diff_removed } : {}),
    ...(row.branch !== undefined ? { branch: row.branch } : {}),
  };
}

function isRow(value: unknown): value is SessionRow {
  if (typeof value !== "object" || value === null) return false;
  const row = value as Record<string, unknown>;
  return typeof row.id === "string" && typeof row.title === "string" && typeof row.cwd === "string"
    && typeof row.busy === "boolean" && typeof row.waiting === "boolean"
    && typeof row.last_activity === "number" && typeof row.created_at === "number";
}

/** Loads persisted rows into memory so `peekFleetRows` can seed queries synchronously. */
export async function loadFleetRows(
  hostIds: readonly string[],
  storage: FleetRowStorage = AsyncStorage,
): Promise<void> {
  await Promise.all(hostIds.map(async (hostId) => {
    if (memory.has(hostId)) return;
    try {
      const encoded = await storage.getItem(KEY_PREFIX + hostId);
      if (!encoded) return;
      const value: unknown = JSON.parse(encoded);
      if (!Array.isArray(value)) return;
      memory.set(hostId, value.filter(isRow).map(project));
      lastWritten.set(hostId, encoded);
    } catch {
      // A corrupt or unreadable cache only costs the instant first paint.
    }
  }));
}

export function peekFleetRows(baseUrl: string | null | undefined): SessionRow[] | undefined {
  const hostId = relayHostIdOf(baseUrl);
  return hostId ? memory.get(hostId) : undefined;
}

export function rememberFleetRows(
  baseUrl: string | null | undefined,
  rows: readonly SessionRow[],
  storage: FleetRowStorage = AsyncStorage,
): void {
  const hostId = relayHostIdOf(baseUrl);
  if (!hostId) return;
  const projected = rows.slice(0, MAX_ROWS).map(project);
  memory.set(hostId, projected);
  const encoded = JSON.stringify(projected);
  if (lastWritten.get(hostId) === encoded) return;
  const pending = timers.get(hostId);
  if (pending) clearTimeout(pending);
  timers.set(hostId, setTimeout(() => {
    timers.delete(hostId);
    lastWritten.set(hostId, encoded);
    void storage.setItem(KEY_PREFIX + hostId, encoded).catch(() => lastWritten.delete(hostId));
  }, WRITE_DELAY_MS));
}

export async function clearFleetRows(
  hostIds: readonly string[],
  storage: FleetRowStorage = AsyncStorage,
): Promise<void> {
  await Promise.all(hostIds.map(async (hostId) => {
    const pending = timers.get(hostId);
    if (pending) clearTimeout(pending);
    timers.delete(hostId);
    memory.delete(hostId);
    lastWritten.delete(hostId);
    await storage.removeItem(KEY_PREFIX + hostId).catch(() => undefined);
  }));
}

export function resetFleetRowCacheForTests(): void {
  for (const timer of timers.values()) clearTimeout(timer);
  timers.clear();
  memory.clear();
  lastWritten.clear();
}
