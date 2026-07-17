export const REPLAY_WINDOW_SIZE = 256;

export function acceptReplaySequences(
  stored: string | string[] | undefined,
  incoming: readonly bigint[],
  windowSize = REPLAY_WINDOW_SIZE,
): { accepted: boolean; next: string[] } {
  if (incoming.length === 0 || !Number.isSafeInteger(windowSize) || windowSize < 1) {
    return { accepted: false, next: normalize(stored) };
  }
  const ordered = [...incoming].sort(compare);
  if (ordered.some((value, index) => value < 0n || (index > 0 && value === ordered[index - 1]))) {
    return { accepted: false, next: normalize(stored) };
  }

  // A scalar is the legacy strict high-water format: every lower tuple was implicitly consumed.
  if (typeof stored === "string" && ordered.some((value) => value <= BigInt(stored))) {
    return { accepted: false, next: [stored] };
  }
  const previous = normalize(stored).map(BigInt);
  const seen = new Set(previous.map(String));
  if (ordered.some((value) => seen.has(value.toString()))) return { accepted: false, next: previous.map(String) };

  const combined = [...previous, ...ordered].sort(compare);
  const highest = combined.at(-1)!;
  const floor = highest >= BigInt(windowSize - 1) ? highest - BigInt(windowSize - 1) : 0n;
  if (ordered.some((value) => value < floor)) return { accepted: false, next: previous.map(String) };
  return { accepted: true, next: combined.filter((value) => value >= floor).map(String) };
}

function normalize(stored: string | string[] | undefined): string[] {
  if (stored === undefined) return [];
  return (Array.isArray(stored) ? stored : [stored])
    .filter((value) => /^\d+$/.test(value))
    .map(BigInt)
    .sort(compare)
    .map(String);
}

function compare(left: bigint, right: bigint): number {
  return left < right ? -1 : left > right ? 1 : 0;
}
