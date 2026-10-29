const MAX_RECENT = 20;

export function touchRecent(list: readonly string[], id: string): string[] {
  return [id, ...list.filter((entry) => entry !== id)].slice(0, MAX_RECENT);
}

/** `base` is the recency list frozen when the cycle began; `index` counts presses since. */
export function recentAt(base: readonly string[], known: ReadonlySet<string>, index: number): string | null {
  const live = base.filter((id) => known.has(id));
  if (live.length < 2) return null;
  return live[index % live.length] ?? null;
}

export function adjacentSession(ids: readonly string[], current: string | null, delta: 1 | -1): string | null {
  if (ids.length === 0) return null;
  const at = current ? ids.indexOf(current) : -1;
  if (at < 0) return ids[delta === 1 ? 0 : ids.length - 1] ?? null;
  return ids[(at + delta + ids.length) % ids.length] ?? null;
}
