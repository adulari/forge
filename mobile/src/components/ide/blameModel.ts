// Why-trace data as plain lookups. `/api/blame` lists only the lines a recorded edit wrote, so a
// line absent from the map is "not written by an agent" (or not recorded), never an error.
import type { BlameLine } from "../../lib/api";

export type BlameMap = ReadonlyMap<number, BlameLine>;

export function blameMap(lines: readonly BlameLine[] | undefined): BlameMap {
  const map = new Map<number, BlameLine>();
  for (const line of lines ?? []) map.set(line.line, line);
  return map;
}

/** Consecutive lines written by the same tool call share one marker, so the gutter reads as bars. */
export function runStarts(map: BlameMap): ReadonlySet<number> {
  const starts = new Set<number>();
  for (const [line, info] of map) {
    const above = map.get(line - 1);
    if (!above || above.tool_call_id !== info.tool_call_id || above.turn !== info.turn || above.session_id !== info.session_id) starts.add(line);
  }
  return starts;
}

/** The new-side line number of every added or context cell is what blame is keyed by. */
export function isTraceable(side: "old" | "new", kind: "context" | "add" | "del"): boolean {
  return side === "new" && kind !== "del";
}

export function whySummary(info: BlameLine): { title: string; detail: string | null } {
  const turn = `turn ${info.turn}`;
  const model = info.model ? ` · ${info.model.split("::").pop()}` : "";
  return { title: `${turn}${model}`, detail: info.prompt_summary?.trim() || null };
}

export function whenLabel(createdAt: number | null, now: number): string | null {
  if (createdAt == null) return null;
  const seconds = Math.max(0, Math.round(now - (createdAt > 1e12 ? createdAt / 1000 : createdAt)));
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  return `${Math.floor(seconds / 86400)} d ago`;
}
