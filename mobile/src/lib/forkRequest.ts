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
  return request;
}

/** Other live sessions running in the same isolated worktree as `sessionId`. A rewind rewrites
 * files on disk, so every one of them would see the files change under it. */
export function sessionsSharingWorktree(sessions: readonly SessionRow[] | undefined, sessionId: string): SessionRow[] {
  const self = sessions?.find((s) => s.id === sessionId);
  const worktree = self?.worktree;
  if (!sessions || !worktree) return [];
  return sessions.filter((s) => s.id !== sessionId && s.worktree === worktree);
}

export function sharedWorktreeWarning(others: readonly SessionRow[]): string | null {
  if (others.length === 0) return null;
  const names = others.slice(0, 2).map((s) => `“${s.title || s.id.slice(0, 8)}”`).join(" and ");
  const more = others.length > 2 ? ` and ${others.length - 2} more` : "";
  return `${names}${more} ${others.length === 1 ? "is" : "are"} live in this worktree. Restoring files changes them there too.`;
}

/** Why "Rewind to here" is unavailable, or null when it can run. */
export function rewindDisabledReason(busy: boolean): string | null {
  return busy ? "Wait for the current turn to finish, or stop it, before rewinding." : null;
}
