// The change queue as plain data: which changed files need a human first. Pure, so the
// ordering rule (waiting on you, then conflicts, then failing checks, then unreviewed, then
// reviewed) is tested without rendering anything.

/** Lower is more urgent; the order is the contract. */
export type AttentionTier = "waiting" | "conflict" | "failing" | "unreviewed" | "reviewed";

export const TIER_ORDER: readonly AttentionTier[] = ["waiting", "conflict", "failing", "unreviewed", "reviewed"];

const TIER_RANK: Record<AttentionTier, number> = {
  waiting: 0,
  conflict: 1,
  failing: 2,
  unreviewed: 3,
  reviewed: 4,
};

export interface ChangedFile {
  path: string;
  /** git porcelain letter: M A D R ?; "U" for an unmerged path. */
  status: string;
  adds: number;
  dels: number;
  /** Identity of the file's changed content. A reviewed mark is only honoured while this holds. */
  hash: string;
  conflicted?: boolean;
  /** A check (test, lint, build) that names this file is failing. */
  failing?: boolean;
  /** The daemon's verdict for the whole file; unset when the daemon predates risk ranking. */
  risk?: "routine" | "look";
  riskReason?: string;
  lookHunks?: number;
  routineHunks?: number;
}

export interface QueueAgent {
  sessionId: string;
  name: string;
  /** The agent is blocked on a permission or a question for the user. */
  waiting: boolean;
  busy: boolean;
  lastActivity: number;
  files: readonly ChangedFile[];
}

/** `sessionId\0path` → the hash that was current when the user marked the file reviewed. */
export type ReviewedMarks = Readonly<Record<string, string>>;

export function reviewKey(sessionId: string, path: string): string {
  return `${sessionId}\0${path}`;
}

export function isReviewed(marks: ReviewedMarks, sessionId: string, path: string, hash: string): boolean {
  return marks[reviewKey(sessionId, path)] === hash;
}

/** Marks a file reviewed at its current hash. */
export function markReviewed(marks: ReviewedMarks, sessionId: string, path: string, hash: string): ReviewedMarks {
  if (marks[reviewKey(sessionId, path)] === hash) return marks;
  return { ...marks, [reviewKey(sessionId, path)]: hash };
}

export function unmarkReviewed(marks: ReviewedMarks, sessionId: string, path: string): ReviewedMarks {
  const key = reviewKey(sessionId, path);
  if (!(key in marks)) return marks;
  const { [key]: _gone, ...rest } = marks;
  return rest;
}

/**
 * Drops every mark whose file changed again (hash differs) or left the change set. A mark that
 * survives is exactly one `isReviewed` would honour, so persisted marks cannot grow without bound
 * and a stale one can never come back if a file later returns to an old hash.
 *
 * `live` only needs the sessions that were actually loaded: a session absent from it keeps its
 * marks, so a failed fetch does not wipe the user's review progress.
 */
export function pruneReviewed(
  marks: ReviewedMarks,
  live: ReadonlyMap<string, ReadonlyMap<string, string>>,
): ReviewedMarks {
  let changed = false;
  const next: Record<string, string> = {};
  for (const [key, hash] of Object.entries(marks)) {
    const split = key.indexOf("\0");
    const sessionId = key.slice(0, split);
    const path = key.slice(split + 1);
    const files = live.get(sessionId);
    if (files && files.get(path) !== hash) {
      changed = true;
      continue;
    }
    next[key] = hash;
  }
  return changed ? next : marks;
}

export interface QueueRow {
  /** Stable row key: `sessionId\0path`. */
  key: string;
  sessionId: string;
  path: string;
  file: ChangedFile;
  tier: AttentionTier;
}

export interface QueueGroup {
  sessionId: string;
  name: string;
  /** The most urgent tier among the group's files; groups sort by it. */
  tier: AttentionTier;
  waiting: boolean;
  busy: boolean;
  adds: number;
  dels: number;
  toReview: number;
  reviewed: number;
  /** One plain line: what this agent left for you. */
  summary: string;
  rows: QueueRow[];
}

/** Files that need a look sort ahead of routine ones; an unranked file is treated as needing one. */
export function riskRank(file: Pick<ChangedFile, "risk">): number {
  return file.risk === "routine" ? 1 : 0;
}

export interface RiskTotals {
  /** Whether any file carries a verdict; without one the queue shows no risk UI. */
  ranked: boolean;
  lookHunks: number;
  routineHunks: number;
  lookFiles: number;
  routineFiles: number;
}

/** Hunk and file counts over the files still to review ("9 look here · 31 routine"). */
export function riskTotals(rows: readonly QueueRow[]): RiskTotals {
  const totals: RiskTotals = { ranked: false, lookHunks: 0, routineHunks: 0, lookFiles: 0, routineFiles: 0 };
  for (const row of rows) {
    if (row.file.risk === undefined) continue;
    totals.ranked = true;
    if (row.tier === "reviewed") continue;
    totals.lookHunks += row.file.lookHunks ?? 0;
    totals.routineHunks += row.file.routineHunks ?? 0;
    if (row.file.risk === "look") totals.lookFiles += 1;
    else totals.routineFiles += 1;
  }
  return totals;
}

