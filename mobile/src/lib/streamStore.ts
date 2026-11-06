// The text of the reply being written, outside React state. One paint commits one value; only the
// components that draw the reply subscribe, so a token does not re-render the screen around it.

export interface StreamStore {
  get(): string;
  /** The newest non-empty text since the last `rememberCurrent()`. The daemon can clear
   * `streaming` a frame or two before the turn ends, and the row must keep showing the reply. */
  last(): string;
  set(text: string): void;
  /** Starts a new turn: forget text left over from the previous one. */
  rememberCurrent(): void;
  subscribe(listener: () => void): () => void;
}

export function createStreamStore(): StreamStore {
  let text = "";
  let last = "";
  const listeners = new Set<() => void>();
  return {
    get: () => text,
    last: () => last,
    set(next) {
      if (next === text) return;
      text = next;
      if (next) last = next;
      for (const listener of [...listeners]) listener();
    },
    rememberCurrent() {
      last = text;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}
