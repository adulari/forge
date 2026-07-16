import type { HistoryRow } from "./api";
import type { Snapshot } from "./ws";

/** Transport-independent result of applying one snapshot revision. */
export type RevisionDecision = "accept" | "duplicate" | "replay";

/** Decide whether a snapshot is the next state, a duplicate, or requires replay from the server. */
export function decideSnapshotRevision(
  lastRevision: number,
  frame: Pick<Snapshot, "revision" | "resync">,
): RevisionDecision {
  if (frame.resync || lastRevision === 0) return "accept";
  if (frame.revision <= lastRevision) return "duplicate";
  if (frame.revision > lastRevision + 1) return "replay";
  return "accept";
}

export interface PendingMessage {
  id: string;
  text: string;
  baselineSeq: number;
}

/** Remove optimistic messages only after their own authoritative user row arrives. */
export function reconcilePendingMessages<T extends PendingMessage>(
  pending: readonly T[],
  history: readonly Pick<HistoryRow, "seq" | "role" | "content">[],
): T[] {
  return pending.filter(
    (message) =>
      !history.some(
        (row) =>
          row.seq > message.baselineSeq &&
          row.role === "user" &&
          row.content === message.text,
      ),
  );
}
