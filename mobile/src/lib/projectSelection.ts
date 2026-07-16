import type { ProjectRow } from "./api";

export function lastProjectStorageKey(serverId: string): string {
  return `forge.lastProject.${serverId}`;
}

export function projectName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  return trimmed.split(/[\\/]/).pop() || path;
}

export function isLoopbackServer(baseUrl: string | null): boolean {
  if (!baseUrl) return false;
  try {
    const host = new URL(baseUrl).hostname;
    return host === "localhost" || host === "127.0.0.1" || host === "::1";
  } catch {
    return false;
  }
}

export function projectChoices(defaultCwd: string, recent: readonly ProjectRow[]): ProjectRow[] {
  const seen = new Set<string>();
  return [
    { path: defaultCwd, name: projectName(defaultCwd), is_git_repo: true, last_activity: null },
    ...recent,
  ].filter((project) => {
    if (seen.has(project.path)) return false;
    seen.add(project.path);
    return true;
  });
}
