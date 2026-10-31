// Per-project notes over `/api/notes` (crates/forge-cli/src/serve_notes.rs): one markdown
// scratchpad and a checklist per project root. The same rows `forge notes` and the TUI `/notes`
// overlay read and write.
import { ApiError, request } from "./api";

export interface NoteTodo {
  id: string;
  text: string;
  done: boolean;
  /** The session started from this item, if any. */
  session_id: string | null;
  created_at: number;
  updated_at: number;
}

export interface ProjectNotes {
  project: string;
  body: string;
  /** Milliseconds; 0 until the scratchpad has been saved once. Echoed back as the base of the
   * next save so a second device's edit is never overwritten silently. */
  updated_at: number;
  todos: NoteTodo[];
}

export interface ScratchpadSaved {
  ok: true;
  updated_at: number;
}

/** A 409 from the scratchpad save: another device saved first. Carries what it saved. */
export interface ScratchpadConflict {
  body: string;
  updated_at: number;
}

const json = (body: unknown): RequestInit => ({ method: "POST", body: JSON.stringify(body) });

export const getProjectNotes = (baseUrl: string, project: string) =>
  request<ProjectNotes>(baseUrl, `/api/notes?project=${encodeURIComponent(project)}`);

export const saveScratchpad = (baseUrl: string, project: string, body: string, expectedUpdatedAt: number) =>
  request<ScratchpadSaved>(baseUrl, "/api/notes", {
    method: "PUT",
    body: JSON.stringify({ project, body, expected_updated_at: expectedUpdatedAt }),
  });

export const addNoteTodo = (baseUrl: string, project: string, text: string) =>
  request<NoteTodo>(baseUrl, "/api/notes/todos", json({ project, text }));

export const updateNoteTodo = (baseUrl: string, id: string, patch: { text?: string; done?: boolean }) =>
  request<NoteTodo>(baseUrl, `/api/notes/todos/${encodeURIComponent(id)}/update`, json(patch));

export const deleteNoteTodo = (baseUrl: string, id: string) =>
  request<{ ok: true }>(baseUrl, `/api/notes/todos/${encodeURIComponent(id)}/delete`, json({}));

export const linkNoteTodo = (baseUrl: string, id: string, sessionId: string) =>
  request<{ ok: true }>(baseUrl, `/api/notes/todos/${encodeURIComponent(id)}/link`, json({ session_id: sessionId }));

export const clearDoneNoteTodos = (baseUrl: string, project: string) =>
  request<{ ok: true; removed: number }>(baseUrl, "/api/notes/clear-done", json({ project }));

/** The saved scratchpad out of a 409, or null for any other error. */
export function scratchpadConflict(error: unknown): ScratchpadConflict | null {
  if (!(error instanceof ApiError) || error.status !== 409) return null;
  const body = error.body as { body?: unknown; updated_at?: unknown } | undefined;
  if (typeof body?.body !== "string" || typeof body.updated_at !== "number") return null;
  return { body: body.body, updated_at: body.updated_at };
}
