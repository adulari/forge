import type { ForkSessionRequest, SessionRow } from "./api";

export const FORK_TEMPERS = ["Read-only", "Ask", "Auto-edit", "Full"] as const;
export type ForkTemper = (typeof FORK_TEMPERS)[number];
export const FORK_EFFORTS = ["low", "medium", "high", "xhigh", "whitehot"] as const;
export type ForkEffort = (typeof FORK_EFFORTS)[number];

export interface ForkDraft {
  atSeq: number;
  model: string | null;
  effort: ForkEffort | null;
  temper: ForkTemper | null;
  prompt: string;
  title: string;
  wholeThread?: boolean;
  moveSource?: boolean;
}

export function buildForkRequest(draft: ForkDraft): ForkSessionRequest {
  const request: ForkSessionRequest = { at_seq: draft.atSeq };
  if (draft.model) request.model = draft.model;
  if (draft.effort) request.effort = draft.effort;
  if (draft.temper) request.temper = draft.temper;
  const prompt = draft.prompt.trim();
  if (prompt) request.prompt = prompt;
  const title = draft.title.trim();
  if (title) request.title = title;
  if (draft.wholeThread) request.whole_thread = true;
  if (draft.moveSource) request.move_source = true;
  return request;
}

/** Other live sessions running in the same directory as `sessionId`. A rewind rewrites files on
 * disk, so every one of them would see the files change under it. A fork child runs in its
 * source's worktree without recording it, so the comparison is on the effective directory
 * (`worktree ?? cwd`), not on the worktree field alone. */
export function sessionsSharingDirectory(sessions: readonly SessionRow[] | undefined, sessionId: string): SessionRow[] {
  const self = sessions?.find((s) => s.id === sessionId);
  const dir = self ? self.worktree ?? self.cwd : null;
  if (!sessions || !dir) return [];
  return sessions.filter((s) => s.id !== sessionId && (s.worktree ?? s.cwd) === dir);
}

export function sharedWorktreeWarning(others: readonly SessionRow[]): string | null {
  if (others.length === 0) return null;
  const names = others.slice(0, 2).map((s) => `“${s.title || s.id.slice(0, 8)}”`).join(" and ");
  const more = others.length > 2 ? ` and ${others.length - 2} more` : "";
  return `${names}${more} ${others.length === 1 ? "is" : "are"} live in this directory. Restoring files changes them there too.`;
}

/** Why "Rewind to here" is unavailable, or null when it can run. */
export function rewindDisabledReason(busy: boolean): string | null {
  return busy ? "Wait for the current turn to finish, or stop it, before rewinding." : null;
}
