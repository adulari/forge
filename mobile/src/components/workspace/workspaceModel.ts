export interface WorkspaceMention {
  start: number;
  query: string;
}

export function parentWorkspacePath(path: string): string {
  const parts = path.split("/").filter(Boolean);
  parts.pop();
  return parts.join("/");
}

export function workspaceBasename(path: string): string {
  return path.split("/").filter(Boolean).pop() ?? path;
}

/** The unfinished `@path` token at the end of the current draft. */
export function workspaceMentionAtEnd(text: string): WorkspaceMention | null {
  const match = /(^|\s)@([^\s@{}]*)$/.exec(text);
  if (!match) return null;
  return {
    start: (match.index ?? 0) + (match[1]?.length ?? 0),
    query: match[2] ?? "",
  };
}

export function workspaceMentionToken(path: string): string {
  const session = /^session:(.+)$/s.exec(path);
  if (session && /\s/.test(session[1])) return `@session:{${session[1]}}`;
  return /\s/.test(path) ? `@{${path}}` : `@${path}`;
}

export function replaceWorkspaceMention(
  text: string,
  mention: WorkspaceMention,
  path: string,
): string {
  return `${text.slice(0, mention.start)}${workspaceMentionToken(path)} `;
}

/** `name` inside `directory` ("" = the workspace root). Leading/trailing slashes in the typed name
 * are dropped; the daemon still validates the result. */
export function joinWorkspacePath(directory: string, name: string): string {
  const cleaned = name.trim().replace(/^\/+|\/+$/g, "");
  if (!cleaned) return "";
  return directory ? `${directory.replace(/\/+$/, "")}/${cleaned}` : cleaned;
}
