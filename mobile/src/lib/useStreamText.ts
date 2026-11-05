import { useSyncExternalStore } from "react";

import type { StreamStore } from "./streamStore";

const NEVER = () => () => {};
const EMPTY = () => "";

/** The reply text so far; the caller re-renders once per committed frame. */
export function useStreamText(store: StreamStore | null): string {
  return useSyncExternalStore(store ? store.subscribe : NEVER, store ? store.get : EMPTY, EMPTY);
}

/** Whether any reply text has arrived; the caller re-renders only when that flips. */
export function useHasStreamText(store: StreamStore | null): boolean {
  return useSyncExternalStore(
    store ? store.subscribe : NEVER,
    store ? () => store.get().length > 0 : () => false,
    () => false,
  );
}
