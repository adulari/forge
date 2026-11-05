// A streaming reply sends a full snapshot per token, and nearly every field is the same as in the
// last one. Keeping the previous object for such a frame (and the previous reference for every
// field that did not change) lets React skip the whole session screen instead of re-rendering it
// once per token. The reply text itself travels through `streamStore`, not through the snapshot.

/** Fields that change on every streamed frame. They never make a frame "different". */
const VOLATILE_KEYS = new Set(["streaming", "streaming_offset", "revision"]);

export function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!deepEqual(a[i], b[i])) return false;
    return true;
  }
  if (Array.isArray(b)) return false;
  const ao = a as Record<string, unknown>;
  const bo = b as Record<string, unknown>;
  const keys = Object.keys(ao);
  if (keys.length !== Object.keys(bo).length) return false;
  for (const key of keys) {
    if (!(key in bo) || !deepEqual(ao[key], bo[key])) return false;
  }
  return true;
}

/** `next` with every unchanged field replaced by `prev`'s reference. Returns `prev` itself when
 * nothing but the volatile fields moved. */
export function stabilizeSnapshot<T extends object>(prev: T | null, next: T): T {
  if (prev === null) return next;
  const p = prev as Record<string, unknown>;
  const n = next as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  let changed = Object.keys(p).length !== Object.keys(n).length;
  for (const key of Object.keys(n)) {
    if (VOLATILE_KEYS.has(key)) {
      out[key] = p[key];
      continue;
    }
    if (key in p && deepEqual(p[key], n[key])) {
      out[key] = p[key];
    } else {
      out[key] = n[key];
      changed = true;
    }
  }
  return changed ? ((out as unknown) as T) : prev;
}
