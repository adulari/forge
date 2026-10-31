// react-query hooks over apiProjects.ts. Every mutation that changes the catalog invalidates
// the shared ["projects"] query so the picker, the sidebar and new-session stay in step.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  cloneProject,
  getGhRepos,
  getHomeProject,
  getWorktrees,
  initProject,
  patchProjectEntry,
  pullWorktree,
  repairProjects,
  type ProjectEntryPatch,
} from "./apiProjects";
import { useAuth } from "./auth";

function useInvalidateProjects() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return () => queryClient.invalidateQueries({ queryKey: ["projects", baseUrl] });
}

export function useCloneProject() {
  const { baseUrl } = useAuth();
  const invalidate = useInvalidateProjects();
  return useMutation({
    mutationFn: (body: { url: string; name?: string; parent?: string }) => cloneProject(baseUrl as string, body),
    onSuccess: invalidate,
  });
}

export function useInitProject() {
  const { baseUrl } = useAuth();
  const invalidate = useInvalidateProjects();
  return useMutation({
    mutationFn: (path: string) => initProject(baseUrl as string, path),
    onSuccess: invalidate,
  });
}

export function useHomeProject() {
  const { baseUrl } = useAuth();
  return useQuery({
    queryKey: ["projects-home", baseUrl],
    queryFn: () => getHomeProject(baseUrl as string),
    enabled: baseUrl != null,
    staleTime: Infinity,
  });
}

export function useGhRepos(enabled: boolean) {
  const { baseUrl } = useAuth();
  return useQuery({
    queryKey: ["projects-gh-repos", baseUrl],
    queryFn: () => getGhRepos(baseUrl as string),
    enabled: enabled && baseUrl != null,
    staleTime: 60_000,
  });
}

export function usePatchProjectEntry() {
  const { baseUrl } = useAuth();
  const invalidate = useInvalidateProjects();
  return useMutation({
    mutationFn: (body: ProjectEntryPatch) => patchProjectEntry(baseUrl as string, body),
    onSuccess: invalidate,
  });
}

export function useRepairProjects() {
  const { baseUrl } = useAuth();
  const invalidate = useInvalidateProjects();
  return useMutation({
    mutationFn: () => repairProjects(baseUrl as string),
    onSuccess: invalidate,
  });
}

export function useWorktrees(cwd: string | null, enabled = true) {
  const { baseUrl } = useAuth();
  return useQuery({
    queryKey: ["worktrees", baseUrl, cwd],
    queryFn: () => getWorktrees(baseUrl as string, cwd as string),
    enabled: enabled && baseUrl != null && !!cwd,
    staleTime: 10_000,
  });
}

export function usePullWorktree() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (cwd: string) => pullWorktree(baseUrl as string, cwd),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["worktrees", baseUrl] }),
  });
}