/** "9 look here · 31 routine", leaving out an empty side. */
export function riskLine(totals: Pick<RiskTotals, "lookHunks" | "routineHunks">): string {
  const parts: string[] = [];
  if (totals.lookHunks > 0) parts.push(`${totals.lookHunks} look here`);
  if (totals.routineHunks > 0) parts.push(`${totals.routineHunks} routine`);
  return parts.join(" · ");
}

function tierOf(agent: QueueAgent, file: ChangedFile, marks: ReviewedMarks): AttentionTier {
  if (agent.waiting) return "waiting";
  if (file.conflicted) return "conflict";
  if (file.failing) return "failing";
  return isReviewed(marks, agent.sessionId, file.path, file.hash) ? "reviewed" : "unreviewed";
}

export function summarizeGroup(group: { toReview: number; reviewed: number; adds: number; dels: number; waiting: boolean; conflicts: number }): string {
  const total = group.toReview + group.reviewed;
  if (total === 0) return "No changes yet";
  const files = `${total} ${total === 1 ? "file" : "files"}`;
  const lead = group.waiting
    ? "Waiting for you"
    : group.conflicts > 0
      ? `${group.conflicts} ${group.conflicts === 1 ? "conflict" : "conflicts"}`
      : group.toReview === 0
        ? "All reviewed"
        : `${group.toReview} to review`;
  return `${lead} · ${files} · +${group.adds} −${group.dels}`;
}

export interface BuildQueueInput {
  agents: readonly QueueAgent[];
  marks: ReviewedMarks;
  /** Limit to one agent; `null` is "All agents". */
  scope: string | null;
}

/** Groups with no changed files are left out: an empty group is not something to review. */
export function buildQueue({ agents, marks, scope }: BuildQueueInput): QueueGroup[] {
  const groups: QueueGroup[] = [];
  for (const agent of agents) {
    if (scope != null && agent.sessionId !== scope) continue;
    if (agent.files.length === 0) continue;
    const rows: QueueRow[] = agent.files.map((file) => ({
      key: reviewKey(agent.sessionId, file.path),
      sessionId: agent.sessionId,
      path: file.path,
      file,
      tier: tierOf(agent, file, marks),
    }));
    rows.sort(
      (a, b) =>
        TIER_RANK[a.tier] - TIER_RANK[b.tier] ||
        riskRank(a.file) - riskRank(b.file) ||
        b.file.adds + b.file.dels - (a.file.adds + a.file.dels) ||
        a.path.localeCompare(b.path),
    );
    let adds = 0;
    let dels = 0;
    let reviewed = 0;
    let conflicts = 0;
    for (const row of rows) {
      adds += row.file.adds;
      dels += row.file.dels;
      if (row.tier === "reviewed") reviewed += 1;
      if (row.file.conflicted) conflicts += 1;
    }
    const toReview = rows.length - reviewed;
    groups.push({
      sessionId: agent.sessionId,
      name: agent.name,
      tier: rows[0].tier,
      waiting: agent.waiting,
      busy: agent.busy,
      adds,
      dels,
      toReview,
      reviewed,
      summary: summarizeGroup({ toReview, reviewed, adds, dels, waiting: agent.waiting, conflicts }),
      rows,
    });
  }
  const activity = new Map(agents.map((agent) => [agent.sessionId, agent.lastActivity]));
  groups.sort(
    (a, b) =>
      TIER_RANK[a.tier] - TIER_RANK[b.tier] ||
      (activity.get(b.sessionId) ?? 0) - (activity.get(a.sessionId) ?? 0) ||
      a.name.localeCompare(b.name),
  );
  return groups;
}

/** Every row in the order j/k walks them. */
export function flattenQueue(groups: readonly QueueGroup[]): QueueRow[] {
  return groups.flatMap((group) => group.rows);
}

/** The row after (`step` 1) or before (`step` -1) `current`, wrapping never: j at the end stays. */
export function stepQueue(rows: readonly QueueRow[], currentKey: string | null, step: 1 | -1): QueueRow | null {
  if (rows.length === 0) return null;
  const index = currentKey == null ? -1 : rows.findIndex((row) => row.key === currentKey);
  if (index === -1) return step === 1 ? rows[0] : rows[rows.length - 1];
  return rows[Math.min(rows.length - 1, Math.max(0, index + step))];
}

/** After marking `currentKey` reviewed, the next row that still needs a look (falls back to the neighbour). */
export function nextToReview(rows: readonly QueueRow[], currentKey: string): QueueRow | null {
  const index = rows.findIndex((row) => row.key === currentKey);
  if (index === -1) return null;
  for (let step = 1; step < rows.length; step += 1) {
    const row = rows[(index + step) % rows.length];
    if (row.tier !== "reviewed" && row.key !== currentKey) return row;
  }
  return null;
}

/** Cheap, stable string hash (djb2) for fingerprints. Not cryptographic. */
export function fingerprint(parts: readonly string[]): string {
  let h = 5381;
  for (const part of parts) {
    for (let i = 0; i < part.length; i += 1) h = ((h << 5) + h + part.charCodeAt(i)) | 0;
    h = ((h << 5) + h + 10) | 0;
  }
  return (h >>> 0).toString(36);
}
