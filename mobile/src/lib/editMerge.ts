// Several edits to one file inside a group read as one change: "Edit src/a.ts ×3  +12 −4", with
// the individual edits one tap away. Only finished, successful edits merge; a failed edit changed
// nothing and keeps its own row, and a running one stays visible as it is.
import { bareToolName, field, type ToolGroupCall } from "./toolGroups";

const EDIT_TOOLS = new Set(["edit_file", "multi_edit", "write_file", "append_file", "notebook_edit"]);

export type DisplayCall =
  | { kind: "call"; call: ToolGroupCall }
  | { kind: "edits"; key: string; path: string; calls: ToolGroupCall[]; adds: number | null; dels: number | null };

function normalize(path: string): string {
  let p = path;
  while (p.startsWith("./")) p = p.slice(2);
  return p;
}

export function editedPath(call: ToolGroupCall): string | null {
  const { tool, args } = call.invocation;
  if (tool == null || !EDIT_TOOLS.has(bareToolName(tool))) return null;
  const path = field(args, ["path", "file_path", "notebook_path"]);
  return path ? normalize(path) : null;
}

function mergeable(call: ToolGroupCall): boolean {
  return call.status === "ok" && call.invocation.result !== null;
}

export function mergeEdits(calls: readonly ToolGroupCall[]): DisplayCall[] {
  const byPath = new Map<string, ToolGroupCall[]>();
  for (const call of calls) {
    const path = editedPath(call);
    if (path === null || !mergeable(call)) continue;
    byPath.set(path, [...(byPath.get(path) ?? []), call]);
  }
  const out: DisplayCall[] = [];
  const placed = new Set<string>();
  for (const call of calls) {
    const path = editedPath(call);
    const same = path !== null && mergeable(call) ? byPath.get(path) : undefined;
    if (!same || same.length < 2) {
      out.push({ kind: "call", call });
      continue;
    }
    if (placed.has(path as string)) continue;
    placed.add(path as string);
    const counted = same.every((c) => c.invocation.adds != null && c.invocation.dels != null);
    out.push({
      kind: "edits",
      key: `m${same[0].key}`,
      path: path as string,
      calls: same,
      adds: counted ? same.reduce((n, c) => n + (c.invocation.adds ?? 0), 0) : null,
      dels: counted ? same.reduce((n, c) => n + (c.invocation.dels ?? 0), 0) : null,
    });
  }
  return out;
}
