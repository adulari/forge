import { ancestorDirectories } from "./workspaceReveal";

export function toggleExpanded(expanded: ReadonlySet<string>, dir: string): Set<string> {
  const next = new Set(expanded);
  if (next.has(dir)) next.delete(dir);
  else next.add(dir);
  return next;
}

export function expandTo(expanded: ReadonlySet<string>, path: string): Set<string> {
  const next = new Set(expanded);
  for (const dir of ancestorDirectories(path)) next.add(dir);
  return next;
}

/** Folding a directory also forgets the folders opened inside it, so reopening starts tidy. */
export function collapseWithin(expanded: ReadonlySet<string>, dir: string): Set<string> {
  const prefix = `${dir}/`;
  return new Set([...expanded].filter((entry) => entry !== dir && !entry.startsWith(prefix)));
}

/** The directory a "New file" lands in: the selected folder, or the selected file's parent. */
export function creationDirectory(selected: { path: string; isDirectory: boolean } | null): string {
  if (!selected) return "";
  if (selected.isDirectory) return selected.path;
  return selected.path.split("/").slice(0, -1).join("/");
}

const remembered = new Map<string, ReadonlySet<string>>();

/** Which folders were open in a session's file browser, kept while its tab is switched away. */
export function loadExpanded(sessionId: string): ReadonlySet<string> {
  return remembered.get(sessionId) ?? new Set<string>();
}

export function saveExpanded(sessionId: string, expanded: ReadonlySet<string>): void {
  if (expanded.size === 0) remembered.delete(sessionId);
  else remembered.set(sessionId, expanded);
}
