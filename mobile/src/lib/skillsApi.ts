import { request } from "./api";

export type SkillScopeChoice = "user" | "project";

export interface SkillSource {
  name: string;
  source: string;
  ref: string | null;
  official: boolean;
}

export interface SkillPack {
  name: string;
  source: string;
  marketplace: string | null;
  scope: SkillScopeChoice;
  skills: string[];
}

export interface SkillsState {
  sources: SkillSource[];
  packs: SkillPack[];
  sync_targets: string[];
}

export interface AvailableSkill {
  name: string;
  description: string;
}

export interface InstallResult {
  name: string;
  installed: string[];
  skipped: string[];
  ignored_components: string[];
}

export interface SyncResult {
  linked: string[];
  conflicts: string[];
  targets: string[];
}

export interface InvokeResult {
  name: string;
  prompt: string;
  uses: number;
}

const post = <T>(baseUrl: string, path: string, body: unknown) =>
  request<T>(baseUrl, path, { method: "POST", body: JSON.stringify(body) }, 120_000);

const withSession = (session?: string | null) => (session ? `?session=${encodeURIComponent(session)}` : "");

export const getSkillsState = (baseUrl: string, session?: string | null) =>
  request<SkillsState>(baseUrl, `/api/skills/state${withSession(session)}`);

export const getAvailableSkills = (baseUrl: string, marketplace: string) =>
  request<AvailableSkill[]>(baseUrl, `/api/skills/available?marketplace=${encodeURIComponent(marketplace)}`, undefined, 120_000);

export const installSkill = (
  baseUrl: string,
  body: { source: string; scope: SkillScopeChoice; marketplace?: string; session?: string | null },
) => post<InstallResult>(baseUrl, "/api/skills/install", body);

export const removeSkillPack = (baseUrl: string, body: { name: string; scope: SkillScopeChoice; session?: string | null }) =>
  post<{ removed: string }>(baseUrl, "/api/skills/remove", body);

export const addSkillSource = (baseUrl: string, body: { name: string; source: string; ref?: string }) =>
  post<{ added: string }>(baseUrl, "/api/skills/marketplaces", body);

export const removeSkillSource = (baseUrl: string, name: string) =>
  post<{ removed: boolean }>(baseUrl, "/api/skills/marketplaces/remove", { name });

export const syncSkills = (baseUrl: string, body: { scope: SkillScopeChoice; session?: string | null }) =>
  post<SyncResult>(baseUrl, "/api/skills/sync", body);

export const invokeSkill = (baseUrl: string, body: { name: string; task?: string; session?: string | null }) =>
  post<InvokeResult>(baseUrl, "/api/skills/invoke", body);

/** Matches a marketplace package against the search box on name and description. */
export function filterAvailable(rows: AvailableSkill[], needle: string): AvailableSkill[] {
  const n = needle.trim().toLocaleLowerCase();
  return n ? rows.filter((row) => `${row.name} ${row.description}`.toLocaleLowerCase().includes(n)) : rows;
}
