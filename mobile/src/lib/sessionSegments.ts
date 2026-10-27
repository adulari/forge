// Which session segment a pathname addresses. Pure, so the session layout and its tests share it.
export type SegmentValue = "chat" | "tasks" | "agents" | "review" | "files" | "terminal" | "replay";

const NAMED_SEGMENTS: ReadonlySet<string> = new Set(["tasks", "agents", "review", "files", "terminal", "replay"]);

export function segmentFromPathname(pathname: string): SegmentValue {
  const last = pathname.split("/").filter(Boolean).pop();
  return last != null && NAMED_SEGMENTS.has(last) ? (last as SegmentValue) : "chat";
}

/** True for `/session/{id}` and `/session/{id}/{segment}`; false for routes pushed on top of the
 * session (`/session/{id}/workflow`, `/session/{id}/workflows`). */
export function isSegmentPath(pathname: string): boolean {
  const parts = pathname.split("/").filter(Boolean);
  if (parts.length <= 2) return true;
  return NAMED_SEGMENTS.has(parts[parts.length - 1]);
}
