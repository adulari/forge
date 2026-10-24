// Folding a freshly fetched newest history page into an already-loaded infinite query.
import type { InfiniteData } from "@tanstack/react-query";

import type { HistoryRow } from "./api";
import { rowKind } from "./toolRows";

/** `data` with its first page replaced by `fresh` (newest-first), keeping every older page.
 *
 * Rows older than the fresh page survive from the old first page, so nothing already on screen
 * disappears. When the fresh page does not reach back to the newest row already loaded, the rows
 * in between are unknown, so the loaded pages are dropped and the query restarts from `fresh`
 * rather than showing a timeline with a silent hole in it. A tool carrier's call rows share its
 * seq and a page never splits a carrier, so filtering by seq cannot drop half of one. */
export function mergeNewestHistoryPage(
  data: InfiniteData<HistoryRow[], unknown> | undefined,
  fresh: HistoryRow[],
): InfiniteData<HistoryRow[], unknown> | undefined {
  if (!data || data.pages.length === 0 || fresh.length === 0) return data;
  const oldestFresh = fresh[fresh.length - 1].seq;
  const [first, ...rest] = data.pages;
  const newestKnown = first[0]?.seq;
  if (newestKnown === undefined || newestKnown < oldestFresh) {
    return { pages: [fresh], pageParams: data.pageParams.slice(0, 1) };
  }
  return {
    pages: [[...fresh, ...first.filter((row) => row.seq < oldestFresh)], ...rest],
    pageParams: data.pageParams,
  };
}

/**
 * Whether REAL (non-blank) content has landed in `rows` (newest-first) at a seq newer than
 * `baselineSeq` — used to decide when it's safe to stop showing the chat screen's retained/
 * streamed-answer bridge (`SessionChat`'s `finalizing` state, session/[id]/index.tsx) and let the
 * settled history row take over.
 *
 * A history row landing with only whitespace content is a known daemon gap (its real text can
 * live only on tool-call rows an `include_tools`-less page never fetched) — that must NOT count
 * as "the reply arrived": clearing the bridge on ANY new row, blank or not, is what made the
 * reply disappear the instant the (blank) row landed, since the blank row itself is filtered out
 * of the rendered history transcript too (see toolRows.ts's buildTranscript).
 */
export function historyHasRealContentSince(rows: readonly HistoryRow[], baselineSeq: number): boolean {
  for (const row of rows) {
    if (row.seq <= baselineSeq) break; // newest-first: nothing at or before baseline is "since"
    if (row.content.trim() !== "") return true;
  }
  return false;
}

/** Bounded retries for the "no live-snapshot evidence yet" branch of
 * `shouldRetryEmptyHistory` — a session that genuinely has no messages must stop polling. */
export const HISTORY_RECOVERY_MAX_BLIND_ATTEMPTS = 4;

/**
 * Whether a settled-but-empty `useHistory` page (SessionChat, session/[id]/index.tsx) should be
 * retried against the daemon.
 *
 * A brand-new session's chat screen mounts and fires its first `useHistory` fetch immediately —
 * before the daemon has necessarily started (or finished) the turn, and before this device's own
 * WebSocket has necessarily reached the daemon at all. Without a live snapshot to compare against,
 * a `true` transcript.length>0 disagreement (`snapshotHasContent`) is the strong signal that
 * rows exist and just haven't been (re)fetched — worth retrying without limit, since a slow turn
 * can legitimately take a while. Absent that signal there is no evidence either way (most likely
 * because the WebSocket hasn't connected yet), so retries are capped at `maxBlindAttempts`: a
 * session that really has zero messages (created blank, never sent anything) must not poll
 * forever.
 */
export function shouldRetryEmptyHistory(
  input: {
    historySettled: boolean;
    rowsEmpty: boolean;
    snapshotHasContent: boolean;
    blindAttempts: number;
  },
  maxBlindAttempts: number = HISTORY_RECOVERY_MAX_BLIND_ATTEMPTS,
): boolean {
  if (!input.historySettled || !input.rowsEmpty) return false;
  if (input.snapshotHasContent) return true;
  return input.blindAttempts < maxBlindAttempts;
}

/**
 * Whether `text`, the answer segment the chat screen retained from the live stream, is already on
 * screen as the newest assistant row. `entries` is the rendered transcript, newest-first. The
 * daemon clears `streaming` when a segment ends (a tool call starts) and the segment lands as a
 * row while the turn keeps running, so without this check the retained copy painted the same
 * sentence twice until the turn ended. Only the newest assistant row is compared, past tool rows
 * and a follow-up the turn picked up: a retained segment that has not landed yet differs from it
 * (an identical earlier answer only hides the bridge for the frames until history catches up).
 * Either side may hold only part of the other: the live stream is a tail of a long answer, and a
 * row can leave out reasoning the stream carried.
 */
export function answerAlreadyShown(
  entries: readonly ({ kind: string; row?: HistoryRow })[],
  text: string,
): boolean {
  const wanted = text.trim();
  if (!wanted) return false;
  for (const entry of entries) {
    const row = entry.row;
    if (entry.kind !== "message" || !row || rowKind(row) !== "assistant") continue;
    const shown = row.content.trim();
    if (!shown) continue;
    return shown.includes(wanted) || wanted.includes(shown);
  }
  return false;
}
