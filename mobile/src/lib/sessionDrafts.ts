import type { Attachment } from "../components/chat/attach";

export interface Draft {
  text: string;
  attachments: Attachment[];
}

export type DraftMap = Record<string, Draft>;
export type AttachmentsUpdate = Attachment[] | ((prev: Attachment[]) => Attachment[]);

export const EMPTY_DRAFT: Draft = { text: "", attachments: [] };

/** Drafts live per session. An update names the session it belongs to, so a file pick or upload
 * that finishes after the user moved to another thread still lands in the thread it started in
 * and never in the one now on screen. */
export function withDraftText(drafts: DraftMap, sessionId: string, text: string): DraftMap {
  return { ...drafts, [sessionId]: { text, attachments: drafts[sessionId]?.attachments ?? [] } };
}

export function withDraftAttachments(drafts: DraftMap, sessionId: string, next: AttachmentsUpdate): DraftMap {
  const current = drafts[sessionId] ?? EMPTY_DRAFT;
  const attachments = typeof next === "function" ? next(current.attachments) : next;
  return { ...drafts, [sessionId]: { text: current.text, attachments } };
}
