// Project onboarding and worktree routes (serve_project_ops.rs, serve_projects.rs): clone, git
// init, the Home scope, project entry management and stash-aware worktree pull.
import { request } from "./api";

export interface ClonedProject {
  path: string;
  name: string;
}

export interface HomeProject {
  path: string;
  name: string;
}

export interface GhRepo {
  name_with_owner: string;
  url: string;
  description: string;
  private: boolean;
}

export interface GhReposResponse {
  available: boolean;
  note: string;
  repos: GhRepo[];
}

export interface ProjectEntryPatch {
  path: string;
  /** "" clears the custom name. */
  name?: string;
  disabled?: boolean;
  /** "" clears the relocation. */
  relocate_to?: string;
}

export interface RepairGroup {
  keep: string;
  duplicates: string[];
}

export interface RepairResponse {
  merged: number;
  groups: RepairGroup[];
}

export interface WorktreeRow {
  path: string;
  branch: string | null;
  head: string;
  main: boolean;
  dirty: boolean;
}

export type WorktreePullStatus = "up_to_date" | "updated" | "conflict" | "stash_conflict" | "failed";

export interface WorktreePullResult {
  status: WorktreePullStatus;
  message: string;
  conflicts: string[];
  handoff_prompt: string | null;
  stashed: boolean;
}

const CLONE_TIMEOUT_MS = 300_000;
const PULL_TIMEOUT_MS = 120_000;

export function cloneProject(
  baseUrl: string,
  body: { url: string; name?: string; parent?: string },
): Promise<ClonedProject> {
  return request(
    baseUrl,
    "/api/projects/clone",
    { method: "POST", body: JSON.stringify(body) },
    CLONE_TIMEOUT_MS,
  );
}

export function initProject(baseUrl: string, path: string): Promise<{ path: string; is_git_repo: true }> {
  return request(baseUrl, "/api/projects/init", { method: "POST", body: JSON.stringify({ path }) });
}

export function getHomeProject(baseUrl: string): Promise<HomeProject> {
  return request(baseUrl, "/api/projects/home");
}

export function getGhRepos(baseUrl: string): Promise<GhReposResponse> {
  return request(baseUrl, "/api/projects/gh-repos", undefined, 30_000);
}

export function patchProjectEntry(baseUrl: string, body: ProjectEntryPatch): Promise<{ ok: true }> {
  return request(baseUrl, "/api/projects/entry", { method: "PATCH", body: JSON.stringify(body) });
}

export function repairProjects(baseUrl: string): Promise<RepairResponse> {
  return request(baseUrl, "/api/projects/repair", { method: "POST", body: "{}" });
}

export function getWorktrees(baseUrl: string, cwd: string): Promise<{ worktrees: WorktreeRow[] }> {
  return request(baseUrl, `/api/worktrees?cwd=${encodeURIComponent(cwd)}`);
}

export function pullWorktree(baseUrl: string, cwd: string): Promise<WorktreePullResult> {
  return request(
    baseUrl,
    "/api/worktrees/pull",
    { method: "POST", body: JSON.stringify({ cwd }) },
    PULL_TIMEOUT_MS,
  );
}
