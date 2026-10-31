// Pure logic behind the project picker's onboarding flows: clone, git init, manage.
import type { GhRepo } from "./apiProjects";
import type { ProjectRow } from "./api";

const BLOCKED_SCHEMES = /^(ext|file|fd|data|javascript):/i;

/** Quick client-side check so an obviously wrong value never makes a round trip. The daemon
 * stays the authority (it also resolves hosts); its message wins when it rejects a URL. */
export function cloneUrlProblem(raw: string): string | null {
  const url = raw.trim();
  if (!url) return "Paste a repository URL.";
  if (BLOCKED_SCHEMES.test(url)) return "That URL scheme is not allowed. Use https:// or an SSH address.";
  if (/\s/.test(url)) return "A repository URL cannot contain spaces.";
  const looksLikeUrl = /^(https?|ssh|git):\/\//i.test(url) || /^[\w.-]+@[\w.-]+:[^\s]+$/.test(url);
  if (!looksLikeUrl) return "Use an https:// URL, or an SSH address like git@github.com:owner/repo.git.";
  return null;
}

/** Folder name a clone would get: the last URL segment without `.git`. */
export function cloneNameFromUrl(raw: string): string {
  const cleaned = raw.trim().replace(/[?#].*$/, "").replace(/\/+$/, "").replace(/\.git$/i, "");
  const last = cleaned.split(/[/:]/).pop() ?? "";
  return last.replace(/[^\w.-]/g, "-");
}

export function filterGhRepos(repos: readonly GhRepo[], query: string): GhRepo[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [...repos];
  return repos.filter((repo) => `${repo.name_with_owner} ${repo.description}`.toLowerCase().includes(needle));
}

export interface ProjectManageDraft {
  name: string;
  relocate: string;
}

/** Fields of a PATCH /api/projects/entry that actually changed; "" means "clear" server-side. */
export function manageChanges(
  project: Pick<ProjectRow, "path" | "name">,
  draft: ProjectManageDraft,
  initialRelocate = "",
): { path: string; name?: string; relocate_to?: string } | null {
  const body: { path: string; name?: string; relocate_to?: string } = { path: project.path };
  const name = draft.name.trim();
  const defaultName = project.path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";
  if (name !== project.name) body.name = name === defaultName ? "" : name;
  const relocate = draft.relocate.trim();
  if (relocate !== initialRelocate) body.relocate_to = relocate;
  return body.name === undefined && body.relocate_to === undefined ? null : body;
}

export function repairSummary(result: { merged: number; groups: { keep: string; duplicates: string[] }[] }): string {
  if (result.merged === 0) return "No duplicate projects found.";
  return `Merged ${result.merged} duplicate ${result.merged === 1 ? "entry" : "entries"} into ${result.groups.length} ${result.groups.length === 1 ? "project" : "projects"}.`;
}

/** The "Initialize git here" offer applies to a folder known NOT to be a repo; unknown stays quiet. */
export function offersGitInit(isGitRepo: boolean | null | undefined, path: string): boolean {
  return isGitRepo === false && path.trim().length > 0;
}
