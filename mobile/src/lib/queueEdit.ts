// The follow-up queue strip's rows and the wire messages its actions send. Server rows are the
// prompts the daemon holds (Snapshot.queued); offline rows are the ones this device holds until
// the socket reconnects. Every server action echoes the row's index + text so the daemon can drop
// an edit made against a queue that has since shifted.
import type { RemoteInput } from "./ws";

export interface QueueRow {
  key: string;
  text: string;
  source: "server" | "offline";
  /** Position within its own list. */
  index: number;
}

export function queueRows(server: readonly string[], offline: readonly { text: string }[]): QueueRow[] {
  return [
    ...server.map((text, index) => ({ key: `s${index}:${text}`, text, source: "server" as const, index })),
    ...offline.map((q, index) => ({ key: `o${index}:${q.text}`, text: q.text, source: "offline" as const, index })),
  ];
}

export function moveTarget(index: number, delta: -1 | 1, length: number): number | null {
  const to = index + delta;
  return to < 0 || to >= length ? null : to;
}

export function moveInput(row: QueueRow, to: number): RemoteInput {
  return { kind: "queue_move", index: row.index, text: row.text, to };
}

export function sendNowInput(row: QueueRow): RemoteInput {
  return { kind: "queue_send_now", index: row.index, text: row.text };
}

export function removeInput(row: QueueRow): RemoteInput {
  return { kind: "dequeue", index: row.index, text: row.text };
}

export function moveInList<T>(list: readonly T[], from: number, to: number): T[] {
  const next = [...list];
  const [item] = next.splice(from, 1);
  if (item === undefined) return next;
  next.splice(to, 0, item);
  return next;
}

/** Edit pulls a prompt back into the composer; an existing draft is kept above it. */
export function mergeIntoDraft(draft: string, text: string): string {
  return draft.trim().length === 0 ? text : `${draft.replace(/\s+$/, "")}\n\n${text}`;
}
