// A snapshot's `streaming` is only the reply's last 2 KiB (crates/forge-tui/src/app_remote.rs
// `streaming_tail`), so past that size the start of a long reply slid out of view while it was
// still being written. A host that also sends `streaming_offset` (UTF-16 units before the tail)
// lets the app keep what it already has and append the tail, so the whole reply stays on screen.

export interface StreamingAccumulator {
  /** Offset in the whole reply where `text` starts: 0 unless the app joined mid-reply. */
  base: number;
  text: string;
}

export const EMPTY_STREAMING: StreamingAccumulator = { base: 0, text: "" };

/** Folds one frame's tail into what the app has. An older host (no offset) or a fresh reply
 * (offset 0) is the tail itself; a window the app has no prefix for starts a new base. */
export function accumulateStreaming(
  prev: StreamingAccumulator,
  tail: string,
  offset: number | undefined,
): StreamingAccumulator {
  const at = offset ?? 0;
  if (at <= 0) return tail === prev.text && prev.base === 0 ? prev : { base: 0, text: tail };
  const keep = at - prev.base;
  if (keep < 0 || keep > prev.text.length) return { base: at, text: tail };
  const text = prev.text.slice(0, keep) + tail;
  return text === prev.text ? prev : { base: prev.base, text };
}

/** The text to show: a reply joined mid-way says so instead of starting mid-sentence silently. */
export function streamingDisplayText(acc: StreamingAccumulator): string {
  return acc.base > 0 && acc.text ? `…${acc.text}` : acc.text;
}
