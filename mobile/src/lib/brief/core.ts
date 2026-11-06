// Pure half of "While you were away": the daemon's `GET /api/brief?since=<unix seconds>` digest
// (also the Anywhere bridge route `brief`), when the card is worth showing, and its wording.
// Free of react-native imports so the rules are unit-testable.

export type BriefOutcome = "finished" | "failed" | "waiting" | "interrupted" | "working";

export interface BriefSession {
  id: string;
  title: string;
  project: string;
  outcome: BriefOutcome;
  reason: string | null;
  waitingSince: number | null;
  lastActivity: number;
  summary: string;
  spendUsd: number;
  unlanded: { added: number; removed: number } | null;
}

export interface BriefTotals {
  sessions: number;
  finished: number;
  failed: number;
  waiting: number;
  interrupted: number;
  working: number;
  changesReady: number;
  spendUsd: number;
}

export interface Brief {
  since: number;
  now: number;
  totals: BriefTotals;
  sessions: BriefSession[];
}

/** Shorter gaps are an ordinary glance at the app, not a return. */
export const AWAY_THRESHOLD_SECS = 30 * 60;
/** Longest window asked of the daemon, which caps it at the same 30 days. */
export const MAX_WINDOW_SECS = 30 * 24 * 60 * 60;
/** Lines the card lists before folding the rest into "+N more". */
export const MAX_CARD_LINES = 5;

const OUTCOMES: readonly string[] = ["finished", "failed", "waiting", "interrupted", "working"];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function num(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function text(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function parseSession(raw: unknown): BriefSession | null {
  if (!isRecord(raw) || typeof raw.id !== "string" || !OUTCOMES.includes(String(raw.outcome))) return null;
  const unlanded = isRecord(raw.unlanded)
    ? { added: num(raw.unlanded.added), removed: num(raw.unlanded.removed) }
    : null;
  return {
    id: raw.id,
    title: text(raw.title) || "Untitled session",
    project: text(raw.project),
    outcome: raw.outcome as BriefOutcome,
    reason: typeof raw.reason === "string" ? raw.reason : null,
    waitingSince: typeof raw.waiting_since === "number" ? raw.waiting_since : null,
    lastActivity: num(raw.last_activity),
    summary: text(raw.summary),
    spendUsd: num(raw.spend_usd),
    unlanded,
  };
}

/** Tolerant of a newer daemon adding fields; rejects anything that is not the documented shape. */
export function parseBrief(raw: unknown): Brief {
  if (!isRecord(raw) || !Array.isArray(raw.sessions) || !isRecord(raw.totals)) {
    throw new Error("unexpected brief response");
  }
  const t = raw.totals;
  return {
    since: num(raw.since),
    now: num(raw.now),
    totals: {
      sessions: num(t.sessions),
      finished: num(t.finished),
      failed: num(t.failed),
      waiting: num(t.waiting),
      interrupted: num(t.interrupted),
      working: num(t.working),
      changesReady: num(t.changes_ready),
      spendUsd: num(t.spend_usd),
    },
    sessions: raw.sessions.flatMap((entry) => {
      const parsed = parseSession(entry);
      return parsed ? [parsed] : [];
    }),
  };
}

/** The window the card covers, or null when the person was here recently (or has no stamp yet:
 * a first launch has nothing to be away from). */
export function awaySince(lastVisit: number | null, nowSecs: number): number | null {
  if (lastVisit == null || lastVisit <= 0) return null;
  if (nowSecs - lastVisit < AWAY_THRESHOLD_SECS) return null;
  return Math.max(lastVisit, nowSecs - MAX_WINDOW_SECS);
}

export function worthShowing(brief: Brief | undefined): brief is Brief {
  return brief != null && brief.sessions.length > 0;
}

export function spanLabel(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  if (s < 90) return "moments";
  if (s < 5400) return `${Math.round(s / 60)} min`;
  if (s < 172_800) return `${Math.round(s / 3600)}h`;
  return `${Math.round(s / 86_400)}d`;
}

export function moneyLabel(usd: number): string {
  return `$${usd.toFixed(2)}`;
}

export function cardTitle(brief: Brief): string {
  return `While you were away (${spanLabel(brief.now - brief.since)})`;
}

/** "3 finished, 1 failed, 1 waiting on you, 2 with changes ready to land, $3.10 spent". */
export function headline(brief: Brief): string {
  const t = brief.totals;
  const parts: string[] = [];
  const buckets: [number, string][] = [
    [t.finished, "finished"],
    [t.failed, "failed"],
    [t.waiting, "waiting on you"],
    [t.interrupted, "interrupted"],
    [t.working, "still working"],
  ];
  for (const [n, label] of buckets) if (n > 0) parts.push(`${n} ${label}`);
  if (t.changesReady > 0) parts.push(`${t.changesReady} with changes ready to land`);
  if (t.spendUsd >= 0.005) parts.push(`${moneyLabel(t.spendUsd)} spent`);
  return parts.join(", ");
}

export function lineState(session: BriefSession, nowSecs: number): string {
  switch (session.outcome) {
    case "waiting":
      return `waiting on you for ${spanLabel(nowSecs - (session.waitingSince ?? session.lastActivity))}`;
    case "failed":
      return `failed: ${session.reason || "turn failed"}`;
    case "interrupted":
      return "interrupted by a restart";
    case "working":
      return "still working";
    default:
      return "finished";
  }
}

export function lineDetail(session: BriefSession): string {
  const bits: string[] = [];
  if (session.unlanded) bits.push(`+${session.unlanded.added}/-${session.unlanded.removed} to land`);
  if (session.spendUsd >= 0.005) bits.push(moneyLabel(session.spendUsd));
  return bits.join(" · ");
}

export function visibleLines(brief: Brief): { lines: BriefSession[]; hidden: number } {
  const lines = brief.sessions.slice(0, MAX_CARD_LINES);
  return { lines, hidden: brief.sessions.length - lines.length };
}
