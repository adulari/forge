// Importing Claude Code and Codex conversations (crates/forge-cli/src/serve_import.rs): the wire
// types and the pure list logic the Import screen is built from. The calls are in importQueries.ts.

export type ImportAgent = "claude" | "codex";
export type ImportAgentFilter = "all" | ImportAgent;

export interface ImportCandidate {
  agent: ImportAgent;
  id: string;
  title: string;
  cwd: string | null;
  cwd_exists: boolean;
  /** Unix seconds the conversation was last written. */
  modified: number;
  bytes: number;
  /** The Forge session an earlier import made of it. */
  imported_session_id: string | null;
}

export interface ImportScan {
  items: ImportCandidate[];
  /** Where a conversation whose folder is gone opens unless another project is picked. */
  fallback_cwd: string;
}

export interface ImportOutcome {
  agent: ImportAgent;
  id: string;
  title: string;
  session_id: string | null;
  messages: number;
  skipped: boolean;
  error: string | null;
}

export interface ImportReply {
  results: ImportOutcome[];
  imported: number;
  skipped: number;
  failed: number;
}

export const NO_FOLDER = "(no folder)";

export function itemKey(c: Pick<ImportCandidate, "agent" | "id">): string {
  return `${c.agent}:${c.id}`;
}

export function folderOf(c: ImportCandidate): string {
  return c.cwd ?? NO_FOLDER;
}

export function agentLabel(agent: ImportAgent): string {
  return agent === "claude" ? "Claude Code" : "Codex";
}

export interface ImportFilters {
  agent: ImportAgentFilter;
  /** A folder path, or null for every folder. */
  folder: string | null;
  query: string;
}

export function filterCandidates(items: readonly ImportCandidate[], f: ImportFilters): ImportCandidate[] {
  const needle = f.query.trim().toLocaleLowerCase();
  return items.filter((c) => {
    if (f.agent !== "all" && c.agent !== f.agent) return false;
    if (f.folder !== null && folderOf(c) !== f.folder) return false;
    if (!needle) return true;
    return `${c.title} ${folderOf(c)} ${agentLabel(c.agent)}`.toLocaleLowerCase().includes(needle);
  });
}

export interface FolderOption {
  folder: string;
  count: number;
}

/** Every folder the conversations came from, busiest first. */
export function folderOptions(items: readonly ImportCandidate[]): FolderOption[] {
  const counts = new Map<string, number>();
  for (const c of items) counts.set(folderOf(c), (counts.get(folderOf(c)) ?? 0) + 1);
  return [...counts.entries()]
    .map(([folder, count]) => ({ folder, count }))
    .sort((a, b) => b.count - a.count || a.folder.localeCompare(b.folder));
}

/** The rows a select-all can tick: what is already in Forge stays out. */
export function selectable(items: readonly ImportCandidate[]): ImportCandidate[] {
  return items.filter((c) => c.imported_session_id === null);
}

export function allSelected(visible: readonly ImportCandidate[], selected: ReadonlySet<string>): boolean {
  const rows = selectable(visible);
  return rows.length > 0 && rows.every((c) => selected.has(itemKey(c)));
}

/** Select every selectable visible row, or clear them all when they already are. */
export function toggleAll(visible: readonly ImportCandidate[], selected: ReadonlySet<string>): Set<string> {
  const next = new Set(selected);
  const rows = selectable(visible);
  if (allSelected(visible, selected)) rows.forEach((c) => next.delete(itemKey(c)));
  else rows.forEach((c) => next.add(itemKey(c)));
  return next;
}

/** Selected keys that still name an importable row (a rescan can drop or import them). */
export function pruneSelection(items: readonly ImportCandidate[], selected: ReadonlySet<string>): Set<string> {
  const live = new Set(selectable(items).map(itemKey));
  return new Set([...selected].filter((k) => live.has(k)));
}

export function sessionsWord(n: number): string {
  return n === 1 ? "session" : "sessions";
}

export function importButtonLabel(n: number): string {
  return n === 0 ? "Import" : `Import ${n} ${sessionsWord(n)}`;
}

export interface ImportToast {
  message: string;
  tone: "success" | "warn" | "danger";
}

export function importToast(reply: Pick<ImportReply, "imported" | "skipped" | "failed">): ImportToast {
  const parts: string[] = [];
  parts.push(
    reply.imported > 0 ? `Imported ${reply.imported} ${sessionsWord(reply.imported)}` : "Nothing new imported",
  );
  if (reply.skipped > 0) parts.push(`${reply.skipped} already in Forge`);
  if (reply.failed > 0) parts.push(`${reply.failed} failed`);
  const tone = reply.failed > 0 ? (reply.imported > 0 ? "warn" : "danger") : "success";
  return { message: parts.join(", "), tone };
}

/** `~/code/api` for a path under the host's home when it is known, else the path as given. */
export function shortFolder(cwd: string | null): string {
  if (!cwd) return NO_FOLDER;
  const m = /^\/(?:home|Users)\/[^/]+(\/.*)?$/.exec(cwd);
  return m ? `~${m[1] ?? ""}` : cwd;
}

export function conversationAge(modifiedSecs: number, nowMs: number): string {
  const s = Math.max(0, Math.floor(nowMs / 1000 - modifiedSecs));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86_400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86_400)}d ago`;
}

/** Conversations per request: well under the host's cap, and short enough to finish in time. */
export const IMPORT_BATCH = 50;

export function chunk<T>(items: readonly T[], size: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < items.length; i += size) out.push(items.slice(i, i + size));
  return out;
}

export function mergeReplies(replies: readonly ImportReply[]): ImportReply {
  return {
    results: replies.flatMap((r) => r.results),
    imported: replies.reduce((n, r) => n + r.imported, 0),
    skipped: replies.reduce((n, r) => n + r.skipped, 0),
    failed: replies.reduce((n, r) => n + r.failed, 0),
  };
}
