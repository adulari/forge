// Pure rules behind the Notes surface: which project the dock follows, how the checklist is
// grouped, what a "Start thread" session is seeded with, and the scratchpad's save state.
import type { NoteTodo, ScratchpadConflict } from "../../lib/apiNotes";
import type { ProjectRow, SessionRow } from "../../lib/api";
import { projectRoot } from "../../lib/sidebarModel";

/** The key the daemon stores a project's notes under: its checkout, not a worktree, no trailing slash. */
export function noteProjectKey(cwd: string): string {
  const root = projectRoot(cwd.trim()).replace(/\/+$/, "");
  return root === "" && cwd.trim().startsWith("/") ? "/" : root;
}

export function noteProjectName(project: string): string {
  const trimmed = project.replace(/\/+$/, "");
  return trimmed.split("/").pop() || trimmed || "/";
}

export interface NoteProjectChoice {
  path: string;
  name: string;
}

/** Projects the Notes surface can show: the session's own first, then recent ones, one row per checkout. */
export function noteProjects(
  sessionCwd: string | null,
  recent: readonly Pick<ProjectRow, "path">[],
  sessions: readonly Pick<SessionRow, "cwd">[] = [],
): NoteProjectChoice[] {
  const seen = new Set<string>();
  const out: NoteProjectChoice[] = [];
  const add = (cwd: string | null | undefined) => {
    if (!cwd) return;
    const path = noteProjectKey(cwd);
    if (path === "" || seen.has(path)) return;
    seen.add(path);
    out.push({ path, name: noteProjectName(path) });
  };
  add(sessionCwd);
  for (const row of recent) add(row.path);
  for (const row of sessions) add(row.cwd);
  return out;
}

export function splitTodos(todos: readonly NoteTodo[]): { open: NoteTodo[]; done: NoteTodo[] } {
  return {
    open: todos.filter((todo) => !todo.done),
    done: todos.filter((todo) => todo.done),
  };
}

const TITLE_MAX = 60;

/** What the new session is called and told: the item itself, so the thread starts where the note left off. */
export function threadSeed(todo: Pick<NoteTodo, "text">): { title: string; prompt: string } {
  const text = todo.text.trim();
  const firstLine = text.split("\n", 1)[0] ?? text;
  const title = firstLine.length > TITLE_MAX ? `${firstLine.slice(0, TITLE_MAX - 1).trimEnd()}…` : firstLine;
  return { title, prompt: text };
}

export type PadStatus = "clean" | "dirty" | "saving" | "error";

export interface PadState {
  /** What the editor shows. */
  draft: string;
  /** What the server last held, as far as this device knows. */
  savedBody: string;
  /** The `updated_at` that body came with; sent back with the next save. */
  baseUpdatedAt: number;
  status: PadStatus;
  conflict: ScratchpadConflict | null;
}

export const EMPTY_PAD: PadState = { draft: "", savedBody: "", baseUpdatedAt: 0, status: "clean", conflict: null };

export type PadAction =
  | { type: "server"; body: string; updatedAt: number }
  | { type: "edit"; text: string }
  | { type: "saving" }
  | { type: "saved"; body: string; updatedAt: number }
  | { type: "failed" }
  | { type: "conflict"; conflict: ScratchpadConflict }
  | { type: "useTheirs" }
  | { type: "keepMine" };

export function padReducer(state: PadState, action: PadAction): PadState {
  switch (action.type) {
    case "server": {
      // While our own save is in flight its result decides; a refetch racing it is just our echo.
      if (state.status === "saving") return state;
      if (action.updatedAt === state.baseUpdatedAt && action.body === state.savedBody) return state;
      if (state.draft === state.savedBody) {
        return { draft: action.body, savedBody: action.body, baseUpdatedAt: action.updatedAt, status: "clean", conflict: null };
      }
      if (action.updatedAt === state.baseUpdatedAt || action.body === state.draft) {
        return { ...state, savedBody: action.body, baseUpdatedAt: action.updatedAt };
      }
      return { ...state, conflict: { body: action.body, updated_at: action.updatedAt } };
    }
    case "edit":
      // Typing during a save keeps the status "saving"; the save's result decides what is left to send.
      return {
        ...state,
        draft: action.text,
        status: state.status === "saving" ? "saving" : action.text === state.savedBody ? "clean" : "dirty",
      };
    case "saving":
      return { ...state, status: "saving" };
    case "saved":
      return {
        ...state,
        savedBody: action.body,
        baseUpdatedAt: action.updatedAt,
        conflict: null,
        status: state.draft === action.body ? "clean" : "dirty",
      };
    case "failed":
      return { ...state, status: "error" };
    case "conflict":
      return { ...state, status: "dirty", conflict: action.conflict };
    case "useTheirs":
      return state.conflict
        ? { draft: state.conflict.body, savedBody: state.conflict.body, baseUpdatedAt: state.conflict.updated_at, status: "clean", conflict: null }
        : state;
    case "keepMine":
      return state.conflict
        ? { ...state, baseUpdatedAt: state.conflict.updated_at, savedBody: state.conflict.body, conflict: null, status: "dirty" }
        : state;
  }
}

/** The caption under the scratchpad. */
export function padCaption(state: PadState): string {
  if (state.conflict) return "Edited on another device";
  switch (state.status) {
    case "saving":
      return "Saving…";
    case "dirty":
      return "Unsaved changes";
    case "error":
      return "Could not save. Retrying on your next edit.";
    case "clean":
      return state.baseUpdatedAt > 0 ? "Saved" : "Nothing saved yet";
  }
}
