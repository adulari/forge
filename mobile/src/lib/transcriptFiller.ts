// The transcript "filler": what the session screen shows while REST history is unavailable —
// the first render before the page lands, a history request that failed or returned empty after
// a daemon restart, a server switch. It used to be `Snapshot.transcript` painted as plain grey
// lines, so the whole conversation flipped to a degraded look whenever history was late — and
// stayed that way until a turn completed and invalidated the query. A v9 host also sends
// `transcript_rows` with provenance; this turns those into `HistoryRow`s so the filler renders
// through the exact same message/tool rows as history, and the two states are indistinguishable.
import type { HistoryRow } from "./api";
import type { TranscriptRow } from "./ws";

/** Synthetic `HistoryRow`s (newest first) from a snapshot's `transcript_rows`, or `null` when the
 * host sent none (pre-v9) so the caller keeps the plain-line fallback.
 *
 * `cutCurrentTurn` drops everything from the last user row onward: while a turn is in flight
 * the styled live rows (sent bubble, streaming reply, tool activity) already show that exchange,
 * and the plain-line path cut at the last "you" marker for the same reason.
 *
 * Seqs are synthetic (position-based) and never valid against the daemon: any action keyed on a
 * row's seq (delete, edit, replay-from) must be disabled on rows that came from here. */
export function fillerHistoryRows(
  rows: TranscriptRow[] | undefined,
  cutCurrentTurn: boolean,
): HistoryRow[] | null {
  if (!rows || rows.length === 0) return null;
  let end = rows.length;
  if (cutCurrentTurn) {
    for (let i = rows.length - 1; i >= 0; i--) {
      if (rows[i].kind === "user") {
        end = i;
        break;
      }
    }
  }
  const out: HistoryRow[] = [];
  for (let i = end - 1; i >= 0; i--) {
    const row = rows[i];
    out.push(toHistoryRow(row, i + 1));
  }
  return out;
}

function toHistoryRow(row: TranscriptRow, seq: number): HistoryRow {
  const base = { seq, content: row.text, model: null, created_at: 0, kind: row.kind } as const;
  switch (row.kind) {
    case "user":
      return { ...base, role: "user", visibility: "llm" };
    case "assistant":
      return { ...base, role: "assistant", visibility: "llm" };
    case "tool":
      // `meta` is "ok"/"failed" on a RESULT row and null on the CALL row that precedes it — the
      // same convention `buildTranscript` pairs history rows with.
      return {
        ...base,
        role: "tool",
        visibility: "llm",
        tool: row.tool ?? null,
        tool_phase: row.meta == null ? "call" : "result",
      };
    case "system":
    default:
      return { ...base, role: "system", visibility: "ui" };
  }
}
