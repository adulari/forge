import { acceptReplaySequences } from "./anywhereReplayWindow";
import type { StoredAnywhereCredentials } from "./transport/anywhereCredentialTypes";

export const OUTBOUND_SEQUENCE_BLOCK = 1024n;
export const REPLAY_PERSIST_DELAY_MS = 1500;

interface BlockIo {
  /** Persisted high-water mark (`nextSequence`), or null when signed out. */
  persistedNext(): bigint | null;
  persistNext(next: bigint): Promise<void>;
  serialize<T>(work: () => Promise<T>): Promise<T>;
}

/**
 * Hands out outbound sequences from memory after persisting `nextSequence = limit` once per
 * block. A crash only skips the unused tail of a block, so a sequence is never reused. Any other
 * writer that reads the persisted `nextSequence` gets `limit` or later, which this block never
 * hands out, and a persisted value below `limit` means the credentials were replaced.
 */
export class OutboundSequenceBlock {
  private cursor = 0n;
  private limit = 0n;

  constructor(private readonly blockSize: bigint = OUTBOUND_SEQUENCE_BLOCK) {}

  async reserve(io: BlockIo): Promise<bigint> {
    if (this.hasRoom(io.persistedNext())) return this.cursor++;
    return io.serialize(async () => {
      const persisted = io.persistedNext();
      if (persisted === null) throw new Error("Forge Anywhere is not signed in");
      if (!this.hasRoom(persisted)) {
        await io.persistNext(persisted + this.blockSize);
        this.cursor = persisted;
        this.limit = persisted + this.blockSize;
      }
      return this.cursor++;
    });
  }

  private hasRoom(persisted: bigint | null): boolean {
    return persisted !== null && this.cursor < this.limit && persisted >= this.limit;
  }
}

/**
 * Authoritative in-memory replay windows. Keeps accepted sequences monotonic even when a slow
 * credential writer (token refresh, epoch change) was derived from an older snapshot.
 */
export class LiveReplayWindows {
  private owner: string | null = null;
  private windows: Record<string, string | string[]> = {};

  adopt(next: StoredAnywhereCredentials): StoredAnywhereCredentials {
    const owner = `${next.accountIdHex}:${next.deviceIdHex}`;
    if (owner !== this.owner) {
      this.owner = owner;
      this.windows = { ...next.acceptedSequences };
      return next;
    }
    this.windows = { ...next.acceptedSequences, ...this.windows };
    return { ...next, acceptedSequences: this.windows };
  }

  accept(sender: string, epoch: number, sequences: readonly bigint[]): boolean {
    if (this.owner === null) return false;
    const namespace = `${sender}:${epoch}`;
    const result = acceptReplaySequences(this.windows[namespace], sequences);
    if (!result.accepted) return false;
    this.windows = { ...this.windows, [namespace]: result.next };
    return true;
  }

  snapshot(): Record<string, string | string[]> {
    return this.windows;
  }

  reset(): void {
    this.owner = null;
    this.windows = {};
  }
}

/** Coalesces writes: the first dirty mark arms one timer, so a busy stream still persists every `delayMs`. */
export class DebouncedPersister {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private dirtyVersion = 0;
  private savedVersion = 0;

  constructor(
    private write: () => Promise<void> = async () => undefined,
    private readonly delayMs: number = REPLAY_PERSIST_DELAY_MS,
  ) {}

  setWriter(write: () => Promise<void>): void {
    this.write = write;
  }

  get version(): number {
    return this.dirtyVersion;
  }

  markDirty(): void {
    this.dirtyVersion += 1;
    if (this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.flush().catch(() => undefined);
    }, this.delayMs);
  }

  /** A full credential write that started at `version` already carried everything up to it. */
  markSaved(version: number): void {
    if (version > this.savedVersion) this.savedVersion = version;
    if (this.savedVersion === this.dirtyVersion) this.cancelTimer();
  }

  async flush(): Promise<void> {
    this.cancelTimer();
    if (this.savedVersion === this.dirtyVersion) return;
    const version = this.dirtyVersion;
    await this.write();
    this.markSaved(version);
  }

  dispose(): void {
    this.cancelTimer();
  }

  private cancelTimer(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }
}
