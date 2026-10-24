// Pure half of the tray's decision rows (src-tauri/src/tray.rs): which waiting session the tray
// shows, whether it may offer Allow/Deny, and whether a click may still be applied. Split out of
// `desktopMenu.ts` so it can be tested without React Native or Tauri.
//
// Safety rules, in the order `answerTrayDecision` checks them:
//   1. A locked app never answers from the tray — the user has to unlock and look.
//   2. The click must carry the exact prompt it was built for (the shell drops rows from a
//      replaced menu), and that prompt must still be the one waiting: the live snapshot's
//      `prompt_seq` has to match. The daemon re-checks the same seq and 409s a stale answer.
//   3. The server comes from the click, not from whichever server is active now.

export interface WaitingRow {
  id: string;
  waiting: boolean;
  last_activity: number;
}

export interface DecisionSnapshot {
  session_id: string;
  prompt_seq: number;
  permission_prompt: unknown;
  question: unknown;
}

/** What the webview pushes in `TraySummary.decision`. Mirrors `tray::TrayDecision`. */
export interface TrayDecisionSummary {
  title: string;
  serverId: string;
  sessionId: string;
  /** Present only when Allow/Deny may be offered. */
  promptSeq?: number;
}

/** What a decision-row click hands back. Mirrors `tray::DecisionTarget`. */
export interface TrayDecisionTarget {
  serverId: string;
  sessionId: string;
  promptSeq?: number | null;
}

/** The newest waiting session: the one the user most likely just got a notification about. */
export function newestWaiting<T extends WaitingRow>(rows: readonly T[]): T | null {
  let newest: T | null = null;
  for (const row of rows) {
    if (!row.waiting) continue;
    if (!newest || row.last_activity > newest.last_activity) newest = row;
  }
  return newest;
}

export function buildTrayDecision(input: {
  title: string;
  serverId: string | null;
  sessionId: string;
  snapshot: DecisionSnapshot | null;
  /** The title is redacted (Anywhere-routed content): nobody should approve what they can't read. */
  contentLocked: boolean;
}): TrayDecisionSummary | undefined {
  if (!input.serverId) return undefined;
  const decision: TrayDecisionSummary = { title: input.title, serverId: input.serverId, sessionId: input.sessionId };
  const snapshot = input.snapshot;
  // Questions and plan approvals arrive as `question`; a yes/no cannot answer them.
  const isPermission = snapshot != null && snapshot.session_id === input.sessionId && snapshot.permission_prompt != null;
  if (isPermission && !input.contentLocked) decision.promptSeq = snapshot.prompt_seq;
  return decision;
}

export type TrayAnswerResult = "answered" | "locked" | "stale" | "no-server" | "failed";

export async function answerTrayDecision(
  target: TrayDecisionTarget | null | undefined,
  allow: boolean,
  deps: {
    appLocked: boolean;
    /** The waiting prompt's seq as the live snapshot sees it now; null when nothing is waiting. */
    currentPromptSeq: (sessionId: string) => number | null;
    /** A stored server's base URL by id; null once that server has been removed. */
    resolveBaseUrl: (serverId: string) => string | null;
    answer: (baseUrl: string, body: { session: string; seq: number; allow: boolean }) => Promise<unknown>;
  },
): Promise<TrayAnswerResult> {
  if (deps.appLocked) return "locked";
  if (!target || target.promptSeq == null) return "stale";
  if (deps.currentPromptSeq(target.sessionId) !== target.promptSeq) return "stale";
  const baseUrl = deps.resolveBaseUrl(target.serverId);
  if (!baseUrl) return "no-server";
  try {
    await deps.answer(baseUrl, { session: target.sessionId, seq: target.promptSeq, allow });
    return "answered";
  } catch {
    return "failed";
  }
}
