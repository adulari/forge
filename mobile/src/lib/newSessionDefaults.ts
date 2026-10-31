// "Forge a task" remembers how the LAST session in a project was set up (model, temper, effort,
// worktree mode) per server, so picking that project again starts from the same choices. Values
// the user already changed on the open screen are never overwritten by a late-arriving restore.
import { repoRootOf } from "./worktreePaths";

export type WorktreeMode = "off" | "new" | "existing";
export type EffortChoice = "auto" | "low" | "medium" | "high" | "max";

export const EFFORT_OPTIONS: { value: EffortChoice; label: string }[] = [
  { value: "auto", label: "Auto" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Med" },
  { value: "high", label: "High" },
  { value: "max", label: "Max" },
];

export interface NewSessionDefaults {
  model?: string;
  temper?: string;
  effort?: EffortChoice;
  /** Only "off" and "new" are remembered: an existing worktree is a path, not a preference. */
  worktree?: "off" | "new";
}

export type DefaultField = keyof NewSessionDefaults;

const EFFORTS: readonly string[] = ["auto", "low", "medium", "high", "max"];
const TEMPERS: readonly string[] = ["Read-only", "Ask", "Auto-edit", "Full"];

function fnv1a(text: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, "0");
}

/** Secure-store keys only allow `[A-Za-z0-9._-]`, so the project path is hashed. Worktree
 * checkouts share their repo's key. */
export function sessionDefaultsKey(serverId: string, projectPath: string): string {
  return `forge.sessionDefaults.${serverId}.${fnv1a(repoRootOf(projectPath.replace(/\/+$/, "")))}`;
}

export function parseSessionDefaults(raw: string | null): NewSessionDefaults {
  if (!raw) return {};
  let data: unknown;
  try {
    data = JSON.parse(raw);
  } catch {
    return {};
  }
  if (typeof data !== "object" || data === null) return {};
  const row = data as Record<string, unknown>;
  const out: NewSessionDefaults = {};
  if (typeof row.model === "string") out.model = row.model;
  if (typeof row.temper === "string" && TEMPERS.includes(row.temper)) out.temper = row.temper;
  if (typeof row.effort === "string" && EFFORTS.includes(row.effort)) out.effort = row.effort as EffortChoice;
  if (row.worktree === "off" || row.worktree === "new") out.worktree = row.worktree;
  return out;
}

export function serializeSessionDefaults(defaults: NewSessionDefaults): string {
  return JSON.stringify(defaults);
}

/** What to restore: every saved field the user has not already touched on this screen. */
export function restorableDefaults(
  saved: NewSessionDefaults,
  touched: ReadonlySet<DefaultField>,
): NewSessionDefaults {
  const out: NewSessionDefaults = {};
  for (const field of Object.keys(saved) as DefaultField[]) {
    if (!touched.has(field)) (out as Record<string, unknown>)[field] = saved[field];
  }
  return out;
}

export function effortForRequest(effort: EffortChoice): string | undefined {
  return effort === "auto" ? undefined : effort;
}

/** Rejects names git would refuse or that would escape the worktree folder. */
export function branchNameProblem(raw: string): string | null {
  const name = raw.trim();
  if (!name) return null;
  if (/\s/.test(name)) return "Branch names cannot contain spaces.";
  if (/[~^:?*[\\]/.test(name) || name.includes("..") || name.includes("@{")) return "That branch name has characters git does not allow.";
  if (name.startsWith("-") || name.startsWith("/") || name.endsWith("/") || name.endsWith(".") || name.endsWith(".lock")) {
    return "That is not a valid branch name.";
  }
  return null;
}

export interface WorktreeChoiceInput {
  mode: WorktreeMode;
  cwd: string;
  branch: string;
  existingPath: string | null;
}

/** The slice of CreateSessionRequest the worktree control decides. Launching into an existing
 * worktree is just that worktree's path as `cwd` with no new worktree. */
export function worktreeRequest(input: WorktreeChoiceInput): { cwd: string; worktree: boolean; branch?: string } {
  if (input.mode === "existing" && input.existingPath) return { cwd: input.existingPath, worktree: false };
  if (input.mode === "new") {
    const branch = input.branch.trim();
    return { cwd: input.cwd, worktree: true, ...(branch ? { branch } : {}) };
  }
  return { cwd: input.cwd, worktree: false };
}

/** Mode shown when the user has not chosen: new worktree for a git project unless it is known
 * not to be one, matching the former "Isolated git worktree" default. */
export function effectiveWorktreeMode(
  explicit: WorktreeMode | null,
  isGitRepo: boolean | null,
  recommended: boolean,
): WorktreeMode {
  if (isGitRepo === false) return "off";
  return explicit ?? (recommended ? "new" : "off");
}

const ADJECTIVES = ["calm", "brisk", "amber", "quiet", "bright", "swift", "mossy", "lucid"];
const ANIMALS = ["otter", "heron", "lynx", "finch", "badger", "wren", "marten", "ibis"];

/** A friendly example for the branch field. It is only a placeholder; the daemon picks the real name. */
export function friendlyBranchHint(seed: number): string {
  const a = ADJECTIVES[Math.abs(seed) % ADJECTIVES.length];
  const b = ANIMALS[Math.abs(Math.floor(seed / ADJECTIVES.length)) % ANIMALS.length];
  return `${a}-${b}`;
}
