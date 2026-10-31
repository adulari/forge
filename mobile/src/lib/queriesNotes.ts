// react-query hooks over apiNotes.ts. Notes need a direct daemon connection (the Anywhere bridge
// has no notes routes), so every query is disabled without one. A write on any device wakes the
// others through the fleet socket's `notes_changed` frame (FleetWatcher), with a slow poll as the
// fallback for edits made from the terminal.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  addNoteTodo,
  clearDoneNoteTodos,
  deleteNoteTodo,
  getProjectNotes,
  linkNoteTodo,
  saveScratchpad,
  updateNoteTodo,
  type NoteTodo,
  type ProjectNotes,
} from "./apiNotes";
import { useAuth } from "./auth";
import { supportsDirectDaemonEndpoints } from "./transport";

const NOTES_POLL_MS = 20_000;

export const notesKey = (baseUrl: string | null) => ["notes", baseUrl] as const;

function useDirectBase(): string | null {
  const { baseUrl } = useAuth();
  return baseUrl && supportsDirectDaemonEndpoints(baseUrl) ? baseUrl : null;
}

export function useNotesAvailable(): boolean {
  return useDirectBase() != null;
}

export function useProjectNotes(project: string | null) {
  const baseUrl = useDirectBase();
  return useQuery<ProjectNotes>({
    queryKey: [...notesKey(baseUrl), project],
    queryFn: () => getProjectNotes(baseUrl as string, project as string),
    enabled: baseUrl != null && project != null && project !== "",
    refetchInterval: NOTES_POLL_MS,
    refetchOnWindowFocus: true,
  });
}

/** Mutations that change the checklist patch the cached list first so a tick or an add lands
 * under the finger, then settle on the server's answer. */
export function useNotesMutations(project: string | null) {
  const baseUrl = useDirectBase();
  const queryClient = useQueryClient();
  const key = [...notesKey(baseUrl), project];
  const refresh = () => queryClient.invalidateQueries({ queryKey: notesKey(baseUrl) });
  const patch = (change: (notes: ProjectNotes) => ProjectNotes) =>
    queryClient.setQueryData<ProjectNotes>(key, (notes) => (notes ? change(notes) : notes));

  const add = useMutation({
    mutationFn: (text: string) => addNoteTodo(baseUrl as string, project as string, text),
    onSuccess: (todo) => patch((notes) => ({ ...notes, todos: [...notes.todos.filter((t) => t.id !== todo.id), todo] })),
    onSettled: refresh,
  });
  const update = useMutation({
    mutationFn: ({ id, ...change }: { id: string; text?: string; done?: boolean }) =>
      updateNoteTodo(baseUrl as string, id, change),
    onMutate: ({ id, ...change }) =>
      patch((notes) => ({
        ...notes,
        todos: notes.todos.map((todo): NoteTodo => (todo.id === id ? { ...todo, ...change } : todo)),
      })),
    onSettled: refresh,
  });
  const remove = useMutation({
    mutationFn: (id: string) => deleteNoteTodo(baseUrl as string, id),
    onMutate: (id) => patch((notes) => ({ ...notes, todos: notes.todos.filter((todo) => todo.id !== id) })),
    onSettled: refresh,
  });
  const link = useMutation({
    mutationFn: ({ id, sessionId }: { id: string; sessionId: string }) => linkNoteTodo(baseUrl as string, id, sessionId),
    onSettled: refresh,
  });
  const clearDone = useMutation({
    mutationFn: () => clearDoneNoteTodos(baseUrl as string, project as string),
    onSettled: refresh,
  });
  const save = useMutation({
    mutationFn: ({ body, expectedUpdatedAt }: { body: string; expectedUpdatedAt: number }) =>
      saveScratchpad(baseUrl as string, project as string, body, expectedUpdatedAt),
  });
  return { add, update, remove, link, clearDone, save, refresh };
}
