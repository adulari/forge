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

/** The optimistic messages to paint as sent. A prompt sent while a turn runs waits in the
 * daemon's queue (`Snapshot.queued`), which the chat already shows as a queued chip; painting it
 * as a sent bubble too showed it twice, above the reply still streaming for the previous prompt.
 * It becomes a bubble once the daemon takes it off the queue. Each queued entry hides one
 * message, so a prompt sent twice with one copy picked up still shows that copy. */
export function unqueuedPendingMessages<T extends PendingMessage>(pending: readonly T[], queued: readonly string[]): T[] {
  const waiting = queued.map((text) => text.trim());
  const hidden = new Set<T>();
  // Newest first: the queue drains oldest first, so a still-queued copy is the later send.
  for (let i = pending.length - 1; i >= 0; i--) {
    const at = waiting.indexOf(pending[i].text.trim());
    if (at < 0) continue;
    waiting.splice(at, 1);
    hidden.add(pending[i]);
  }
  return pending.filter((message) => !hidden.has(message));
}
