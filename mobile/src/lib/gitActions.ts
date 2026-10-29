// Mutations behind the git dock's remote, discard, commit-message and workspace-management
// controls (crates/forge-cli/src/serve_git/{sync,message}.rs, serve_workspace/edit.rs). Kept out
// of queries.ts/api.ts: they are one feature, and every one of them invalidates the same set of
// git queries after it runs.
import { useMutation, useQueryClient } from "@tanstack/react-query";

import { request } from "./api";
import { useAuth } from "./auth";

export interface GitSyncRequest {
  session: string;
  remote?: string;
  /** `pull` only. The daemon defaults to fast-forward-only. */
  strategy?: "ff" | "merge" | "rebase";
}
export interface GitSyncResponse {
  ok: boolean;
  remote: string | null;
  branch: string | null;
  output: string;
}
export interface GitDiscardRequest {
  session: string;
  paths: string[];
}
export interface GitDiscardResponse {
  ok: boolean;
  discarded: number;
}
export interface GitCommitMessageRequest {
  session: string;
  language?: string;
}
export interface GitCommitMessageResponse {
  message: string;
  model: string;
  language: string;
}
export interface WorkspaceEditResponse {
  ok: boolean;
  path: string;
  kind: "file" | "directory";
}

function post<T>(baseUrl: string, route: string, body: unknown): Promise<T> {
  return request(baseUrl, route, { method: "POST", body: JSON.stringify(body) });
}

/** What changed on disk or in refs: the status, every open diff, the branch list and the
 * fork-vs-base diff all derive from it. */
function invalidateGit(
  queryClient: ReturnType<typeof useQueryClient>,
  baseUrl: string | null,
  session: string,
): void {
  for (const key of [
    ["git", "status", baseUrl, session],
    ["git", "diff", baseUrl, session],
    ["git", "branches", baseUrl, session],
    ["sessions", "diff", baseUrl, session],
  ]) {
    void queryClient.invalidateQueries({ queryKey: key });
  }
}

function invalidateWorkspace(
  queryClient: ReturnType<typeof useQueryClient>,
  baseUrl: string | null,
  session: string,
): void {
  void queryClient.invalidateQueries({ queryKey: ["workspace", "entries", baseUrl, session] });
  void queryClient.invalidateQueries({ queryKey: ["workspace", "file", baseUrl, session] });
}

export function useGenerateCommitMessage() {
  const { baseUrl } = useAuth();
  return useMutation<GitCommitMessageResponse, Error, GitCommitMessageRequest>({
    mutationFn: (body) => post(baseUrl as string, "/api/git/commit-message", body),
  });
}

function useSyncMutation(route: "fetch" | "pull" | "push", touchesWorkspace: boolean) {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation<GitSyncResponse, Error, GitSyncRequest>({
    mutationFn: (body) => post(baseUrl as string, `/api/git/${route}`, body),
    // A failed pull can still have left conflicts behind, and a failed push has still changed
    // nothing the status shows — refreshing on settle covers both.
    onSettled: (_data, _error, body) => {
      invalidateGit(queryClient, baseUrl, body.session);
      if (touchesWorkspace) invalidateWorkspace(queryClient, baseUrl, body.session);
    },
  });
}

export const useFetchGit = () => useSyncMutation("fetch", false);
export const usePullGit = () => useSyncMutation("pull", true);
export const usePushGit = () => useSyncMutation("push", false);

export function useDiscardPaths() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation<GitDiscardResponse, Error, GitDiscardRequest>({
    mutationFn: (body) => post(baseUrl as string, "/api/git/discard", body),
    onSettled: (_data, _error, body) => {
      invalidateGit(queryClient, baseUrl, body.session);
      invalidateWorkspace(queryClient, baseUrl, body.session);
    },
  });
}

function useWorkspaceEdit<TBody extends { session: string }>(route: "create" | "rename" | "delete") {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation<WorkspaceEditResponse, Error, TBody>({
    mutationFn: (body) => post(baseUrl as string, `/api/workspace/${route}`, body),
    onSettled: (_data, _error, body) => {
      invalidateWorkspace(queryClient, baseUrl, body.session);
      invalidateGit(queryClient, baseUrl, body.session);
    },
  });
}

export const useCreateWorkspaceEntry = () =>
  useWorkspaceEdit<{ session: string; path: string; kind: "file" | "directory" }>("create");
export const useRenameWorkspaceEntry = () =>
  useWorkspaceEdit<{ session: string; from: string; to: string }>("rename");
export const useDeleteWorkspaceEntry = () =>
  useWorkspaceEdit<{ session: string; path: string }>("delete");
