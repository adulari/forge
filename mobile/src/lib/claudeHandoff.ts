// Moving one conversation between Claude Code and Forge (crates/forge-cli/src/serve_claude_link.rs):
// the wire types, the command the terminal gets, the queue that types it once the shell is
// attached, and the pure list logic behind the "Continue a Claude Code session" picker.
// Pure on purpose: nothing here imports react-native, so vitest runs it.
import type { ImportCandidate } from "./importSessions";

export interface ClaudeLinkStatus {
  linked: boolean;
  cc_session_id: string | null;
  cwd: string;
  /** Nothing in Forge is newer than the Claude Code transcript. */
  synced: boolean;
}

export interface ClaudeExportReply {
  cc_session_id: string;
  cwd: string;
  /** What resumes the conversation in Claude Code, e.g. `claude --resume <id>`. */
  command: string;
  warning?: string;
}

export interface ClaudeContinueReply {
  session_id: string;
  /** `imported` the first time, `synced` when the session was already linked. */
  mode: "imported" | "synced";
  appended: number;
  warning?: string;
}

/** The daemon terminal Claude Code runs in, so it never types into a shell already busy elsewhere. */
export const CLAUDE_TERMINAL_ID = "claude";
export const CLAUDE_TERMINAL_TITLE = "Claude Code";

const SAFE_SHELL_WORD = /^[A-Za-z0-9_@%+=:,./-]+$/;

export function shellQuote(word: string): string {
  if (SAFE_SHELL_WORD.test(word)) return word;
  return `'${word.replace(/'/g, `'\\''`)}'`;
}

/** What "Copy command" puts on the clipboard: runnable from any shell, anywhere. */
export function copyableCommand(reply: Pick<ClaudeExportReply, "cwd" | "command">): string {
  return `cd ${shellQuote(reply.cwd)} && ${reply.command}`;
}

// --- the command a terminal types once its shell is attached -------------------------------

const pending = new Map<string, string>();

const waiting = new Set<(sessionId: string, terminalId: string) => void>();

const pendingKey = (sessionId: string, terminalId: string) => `${sessionId}\u0000${terminalId}`;

/** Remember a line for the terminal to type as soon as it attaches (once). A terminal that is
 * already attached hears about it through [`onInitialCommand`] and types it right away. */
export function queueInitialCommand(sessionId: string, terminalId: string, command: string): void {
  pending.set(pendingKey(sessionId, terminalId), command);
  for (const listener of waiting) listener(sessionId, terminalId);
}

export function onInitialCommand(listener: (sessionId: string, terminalId: string) => void): () => void {
  waiting.add(listener);
  return () => {
    waiting.delete(listener);
  };
}

/** The queued line for this terminal, removed so it is typed exactly once. */
export function takeInitialCommand(sessionId: string, terminalId: string): string | null {
  const key = pendingKey(sessionId, terminalId);
  const command = pending.get(key) ?? null;
  pending.delete(key);
  return command;
}

// --- the picker ----------------------------------------------------------------------------

export interface PickerRow {
  key: string;
  id: string;
  title: string;
  cwd: string | null;
  cwdExists: boolean;
  modified: number;
  /** The Forge session already mirroring it: choosing the row syncs instead of importing. */
  linkedSessionId: string | null;
}

/** Claude Code conversations only, newest first, narrowed by what the person typed. */
export function claudePickerRows(items: readonly ImportCandidate[], query: string): PickerRow[] {
  const needle = query.trim().toLocaleLowerCase();
  return items
    .filter((c) => c.agent === "claude")
    .filter((c) => !needle || `${c.title} ${c.cwd ?? ""}`.toLocaleLowerCase().includes(needle))
    .map((c) => ({
      key: c.id,
      id: c.id,
      title: c.title,
      cwd: c.cwd,
      cwdExists: c.cwd_exists,
      modified: c.modified,
      linkedSessionId: c.linked_session_id ?? c.imported_session_id,
    }))
    .sort((a, b) => b.modified - a.modified || a.id.localeCompare(b.id));
}

export function continueToast(reply: Pick<ClaudeContinueReply, "mode" | "appended">): string {
  if (reply.mode === "imported") return "Imported from Claude Code";
  if (reply.appended === 0) return "Already up to date";
  return `Synced ${reply.appended} new ${reply.appended === 1 ? "turn" : "turns"} from Claude Code`;
}

/** The scan row for a Claude Code conversation, by its Forge session. */
export function linkedClaudeId(items: readonly ImportCandidate[], forgeSessionId: string): string | null {
  const hit = items.find(
    (c) => c.agent === "claude" && (c.linked_session_id ?? c.imported_session_id) === forgeSessionId,
  );
  return hit?.id ?? null;
}

// --- opening the picker from anywhere (palette, Home, New session) ---------------------------

let pickerListener: (() => void) | null = null;

/** The picker's host registers once, for as long as the app is mounted. */
export function onClaudePickerOpen(listener: () => void): () => void {
  pickerListener = listener;
  return () => {
    if (pickerListener === listener) pickerListener = null;
  };
}

/** True when a host was there to show the picker. */
export function openClaudePicker(): boolean {
  if (!pickerListener) return false;
  pickerListener();
  return true;
}
