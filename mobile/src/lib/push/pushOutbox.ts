// Unsubscribes the daemon has not yet acknowledged. Turning push off takes effect locally at once,
// but the host may be unreachable at that moment; without a record the daemon keeps a dead
// subscription until its next failed send. The outbox keeps the request and replays it later.
import type { PushUnsubscribeRequest } from "../api";

export interface PendingUnsubscribe {
  baseUrl: string;
  body: PushUnsubscribeRequest;
}

export interface OutboxStore {
  read(): Promise<string | null>;
  write(value: string): Promise<void>;
}

const MAX_PENDING = 20;

const sameRequest = (a: PendingUnsubscribe, b: PendingUnsubscribe): boolean =>
  a.baseUrl === b.baseUrl && JSON.stringify(a.body) === JSON.stringify(b.body);

export function withPendingUnsubscribe(queue: readonly PendingUnsubscribe[], item: PendingUnsubscribe): PendingUnsubscribe[] {
  if (queue.some((queued) => sameRequest(queued, item))) return [...queue];
  return [...queue, item].slice(-MAX_PENDING);
}

async function load(store: OutboxStore): Promise<PendingUnsubscribe[]> {
  try {
    const parsed: unknown = JSON.parse((await store.read()) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (item): item is PendingUnsubscribe =>
        item != null && typeof item === "object" && typeof (item as PendingUnsubscribe).baseUrl === "string" && typeof (item as PendingUnsubscribe).body === "object",
    );
  } catch {
    return [];
  }
}

export async function queueUnsubscribe(store: OutboxStore, item: PendingUnsubscribe): Promise<void> {
  await store.write(JSON.stringify(withPendingUnsubscribe(await load(store), item)));
}

/** Replays every queued request in order. A request that fails stays queued for the next open;
 * the ones that went through are dropped. */
export async function flushUnsubscribes(
  store: OutboxStore,
  send: (item: PendingUnsubscribe) => Promise<unknown>,
): Promise<number> {
  const queue = await load(store);
  if (queue.length === 0) return 0;
  const remaining: PendingUnsubscribe[] = [];
  for (const item of queue) {
    try {
      await send(item);
    } catch {
      remaining.push(item);
    }
  }
  await store.write(JSON.stringify(remaining));
  return queue.length - remaining.length;
}
